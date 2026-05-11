//! banglakit-converter CLI front-end.
//!
//! Dispatches by file extension:
//!   - `.docx` → DOCX adapter, run-by-run conversion preserving format.
//!   - anything else (or `-`) → plain-text path; whole input treated as
//!     one run, classified, transliterated if appropriate.
//!
//! Exit codes (PRD FR-9):
//!   - 0: no changes were made (or would have been made under `--dry-run`).
//!   - 1: changes were made (or would be made under `--dry-run`).
//!   - 2: an error occurred.

use anyhow::{anyhow, Context, Result};
use banglakit_core::{
    classify, transliterate_with_audit, Classification, Decision, Encoding, Mode, RunAction,
    RunRef, RunVisitor, Stage,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Safe,
    Aggressive,
}

impl From<ModeArg> for Mode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Safe => Mode::Safe,
            ModeArg::Aggressive => Mode::Aggressive,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EncodingArg {
    Bijoy,
}

impl From<EncodingArg> for Encoding {
    fn from(e: EncodingArg) -> Self {
        match e {
            EncodingArg::Bijoy => Encoding::Bijoy,
        }
    }
}

/// Convert ANSI/ASCII Bengali (Bijoy and friends) to Unicode.
#[derive(Parser, Debug)]
#[command(name = "banglakit-converter", version)]
struct Cli {
    /// Input path (use `-` for stdin / treated as plain text).
    #[arg(short, long)]
    input: String,

    /// Output path (use `-` for stdout). For `.docx` input, must be a path.
    #[arg(short, long)]
    output: String,

    /// Confidence policy for fontless runs.
    #[arg(long, value_enum, default_value_t = ModeArg::Safe)]
    mode: ModeArg,

    /// Override mode's default convert-threshold (0.0..=1.0).
    #[arg(long)]
    threshold: Option<f32>,

    /// ANSI Bengali encoding family. v0.1.0 ships only `bijoy`.
    #[arg(long, value_enum, default_value_t = EncodingArg::Bijoy)]
    encoding: EncodingArg,

    /// Target Unicode Bengali font name written into converted DOCX runs.
    #[arg(long, default_value = "Kalpurush")]
    unicode_font: String,

    /// Do not write output; emit a unified-diff-style summary instead.
    #[arg(long)]
    dry_run: bool,

    /// Audit log path (JSONL, one entry per run).
    #[arg(long)]
    audit: Option<PathBuf>,

    /// Emit audit log to stdout instead of a file.
    #[arg(long, conflicts_with = "audit")]
    audit_stdout: bool,

    /// Print per-run classifier signals to stderr.
    #[arg(long)]
    explain: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    let encoding: Encoding = cli.encoding.into();
    let mode: Mode = cli.mode.into();
    let threshold = cli.threshold.unwrap_or_else(|| mode.default_threshold());

    let mut audit_sink = open_audit_sink(cli)?;

    let extension = if cli.input != "-" {
        Path::new(&cli.input)
            .extension()
            .map(|e| e.to_ascii_lowercase().to_string_lossy().into_owned())
    } else {
        None
    };

    let any_change = match extension.as_deref() {
        Some("docx") => process_docx_input(cli, encoding, threshold, &mut audit_sink)?,
        Some("pptx") => process_pptx_input(cli, encoding, threshold, &mut audit_sink)?,
        _ => process_text_input(cli, encoding, mode, threshold, &mut audit_sink)?,
    };

