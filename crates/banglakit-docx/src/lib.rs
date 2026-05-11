//! DOCX read/write adapter for banglakit-converter.
//!
//! The public surface is [`process_docx`], which streams every `<w:r>` (run)
//! past a caller-supplied [`RunVisitor`] and writes the result to a new
//! DOCX. All zip entries other than `word/document.xml` are copied
//! byte-for-byte, satisfying the SDD §11 format-fidelity requirement.
//!
//! Font resolution for a run, in order:
//!   run rPr → paragraph rPr defaults → paragraph style + basedOn chain
//!   → default paragraph style → docDefaults rPrDefault.
//!
//! Each `<w:rFonts>` element resolves through [`theme::font_from_rfonts`],
//! which honours `w:asciiTheme` / `w:hAnsiTheme` references into
//! `word/theme/theme1.xml`. Modern Word output writes its default font
//! that way; without theme resolution our cascade would silently return
//! `None` for those runs.

pub mod styles;
pub mod theme;

use anyhow::{anyhow, Context, Result};
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::borrow::Cow;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::styles::Stylesheet;
use crate::theme::{font_from_rfonts, Theme};
pub use banglakit_core::{RunAction, RunRef, RunVisitor};

const DOCUMENT_XML: &str = "word/document.xml";
const STYLES_XML: &str = "word/styles.xml";
const THEME_XML: &str = "word/theme/theme1.xml";

/// Read `in_path`, stream each run past `visitor`, and write the result to
/// `out_path`. Non-`word/document.xml` zip entries are copied verbatim.
pub fn process_docx<V: RunVisitor>(in_path: &Path, out_path: &Path, visitor: &mut V) -> Result<()> {
    let bytes = std::fs::read(in_path).with_context(|| format!("opening {}", in_path.display()))?;
    let out = process_docx_bytes(&bytes, visitor)?;
    std::fs::write(out_path, out).with_context(|| format!("creating {}", out_path.display()))?;
    Ok(())
}

/// In-memory variant of [`process_docx`]. Takes the raw DOCX zip bytes and
/// returns the converted DOCX bytes. Used by `banglakit-wasm` so the browser
/// can round-trip a `.docx` file entirely client-side.
pub fn process_docx_bytes<V: RunVisitor>(input: &[u8], visitor: &mut V) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(input))?;

    let mut document_xml = String::new();
    {
        let mut entry = archive
            .by_name(DOCUMENT_XML)
            .with_context(|| format!("{DOCUMENT_XML} not in archive"))?;
        entry.read_to_string(&mut document_xml)?;
    }

    // theme1.xml and styles.xml are both optional in OOXML. Treat
    // absences as "empty" and let cascade resolution fall through.
    let theme = match archive.by_name(THEME_XML) {
        Ok(mut entry) => {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            theme::parse_theme(&s)?
        }
        Err(_) => Theme::default(),
    };
    let stylesheet = match archive.by_name(STYLES_XML) {
        Ok(mut entry) => {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            styles::parse_styles(&s, Some(&theme))?
        }
        Err(_) => Stylesheet::default(),
    };

    let new_document_xml =
        transform_document_xml(&document_xml, &stylesheet, Some(&theme), visitor)?;

    let mut zip_out = ZipWriter::new(Cursor::new(Vec::<u8>::new()));

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        let options = SimpleFileOptions::default().compression_method(match entry.compression() {
            CompressionMethod::Stored => CompressionMethod::Stored,
            _ => CompressionMethod::Deflated,
        });
        zip_out.start_file(&name, options)?;
        if name == DOCUMENT_XML {
            zip_out.write_all(new_document_xml.as_bytes())?;
        } else {
            std::io::copy(&mut entry, &mut zip_out)?;
        }
    }
    let cursor = zip_out.finish()?;
    Ok(cursor.into_inner())
}

#[derive(Default, Clone)]
struct ParagraphContext {
    style_id: Option<String>,
    run_default_font: Option<String>,
}

