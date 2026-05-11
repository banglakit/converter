//! PPTX read/write adapter for banglakit-converter.
//!
//! Walks every `ppt/slides/slideN.xml` inside the input PPTX zip, streams
//! each `<a:r>` run past the caller-supplied [`RunVisitor`] (re-exported
//! from [`banglakit_core`]), and writes the result to a new PPTX. All
//! other zip entries — slide masters, layouts, theme, media — are copied
//! byte-for-byte to preserve the deck's formatting and assets.
//!
//! Font extraction reads the `typeface` attribute of `<a:latin>` inside
//! `<a:rPr>`. PPTX style cascade (shape → layout → master → theme) is
//! **not** implemented in v0.2; runs whose font is inherited fall through
//! to the classifier's heuristic stage. The slide master / theme font is
//! commonly `+mn-lt` (a theme-font reference), which our run-level
//! extractor does not currently resolve.

pub use banglakit_core::{RunAction, RunRef, RunVisitor};

use anyhow::{anyhow, Context, Result};
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::borrow::Cow;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const SLIDES_PREFIX: &str = "ppt/slides/slide";
const SLIDES_SUFFIX: &str = ".xml";

/// Open `in_path`, stream every run through `visitor`, and write the result
/// to `out_path`. Non-slide zip entries are copied verbatim.
pub fn process_pptx<V: RunVisitor>(
    in_path: &Path,
    out_path: &Path,
    visitor: &mut V,
) -> Result<()> {
    let bytes = std::fs::read(in_path)
        .with_context(|| format!("opening {}", in_path.display()))?;
    let out = process_pptx_bytes(&bytes, visitor)?;
    std::fs::write(out_path, out)
        .with_context(|| format!("creating {}", out_path.display()))?;
    Ok(())
}

/// In-memory variant of [`process_pptx`]. Takes the raw PPTX zip bytes and
/// returns the converted PPTX bytes. Used by `banglakit-wasm` so the browser
/// can round-trip a `.pptx` deck entirely client-side.
pub fn process_pptx_bytes<V: RunVisitor>(input: &[u8], visitor: &mut V) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(input))?;

    let mut zip_out = ZipWriter::new(Cursor::new(Vec::<u8>::new()));

    // Single-pass: for each input entry, transform-and-write if it's a slide
    // XML, otherwise copy through. Peak in-memory slide XML is one slide,
    // not the whole deck.
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        let options = SimpleFileOptions::default().compression_method(
            match entry.compression() {
                CompressionMethod::Stored => CompressionMethod::Stored,
                _ => CompressionMethod::Deflated,
            },
        );
        zip_out.start_file(&name, options)?;
        if is_slide_xml(&name) {
            let slide_idx = parse_slide_index(&name).unwrap_or(0);
            let mut xml = String::new();
            entry.read_to_string(&mut xml)?;
            let new_xml = transform_slide_xml(&xml, slide_idx, visitor)?;
            zip_out.write_all(new_xml.as_bytes())?;
        } else {
            std::io::copy(&mut entry, &mut zip_out)?;
        }
    }
    let cursor = zip_out.finish()?;
    Ok(cursor.into_inner())
}

fn is_slide_xml(name: &str) -> bool {
    name.starts_with(SLIDES_PREFIX)
        && name.ends_with(SLIDES_SUFFIX)
        // Filter out slideLayoutN.xml / slideMasterN.xml which sit in
        // separate directories but could match a less-precise prefix.
        && !name.contains("Layout")
        && !name.contains("Master")
        && !name.contains("_rels")
}

fn parse_slide_index(name: &str) -> Option<usize> {
    // ppt/slides/slide12.xml -> 12
    let stem = name.strip_prefix(SLIDES_PREFIX)?.strip_suffix(SLIDES_SUFFIX)?;
    stem.parse().ok()
}

