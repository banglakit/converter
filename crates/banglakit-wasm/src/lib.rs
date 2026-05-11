//! WebAssembly bindings for `banglakit-core`.
//!
//! Surface shape:
//! - [`transliterate_run`] — Bijoy bytes → Unicode Bengali. Pure string in,
//!   string out; no allocation contract crosses the JS boundary beyond the
//!   string itself.
//! - [`classify_run`] — runs the five-stage classifier and returns a JS
//!   object `{ decision, stage, confidence, signals }`.
//! - [`convert_run`] — the one-call form Office Add-ins want: given a run's
//!   text and (optional) font name, decide whether to convert and return
//!   `{ text, changed, decision, stage, confidence, suggestedFont }`. The
//!   caller can blindly write `text` back into the Word/Excel run and set
//!   the font when `changed === true`.
//!
//! `convert_run` is the function the Word.run loop calls; the other two are
//! exposed so callers can build their own policies on top of the primitives.

use banglakit_core::{
    classify, convert_run, transliterate, ConvertOptions, Decision, DefaultRunVisitor, Encoding,
    Mode, Stage,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[derive(Serialize)]
struct SignalJs {
    name: &'static str,
    value: f32,
}

#[derive(Serialize)]
struct ClassificationJs {
    decision: &'static str,
    encoding: Option<&'static str>,
    stage: &'static str,
    confidence: f32,
    signals: Vec<SignalJs>,
}

#[derive(Serialize)]
struct ConversionJs {
    text: String,
    changed: bool,
    decision: &'static str,
    encoding: Option<&'static str>,
    stage: &'static str,
    confidence: f32,
    suggested_font: Option<&'static str>,
}

fn parse_mode(mode: Option<String>) -> Result<Mode, String> {
    match mode.as_deref().unwrap_or("safe") {
        "safe" => Ok(Mode::Safe),
        "aggressive" => Ok(Mode::Aggressive),
        other => Err(format!(
            "unknown mode: {other:?}; expected \"safe\" or \"aggressive\""
        )),
    }
}

fn parse_encoding(encoding: Option<String>) -> Result<Encoding, String> {
    match encoding.as_deref().unwrap_or("bijoy") {
        "bijoy" | "sutonnymj" => Ok(Encoding::Bijoy),
        other => Err(format!("unknown encoding family: {other:?}")),
    }
}

fn parse_unicode_font(name: Option<String>) -> Result<&'static str, String> {
    match name.as_deref() {
        Some("Kalpurush") | None => Ok("Kalpurush"),
        Some("Nikosh") => Ok("Nikosh"),
        Some("SolaimanLipi") => Ok("SolaimanLipi"),
        Some("Noto Sans Bengali") => Ok("Noto Sans Bengali"),
        Some(other) => Err(format!(
            "unicode_font {other:?} not in known allowlist; pass one of Kalpurush, Nikosh, SolaimanLipi, \"Noto Sans Bengali\""
        )),
    }
}

fn decision_label(d: Decision) -> (&'static str, Option<&'static str>) {
    match d {
        Decision::AnsiBengali(e) => ("ansi_bengali", Some(e.as_str())),
        Decision::UnicodeBengali => ("unicode_bengali", None),
        Decision::Latin => ("latin", None),
        Decision::Ambiguous => ("ambiguous", None),
    }
}

fn stage_label(s: Stage) -> &'static str {
    match s {
        Stage::UnicodeRange => "unicode_range",
        Stage::AnsiFont => "ansi_font",
        Stage::UnicodeFont => "unicode_font",
        Stage::Heuristic => "heuristic",
    }
}

/// Transliterate a Bijoy-encoded run to Unicode Bengali.
///
/// Does **not** consult the classifier — caller is responsible for deciding
/// whether the input is Bijoy. Use [`convert_run`] for the
/// classify-then-transliterate combo.
#[wasm_bindgen(js_name = transliterateRun)]
pub fn transliterate_run(text: &str, encoding: Option<String>) -> Result<String, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    Ok(transliterate(text, enc))
}

