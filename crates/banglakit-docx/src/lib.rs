//! DOCX read/write adapter for banglakit-converter.
//!
//! The public surface is [`process_docx`], which streams every `<w:r>` (run)
//! past a caller-supplied [`RunVisitor`] and writes the result to a new
//! DOCX. All zip entries other than `word/document.xml` are copied
//! byte-for-byte, satisfying the SDD §11 format-fidelity requirement.
//!
//! Font extraction reads `w:rPr/w:rFonts/@w:ascii` first, then `@w:hAnsi`.
//! Full style-cascade resolution (paragraph style → docDefaults → Normal
//! style) is **not** implemented in v0.1.0; runs whose font is inherited
//! from style appear with `font_name = None` and the caller's classifier
//! falls through to heuristic scoring (PRD §3.2 closing paragraph).

use anyhow::{anyhow, Context, Result};
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::borrow::Cow;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const DOCUMENT_XML: &str = "word/document.xml";

/// A single run handed to the visitor.
pub struct RunRef<'a> {
    pub paragraph_index: usize,
    pub run_index: usize,
    pub font_name: Option<&'a str>,
    pub text: &'a str,
}

/// What the visitor wants done with a run.
pub enum RunAction {
    Keep,
    Replace {
        new_text: String,
        new_font: Option<String>,
    },
}

pub trait RunVisitor {
    fn visit(&mut self, run: RunRef<'_>) -> RunAction;
}

impl<F: FnMut(RunRef<'_>) -> RunAction> RunVisitor for F {
    fn visit(&mut self, run: RunRef<'_>) -> RunAction {
        (self)(run)
    }
}

/// Read `in_path`, stream each run past `visitor`, and write the result to
/// `out_path`. Non-`word/document.xml` zip entries are copied verbatim.
pub fn process_docx<V: RunVisitor>(
    in_path: &Path,
    out_path: &Path,
    visitor: &mut V,
) -> Result<()> {
    let file = File::open(in_path)
        .with_context(|| format!("opening {}", in_path.display()))?;
    let mut archive = ZipArchive::new(file)?;

    let mut document_xml = String::new();
    {
        let mut entry = archive
            .by_name(DOCUMENT_XML)
            .with_context(|| format!("{DOCUMENT_XML} not in archive"))?;
        entry.read_to_string(&mut document_xml)?;
    }

    let new_document_xml = transform_document_xml(&document_xml, visitor)?;

    let out_file = File::create(out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    let mut zip_out = ZipWriter::new(out_file);

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
    zip_out.finish()?;
    Ok(())
}

fn transform_document_xml<V: RunVisitor>(xml: &str, visitor: &mut V) -> Result<String> {
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
                let event = event.into_owned();
                let local = local_name(&event);
                match (&event, local.as_deref()) {
                    (Event::Start(_), Some("p")) => {
                        in_paragraph = true;
                        run_index = 0;
                        write_event(&mut writer, &event)?;
                    }
                    (Event::End(_), Some("p")) => {
                        if let Some(rb) = run_buffer.take() {
                            flush_run(&mut writer, rb, paragraph_index, run_index, visitor)?;
                        }
                        in_paragraph = false;
                        paragraph_index += 1;
                        write_event(&mut writer, &event)?;
                    }
                    (Event::Start(_), Some("r")) if in_paragraph => {
                        run_buffer = Some(RunBuffer::new(event.clone()));
                    }
                    (Event::End(_), Some("r")) if run_buffer.is_some() => {
                        let mut rb = run_buffer.take().unwrap();
                        rb.end_event = Some(event);
                        flush_run(&mut writer, rb, paragraph_index, run_index, visitor)?;
                        run_index += 1;
                    }
                    _ if run_buffer.is_some() => {
                        run_buffer.as_mut().unwrap().push(event);
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

fn local_name(event: &Event<'_>) -> Option<String> {
    let bytes = match event {
        Event::Start(b) | Event::Empty(b) => b.name().into_inner().to_vec(),
        Event::End(b) => b.name().into_inner().to_vec(),
        _ => return None,
    };
    let s = String::from_utf8(bytes).ok()?;
    Some(match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s,
    })
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
    visitor: &mut V,
) -> Result<()> {
    let (font_name, text) = extract_font_and_text(&rb.events);

    let action = {
        let run = RunRef {
            paragraph_index,
            run_index,
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
    emit_run_events(writer, &rb.events, new_text.as_deref(), new_font.as_deref())?;
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
            Event::Empty(b) if is_rfonts(b) => {
                font = font.or_else(|| read_font_from_attrs(b));
            }
            Event::Start(b) if is_rfonts(b) => {
                font = font.or_else(|| read_font_from_attrs(b));
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

fn local_name_bytes(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
}

fn read_font_from_attrs(b: &BytesStart<'_>) -> Option<String> {
    let mut ascii: Option<String> = None;
    let mut hansi: Option<String> = None;
    for attr in b.attributes().with_checks(false).flatten() {
        let key = attr.key.into_inner();
        let local = local_name_bytes(key);
        let val = attr.unescape_value().ok()?;
        match local {
            b"ascii" => ascii = Some(val.into_owned()),
            b"hAnsi" => hansi = Some(val.into_owned()),
            _ => {}
        }
    }
    ascii.or(hansi)
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
        let out = transform_document_xml(&xml, &mut |_run: RunRef<'_>| RunAction::Keep).unwrap();
        assert!(out.contains("Avwg evsjvq"), "output: {out}");
        assert!(out.contains("Hello world"));
    }

    #[test]
    fn replaces_run_text() {
        let xml = MINIMAL_DOC.to_string();
        let out = transform_document_xml(&xml, &mut |run: RunRef<'_>| {
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
