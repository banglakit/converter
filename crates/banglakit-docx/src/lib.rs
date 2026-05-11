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
pub fn process_docx<V: RunVisitor>(
    in_path: &Path,
    out_path: &Path,
    visitor: &mut V,
) -> Result<()> {
    let bytes = std::fs::read(in_path)
        .with_context(|| format!("opening {}", in_path.display()))?;
    let out = process_docx_bytes(&bytes, visitor)?;
    std::fs::write(out_path, out)
        .with_context(|| format!("creating {}", out_path.display()))?;
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
        let options = SimpleFileOptions::default()
            .compression_method(match entry.compression() {
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
    let mut ppr_buffer: Option<Vec<Event<'static>>> = None;
    let mut paragraph_index: usize = 0;
    let mut run_index: usize = 0;
    let mut in_paragraph = false;
    let mut paragraph_ctx = ParagraphContext::default();

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => {
                let local = local_name_bytes_of(&event);

                // Inside a run buffer, everything is captured until </w:r>.
                if let Some(rb) = run_buffer.as_mut() {
                    if matches!(event, Event::End(_)) && local == Some(b"r") {
                        // close run, flush
                        rb.end_event = Some(event.into_owned());
                        let rb_taken = run_buffer.take().unwrap();
                        flush_run(
                            &mut writer,
                            rb_taken,
                            paragraph_index,
                            run_index,
                            &paragraph_ctx,
                            stylesheet,
                            theme,
                            visitor,
                        )?;
                        run_index += 1;
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
                        run_index = 0;
                        paragraph_ctx = ParagraphContext::default();
                        write_event(&mut writer, &event)?;
                    }
                    (Event::End(_), Some(b"p")) => {
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

fn flush_run<V: RunVisitor>(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    rb: RunBuffer,
    paragraph_index: usize,
    run_index: usize,
    paragraph: &ParagraphContext,
    stylesheet: &Stylesheet,
    theme: Option<&Theme>,
    visitor: &mut V,
) -> Result<()> {
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

    write_event(writer, &rb.start_event)?;
    let (new_text, new_font) = match action {
        RunAction::Keep => (None, None),
        RunAction::Replace { new_text, new_font } => (Some(new_text), new_font),
    };
    // If we're changing the font on a run that has no existing <w:rFonts>
    // (or even no <w:rPr> at all), inject the structure so the change
    // survives. Without this, the new font name would be lost and the run
    // would inherit from the original style — printing Unicode Bengali
    // through a SutonnyMJ-styled paragraph.
    let events_to_emit: Vec<Event<'static>> = match new_font.as_deref() {
        Some(font) => inject_font_if_missing(&rb.events, font),
        None => rb.events.clone(),
    };
    emit_run_events(writer, &events_to_emit, new_text.as_deref(), new_font.as_deref())?;
    if let Some(end) = rb.end_event {
        write_event(writer, &end)?;
    }
    Ok(())
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
            transform_document_xml(&xml, &sheet, None, &mut |_run: RunRef<'_>| RunAction::Keep).unwrap();
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