fn transform_slide_xml<V: RunVisitor>(
    xml: &str,
    slide_index: usize,
    visitor: &mut V,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::<u8>::new()));

    let mut run_buffer: Option<RunBuffer> = None;
    let mut paragraph_index: usize = 0;
    let mut run_index: usize = 0;
    let mut in_paragraph = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => {
                let local = local_name_bytes_of(&event);

                // Inside a run buffer, everything is captured until </a:r>.
                if let Some(rb) = run_buffer.as_mut() {
                    if matches!(event, Event::End(_)) && local == Some(b"r") {
                        rb.end_event = Some(event.into_owned());
                        let rb_taken = run_buffer.take().unwrap();
                        flush_run(
                            &mut writer,
                            rb_taken,
                            slide_index,
                            paragraph_index,
                            run_index,
                            visitor,
                        )?;
                        run_index += 1;
                    } else {
                        rb.push(event.into_owned());
                    }
                    buf.clear();
                    continue;
                }

                // Outside a run: track structural boundaries, pass through
                // without owning.
                match (&event, local) {
                    (Event::Start(_), Some(b"p")) => {
                        in_paragraph = true;
                        run_index = 0;
                        write_event(&mut writer, &event)?;
                    }
                    (Event::End(_), Some(b"p")) => {
                        in_paragraph = false;
                        paragraph_index += 1;
                        write_event(&mut writer, &event)?;
                    }
                    (Event::Start(_), Some(b"r")) if in_paragraph => {
                        run_buffer = Some(RunBuffer::new(event.into_owned()));
                    }
                    _ => {
                        write_event(&mut writer, &event)?;
                    }
                }
            }
            Err(e) => return Err(anyhow!("slide XML parse error: {e}")),
        }
        buf.clear();
    }

    let bytes = writer.into_inner().into_inner();
    Ok(String::from_utf8(bytes).context("invalid UTF-8 in slide output")?)
}

/// Zero-allocation local-name accessor for an Event.
fn local_name_bytes_of<'a>(event: &'a Event<'_>) -> Option<&'a [u8]> {
    let bytes = match event {
        Event::Start(b) | Event::Empty(b) => b.name().into_inner(),
        Event::End(b) => b.name().into_inner(),
        _ => return None,
    };
    Some(local_name_bytes(bytes))
}

