//! Parse `word/theme/theme1.xml` into a tiny [`Theme`] and resolve a
//! `<w:rFonts>` element to a concrete font name, honouring theme
//! references.
//!
//! Modern Word output rarely writes `<w:rFonts w:ascii="Calibri"/>`
//! directly; it writes `<w:rFonts w:asciiTheme="minorHAnsi"/>` and stores
//! the actual font in theme1.xml. Without theme resolution our cascade
//! returns `None` for those elements and the run falls through to the
//! heuristic classifier — silently missing a font hint that was actually
//! there all along.
//!
//! We only resolve the Latin-script font slots (`minorFont/latin` and
//! `majorFont/latin`). East-Asian and complex-script slots are irrelevant
//! to the Bengali ↔ Latin classification this product cares about.

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// The relevant part of `word/theme/theme1.xml`.
#[derive(Debug, Default, Clone)]
pub struct Theme {
    /// `<a:fontScheme>/<a:minorFont>/<a:latin typeface="…">`. Resolved by
    /// tokens `minorHAnsi` and `minorAscii`.
    pub minor_latin: Option<String>,
    /// `<a:fontScheme>/<a:majorFont>/<a:latin typeface="…">`. Resolved by
    /// tokens `majorHAnsi` and `majorAscii`.
    pub major_latin: Option<String>,
}

impl Theme {
    /// Resolve a `w:asciiTheme` / `w:hAnsiTheme` token to a font name.
    fn resolve_token(&self, token: &str) -> Option<&str> {
        match token {
            "minorHAnsi" | "minorAscii" => self.minor_latin.as_deref(),
            "majorHAnsi" | "majorAscii" => self.major_latin.as_deref(),
            // Bidi / EastAsia / cstheme are non-Latin slots we don't model.
            _ => None,
        }
    }
}