    if let Some(sink) = audit_sink.as_mut() {
        sink.flush()?;
    }
    Ok(if any_change { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

type AuditSink = Box<dyn Write>;

fn open_audit_sink(cli: &Cli) -> Result<Option<AuditSink>> {
    if cli.audit_stdout {
        Ok(Some(Box::new(io::stdout()) as AuditSink))
    } else if let Some(p) = &cli.audit {
        let f = fs::File::create(p)
            .with_context(|| format!("creating audit file {}", p.display()))?;
        Ok(Some(Box::new(f) as AuditSink))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Serialize)]
struct AuditEntry<'a> {
    slide_index: Option<usize>,
    paragraph_index: Option<usize>,
    run_index: Option<usize>,
    source_format: &'a str,
    font_name: Option<&'a str>,
    stage: &'a str,
    decision: &'a str,
    confidence: f32,
    original_text_b64: String,
    unicode_output: Option<String>,
}

fn write_audit(sink: &mut Option<AuditSink>, entry: &AuditEntry<'_>) -> Result<()> {
    if let Some(s) = sink.as_mut() {
        let line = serde_json::to_string(entry)?;
        writeln!(s, "{line}")?;
    }
    Ok(())
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::UnicodeRange => "unicode_range",
        Stage::AnsiFont => "ansi_font",
        Stage::UnicodeFont => "unicode_font",
        Stage::Heuristic => "heuristic",
    }
}

fn decision_name(d: Decision) -> &'static str {
    match d {
        Decision::AnsiBengali(_) => "ansi_bengali",
        Decision::UnicodeBengali => "unicode_bengali",
        Decision::Latin => "latin",
        Decision::Ambiguous => "ambiguous",
    }
}

fn maybe_explain(cli: &Cli, label: &str, c: &Classification) {
    if !cli.explain {
        return;
    }
    let signals: Vec<String> = c
        .signals
        .iter()
        .map(|s| format!("{}={:.3}", s.name, s.value))
        .collect();
    eprintln!(
        "[explain] {label} stage={:?} decision={:?} p={:.3} signals=[{}]",
        c.stage,
        c.decision,
        c.confidence,
        signals.join(", ")
    );
}

fn process_text_input(
    cli: &Cli,
    encoding: Encoding,
    mode: Mode,
    threshold: f32,
    audit_sink: &mut Option<AuditSink>,
) -> Result<bool> {
    let input_bytes = read_input(&cli.input)?;
    let input_str = std::str::from_utf8(&input_bytes)
        .context("input is not valid UTF-8")?;
    let c = classify(input_str, None, encoding, mode);
    maybe_explain(cli, &format!("text({} bytes)", input_bytes.len()), &c);

    let should_convert = match c.decision {
        Decision::AnsiBengali(_) => true,
        Decision::Ambiguous => c.confidence >= threshold,
        _ => false,
    };

    let (output, audit_unicode): (String, Option<String>) = if should_convert {
        let (out, _audit) = transliterate_with_audit(input_str, encoding);
        let preview = out.clone();
        (out, Some(preview))
    } else {
        (input_str.to_string(), None)
    };

    write_audit(
        audit_sink,
        &AuditEntry {
            slide_index: None,
            paragraph_index: None,
            run_index: None,
            source_format: "plain_text",
            font_name: None,
            stage: stage_name(c.stage),
            decision: decision_name(c.decision),
            confidence: c.confidence,
            original_text_b64: B64.encode(&input_bytes),
            unicode_output: audit_unicode,
        },
    )?;

    let changed = output != input_str;
    if cli.dry_run {
        if changed {
            eprintln!("--- {} (dry-run, would change)", cli.input);
            eprintln!("+++ {} (post-conversion)", cli.output);
        } else {
            eprintln!("--- {} (dry-run, no change)", cli.input);
        }
        return Ok(changed);
    }

    write_output(&cli.output, output.as_bytes())?;
    Ok(changed)
}

fn read_input(path: &str) -> Result<Vec<u8>> {
    if path == "-" {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        fs::read(path).with_context(|| format!("reading {path}"))
    }
}

fn write_output(path: &str, bytes: &[u8]) -> Result<()> {
    if path == "-" {
        io::stdout().write_all(bytes)?;
    } else {
        fs::write(path, bytes).with_context(|| format!("writing {path}"))?;
    }
    Ok(())
}

