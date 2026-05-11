//! Parse `word/styles.xml` and expose font-cascade resolution.
//!
//! OOXML font resolution for a run:
//! 1. Run-level `<w:rPr>/<w:rFonts>` (handled by the run extractor in `lib.rs`).
//! 2. Paragraph-level run defaults: `<w:p>/<w:pPr>/<w:rPr>/<w:rFonts>`.
//! 3. Paragraph style: `<w:p>/<w:pPr>/<w:pStyle w:val="X"/>` — look up `X` in
//!    this stylesheet and walk its `basedOn` chain.
//! 4. The default paragraph style (the one with `w:default="1"` and
//!    `w:type="paragraph"`), typically `Normal`.
//! 5. `<w:docDefaults>/<w:rPrDefault>/<w:rPr>/<w:rFonts>`.
//!
//! All other style types (character, table, numbering) are ignored in v0.2;
//! they could be added to `Style` later by recording `w:type`.

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;

/// One named style: its font and its `basedOn` parent.
#[derive(Debug, Default, Clone)]
pub struct Style {
    pub based_on: Option<String>,
    pub font: Option<String>,
}

/// Parsed `word/styles.xml`.
#[derive(Debug, Default, Clone)]
pub struct Stylesheet {
    /// `<w:docDefaults>/<w:rPrDefault>/<w:rPr>/<w:rFonts>` font.
    pub default_font: Option<String>,
    /// All declared styles, keyed by `w:styleId`.
    pub styles: HashMap<String, Style>,
    /// The `w:styleId` of the paragraph style flagged `w:default="1"` and
    /// `w:type="paragraph"` (typically `"Normal"`).
    pub default_paragraph_style: Option<String>,
}

impl Stylesheet {
    /// Walk the `basedOn` chain for `style_id` and return the first font we
    /// find. Cycle-guarded.
    pub fn resolve_style_font(&self, style_id: &str) -> Option<&str> {
        let mut current = Some(style_id);
        for _ in 0..64 {
            let id = current?;
            let style = self.styles.get(id)?;
            if let Some(f) = style.font.as_deref() {
                return Some(f);
            }
            current = style.based_on.as_deref();
        }
        None
    }

    /// Resolve a run's font using the OOXML cascade. Returns the first
    /// non-`None` source.
    pub fn resolve_run_font<'a>(
        &'a self,
        run_font: Option<&'a str>,
        paragraph_run_default_font: Option<&'a str>,
        paragraph_style_id: Option<&str>,
    ) -> Option<&'a str> {
        if let Some(f) = run_font {
            return Some(f);
        }
        if let Some(f) = paragraph_run_default_font {
            return Some(f);
        }
        if let Some(id) = paragraph_style_id {
            if let Some(f) = self.resolve_style_font(id) {
                return Some(f);
            }
        }
        if let Some(id) = self.default_paragraph_style.as_deref() {
            if let Some(f) = self.resolve_style_font(id) {
                return Some(f);
            }
        }
        self.default_font.as_deref()
    }
}

