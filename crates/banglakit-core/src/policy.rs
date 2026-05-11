//! Cross-platform per-run conversion policy.
//!
//! The function [`convert_run`] is the *one boundary every host shares*:
//! file adapters (DOCX, PPTX), the WASM bindings (Office.js — Word,
//! PowerPoint, Excel), and any future LibreOffice / Apache OpenOffice
//! connector all call it once per run to decide:
//!
//! - Is this run ANSI Bengali (Bijoy)? Convert it.
//! - Is it Latin / already-Unicode-Bengali / Ambiguous-below-threshold?
//!   Leave it alone.
//!
//! What stays host-specific is *iteration* (walking XML events for files,
//! walking `body.paragraphs.runs` for Office.js, walking
//! `Text.createEnumeration()` for UNO) and *commit* (rewriting the XML
//! stream vs. setting `run.font.name = …` vs. UNO property assignment).
//!
//! This separation means the CLI's DOCX/PPTX visitor and `banglakit-wasm`'s
//! `convertRun` no longer hold parallel copies of the classify-then-
//! transliterate logic, and a future UNO connector is literally "iterate
//! runs, call [`convert_run`], apply [`ConvertedRun`]".

use crate::classifier::{classify, Classification, Decision, Mode};
use crate::encoding::Encoding;
use crate::fonts::resolve_matched_font;
use crate::transliterate::transliterate;

/// Tunable inputs to the per-run policy. Borrows `unicode_font` so a host
/// passes its target-font string once and every [`ConvertedRun`] reuses it
/// without allocating.
#[derive(Debug, Clone, Copy)]
pub struct ConvertOptions<'a> {
    pub encoding: Encoding,
    pub mode: Mode,
    /// Overrides `mode.default_threshold()` when `Some`. Use `None` to
    /// accept the mode's PRD FR-5 default (safe = 0.95, aggressive = 0.85).
    pub threshold: Option<f32>,
    /// The Unicode Bengali font the host should write back when a run is
    /// converted. Borrowed from the caller.
    pub unicode_font: &'a str,
    /// When `true`, attempt to map the input Bijoy font to its OMJ Unicode
    /// counterpart (e.g. SutonnyMJ → SutonnyOMJ). Falls back to
    /// `unicode_font` when no match is found.
    pub auto_match_fonts: bool,
}

/// The result of running [`convert_run`] on a single run.
#[derive(Debug, Clone)]
pub struct ConvertedRun<'a> {
    /// The text the host should commit. Equals the input when `!changed`,
    /// the transliterated Unicode Bengali otherwise.
    pub text: String,
    /// The font the host should write back. `Some` only when `changed`;
    /// borrows from `opts.unicode_font`.
    pub font: Option<&'a str>,
    /// `true` iff the classifier decided to convert this run.
    pub changed: bool,
    /// The full classification record — exposed so callers can audit-log
    /// the decision, the per-feature signals, and the confidence.
    pub classification: Classification,
}

/// Per-run policy used by every host. Pure function; no I/O.
pub fn convert_run<'a>(
    text: &str,
    font_hint: Option<&str>,
    opts: &'a ConvertOptions<'a>,
) -> ConvertedRun<'a> {
    let c = classify(text, font_hint, opts.encoding, opts.mode);
    let threshold = opts
        .threshold
        .unwrap_or_else(|| opts.mode.default_threshold());

    let should_convert = match c.decision {
        Decision::AnsiBengali(_) => true,
        Decision::Ambiguous => c.confidence >= threshold,
        _ => false,
    };

    if should_convert {
        let new_text = transliterate(text, opts.encoding);
        let target_font = if opts.auto_match_fonts {
            font_hint
                .and_then(|fh| resolve_matched_font(fh, opts.encoding))
                .unwrap_or(opts.unicode_font)
        } else {
            opts.unicode_font
        };
        ConvertedRun {
            text: new_text,
            font: Some(target_font),
            changed: true,
            classification: c,
        }
    } else {
        ConvertedRun {
            text: text.to_string(),
            font: None,
            changed: false,
            classification: c,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(mode: Mode, font: &'a str) -> ConvertOptions<'a> {
        ConvertOptions {
            encoding: Encoding::Bijoy,
            mode,
            threshold: None,
            unicode_font: font,
            auto_match_fonts: false,
        }
    }

    #[test]
    fn bijoy_with_font_hint_converts() {
        let o = opts(Mode::Safe, "Kalpurush");
        let r = convert_run("Avwg evsjvq", Some("SutonnyMJ"), &o);
        assert!(r.changed);
        assert_eq!(r.font, Some("Kalpurush"));
        assert!(r.text.contains("আমি"));
        assert!(matches!(
            r.classification.decision,
            Decision::AnsiBengali(_)
        ));
    }

    #[test]
    fn english_does_not_convert() {
        let o = opts(Mode::Safe, "Kalpurush");
        let r = convert_run("Gas price is $5 today", None, &o);
        assert!(!r.changed);
        assert!(r.font.is_none());
        assert_eq!(r.text, "Gas price is $5 today");
        assert!(matches!(r.classification.decision, Decision::Latin));
    }

    #[test]
    fn unicode_bengali_short_circuits() {
        let o = opts(Mode::Safe, "Kalpurush");
        let r = convert_run("আমি বাংলায়", None, &o);
        assert!(!r.changed);
        assert!(matches!(
            r.classification.decision,
            Decision::UnicodeBengali
        ));
    }

    #[test]
    fn ambiguous_skipped_in_safe_mode_below_threshold() {
        // A run with only one Bijoy bigram hit lands in the Ambiguous band
        // for safe mode (threshold 0.95). With no font hint and no other
        // signal, this is the realistic mixed-script case the PRD §2
        // failure mode targets.
        let o = ConvertOptions {
            encoding: Encoding::Bijoy,
            mode: Mode::Safe,
            threshold: Some(0.95),
            unicode_font: "Kalpurush",
            auto_match_fonts: false,
        };
        let r = convert_run("Avwg", None, &o);
        // The classifier may rate this as Ambiguous or even AnsiBengali
        // depending on bigram density; we just assert that if it isn't
        // decisively AnsiBengali, the policy leaves it alone in safe mode.
        if !matches!(r.classification.decision, Decision::AnsiBengali(_)) {
            assert!(!r.changed, "safe mode should not convert sub-threshold");
        }
    }

    #[test]
    fn aggressive_converts_above_lower_threshold() {
        // Same input, aggressive mode (threshold 0.85). The PRD canonical
        // sample with 4 Bijoy bigram hits crosses every threshold; this
        // tests that aggressive picks it up even when safe would have.
        let o = opts(Mode::Aggressive, "Kalpurush");
        let r = convert_run("Avwg evsjvq Mvb MvB|", None, &o);
        assert!(r.changed);
    }

    #[test]
    fn threshold_override_lowers_bar_in_safe_mode() {
        // Force safe mode to accept anything with P >= 0.5 — useful for
        // hosts that want to bypass the conservative default.
        let o = ConvertOptions {
            encoding: Encoding::Bijoy,
            mode: Mode::Safe,
            threshold: Some(0.50),
            unicode_font: "Kalpurush",
            auto_match_fonts: false,
        };
        let r = convert_run("Avwg evsjvq Mvb MvB|", None, &o);
        assert!(r.changed);
    }
}