fn process_docx_input(
    cli: &Cli,
    encoding: Encoding,
    threshold: f32,
    audit_sink: &mut Option<AuditSink>,
) -> Result<bool> {
    if cli.dry_run {
        return Err(anyhow!("--dry-run for DOCX is not implemented in v0.1.0"));
    }
    if cli.output == "-" {
        return Err(anyhow!("DOCX output must be a file path, not stdout"));
    }
    let in_path = Path::new(&cli.input);
    let out_path = Path::new(&cli.output);

    let mut visitor = OoxmlVisitor {
        format: "docx",
        encoding,
        mode: cli.mode.into(),
        threshold,
        unicode_font: cli.unicode_font.clone(),
        explain: cli.explain,
        any_change: false,
        audit_sink,
    };
    banglakit_docx::process_docx(in_path, out_path, &mut visitor)?;
    Ok(visitor.any_change)
}

fn process_pptx_input(
    cli: &Cli,
    encoding: Encoding,
    threshold: f32,
    audit_sink: &mut Option<AuditSink>,
) -> Result<bool> {
    if cli.dry_run {
        return Err(anyhow!("--dry-run for PPTX is not implemented in v0.2.0"));
    }
    if cli.output == "-" {
        return Err(anyhow!("PPTX output must be a file path, not stdout"));
    }
    let in_path = Path::new(&cli.input);
    let out_path = Path::new(&cli.output);

    let mut visitor = OoxmlVisitor {
        format: "pptx",
        encoding,
        mode: cli.mode.into(),
        threshold,
        unicode_font: cli.unicode_font.clone(),
        explain: cli.explain,
        any_change: false,
        audit_sink,
    };
    banglakit_pptx::process_pptx(in_path, out_path, &mut visitor)?;
    Ok(visitor.any_change)
}

/// Shared visitor for OOXML formats (DOCX and PPTX). The format string is
/// stamped into the audit log and the explain output.
struct OoxmlVisitor<'a> {
    format: &'static str,
    encoding: Encoding,
    mode: Mode,
    threshold: f32,
    unicode_font: String,
    explain: bool,
    any_change: bool,
    audit_sink: &'a mut Option<AuditSink>,
}

impl<'a> RunVisitor for OoxmlVisitor<'a> {
    fn visit(&mut self, run: RunRef<'_>) -> RunAction {
        let c = classify(run.text, run.font_name, self.encoding, self.mode);
        if self.explain {
            let signals: Vec<String> = c
                .signals
                .iter()
                .map(|s| format!("{}={:.3}", s.name, s.value))
                .collect();
            let slide_part = run
                .slide_index
                .map(|s| format!("s{s}/"))
                .unwrap_or_default();
            eprintln!(
                "[explain] {fmt} {slide_part}p{}/r{} font={:?} stage={:?} decision={:?} p={:.3} signals=[{}]",
                run.paragraph_index,
                run.run_index,
                run.font_name,
                c.stage,
                c.decision,
                c.confidence,
                signals.join(", "),
                fmt = self.format,
            );
        }

        let should_convert = match c.decision {
            Decision::AnsiBengali(_) => true,
            Decision::Ambiguous => c.confidence >= self.threshold,
            _ => false,
        };

        let converted: Option<String> = if should_convert {
            let (out, _audit) = transliterate_with_audit(run.text, self.encoding);
            Some(out)
        } else {
            None
        };

        let _ = write_audit(
            self.audit_sink,
            &AuditEntry {
                slide_index: run.slide_index,
                paragraph_index: Some(run.paragraph_index),
                run_index: Some(run.run_index),
                source_format: self.format,
                font_name: run.font_name,
                stage: stage_name(c.stage),
                decision: decision_name(c.decision),
                confidence: c.confidence,
                original_text_b64: B64.encode(run.text.as_bytes()),
                unicode_output: converted.clone(),
            },
        );

        if let Some(new_text) = converted {
            if new_text != run.text {
                self.any_change = true;
            }
            RunAction::Replace {
                new_text,
                new_font: Some(self.unicode_font.clone()),
            }
        } else {
            RunAction::Keep
        }
    }
}