/// Run the five-stage classifier on `text` with an optional `fontName` hint.
///
/// Returns `{ decision, encoding, stage, confidence, signals }`. `decision`
/// is one of `"ansi_bengali"`, `"unicode_bengali"`, `"latin"`, `"ambiguous"`.
#[wasm_bindgen(js_name = classifyRun)]
pub fn classify_run(
    text: &str,
    font_name: Option<String>,
    encoding: Option<String>,
    mode: Option<String>,
) -> Result<JsValue, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    let m = parse_mode(mode).map_err(|e| JsError::new(&e))?;
    let c = classify(text, font_name.as_deref(), enc, m);
    let (decision, encoding) = decision_label(c.decision);
    let out = ClassificationJs {
        decision,
        encoding,
        stage: stage_label(c.stage),
        confidence: c.confidence,
        signals: c
            .signals
            .into_iter()
            .map(|s| SignalJs { name: s.name, value: s.value })
            .collect(),
    };
    serde_wasm_bindgen::to_value(&out).map_err(JsError::from)
}

/// Classify-then-transliterate. Returns `{ text, changed, decision, stage,
/// confidence, suggestedFont }`.
///
/// - If the classifier returns `AnsiBengali`, `text` is the converted run
///   and `changed === true`. `suggestedFont` is the Unicode Bengali font to
///   write back into the run's `font.name`.
/// - Otherwise `text` is the original input and `changed === false`.
///
/// `Ambiguous` runs are left unchanged under `safe` mode and converted under
/// `aggressive` mode — matching the CLI's behavior.
#[wasm_bindgen(js_name = convertRun)]
pub fn convert_run_js(
    text: &str,
    font_name: Option<String>,
    encoding: Option<String>,
    mode: Option<String>,
    unicode_font: Option<String>,
) -> Result<JsValue, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    let m = parse_mode(mode).map_err(|e| JsError::new(&e))?;
    let target_font = parse_unicode_font(unicode_font).map_err(|e| JsError::new(&e))?;

    // Route through the shared per-run policy in `banglakit-core`. This is
    // the same code path the CLI's DOCX/PPTX visitor takes, and the same
    // code path a future LibreOffice/UNO connector would take: classify,
    // decide, transliterate. Host-specific iteration and commit live above
    // this call; the decision itself is identical across hosts.
    let opts = ConvertOptions {
        encoding: enc,
        mode: m,
        threshold: None,
        unicode_font: target_font,
    };
    let r = convert_run(text, font_name.as_deref(), &opts);
    let (decision, decoded_encoding) = decision_label(r.classification.decision);
    // r.font borrows from `opts.unicode_font` (i.e. from `target_font`); use
    // the original `&'static str` directly to satisfy ConversionJs's lifetime.
    let suggested_font: Option<&'static str> = if r.changed { Some(target_font) } else { None };

    let out = ConversionJs {
        text: r.text,
        changed: r.changed,
        decision,
        encoding: decoded_encoding,
        stage: stage_label(r.classification.stage),
        confidence: r.classification.confidence,
        suggested_font,
    };
    serde_wasm_bindgen::to_value(&out).map_err(JsError::from)
}

/// Version of the bundled core, useful for cache-busting on the JS side.
#[wasm_bindgen(js_name = coreVersion)]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Serialize)]
struct FileConversionJs {
    bytes: Vec<u8>,
    any_change: bool,
    runs_converted: usize,
}

#[derive(Serialize)]
struct TextConversionJs {
    text: String,
    changed: bool,
    runs_converted: usize,
}

fn build_opts(
    encoding: Option<String>,
    mode: Option<String>,
    unicode_font: Option<String>,
) -> Result<ConvertOptions<'static>, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    let m = parse_mode(mode).map_err(|e| JsError::new(&e))?;
    let target_font = parse_unicode_font(unicode_font).map_err(|e| JsError::new(&e))?;
    Ok(ConvertOptions {
        encoding: enc,
        mode: m,
        threshold: None,
        unicode_font: target_font,
    })
}

