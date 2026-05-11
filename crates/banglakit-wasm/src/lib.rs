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
    classify, convert_run, transliterate, ConvertOptions, DefaultRunVisitor, Encoding, Mode,
};
use serde::Serialize;
use std::str::FromStr;
use wasm_bindgen::prelude::*;

fn file_result_to_js(bytes: Vec<u8>, any_change: bool, runs_converted: usize) -> JsValue {
    let obj = js_sys::Object::new();
    let ua = js_sys::Uint8Array::from(bytes.as_slice());
    js_sys::Reflect::set(&obj, &"bytes".into(), &ua).unwrap();
    js_sys::Reflect::set(&obj, &"anyChange".into(), &any_change.into()).unwrap();
    js_sys::Reflect::set(&obj, &"runsConverted".into(), &runs_converted.into()).unwrap();
    obj.into()
}

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
    suggested_font: Option<String>,
}

fn parse_mode(mode: Option<String>) -> Result<Mode, String> {
    Mode::from_str(mode.as_deref().unwrap_or("safe"))
}

fn parse_encoding(encoding: Option<String>) -> Result<Encoding, String> {
    Encoding::from_str(encoding.as_deref().unwrap_or("bijoy"))
}

fn parse_unicode_font(name: Option<String>) -> Result<&'static str, String> {
    // Closed allowlist that returns `&'static str` so `ConvertOptions.unicode_font`
    // can borrow it without forcing the caller to keep the string alive. Lives
    // in this crate (rather than `banglakit-core`) because the lifetime
    // contract is host-specific.
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
    let out = ClassificationJs {
        decision: c.decision.as_str(),
        encoding: c.decision.encoding().map(Encoding::as_str),
        stage: c.stage.as_str(),
        confidence: c.confidence,
        signals: c
            .signals
            .into_iter()
            .map(|s| SignalJs {
                name: s.name,
                value: s.value,
            })
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
    auto_match_fonts: Option<bool>,
) -> Result<JsValue, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    let m = parse_mode(mode).map_err(|e| JsError::new(&e))?;
    let target_font = parse_unicode_font(unicode_font).map_err(|e| JsError::new(&e))?;

    let opts = ConvertOptions {
        encoding: enc,
        mode: m,
        threshold: None,
        unicode_font: target_font,
        auto_match_fonts: auto_match_fonts.unwrap_or(false),
    };
    let r = convert_run(text, font_name.as_deref(), &opts);
    let suggested_font: Option<String> = if r.changed {
        Some(r.font.unwrap_or(target_font).to_string())
    } else {
        None
    };

    let out = ConversionJs {
        text: r.text,
        changed: r.changed,
        decision: r.classification.decision.as_str(),
        encoding: r.classification.decision.encoding().map(Encoding::as_str),
        stage: r.classification.stage.as_str(),
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
struct TextConversionJs {
    text: String,
    changed: bool,
    runs_converted: usize,
}

fn build_opts(
    encoding: Option<String>,
    mode: Option<String>,
    unicode_font: Option<String>,
    auto_match_fonts: Option<bool>,
) -> Result<ConvertOptions<'static>, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    let m = parse_mode(mode).map_err(|e| JsError::new(&e))?;
    let target_font = parse_unicode_font(unicode_font).map_err(|e| JsError::new(&e))?;
    Ok(ConvertOptions {
        encoding: enc,
        mode: m,
        threshold: None,
        unicode_font: target_font,
        auto_match_fonts: auto_match_fonts.unwrap_or(false),
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
    auto_match_fonts: Option<bool>,
) -> Result<JsValue, JsError> {
    let opts = build_opts(encoding, mode, unicode_font, auto_match_fonts)?;
    let mut visitor = DefaultRunVisitor::new(opts);
    let out = banglakit_docx::process_docx_bytes(bytes, &mut visitor)
        .map_err(|e| JsError::new(&format!("{e:#}")))?;
    Ok(file_result_to_js(
        out,
        visitor.any_change,
        visitor.runs_converted,
    ))
}

/// Convert an in-memory `.pptx` deck. Same return shape as [`convert_docx`].
#[wasm_bindgen(js_name = convertPptx)]
pub fn convert_pptx(
    bytes: &[u8],
    mode: Option<String>,
    encoding: Option<String>,
    unicode_font: Option<String>,
    auto_match_fonts: Option<bool>,
) -> Result<JsValue, JsError> {
    let opts = build_opts(encoding, mode, unicode_font, auto_match_fonts)?;
    let mut visitor = DefaultRunVisitor::new(opts);
    let out = banglakit_pptx::process_pptx_bytes(bytes, &mut visitor)
        .map_err(|e| JsError::new(&format!("{e:#}")))?;
    Ok(file_result_to_js(
        out,
        visitor.any_change,
        visitor.runs_converted,
    ))
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
    auto_match_fonts: Option<bool>,
) -> Result<JsValue, JsError> {
    let opts = build_opts(encoding, mode, unicode_font, auto_match_fonts)?;
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