/// Walker for `word/document.xml`.
///
/// Three concerns kept separate:
/// 1. **Pass-through writing.** The vast majority of events go straight to
///    the output without ownership transfer.
/// 2. **`<w:pPr>` capture-and-parse.** Once we see `<w:pPr>`, we buffer
///    its events into a small Vec (bounded by paragraph-property
///    complexity, ~10–20 events), then parse the buffer to a typed
///    `ParagraphContext` at `</w:pPr>` and emit the captured events
///    verbatim. The data extraction is then a tiny pure function rather
///    than tangled flags interleaved with the main walker.
/// 3. **`<w:r>` capture-and-rewrite.** Same shape: buffer until `</w:r>`,
///    hand the contents (plus the paragraph context and stylesheet) to
///    the visitor, emit possibly-modified events back to the writer.
fn transform_document_xml<V: RunVisitor>(
    xml: &str,
    stylesheet: &Stylesheet,
    theme: Option<&Theme>,
    visitor: &mut V,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::<u8>::new()));

    let mut run_buffer: Option<RunBuffer> = None;
    let mut run_depth: usize = 0; // nesting depth of <w:r> inside a run buffer
    let mut ppr_buffer: Option<Vec<Event<'static>>> = None;
    let mut paragraph_index: usize = 0;
    let mut in_paragraph = false;
    let mut paragraph_ctx = ParagraphContext::default();
    // Collect all runs in a paragraph so we can merge orphaned combining-mark
    // runs before conversion.
    let mut paragraph_runs: Vec<RunBuffer> = Vec::new();

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => {
                let local = local_name_bytes_of(&event);

                // Inside a run buffer, everything is captured until the matching </w:r>.
                // Track nesting depth to handle embedded <w:r> inside <mc:AlternateContent>.
                if let Some(rb) = run_buffer.as_mut() {
                    if matches!(event, Event::Start(_)) && local == Some(b"r") {
                        run_depth += 1;
                        rb.push(event.into_owned());
                    } else if matches!(event, Event::End(_)) && local == Some(b"r") {
                        if run_depth == 0 {
                            // Matching close for the outer <w:r>.
                            rb.end_event = Some(event.into_owned());
                            paragraph_runs.push(run_buffer.take().unwrap());
                        } else {
                            run_depth -= 1;
                            rb.push(event.into_owned());
                        }
                    } else {
                        rb.push(event.into_owned());
                    }
                    buf.clear();
                    continue;
                }

                // Inside a pPr buffer, everything is captured until </w:pPr>.
                if let Some(pb) = ppr_buffer.as_mut() {
                    if matches!(event, Event::End(_)) && local == Some(b"pPr") {
                        pb.push(event.into_owned());
                        let captured = ppr_buffer.take().unwrap();
                        paragraph_ctx = parse_ppr(&captured, theme);
                        // Emit captured events verbatim.
                        for e in &captured {
                            write_event(&mut writer, e)?;
                        }
                    } else {
                        pb.push(event.into_owned());
                    }
                    buf.clear();
                    continue;
                }

                // Outside any capture: track structural boundaries, pass through.
                match (&event, local) {
                    (Event::Start(_), Some(b"p")) => {
                        in_paragraph = true;
                        paragraph_runs.clear();
                        paragraph_ctx = ParagraphContext::default();
                        write_event(&mut writer, &event)?;
                    }
                    (Event::End(_), Some(b"p")) => {
                        // End of paragraph: three-phase pipeline.
                        // Phase 1: merge orphaned Bijoy combining-mark runs
                        //   before conversion (pre-base ‡†ˆ‰w → next, post-base ©Š → prev).
                        merge_orphaned_bijoy_runs(
                            &mut paragraph_runs,
                            stylesheet,
                            &paragraph_ctx,
                            theme,
                        );

                        // Phase 2: convert runs through the visitor.
                        //   Adjacent same-font Bijoy runs are grouped and
                        //   converted as one unit so normalize passes see
                        //   the full word across run boundaries.
                        let mut converted = convert_paragraph_runs(
                            &paragraph_runs,
                            paragraph_index,
                            &paragraph_ctx,
                            stylesheet,
                            theme,
                            visitor,
                        );

                        // Phase 3 (post-conversion merge) is no longer needed —
                        // Phase 2's grouped conversion handles all cross-run
                        // issues by converting adjacent Bijoy runs as one unit.

                        // Emit.
                        for (rb, cr) in paragraph_runs.drain(..).zip(converted.drain(..)) {
                            write_converted_run(&mut writer, rb, cr)?;
                        }
                        in_paragraph = false;
                        paragraph_index += 1;
                        write_event(&mut writer, &event)?;
                    }
                    (Event::Start(_), Some(b"pPr")) => {
                        // Begin capturing pPr.
                        let mut v = Vec::with_capacity(16);
                        v.push(event.into_owned());
                        ppr_buffer = Some(v);
                    }
                    (Event::Start(_), Some(b"r")) if in_paragraph => {
                        run_buffer = Some(RunBuffer::new(event.into_owned()));
                        run_depth = 0;
                    }
                    _ => {
                        write_event(&mut writer, &event)?;
                    }
                }
            }
            Err(e) => return Err(anyhow!("XML parse error: {e}")),
        }
        buf.clear();
    }

    let bytes = writer.into_inner().into_inner();
    Ok(String::from_utf8(bytes).context("invalid UTF-8 in output XML")?)
}