/// Parse `theme1.xml` to a [`Theme`]. Returns a default ([`Theme::default`])
/// when input is empty so callers can pass an absent theme through without
/// branching.
pub fn parse_theme(xml: &str) -> Result<Theme> {
    if xml.trim().is_empty() {
        return Ok(Theme::default());
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut theme = Theme::default();
    // Which font scheme slot we're currently inside (None when outside both).
    let mut current_slot: Option<Slot> = None;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => match (&event, super::local_name_bytes_of(&event)) {
                (Event::Start(_), Some(b"majorFont")) => current_slot = Some(Slot::Major),
                (Event::Start(_), Some(b"minorFont")) => current_slot = Some(Slot::Minor),
                (Event::End(_), Some(b"majorFont")) | (Event::End(_), Some(b"minorFont")) => {
                    current_slot = None;
                }
                (Event::Empty(b), Some(b"latin")) | (Event::Start(b), Some(b"latin")) => {
                    if let Some(slot) = current_slot {
                        let typeface = attr_typeface(b);
                        match slot {
                            Slot::Major => {
                                if theme.major_latin.is_none() {
                                    theme.major_latin = typeface;
                                }
                            }
                            Slot::Minor => {
                                if theme.minor_latin.is_none() {
                                    theme.minor_latin = typeface;
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            Err(e) => return Err(anyhow!("theme1.xml parse error: {e}")),
        }
        buf.clear();
    }

    Ok(theme)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Slot {
    Major,
    Minor,
}

fn attr_typeface(b: &BytesStart<'_>) -> Option<String> {
    for a in b.attributes().with_checks(false).flatten() {
        let key = a.key.into_inner();
        if super::local_name_bytes(key) == b"typeface" {
            return a.unescape_value().ok().map(|v| v.into_owned());
        }
    }
    None
}

/// Read the font from a `<w:rFonts>` element, preferring direct
/// `@w:ascii` / `@w:hAnsi`, then theme-resolved
/// `@w:asciiTheme` / `@w:hAnsiTheme`.
///
/// `theme = None` is equivalent to "theme1.xml is absent or empty"; the
/// theme-attribute fallback simply yields `None` for every token.
pub(crate) fn font_from_rfonts(b: &BytesStart<'_>, theme: Option<&Theme>) -> Option<String> {
    let mut ascii: Option<String> = None;
    let mut hansi: Option<String> = None;
    let mut ascii_theme: Option<String> = None;
    let mut hansi_theme: Option<String> = None;
    for attr in b.attributes().with_checks(false).flatten() {
        let key = attr.key.into_inner();
        let local = super::local_name_bytes(key);
        let val = match attr.unescape_value() {
            Ok(v) => v.into_owned(),
            Err(_) => continue,
        };
        match local {
            b"ascii" => ascii = Some(val),
            b"hAnsi" => hansi = Some(val),
            b"asciiTheme" => ascii_theme = Some(val),
            b"hAnsiTheme" => hansi_theme = Some(val),
            _ => {}
        }
    }
    // Direct ascii / hAnsi wins.
    if let Some(v) = ascii.or(hansi) {
        return Some(v);
    }
    // Fall through to theme-resolved variants.
    let theme = theme?;
    if let Some(tok) = ascii_theme.or(hansi_theme) {
        if let Some(resolved) = theme.resolve_token(&tok) {
            return Some(resolved.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <a:themeElements>
    <a:fontScheme>
      <a:majorFont>
        <a:latin typeface="Calibri Light"/>
        <a:ea typeface=""/>
        <a:cs typeface=""/>
      </a:majorFont>
      <a:minorFont>
        <a:latin typeface="SutonnyMJ"/>
        <a:ea typeface=""/>
        <a:cs typeface=""/>
      </a:minorFont>
    </a:fontScheme>
  </a:themeElements>
</a:theme>"#;

    #[test]
    fn parses_both_slots() {
        let t = parse_theme(THEME_XML).unwrap();
        assert_eq!(t.minor_latin.as_deref(), Some("SutonnyMJ"));
        assert_eq!(t.major_latin.as_deref(), Some("Calibri Light"));
    }

    #[test]
    fn empty_input_is_default() {
        let t = parse_theme("").unwrap();
        assert!(t.minor_latin.is_none());
        assert!(t.major_latin.is_none());
    }

    #[test]
    fn missing_major_slot() {
        let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <a:themeElements><a:fontScheme>
    <a:minorFont><a:latin typeface="X"/></a:minorFont>
  </a:fontScheme></a:themeElements>
</a:theme>"#;
        let t = parse_theme(xml).unwrap();
        assert_eq!(t.minor_latin.as_deref(), Some("X"));
        assert!(t.major_latin.is_none());
    }

    #[test]
    fn resolve_token_maps_correctly() {
        let t = parse_theme(THEME_XML).unwrap();
        assert_eq!(t.resolve_token("minorHAnsi"), Some("SutonnyMJ"));
        assert_eq!(t.resolve_token("minorAscii"), Some("SutonnyMJ"));
        assert_eq!(t.resolve_token("majorHAnsi"), Some("Calibri Light"));
        assert_eq!(t.resolve_token("majorAscii"), Some("Calibri Light"));
        assert_eq!(t.resolve_token("minorBidi"), None);
        assert_eq!(t.resolve_token("nonsense"), None);
    }

    #[test]
    fn font_from_rfonts_prefers_direct_ascii() {
        let theme = parse_theme(THEME_XML).unwrap();
        let xml = r#"<w:rFonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:ascii="Arial" w:asciiTheme="minorHAnsi"/>"#;
        let mut reader = quick_xml::Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let event = reader.read_event_into(&mut buf).unwrap();
        if let Event::Empty(b) = event {
            assert_eq!(font_from_rfonts(&b, Some(&theme)).as_deref(), Some("Arial"));
        } else {
            panic!("expected Empty: {event:?}");
        }
    }

    #[test]
    fn font_from_rfonts_falls_back_to_theme() {
        let theme = parse_theme(THEME_XML).unwrap();
        let xml = r#"<w:rFonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/>"#;
        let mut reader = quick_xml::Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let event = reader.read_event_into(&mut buf).unwrap();
        if let Event::Empty(b) = event {
            assert_eq!(
                font_from_rfonts(&b, Some(&theme)).as_deref(),
                Some("SutonnyMJ")
            );
        } else {
            panic!("expected Empty: {event:?}");
        }
    }

    #[test]
    fn font_from_rfonts_without_theme_yields_none_for_theme_only() {
        let xml = r#"<w:rFonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:asciiTheme="minorHAnsi"/>"#;
        let mut reader = quick_xml::Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let event = reader.read_event_into(&mut buf).unwrap();
        if let Event::Empty(b) = event {
            assert!(font_from_rfonts(&b, None).is_none());
        }
    }
}