/// Convert an in-memory `.docx` file. Takes the raw zip bytes and returns
/// `{ bytes, anyChange, runsConverted }`. The returned `bytes` are a fresh
/// DOCX the caller can wrap in a `Blob` and offer as a download.
#[wasm_bindgen(js_name = convertDocx)]
pub fn convert_docx(
    bytes: &[u8],
    mode: Option<String>,
    encoding: Option<String>,
    unicode_font: Option<String>,
) -> Result<JsValue, JsError> {
    let opts = build_opts(encoding, mode, unicode_font)?;
    let mut visitor = DefaultRunVisitor::new(opts);
    let out = banglakit_docx::process_docx_bytes(bytes, &mut visitor)
        .map_err(|e| JsError::new(&format!("{e:#}")))?;
    let result = FileConversionJs {
        bytes: out,
        any_change: visitor.any_change,
        runs_converted: visitor.runs_converted,
    };
    serde_wasm_bindgen::to_value(&result).map_err(JsError::from)
}

/// Convert an in-memory `.pptx` deck. Same return shape as [`convert_docx`].
#[wasm_bindgen(js_name = convertPptx)]
pub fn convert_pptx(
    bytes: &[u8],
    mode: Option<String>,
    encoding: Option<String>,
    unicode_font: Option<String>,
) -> Result<JsValue, JsError> {
    let opts = build_opts(encoding, mode, unicode_font)?;
    let mut visitor = DefaultRunVisitor::new(opts);
    let out = banglakit_pptx::process_pptx_bytes(bytes, &mut visitor)
        .map_err(|e| JsError::new(&format!("{e:#}")))?;
    let result = FileConversionJs {
        bytes: out,
        any_change: visitor.any_change,
        runs_converted: visitor.runs_converted,
    };
    serde_wasm_bindgen::to_value(&result).map_err(JsError::from)
}

/// Convert a plain-text blob — treats the whole input as one run, runs the
/// classifier, and returns `{ text, changed, runsConverted }` (with
/// `runsConverted` always 0 or 1, mirroring the DOCX/PPTX surface).
#[wasm_bindgen(js_name = convertText)]
pub fn convert_text(
    text: &str,
    mode: Option<String>,
    encoding: Option<String>,
    unicode_font: Option<String>,
) -> Result<JsValue, JsError> {
    let opts = build_opts(encoding, mode, unicode_font)?;
    let r = convert_run(text, None, &opts);
    let result = TextConversionJs {
        text: r.text,
        changed: r.changed,
        runs_converted: if r.changed { 1 } else { 0 },
    };
    serde_wasm_bindgen::to_value(&result).map_err(JsError::from)
}

#[cfg(test)]
mod tests {
    //! These tests run on the native target so they exercise the wiring
    //! between the wasm crate and `banglakit-core`. They do not exercise
    //! `wasm-bindgen`'s JS marshalling.
    use super::*;

    #[test]
    fn defaults_match_cli() {
        assert!(matches!(parse_mode(None).unwrap(), Mode::Safe));
        assert!(matches!(parse_encoding(None).unwrap(), Encoding::Bijoy));
    }

    #[test]
    fn parse_mode_rejects_garbage() {
        assert!(parse_mode(Some("yolo".into())).is_err());
    }

    #[test]
    fn parse_encoding_rejects_garbage() {
        assert!(parse_encoding(Some("klingon".into())).is_err());
    }

    #[test]
    fn parse_unicode_font_known() {
        assert_eq!(parse_unicode_font(None).unwrap(), "Kalpurush");
        assert_eq!(
            parse_unicode_font(Some("SolaimanLipi".into())).unwrap(),
            "SolaimanLipi"
        );
        assert!(parse_unicode_font(Some("Comic Sans".into())).is_err());
    }
}