/// Parse a captured `<w:pPr>…</w:pPr>` event sequence into a typed
/// [`ParagraphContext`]. Reads `<w:pStyle w:val="…"/>` for the style id
/// and `<w:rFonts>` inside the inner `<w:rPr>` for the paragraph-level
/// run default font.
fn parse_ppr(events: &[Event<'static>], theme: Option<&Theme>) -> ParagraphContext {
    let mut ctx = ParagraphContext::default();
    // Depth into nested rPr elements (only one in practice, but be defensive).
    let mut in_rpr = 0usize;
    for ev in events {
        let local = local_name_bytes_of(ev);
        match (ev, local) {
            (Event::Empty(b), Some(b"pStyle")) | (Event::Start(b), Some(b"pStyle")) => {
                if let Some(val) = styles::attr(b, b"val") {
                    ctx.style_id = Some(val);
                }
            }
            (Event::Start(_), Some(b"rPr")) => in_rpr += 1,
            (Event::End(_), Some(b"rPr")) => in_rpr = in_rpr.saturating_sub(1),
            (Event::Empty(b), Some(b"rFonts")) | (Event::Start(b), Some(b"rFonts"))
                if in_rpr > 0 =>
            {
                if ctx.run_default_font.is_none() {
                    ctx.run_default_font = font_from_rfonts(b, theme);
                }
            }
            _ => {}
        }
    }
    ctx
}

fn write_event(writer: &mut Writer<Cursor<Vec<u8>>>, event: &Event<'_>) -> Result<()> {
    writer
        .write_event(event.clone())
        .map_err(|e| anyhow!("XML write error: {e}"))
}

struct RunBuffer {
    start_event: Event<'static>,
    end_event: Option<Event<'static>>,
    events: Vec<Event<'static>>,
}

impl RunBuffer {
    fn new(start_event: Event<'static>) -> Self {
        Self {
            start_event,
            end_event: None,
            events: Vec::new(),
        }
    }

    fn push(&mut self, e: Event<'static>) {
        self.events.push(e);
    }
}

/// Bijoy characters that unambiguously map to pre-base combining marks
/// (they precede the consonant in Bijoy byte order and should be merged
/// into the *next* run):
///   `†` → ে, `‡` → ে, `ˆ` → ৈ, `‰` → ৈ, `w` → ি
///
/// And post-base characters (they follow the consonant cluster and should
/// be merged into the *previous* run):
///   `©` → র্ (reph), `Š` → ৗ (au-length mark)
///
/// `w` is also a normal ASCII letter, so we only treat it as a Bijoy
/// pre-base marker when the run is in a recognized Bijoy font.

/// Returns `true` if a Bijoy run's text consists entirely of characters that
/// produce orphaned combining marks — pre-base ones that belong on the next
/// run. Whitespace-only tails are not merged.
fn is_bijoy_prebase_only(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    text.chars().all(|c| matches!(c, '†' | '‡' | 'ˆ' | '‰'))
}

/// Like `is_bijoy_prebase_only` but includes `w` (ি), which is only safe
/// to treat as a pre-base marker when the run is known to be Bijoy-encoded
/// (i.e. has a Bijoy font).
fn is_bijoy_prebase_only_with_font(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    text.chars()
        .all(|c| matches!(c, '†' | '‡' | 'ˆ' | '‰' | 'w'))
}

/// Returns `true` if a Bijoy run's text consists entirely of post-base
/// combining-mark characters: `©` (reph) or `Š` (au-length mark).
fn is_bijoy_postbase_only(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    text.chars().all(|c| matches!(c, '©' | 'Š'))
}

/// Replace the `<w:t>` text content inside a `RunBuffer`'s events.
fn set_run_text(events: &mut Vec<Event<'static>>, new_text: &str) {
    let mut in_text = false;
    let mut replaced = false;
    let mut new_events: Vec<Event<'static>> = Vec::with_capacity(events.len());
    for ev in events.drain(..) {
        let local = local_name_bytes_of(&ev);
        match (&ev, local) {
            (Event::Start(_), Some(b"t")) => {
                in_text = true;
                // Ensure xml:space="preserve" so leading/trailing spaces survive.
                let mut tag = BytesStart::new("w:t");
                tag.push_attribute(("xml:space", "preserve"));
                new_events.push(Event::Start(tag));
            }
            (Event::End(_), Some(b"t")) => {
                if in_text && !replaced {
                    new_events.push(Event::Text(
                        quick_xml::events::BytesText::new(new_text).into_owned(),
                    ));
                    replaced = true;
                }
                in_text = false;
                new_events.push(ev);
            }
            (Event::Text(_), _) if in_text => {
                // Skip original text; we emit replacement at </w:t>.
            }
            _ => {
                new_events.push(ev);
            }
        }
    }
    *events = new_events;
}

/// Extract the raw Bijoy text from a `RunBuffer`'s events.
fn extract_text_from_events(events: &[Event<'static>]) -> String {
    let mut text = String::new();
    let mut in_text = false;
    for ev in events {
        match ev {
            Event::Start(b) if local_name_bytes(b.name().into_inner()) == b"t" => {
                in_text = true;
            }
            Event::End(b) if local_name_bytes(b.name().into_inner()) == b"t" => {
                in_text = false;
            }
            Event::Text(t) if in_text => {
                if let Ok(s) = t.unescape() {
                    text.push_str(&s);
                }
            }
            _ => {}
        }
    }
    text
}

/// Resolve the font for a run buffer, using the same cascade as `flush_run`.
fn resolve_run_font_from_buffer(
    events: &[Event<'static>],
    stylesheet: &Stylesheet,
    paragraph: &ParagraphContext,
    theme: Option<&Theme>,
) -> Option<String> {
    let (run_font, _) = extract_font_and_text(events, theme);
    stylesheet
        .resolve_run_font(
            run_font.as_deref(),
            paragraph.run_default_font.as_deref(),
            paragraph.style_id.as_deref(),
        )
        .map(|s| s.to_string())
}

/// Returns `true` if the font name belongs to a known Bijoy/ANSI Bengali family.
fn is_bijoy_font(font: Option<&str>) -> bool {
    use banglakit_core::encoding::Encoding;
    match font {
        Some(f) => banglakit_core::fonts::is_ansi_font(f, Encoding::Bijoy),
        None => false,
    }
}

/// Merge orphaned Bijoy combining-mark runs into their neighbours.
///
/// Pre-base orphans (`†` `‡` `ˆ` `‰`, and `w` when in Bijoy font) are
/// prepended to the **next** run's Bijoy text. Post-base orphans (`©` `Š`)
/// are appended to the **previous** run's text. The orphan run's XML events
/// are removed (the merged run keeps its own formatting, which carries the
/// consonant and will produce the correct glyph after transliteration).
///
/// Only merges when both runs share a Bijoy font so we never accidentally
/// merge Latin text into a non-Bijoy run.
fn merge_orphaned_bijoy_runs(
    runs: &mut Vec<RunBuffer>,
    stylesheet: &Stylesheet,
    paragraph: &ParagraphContext,
    theme: Option<&Theme>,
) {
    if runs.len() < 2 {
        return;
    }

    // Pre-compute font and text for each run.
    let fonts: Vec<Option<String>> = runs
        .iter()
        .map(|rb| resolve_run_font_from_buffer(&rb.events, stylesheet, paragraph, theme))
        .collect();
    let mut texts: Vec<String> = runs
        .iter()
        .map(|rb| extract_text_from_events(&rb.events))
        .collect();

    // Indices of runs to remove after merging (orphans whose text was merged elsewhere).
    let mut remove: Vec<bool> = vec![false; runs.len()];

    // Forward pass: merge pre-base orphans into the next run.
    for i in 0..runs.len() - 1 {
        if remove[i] {
            continue;
        }
        let font_i = fonts[i].as_deref();
        if !is_bijoy_font(font_i) {
            continue;
        }
        let is_orphan = is_bijoy_prebase_only(&texts[i])
            || (is_bijoy_prebase_only_with_font(&texts[i]) && is_bijoy_font(font_i));
        if !is_orphan {
            continue;
        }
        // Find next non-removed run.
        let mut j = i + 1;
        while j < runs.len() && remove[j] {
            j += 1;
        }
        if j >= runs.len() {
            continue;
        }
        // Only merge if the target run is also Bijoy.
        if !is_bijoy_font(fonts[j].as_deref()) {
            continue;
        }
        // Prepend orphan text to next run.
        let orphan_text = texts[i].clone();
        texts[j] = format!("{}{}", orphan_text, texts[j]);
        set_run_text(&mut runs[j].events, &texts[j]);
        remove[i] = true;
    }

    // Backward pass: merge post-base orphans into the previous run.
    for i in (1..runs.len()).rev() {
        if remove[i] {
            continue;
        }
        let font_i = fonts[i].as_deref();
        if !is_bijoy_font(font_i) {
            continue;
        }
        if !is_bijoy_postbase_only(&texts[i]) {
            continue;
        }
        // Find previous non-removed run.
        let mut j = i - 1;
        loop {
            if !remove[j] {
                break;
            }
            if j == 0 {
                break;
            }
            j -= 1;
        }
        if remove[j] {
            continue;
        }
        if !is_bijoy_font(fonts[j].as_deref()) {
            continue;
        }
        // Append orphan text to previous run.
        let orphan_text = texts[i].clone();
        texts[j] = format!("{}{}", texts[j], orphan_text);
        set_run_text(&mut runs[j].events, &texts[j]);
        remove[i] = true;
    }

    // Remove orphan runs (iterate in reverse to preserve indices).
    for i in (0..runs.len()).rev() {
        if remove[i] {
            runs.remove(i);
        }
    }
}

/// The result of converting a single run through the visitor, before writing.
struct ConvertedRunResult {
    /// Replacement text, if any.
    new_text: Option<String>,
    /// Replacement font, if any.
    new_font: Option<String>,
    /// Whether this run should be suppressed (its content was merged elsewhere).
    suppress: bool,
}

/// Convert all runs in a paragraph, grouping adjacent same-font Bijoy runs
/// and converting them as a single unit so normalize passes see full words.
fn convert_paragraph_runs<V: RunVisitor>(
    runs: &[RunBuffer],
    paragraph_index: usize,
    paragraph: &ParagraphContext,
    stylesheet: &Stylesheet,
    theme: Option<&Theme>,
    visitor: &mut V,
) -> Vec<ConvertedRunResult> {
    let mut converted: Vec<ConvertedRunResult> = (0..runs.len())
        .map(|_| ConvertedRunResult {
            new_text: None,
            new_font: None,
            suppress: false,
        })
        .collect();

    // Pre-compute font for each run.
    let fonts: Vec<Option<String>> = runs
        .iter()
        .map(|rb| {
            let (run_font, _) = extract_font_and_text(&rb.events, theme);
            stylesheet
                .resolve_run_font(
                    run_font.as_deref(),
                    paragraph.run_default_font.as_deref(),
                    paragraph.style_id.as_deref(),
                )
                .map(|s| s.to_string())
        })
        .collect();

    let mut i = 0;
    while i < runs.len() {
        let font_i = fonts[i].as_deref();

        if !is_bijoy_font(font_i) {
            // Non-Bijoy run: convert individually.
            converted[i] = convert_run_buffer(
                &runs[i],
                paragraph_index,
                i,
                paragraph,
                stylesheet,
                theme,
                visitor,
            );
            i += 1;
            continue;
        }

        // Find the extent of this Bijoy group (adjacent same-font runs).
        let group_start = i;
        let group_font = font_i;
        let mut group_end = i + 1;
        while group_end < runs.len()
            && is_bijoy_font(fonts[group_end].as_deref())
            && fonts[group_end].as_deref() == group_font
        {
            group_end += 1;
        }

        if group_end == group_start + 1 {
            // Single Bijoy run: convert individually.
            converted[i] = convert_run_buffer(
                &runs[i],
                paragraph_index,
                i,
                paragraph,
                stylesheet,
                theme,
                visitor,
            );
            i += 1;
            continue;
        }

        // Multiple adjacent Bijoy runs: concatenate text and convert as one.
        let mut combined_text = String::new();
        for j in group_start..group_end {
            let (_, text) = extract_font_and_text(&runs[j].events, theme);
            combined_text.push_str(&text);
        }

        // Visit the combined text as a single run.
        let action = {
            let run = RunRef {
                paragraph_index,
                run_index: group_start,
                slide_index: None,
                font_name: group_font,
                text: &combined_text,
            };
            visitor.visit(run)
        };

        match action {
            RunAction::Keep => {
                // Classifier said don't convert — leave all runs as-is.
                i = group_end;
            }
            RunAction::Replace { new_text, new_font } => {
                // Put the full converted text in the first run, suppress the rest.
                converted[group_start] = ConvertedRunResult {
                    new_text: Some(new_text),
                    new_font: new_font.clone(),
                    suppress: false,
                };
                for j in (group_start + 1)..group_end {
                    converted[j] = ConvertedRunResult {
                        new_text: None,
                        new_font: None,
                        suppress: true,
                    };
                }
                i = group_end;
            }
        }
    }

    converted
}

/// Convert a run through the visitor without writing to the output.
fn convert_run_buffer<V: RunVisitor>(
    rb: &RunBuffer,
    paragraph_index: usize,
    run_index: usize,
    paragraph: &ParagraphContext,
    stylesheet: &Stylesheet,
    theme: Option<&Theme>,
    visitor: &mut V,
) -> ConvertedRunResult {
    let (run_font, text) = extract_font_and_text(&rb.events, theme);
    let resolved_font_owned: Option<String> = stylesheet
        .resolve_run_font(
            run_font.as_deref(),
            paragraph.run_default_font.as_deref(),
            paragraph.style_id.as_deref(),
        )
        .map(|s| s.to_string());

    let action = {
        let run = RunRef {
            paragraph_index,
            run_index,
            slide_index: None,
            font_name: resolved_font_owned.as_deref(),
            text: &text,
        };
        visitor.visit(run)
    };

    match action {
        RunAction::Keep => ConvertedRunResult {
            new_text: None,
            new_font: None,
            suppress: false,
        },
        RunAction::Replace { new_text, new_font } => ConvertedRunResult {
            new_text: Some(new_text),
            new_font: new_font,
            suppress: false,
        },
    }
}

/// Write a converted run to the XML writer.
fn write_converted_run(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    rb: RunBuffer,
    cr: ConvertedRunResult,
) -> Result<()> {
    if cr.suppress {
        return Ok(());
    }
    write_event(writer, &rb.start_event)?;
    let events_to_emit: Vec<Event<'static>> = match cr.new_font.as_deref() {
        Some(font) => inject_font_if_missing(&rb.events, font),
        None => rb.events.clone(),
    };
    emit_run_events(
        writer,
        &events_to_emit,
        cr.new_text.as_deref(),
        cr.new_font.as_deref(),
    )?;
    if let Some(end) = rb.end_event {
        write_event(writer, &end)?;
    }
    Ok(())
}

/// Returns `true` if `c` is a Unicode Bengali combining mark (dependent vowel
/// sign, hasanta, anusvara, visarga, candrabindu, nukta, or au-length mark).
fn is_bengali_combining(c: char) -> bool {
    matches!(c,
        '\u{0981}'..='\u{0983}' | // candrabindu, anusvara, visarga
        '\u{09BC}'              | // nukta
        '\u{09BE}'..='\u{09CC}' | // dependent vowel signs (aa through au)
        '\u{09CD}'              | // hasanta
        '\u{09D7}'              | // au-length mark
        '\u{09E2}'..='\u{09E3}'   // vocalic l/ll
    )
}

/// Post-conversion pass: merge runs whose converted text starts with Bengali
/// combining marks into the preceding run. This handles cases where Bijoy
/// conjunct-formers (e.g. `¨` → `্য`, `ª` → `্র`) are split into separate
/// XML runs from their base consonants.
fn merge_unicode_combining_runs(
    runs: &mut Vec<RunBuffer>,
    converted: &mut Vec<ConvertedRunResult>,
) {
    if runs.len() < 2 {
        return;
    }
    debug_assert_eq!(runs.len(), converted.len());

    // Walk right-to-left so we can chain merges (if run i+1 merged into i,
    // and run i now starts with a combining mark, it can merge into i-1).
    for i in (1..runs.len()).rev() {
        if converted[i].suppress {
            continue;
        }
        // Determine the effective text of this run after conversion.
        let text_i = effective_text(&runs[i], &converted[i]);
        if text_i.is_empty() {
            continue;
        }
        let first_char = text_i.chars().next().unwrap();
        // Merge if run starts with combining mark OR if previous run ends
        // with hasanta and this run starts with a consonant (forming a conjunct).
        let starts_combining = is_bengali_combining(first_char);
        let completes_conjunct = matches!(
            first_char,
            '\u{0995}'..='\u{09B9}' | '\u{09DC}' | '\u{09DD}' | '\u{09DF}'
        ) && {
            // Check if previous non-suppressed run ends with hasanta.
            let mut prev = if i > 0 { i - 1 } else { 0 };
            while prev > 0 && converted[prev].suppress {
                prev -= 1;
            }
            let pt = effective_text(&runs[prev], &converted[prev]);
            pt.ends_with('\u{09CD}')
        };
        if !starts_combining && !completes_conjunct {
            continue;
        }

        // Find the nearest preceding non-suppressed run.
        let mut j = i - 1;
        loop {
            if !converted[j].suppress {
                break;
            }
            if j == 0 {
                break;
            }
            j -= 1;
        }
        if converted[j].suppress {
            continue;
        }

        let text_j = effective_text(&runs[j], &converted[j]);
        if text_j.is_empty() {
            continue;
        }
        // Only merge if the previous run's text ends with Bengali script
        // content (a consonant, vowel sign, or other Bengali character).
        // This prevents merging a combining mark onto a Latin/number run.
        let last_char_j = text_j.chars().last().unwrap();
        let is_bengali_context = matches!(
            last_char_j,
            '\u{0980}'..='\u{09FF}' // Bengali block
        );
        if !is_bengali_context {
            continue;
        }

        // Merge: append run i's text to run j's text, then re-run
        // reorder passes to fix ordering at the merge boundary.
        let concat = format!("{}{}", text_j, text_i);
        let merged = banglakit_core::normalize::anusvara_reorder(
            &banglakit_core::normalize::subjoiner_reorder(&concat),
        );
        converted[j].new_text = Some(merged);
        // If run j had a font change, keep it; otherwise inherit from run i.
        if converted[j].new_font.is_none() {
            converted[j].new_font = converted[i].new_font.clone();
        }
        converted[i].suppress = true;
    }
}

/// Fix cross-run pre-base vowel ordering: when a run ends with a pre-base
/// vowel sign (ি, ে, ৈ) and the next run starts with a consonant cluster,
/// move the vowel sign into the next run after the cluster.
fn fix_cross_run_prebase(runs: &[RunBuffer], converted: &mut [ConvertedRunResult]) {
    if runs.len() < 2 {
        return;
    }

    for i in 0..runs.len() - 1 {
        if converted[i].suppress {
            continue;
        }
        let text_i = effective_text_ref(runs, converted, i);
        if text_i.is_empty() {
            continue;
        }
        let last = text_i.chars().last().unwrap();
        if !matches!(last, '\u{09BF}' | '\u{09C7}' | '\u{09C8}') {
            continue;
        }

        // Find next non-suppressed run.
        let mut j = i + 1;
        while j < runs.len() && converted[j].suppress {
            j += 1;
        }
        if j >= runs.len() {
            continue;
        }

        let text_j = effective_text_ref(runs, converted, j);
        if text_j.is_empty() {
            continue;
        }
        let first_j = text_j.chars().next().unwrap();
        if !matches!(
            first_j,
            '\u{0995}'..='\u{09B9}' | '\u{09DC}' | '\u{09DD}' | '\u{09DF}'
        ) {
            continue;
        }

        // Strip vowel from end of run i.
        let new_i: String = text_i.chars().take(text_i.chars().count() - 1).collect();

        // Insert vowel after the consonant cluster in run j.
        let chars_j: Vec<char> = text_j.chars().collect();
        let mut insert_pos = 1;
        while insert_pos + 1 < chars_j.len()
            && chars_j[insert_pos] == '\u{09CD}'
            && matches!(
                chars_j.get(insert_pos + 1),
                Some('\u{0995}'..='\u{09B9}')
                    | Some('\u{09DC}')
                    | Some('\u{09DD}')
                    | Some('\u{09DF}')
            )
        {
            insert_pos += 2;
        }
        let mut new_j = String::with_capacity(text_j.len() + 4);
        new_j.extend(chars_j[..insert_pos].iter());
        new_j.push(last);
        new_j.extend(chars_j[insert_pos..].iter());

        // Apply subjoiner_reorder on the modified run to fix any remaining issues.
        let new_j = banglakit_core::normalize::subjoiner_reorder(&new_j);

        converted[i].new_text = Some(new_i);
        converted[j].new_text = Some(new_j);
    }
}

fn effective_text_ref(runs: &[RunBuffer], converted: &[ConvertedRunResult], i: usize) -> String {
    match &converted[i].new_text {
        Some(t) => t.clone(),
        None => extract_text_from_events(&runs[i].events),
    }
}

/// Get the effective text of a run after conversion.
fn effective_text(rb: &RunBuffer, cr: &ConvertedRunResult) -> String {
    match &cr.new_text {
        Some(t) => t.clone(),
        None => extract_text_from_events(&rb.events),
    }
}

/// If `events` has no `<w:rFonts>`, inject one so the run carries the new
/// font. If `<w:rPr>` is absent too, wrap the new `<w:rFonts/>` in a fresh
/// `<w:rPr>...</w:rPr>` block at the start of the run.
fn inject_font_if_missing(events: &[Event<'static>], font: &str) -> Vec<Event<'static>> {
    let mut has_rpr = false;
    let mut has_rfonts = false;
    let mut rpr_end_idx: Option<usize> = None;
    for (idx, ev) in events.iter().enumerate() {
        match ev {
            Event::Start(b) | Event::Empty(b) => {
                let local = local_name_bytes(b.name().into_inner());
                if local == b"rPr" {
                    has_rpr = true;
                }
                if local == b"rFonts" {
                    has_rfonts = true;
                }
            }
            Event::End(b) if local_name_bytes(b.name().into_inner()) == b"rPr" => {
                rpr_end_idx = Some(idx);
            }
            _ => {}
        }
    }
    if has_rfonts {
        // Existing `rFonts` will be rewritten by `emit_run_events`.
        return events.to_vec();
    }
    let mut out: Vec<Event<'static>> = Vec::with_capacity(events.len() + 4);
    if has_rpr {
        for (idx, ev) in events.iter().enumerate() {
            if Some(idx) == rpr_end_idx {
                out.push(Event::Empty(make_rfonts(font)));
            }
            out.push(ev.clone());
        }
    } else {
        out.push(Event::Start(BytesStart::new("w:rPr")));
        out.push(Event::Empty(make_rfonts(font)));
        out.push(Event::End(quick_xml::events::BytesEnd::new("w:rPr")));
        out.extend(events.iter().cloned());
    }
    out
}

fn make_rfonts(font: &str) -> BytesStart<'static> {
    let mut b = BytesStart::new("w:rFonts");
    b.push_attribute(("w:ascii", font));
    b.push_attribute(("w:hAnsi", font));
    b
}

fn extract_font_and_text(
    events: &[Event<'static>],
    theme: Option<&Theme>,
) -> (Option<String>, String) {
    let mut font: Option<String> = None;
    let mut text = String::new();
    let mut in_text = false;
    for ev in events {
        match ev {
            Event::Empty(b) if is_rfonts(b) => {
                font = font.or_else(|| font_from_rfonts(b, theme));
            }
            Event::Start(b) if is_rfonts(b) => {
                font = font.or_else(|| font_from_rfonts(b, theme));
            }
            Event::Start(b) if local_name_bytes(b.name().into_inner()) == b"t" => {
                in_text = true;
            }
            Event::End(b) if local_name_bytes(b.name().into_inner()) == b"t" => {
                in_text = false;
            }
            Event::Text(t) if in_text => {
                if let Ok(s) = t.unescape() {
                    text.push_str(&s);
                }
            }
            _ => {}
        }
    }
    (font, text)
}

fn is_rfonts(b: &BytesStart<'_>) -> bool {
    local_name_bytes(b.name().into_inner()) == b"rFonts"
}

pub(crate) fn local_name_bytes(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
}

/// Zero-allocation local-name accessor for an Event. The returned slice
/// borrows from the event's underlying buffer.
pub(crate) fn local_name_bytes_of<'a>(event: &'a Event<'_>) -> Option<&'a [u8]> {
    let bytes = match event {
        Event::Start(b) | Event::Empty(b) => b.name().into_inner(),
        Event::End(b) => b.name().into_inner(),
        _ => return None,
    };
    Some(local_name_bytes(bytes))
}

fn emit_run_events(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    events: &[Event<'static>],
    new_text: Option<&str>,
    new_font: Option<&str>,
) -> Result<()> {
    let mut in_text = false;
    let mut emitted_text = false;
    for ev in events {
        match ev {
            Event::Empty(b) if is_rfonts(b) => {
                let updated = if let Some(font) = new_font {
                    rewrite_rfonts(b, font)?
                } else {
                    b.clone().into_owned()
                };
                writer
                    .write_event(Event::Empty(updated))
                    .map_err(|e| anyhow!("XML write error: {e}"))?;
            }
            Event::Start(b) if is_rfonts(b) => {
                let updated = if let Some(font) = new_font {
                    rewrite_rfonts(b, font)?
                } else {
                    b.clone().into_owned()
                };
                writer
                    .write_event(Event::Start(updated))
                    .map_err(|e| anyhow!("XML write error: {e}"))?;
            }
            Event::Start(b) if local_name_bytes(b.name().into_inner()) == b"t" => {
                in_text = true;
                writer
                    .write_event(Event::Start(b.clone().into_owned()))
                    .map_err(|e| anyhow!("XML write error: {e}"))?;
            }
            Event::End(b) if local_name_bytes(b.name().into_inner()) == b"t" => {
                if in_text && !emitted_text {
                    if let Some(t) = new_text {
                        writer
                            .write_event(Event::Text(quick_xml::events::BytesText::new(t)))
                            .map_err(|e| anyhow!("XML write error: {e}"))?;
                        emitted_text = true;
                    }
                }
                in_text = false;
                writer
                    .write_event(Event::End(b.clone().into_owned()))
                    .map_err(|e| anyhow!("XML write error: {e}"))?;
            }
            Event::Text(_) if in_text && new_text.is_some() => {
                // Skip; replacement emitted on </w:t>.
            }
            _ => {
                write_event(writer, ev)?;
            }
        }
    }
    Ok(())
}

fn rewrite_rfonts(b: &BytesStart<'_>, font: &str) -> Result<BytesStart<'static>> {
    let name = String::from_utf8_lossy(b.name().into_inner()).into_owned();
    let mut new = BytesStart::new(name);

    // Snapshot every attribute as owned (key_bytes, value_bytes), so we can
    // freely re-key/re-value without lifetime entanglement with `b`.
    let owned_attrs: Vec<(Vec<u8>, Vec<u8>)> = b
        .attributes()
        .with_checks(false)
        .flatten()
        .map(|a| (a.key.into_inner().to_vec(), a.value.into_owned().to_vec()))
        .collect();

    let mut have_ascii = false;
    let mut have_hansi = false;
    for (key_bytes, value_bytes) in &owned_attrs {
        let local = local_name_bytes(key_bytes);
        let new_value: Cow<'_, [u8]> = if local == b"ascii" {
            have_ascii = true;
            Cow::Borrowed(font.as_bytes())
        } else if local == b"hAnsi" {
            have_hansi = true;
            Cow::Borrowed(font.as_bytes())
        } else {
            Cow::Borrowed(value_bytes.as_slice())
        };
        new.push_attribute(Attribute {
            key: quick_xml::name::QName(key_bytes.as_slice()),
            value: new_value,
        });
    }
    if !have_ascii {
        new.push_attribute(("w:ascii", font));
    }
    if !have_hansi {
        new.push_attribute(("w:hAnsi", font));
    }
    Ok(new.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_no_changes() {
        let xml = MINIMAL_DOC.to_string();
        let sheet = Stylesheet::default();
        let out =
            transform_document_xml(&xml, &sheet, None, &mut |_run: RunRef<'_>| RunAction::Keep)
                .unwrap();
        assert!(out.contains("Avwg evsjvq"), "output: {out}");
        assert!(out.contains("Hello world"));
    }

    #[test]
    fn replaces_run_text() {
        let xml = MINIMAL_DOC.to_string();
        let sheet = Stylesheet::default();
        let out = transform_document_xml(&xml, &sheet, None, &mut |run: RunRef<'_>| {
            if run.text.starts_with("Avwg") {
                RunAction::Replace {
                    new_text: "আমি বাংলায়".to_string(),
                    new_font: Some("Kalpurush".to_string()),
                }
            } else {
                RunAction::Keep
            }
        })
        .unwrap();
        assert!(out.contains("আমি বাংলায়"), "output: {out}");
        assert!(out.contains("Kalpurush"), "output: {out}");
        assert!(out.contains("Hello world"), "output: {out}");
    }

    #[test]
    fn cascade_from_paragraph_style_visible_to_visitor() {
        // Run has no font of its own; paragraph references style "BijoyBody".
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="BijoyBody"/></w:pPr>
      <w:r><w:t>Avwg</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let mut sheet = Stylesheet::default();
        sheet.styles.insert(
            "BijoyBody".to_string(),
            styles::Style {
                based_on: None,
                font: Some("SutonnyMJ".to_string()),
            },
        );

        let mut seen: Option<String> = None;
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen = run.font_name.map(|s| s.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(seen.as_deref(), Some("SutonnyMJ"));
    }

    #[test]
    fn cascade_from_paragraph_run_default() {
        // Run has no rFonts, paragraph's pPr/rPr/rFonts says SutonnyMJ.
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:rPr><w:rFonts w:ascii="SutonnyMJ"/></w:rPr></w:pPr>
      <w:r><w:t>Avwg</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        let mut seen: Option<String> = None;
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen = run.font_name.map(|s| s.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(seen.as_deref(), Some("SutonnyMJ"));
    }

    #[test]
    fn cascade_to_doc_defaults_when_nothing_else() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body><w:p><w:r><w:t>Avwg</w:t></w:r></w:p></w:body>
</w:document>"#;
        let mut sheet = Stylesheet::default();
        sheet.default_font = Some("DocDefault".to_string());
        let mut seen: Option<String> = None;
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen = run.font_name.map(|s| s.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(seen.as_deref(), Some("DocDefault"));
    }

    #[test]
    fn run_level_font_wins_over_cascade() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="BijoyBody"/></w:pPr>
      <w:r><w:rPr><w:rFonts w:ascii="Arial"/></w:rPr><w:t>x</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let mut sheet = Stylesheet::default();
        sheet.styles.insert(
            "BijoyBody".to_string(),
            styles::Style {
                based_on: None,
                font: Some("SutonnyMJ".to_string()),
            },
        );
        let mut seen: Option<String> = None;
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen = run.font_name.map(|s| s.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(seen.as_deref(), Some("Arial"));
    }

    #[test]
    fn merge_prebase_orphan_ekar_into_next_run() {
        // Simulates: run1 = "‡" (e-kar), run2 = "`k" (দেশ in Bijoy).
        // After merge, run1 should be removed and run2 should have "‡`k".
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>‡</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>`k</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        let mut seen_texts: Vec<String> = Vec::new();
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen_texts.push(run.text.to_string());
            RunAction::Keep
        })
        .unwrap();
        // After merge the visitor should see one run with the combined text.
        assert_eq!(
            seen_texts.len(),
            1,
            "orphan run should be merged: {seen_texts:?}"
        );
        assert_eq!(seen_texts[0], "‡`k");
    }

    #[test]
    fn merge_postbase_orphan_reph_into_previous_run() {
        // Simulates: run1 = "Kv" (consonant cluster), run2 = "©" (reph).
        // After merge, run2 should be removed and run1 should have "Kv©".
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>Kv</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>©</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        let mut seen_texts: Vec<String> = Vec::new();
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen_texts.push(run.text.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(
            seen_texts.len(),
            1,
            "orphan run should be merged: {seen_texts:?}"
        );
        assert_eq!(seen_texts[0], "Kv©");
    }

    #[test]
    fn no_merge_across_different_fonts() {
        // run1 = "‡" in SutonnyMJ, run2 = "hello" in Arial.
        // Should NOT merge because fonts differ.
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>‡</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/></w:rPr>
        <w:t>hello</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        let mut seen_texts: Vec<String> = Vec::new();
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen_texts.push(run.text.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(
            seen_texts.len(),
            2,
            "should NOT merge across fonts: {seen_texts:?}"
        );
    }

    #[test]
    fn no_merge_when_non_bijoy_font() {
        // run1 = "‡" in Arial (not Bijoy), run2 = "text" in Arial.
        // Should NOT merge — the orphan detection requires a Bijoy font.
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/></w:rPr>
        <w:t>‡</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/></w:rPr>
        <w:t>text</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        let mut seen_texts: Vec<String> = Vec::new();
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen_texts.push(run.text.to_string());
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(
            seen_texts.len(),
            2,
            "should NOT merge non-Bijoy fonts: {seen_texts:?}"
        );
    }

    #[test]
    fn merge_multiple_prebase_orphans_chain() {
        // run1 = "‡" (e-kar), run2 = "‰" (ai-kar), run3 = "consonant".
        // Both orphans should merge into run3.
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>‡</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>‰</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>`k</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        let mut seen_texts: Vec<String> = Vec::new();
        let _ = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            seen_texts.push(run.text.to_string());
            RunAction::Keep
        })
        .unwrap();
        // run1 (‡) merges into run2 (‰), then ‡‰ merges into run3.
        assert_eq!(
            seen_texts.len(),
            1,
            "chained orphans should merge: {seen_texts:?}"
        );
        assert!(
            seen_texts[0].starts_with("‡"),
            "merged text: {}",
            seen_texts[0]
        );
    }

    #[test]
    fn post_conversion_merge_combining_mark_into_previous() {
        // Two SutonnyMJ runs that, after conversion, produce a combining mark
        // at the start of the second run. The post-conversion merge should
        // absorb it into the first.
        //
        // run1: "K" → converts to "ক" (ka)
        // run2: "¨" → converts to "্য" (hasanta + ya = ya-phala)
        // After merge: output should be one run containing "ক্য" (kya conjunct).
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>K</w:t>
      </w:r>
      <w:r>
        <w:rPr><w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/></w:rPr>
        <w:t>¨</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
        let sheet = Stylesheet::default();
        use banglakit_core::classifier::Mode;
        use banglakit_core::encoding::Encoding;
        use banglakit_core::policy::{convert_run, ConvertOptions};
        let opts = ConvertOptions {
            encoding: Encoding::Bijoy,
            mode: Mode::Safe,
            threshold: None,
            unicode_font: "Kalpurush",
            auto_match_fonts: false,
        };
        let out = transform_document_xml(xml, &sheet, None, &mut |run: RunRef<'_>| {
            RunAction::from(convert_run(run.text, run.font_name, &opts))
        })
        .unwrap();
        // The output should NOT have a run starting with ্য (hasanta).
        // Instead, "ক" and "্য" should be merged into one run.
        let texts: Vec<&str> = out
            .split("<w:t")
            .skip(1)
            .filter_map(|s| {
                let start = s.find('>')? + 1;
                let end = s.find("</w:t>")?;
                Some(&s[start..end])
            })
            .collect();
        assert_eq!(texts.len(), 1, "should merge into one run, got: {texts:?}");
        assert!(
            texts[0].contains("ক") && texts[0].contains("্য"),
            "merged text should contain kya conjunct: {}",
            texts[0]
        );
    }

    const MINIMAL_DOC: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr>
          <w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/>
        </w:rPr>
        <w:t>Avwg evsjvq</w:t>
      </w:r>
    </w:p>
    <w:p>
      <w:r>
        <w:rPr>
          <w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/>
        </w:rPr>
        <w:t>Hello world</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>"#;
}