/// Parse a `word/styles.xml` source string into a [`Stylesheet`].
///
/// Returns `Ok(Default::default())` if the input is empty or has no
/// recognized elements. Returns `Err` only on hard XML parse failure.
pub fn parse_styles(xml: &str) -> Result<Stylesheet> {
    if xml.trim().is_empty() {
        return Ok(Stylesheet::default());
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut sheet = Stylesheet::default();

    // Active style being parsed (None when we're outside a <w:style>).
    let mut current_style: Option<(String, Style, bool /* is_default_paragraph */)> = None;
    // Nesting depth inside <w:rPr>. We accept rFonts inside any rPr we're
    // currently parsing — caller context (in_doc_defaults vs in_style)
    // disambiguates the target.
    let mut in_rpr = 0usize;
    // True while inside <w:docDefaults>/<w:rPrDefault>.
    let mut in_doc_defaults_rpr_default = false;
    let mut doc_defaults_depth = 0usize;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => {
                let local = local_name(&event);
                match (&event, local.as_deref()) {
                    (Event::Start(_), Some("docDefaults")) => {
                        doc_defaults_depth += 1;
                    }
                    (Event::End(_), Some("docDefaults")) => {
                        doc_defaults_depth = doc_defaults_depth.saturating_sub(1);
                    }
                    (Event::Start(_), Some("rPrDefault")) if doc_defaults_depth > 0 => {
                        in_doc_defaults_rpr_default = true;
                    }
                    (Event::End(_), Some("rPrDefault")) => {
                        in_doc_defaults_rpr_default = false;
                    }
                    (Event::Start(b), Some("style")) => {
                        let style_id = attr(b, b"styleId").unwrap_or_default();
                        let ty = attr(b, b"type").unwrap_or_default();
                        let is_default = attr(b, b"default").as_deref() == Some("1");
                        current_style =
                            Some((style_id, Style::default(), is_default && ty == "paragraph"));
                    }
                    (Event::End(_), Some("style")) => {
                        if let Some((id, style, is_default_para)) = current_style.take() {
                            if !id.is_empty() {
                                if is_default_para && sheet.default_paragraph_style.is_none() {
                                    sheet.default_paragraph_style = Some(id.clone());
                                }
                                sheet.styles.insert(id, style);
                            }
                        }
                    }
                    (Event::Empty(b), Some("basedOn")) | (Event::Start(b), Some("basedOn")) => {
                        if let Some((_, style, _)) = current_style.as_mut() {
                            style.based_on = attr(b, b"val");
                        }
                    }
                    (Event::Start(_), Some("rPr")) => {
                        in_rpr += 1;
                    }
                    (Event::End(_), Some("rPr")) => {
                        in_rpr = in_rpr.saturating_sub(1);
                    }
                    (Event::Empty(b), Some("rFonts")) | (Event::Start(b), Some("rFonts"))
                        if in_rpr > 0 =>
                    {
                        let font = font_from_rfonts(b);
                        if in_doc_defaults_rpr_default && sheet.default_font.is_none() {
                            sheet.default_font = font;
                        } else if let Some((_, style, _)) = current_style.as_mut() {
                            if style.font.is_none() {
                                style.font = font;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => return Err(anyhow!("styles.xml parse error: {e}")),
        }
        buf.clear();
    }

    Ok(sheet)
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

fn attr(b: &BytesStart<'_>, want_local: &[u8]) -> Option<String> {
    for a in b.attributes().with_checks(false).flatten() {
        let key = a.key.into_inner();
        let local = local_name_bytes(key);
        if local == want_local {
            return a.unescape_value().ok().map(|v| v.into_owned());
        }
    }
    None
}

fn font_from_rfonts(b: &BytesStart<'_>) -> Option<String> {
    attr(b, b"ascii").or_else(|| attr(b, b"hAnsi"))
}

fn local_name_bytes(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/>
      </w:rPr>
    </w:rPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:styleId="Normal" w:default="1">
    <w:name w:val="Normal"/>
    <w:rPr>
      <w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/>
    </w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading1">
    <w:name w:val="heading 1"/>
    <w:basedOn w:val="Normal"/>
    <w:rPr>
      <w:rFonts w:ascii="Arial Bold"/>
    </w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BijoyBody">
    <w:name w:val="Bijoy Body"/>
    <w:basedOn w:val="Normal"/>
    <w:rPr>
      <w:rFonts w:ascii="SutonnyMJ" w:hAnsi="SutonnyMJ"/>
    </w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="UnstyledChild">
    <w:name w:val="Unstyled Child"/>
    <w:basedOn w:val="BijoyBody"/>
  </w:style>
</w:styles>"#;

    #[test]
    fn parses_doc_defaults() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(s.default_font.as_deref(), Some("Calibri"));
    }

    #[test]
    fn parses_default_paragraph_style_id() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(s.default_paragraph_style.as_deref(), Some("Normal"));
    }

    #[test]
    fn resolves_style_font_directly() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(s.resolve_style_font("Normal"), Some("Times New Roman"));
        assert_eq!(s.resolve_style_font("BijoyBody"), Some("SutonnyMJ"));
    }

    #[test]
    fn resolves_via_based_on_chain() {
        let s = parse_styles(STYLES_XML).unwrap();
        // UnstyledChild has no font of its own; based_on=BijoyBody resolves
        // to SutonnyMJ.
        assert_eq!(s.resolve_style_font("UnstyledChild"), Some("SutonnyMJ"));
    }

    #[test]
    fn cascade_run_font_wins() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(
            s.resolve_run_font(Some("Direct"), Some("Para"), Some("Normal")),
            Some("Direct")
        );
    }

    #[test]
    fn cascade_paragraph_default_runs_second() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(
            s.resolve_run_font(None, Some("Para"), Some("Normal")),
            Some("Para")
        );
    }

    #[test]
    fn cascade_paragraph_style_runs_third() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(
            s.resolve_run_font(None, None, Some("BijoyBody")),
            Some("SutonnyMJ")
        );
    }

    #[test]
    fn cascade_default_paragraph_style_runs_fourth() {
        let s = parse_styles(STYLES_XML).unwrap();
        assert_eq!(s.resolve_run_font(None, None, None), Some("Times New Roman"));
    }

    #[test]
    fn cascade_doc_defaults_run_last() {
        // Sheet with only docDefaults, no styles.
        let xml = r#"<?xml version="1.0"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:rFonts w:ascii="OnlyDefault"/>
      </w:rPr>
    </w:rPrDefault>
  </w:docDefaults>
</w:styles>"#;
        let s = parse_styles(xml).unwrap();
        assert_eq!(s.resolve_run_font(None, None, None), Some("OnlyDefault"));
    }

    #[test]
    fn empty_xml_yields_empty_sheet() {
        let s = parse_styles("").unwrap();
        assert!(s.styles.is_empty());
        assert!(s.default_font.is_none());
    }

    #[test]
    fn cycle_guard_does_not_loop() {
        // Manually inject a cyclic style and verify resolution terminates.
        let mut s = Stylesheet::default();
        s.styles.insert(
            "A".to_string(),
            Style { based_on: Some("B".to_string()), font: None },
        );
        s.styles.insert(
            "B".to_string(),
            Style { based_on: Some("A".to_string()), font: None },
        );
        assert_eq!(s.resolve_style_font("A"), None);
    }
}
