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

use banglakit_core::{classify, transliterate, Decision, Encoding, Mode, Stage};
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
pub fn convert_run(
    text: &str,
    font_name: Option<String>,
    encoding: Option<String>,
    mode: Option<String>,
    unicode_font: Option<String>,
) -> Result<JsValue, JsError> {
    let enc = parse_encoding(encoding).map_err(|e| JsError::new(&e))?;
    let m = parse_mode(mode).map_err(|e| JsError::new(&e))?;
    let target_font = parse_unicode_font(unicode_font).map_err(|e| JsError::new(&e))?;

    let c = classify(text, font_name.as_deref(), enc, m);
    let (decision, decoded_encoding) = decision_label(c.decision);
    let stage = stage_label(c.stage);

    let should_convert = matches!(c.decision, Decision::AnsiBengali(_))
        || (matches!(c.decision, Decision::Ambiguous) && matches!(m, Mode::Aggressive));

    let (out_text, changed, suggested_font) = if should_convert {
        (transliterate(text, enc), true, Some(target_font))
    } else {
        (text.to_string(), false, None)
    };

    let out = ConversionJs {
        text: out_text,
        changed,
        decision,
        encoding: decoded_encoding,
        stage,
        confidence: c.confidence,
        suggested_font,
    };
    serde_wasm_bindgen::to_value(&out).map_err(JsError::from)
}

/// Version of the bundled core, useful for cache-busting on the JS side.
#[wasm_bindgen(js_name = coreVersion)]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
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