fn local_name_bytes(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
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
    slide_index: usize,
    paragraph_index: usize,
    run_index: usize,
    visitor: &mut V,
) -> Result<()> {
    let (font_name, text) = extract_font_and_text(&rb.events);

    let action = {
        let run = RunRef {
            paragraph_index,
            run_index,
            slide_index: Some(slide_index),
            font_name: font_name.as_deref(),
            text: &text,
        };
        visitor.visit(run)
    };

    write_event(writer, &rb.start_event)?;
    let (new_text, new_font) = match action {
        RunAction::Keep => (None, None),
        RunAction::Replace { new_text, new_font } => (Some(new_text), new_font),
    };
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

fn extract_font_and_text(events: &[Event<'static>]) -> (Option<String>, String) {
    let mut font: Option<String> = None;
    let mut text = String::new();
    let mut in_text = false;
    for ev in events {
        match ev {
            Event::Empty(b) if is_latin(b) => {
                font = font.or_else(|| read_typeface(b));
            }
            Event::Start(b) if is_latin(b) => {
                font = font.or_else(|| read_typeface(b));
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

fn is_latin(b: &BytesStart<'_>) -> bool {
    local_name_bytes(b.name().into_inner()) == b"latin"
}

fn read_typeface(b: &BytesStart<'_>) -> Option<String> {
    for attr in b.attributes().with_checks(false).flatten() {
        let key = attr.key.into_inner();
        if local_name_bytes(key) == b"typeface" {
            return attr.unescape_value().ok().map(|v| v.into_owned());
        }
    }
    None
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
            Event::Empty(b) if is_latin(b) => {
                let updated = if let Some(font) = new_font {
                    rewrite_latin(b, font)?
                } else {
                    b.clone().into_owned()
                };
                writer
                    .write_event(Event::Empty(updated))
                    .map_err(|e| anyhow!("XML write error: {e}"))?;
            }
            Event::Start(b) if is_latin(b) => {
                let updated = if let Some(font) = new_font {
                    rewrite_latin(b, font)?
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
                // Suppress original; we already emitted the replacement on </a:t>.
            }
            _ => {
                write_event(writer, ev)?;
            }
        }
    }
    Ok(())
}

fn rewrite_latin(b: &BytesStart<'_>, font: &str) -> Result<BytesStart<'static>> {
    let name = String::from_utf8_lossy(b.name().into_inner()).into_owned();
    let mut new = BytesStart::new(name);
    let owned: Vec<(Vec<u8>, Vec<u8>)> = b
        .attributes()
        .with_checks(false)
        .flatten()
        .map(|a| (a.key.into_inner().to_vec(), a.value.into_owned().to_vec()))
        .collect();

    let mut have_typeface = false;
    for (key_bytes, value_bytes) in &owned {
        let local = local_name_bytes(key_bytes);
        let new_value: Cow<'_, [u8]> = if local == b"typeface" {
            have_typeface = true;
            Cow::Borrowed(font.as_bytes())
        } else {
            Cow::Borrowed(value_bytes.as_slice())
        };
        new.push_attribute(Attribute {
            key: quick_xml::name::QName(key_bytes.as_slice()),
            value: new_value,
        });
    }
    if !have_typeface {
        new.push_attribute(("typeface", font));
    }
    Ok(new.into_owned())
}

/// If the run has no `<a:latin>` font element, inject one so the new font
/// survives. Wrap in a synthetic `<a:rPr>` if `<a:rPr>` is absent too.
fn inject_font_if_missing(events: &[Event<'static>], font: &str) -> Vec<Event<'static>> {
    let mut has_rpr = false;
    let mut has_latin = false;
    let mut rpr_empty_idx: Option<usize> = None;
    let mut rpr_end_idx: Option<usize> = None;
    for (idx, ev) in events.iter().enumerate() {
        match ev {
            Event::Empty(b) if local_name_bytes(b.name().into_inner()) == b"rPr" => {
                has_rpr = true;
                rpr_empty_idx = Some(idx);
            }
            Event::Start(b) if local_name_bytes(b.name().into_inner()) == b"rPr" => {
                has_rpr = true;
            }
            Event::End(b) if local_name_bytes(b.name().into_inner()) == b"rPr" => {
                rpr_end_idx = Some(idx);
            }
            Event::Empty(b) | Event::Start(b) if local_name_bytes(b.name().into_inner()) == b"latin" => {
                has_latin = true;
            }
            _ => {}
        }
    }
    if has_latin {
        return events.to_vec();
    }
    let mut out: Vec<Event<'static>> = Vec::with_capacity(events.len() + 4);
    if let Some(empty_idx) = rpr_empty_idx {
        // <a:rPr lang="..."/> needs to become <a:rPr lang="..."><a:latin/></a:rPr>.
        for (idx, ev) in events.iter().enumerate() {
            if idx == empty_idx {
                if let Event::Empty(b) = ev {
                    // Snapshot attrs, emit as Start + Latin + End.
                    let name = String::from_utf8_lossy(b.name().into_inner()).into_owned();
                    let mut new = BytesStart::new(name.clone());
                    for a in b.attributes().with_checks(false).flatten() {
                        new.push_attribute(Attribute {
                            key: a.key,
                            value: a.value.into_owned().into(),
                        });
                    }
                    out.push(Event::Start(new.into_owned()));
                    out.push(Event::Empty(make_latin(font)));
                    out.push(Event::End(quick_xml::events::BytesEnd::new(name)));
                    continue;
                }
            }
            out.push(ev.clone());
        }
    } else if has_rpr {
        // Inject <a:latin/> just before </a:rPr>.
        for (idx, ev) in events.iter().enumerate() {
            if Some(idx) == rpr_end_idx {
                out.push(Event::Empty(make_latin(font)));
            }
            out.push(ev.clone());
        }
    } else {
        // No rPr at all: prepend <a:rPr><a:latin/></a:rPr>.
        out.push(Event::Start(BytesStart::new("a:rPr")));
        out.push(Event::Empty(make_latin(font)));
        out.push(Event::End(quick_xml::events::BytesEnd::new("a:rPr")));
        out.extend(events.iter().cloned());
    }
    out
}

fn make_latin(font: &str) -> BytesStart<'static> {
    let mut b = BytesStart::new("a:latin");
    b.push_attribute(("typeface", font));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLIDE_WITH_BIJOY_AND_ENGLISH: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:txBody>
          <a:p>
            <a:r>
              <a:rPr lang="en-US">
                <a:latin typeface="SutonnyMJ"/>
              </a:rPr>
              <a:t>Avwg evsjvq</a:t>
            </a:r>
          </a:p>
          <a:p>
            <a:r>
              <a:rPr lang="en-US">
                <a:latin typeface="Calibri"/>
              </a:rPr>
              <a:t>Hello world</a:t>
            </a:r>
          </a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

    #[test]
    fn extracts_runs_and_fonts() {
        let mut collected: Vec<(usize, Option<String>, String)> = vec![];
        let _ = transform_slide_xml(SLIDE_WITH_BIJOY_AND_ENGLISH, 1, &mut |run: RunRef<'_>| {
            collected.push((
                run.run_index,
                run.font_name.map(|s| s.to_string()),
                run.text.to_string(),
            ));
            RunAction::Keep
        })
        .unwrap();
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0].1.as_deref(), Some("SutonnyMJ"));
        assert_eq!(collected[0].2, "Avwg evsjvq");
        assert_eq!(collected[1].1.as_deref(), Some("Calibri"));
        assert_eq!(collected[1].2, "Hello world");
    }

    #[test]
    fn rewrites_text_and_typeface() {
        let out = transform_slide_xml(SLIDE_WITH_BIJOY_AND_ENGLISH, 1, &mut |run: RunRef<'_>| {
            if run.font_name == Some("SutonnyMJ") {
                RunAction::Replace {
                    new_text: "আমি বাংলায়".to_string(),
                    new_font: Some("Kalpurush".to_string()),
                }
            } else {
                RunAction::Keep
            }
        })
        .unwrap();
        assert!(out.contains("আমি বাংলায়"), "{out}");
        assert!(out.contains("Kalpurush"), "{out}");
        assert!(out.contains("Hello world"), "{out}");
        assert!(out.contains("Calibri"), "Calibri lost: {out}");
    }

    #[test]
    fn injects_latin_when_missing() {
        // Run has <a:rPr/> (Empty) and no <a:latin>; we expect injection
        // to add a:latin to the output.
        let xml = r#"<?xml version="1.0"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody>
    <a:p><a:r><a:rPr lang="en-US"/><a:t>x</a:t></a:r></a:p>
  </p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#;
        let out = transform_slide_xml(xml, 1, &mut |_run: RunRef<'_>| RunAction::Replace {
            new_text: "y".to_string(),
            new_font: Some("Kalpurush".to_string()),
        })
        .unwrap();
        assert!(out.contains("Kalpurush"), "{out}");
        assert!(out.contains(">y<"), "{out}");
    }

    #[test]
    fn slide_xml_name_filter() {
        assert!(is_slide_xml("ppt/slides/slide1.xml"));
        assert!(is_slide_xml("ppt/slides/slide12.xml"));
        assert!(!is_slide_xml("ppt/slideLayouts/slideLayout1.xml"));
        assert!(!is_slide_xml("ppt/slideMasters/slideMaster1.xml"));
        assert!(!is_slide_xml("ppt/slides/_rels/slide1.xml.rels"));
    }
}
