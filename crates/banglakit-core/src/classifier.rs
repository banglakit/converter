//! Multi-stage classifier (SDD §4).
//!
//! Stages, short-circuiting in order:
//! 1. **Unicode range pre-check** — any codepoint in U+0980–U+09FF means the
//!    run is already Unicode Bengali; skip.
//! 2. **ANSI font allowlist** — font name matches a Bijoy-family font, e.g.
//!    SutonnyMJ, AdorshoLipi, or any `*MJ`-suffix variant.
//! 3. **Unicode font allowlist** — font is Kalpurush / Noto / SolaimanLipi
//!    etc.; skip.
//! 4. **Heuristic scorer** — five lightweight features combined into a
//!    sigmoid probability.
//! 5. **Threshold policy** — `safe` requires ≥ 0.95 to auto-convert,
//!    `aggressive` requires ≥ 0.85. Below 0.50 is `Latin`. Between thresholds
//!    is `Ambiguous`.

use once_cell::sync::Lazy;
use regex::Regex;
use std::str::FromStr;

use crate::encoding::{registry, Encoding};
use crate::english;
use crate::fonts;

/// Operator-selected confidence policy. Mirrors PRD FR-5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Safe,
    Aggressive,
}

impl Mode {
    pub fn default_threshold(self) -> f32 {
        match self {
            Mode::Safe => 0.95,
            Mode::Aggressive => 0.85,
        }
    }
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "safe" => Ok(Mode::Safe),
            "aggressive" => Ok(Mode::Aggressive),
            other => Err(format!(
                "unknown mode: {other:?}; expected \"safe\" or \"aggressive\""
            )),
        }
    }
}

/// Final classification decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AnsiBengali(Encoding),
    UnicodeBengali,
    Latin,
    Ambiguous,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::AnsiBengali(_) => "ansi_bengali",
            Decision::UnicodeBengali => "unicode_bengali",
            Decision::Latin => "latin",
            Decision::Ambiguous => "ambiguous",
        }
    }

    /// The detected encoding family when this decision is `AnsiBengali`.
    pub fn encoding(self) -> Option<Encoding> {
        match self {
            Decision::AnsiBengali(e) => Some(e),
            _ => None,
        }
    }
}

/// Which classifier stage produced the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    UnicodeRange,
    AnsiFont,
    UnicodeFont,
    Heuristic,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::UnicodeRange => "unicode_range",
            Stage::AnsiFont => "ansi_font",
            Stage::UnicodeFont => "unicode_font",
            Stage::Heuristic => "heuristic",
        }
    }
}

/// Per-feature signal, useful for `--explain`.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub name: &'static str,
    pub value: f32,
}

/// A classifier result.
#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    pub decision: Decision,
    pub confidence: f32,
    pub stage: Stage,
    pub signals: Vec<Signal>,
}

const W_HIGH_BYTE: f32 = 8.0;
const W_DISTINCTIVE: f32 = 4.0;
const W_BIGRAM: f32 = 6.0;
const W_ENGLISH: f32 = 6.0;
const BIAS: f32 = -1.0;

/// Run the five-stage classifier on `text` with optional `font_hint`.
pub fn classify(
    text: &str,
    font_hint: Option<&str>,
    encoding: Encoding,
    mode: Mode,
) -> Classification {
    if text.is_empty() {
        return Classification {
            decision: Decision::Latin,
            confidence: 0.0,
            stage: Stage::Heuristic,
            signals: vec![],
        };
    }

    // Stage 1: Unicode-range pre-check.
    if has_bengali_unicode(text) {
        return Classification {
            decision: Decision::UnicodeBengali,
            confidence: 1.0,
            stage: Stage::UnicodeRange,
            signals: vec![],
        };
    }

    // Stage 2: ANSI font allowlist.
    if let Some(name) = font_hint {
        if fonts::is_ansi_font(name, encoding) {
            return Classification {
                decision: Decision::AnsiBengali(encoding),
                confidence: 0.99,
                stage: Stage::AnsiFont,
                signals: vec![],
            };
        }
        // Stage 3: Unicode font allowlist.
        if fonts::is_unicode_bengali_font(name) {
            return Classification {
                decision: Decision::UnicodeBengali,
                confidence: 0.99,
                stage: Stage::UnicodeFont,
                signals: vec![],
            };
        }
    }

    // Stage 4: heuristic scorer.
    let signals = heuristic_features(text, encoding);
    let logit = signals.iter().fold(BIAS, |acc, s| acc + s.value * weight_for(s.name));
    let p = sigmoid(logit);

    // Stage 5: threshold policy.
    let threshold = mode.default_threshold();
    let decision = if p >= threshold {
        Decision::AnsiBengali(encoding)
    } else if p < 0.50 {
        Decision::Latin
    } else {
        Decision::Ambiguous
    };

    Classification {
        decision,
        confidence: p,
        stage: Stage::Heuristic,
        signals,
    }
}

fn has_bengali_unicode(text: &str) -> bool {
    text.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c))
}

fn weight_for(name: &str) -> f32 {
    match name {
        "high_byte_density" => W_HIGH_BYTE,
        "distinctive_chars" => W_DISTINCTIVE,
        "bigram_hits" => W_BIGRAM,
        "english_coverage" => -W_ENGLISH,
        _ => 0.0,
    }
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

fn heuristic_features(text: &str, encoding: Encoding) -> Vec<Signal> {
    let total = text.chars().count().max(1) as f32;
    let high_byte = text
        .chars()
        .filter(|c| {
            let cp = *c as u32;
            (0x80..=0xFF).contains(&cp)
        })
        .count() as f32
        / total;

    let reg = registry(encoding);
    let distinctive = text
        .chars()
        .filter(|c| reg.distinctive_chars.contains(c))
        .count() as f32
        / total;

    let bigram = bigram_score(text, encoding);
    let english = english_coverage(text);

    vec![
        Signal { name: "high_byte_density", value: high_byte },
        Signal { name: "distinctive_chars", value: distinctive },
        Signal { name: "bigram_hits", value: bigram },
        Signal { name: "english_coverage", value: english },
    ]
}

static DAGGER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\u{2020}\u{2021}][A-Za-z]").unwrap());

fn bigram_score(text: &str, encoding: Encoding) -> f32 {
    let reg = registry(encoding);
    let mut hits = 0usize;
    for pat in reg.bigram_patterns {
        // Count non-overlapping occurrences.
        let mut start = 0;
        while let Some(idx) = text[start..].find(pat) {
            hits += 1;
            start += idx + pat.len();
        }
    }
    hits += DAGGER_RE.find_iter(text).count();
    let word_count = text.split_ascii_whitespace().count().max(1) as f32;
    (hits as f32 / word_count).min(1.0)
}

fn english_coverage(text: &str) -> f32 {
    let tokens = english::tokenize(text);
    if tokens.is_empty() {
        return 0.0;
    }
    let hits = tokens.iter().filter(|t| english::contains(t)).count();
    hits as f32 / tokens.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_range_short_circuits() {
        let c = classify("আমি বাংলায়", None, Encoding::Bijoy, Mode::Safe);
        assert!(matches!(c.decision, Decision::UnicodeBengali));
        assert_eq!(c.stage, Stage::UnicodeRange);
    }

    #[test]
    fn ansi_font_short_circuits() {
        let c = classify("hello", Some("SutonnyMJ"), Encoding::Bijoy, Mode::Safe);
        assert!(matches!(c.decision, Decision::AnsiBengali(Encoding::Bijoy)));
        assert_eq!(c.stage, Stage::AnsiFont);
    }

    #[test]
    fn unicode_font_short_circuits() {
        let c = classify("hello", Some("Kalpurush"), Encoding::Bijoy, Mode::Safe);
        assert!(matches!(c.decision, Decision::UnicodeBengali));
        assert_eq!(c.stage, Stage::UnicodeFont);
    }

    #[test]
    fn english_prose_classifies_latin() {
        for s in [
            "Gas price is $5 today",
            "Figure 1: GDP Growth 2023",
            "The quick brown fox jumps over the lazy dog",
            "Source: World Bank annual report",
        ] {
            let c = classify(s, None, Encoding::Bijoy, Mode::Safe);
            assert!(
                matches!(c.decision, Decision::Latin),
                "expected Latin for {s:?}, got {:?} p={}",
                c.decision,
                c.confidence
            );
            assert!(c.confidence < 0.20, "p={} for {s:?}", c.confidence);
        }
    }

    #[test]
    fn bijoy_prose_classifies_ansi() {
        let bijoy = "Avwg evsjvq Mvb MvB|";
        let c = classify(bijoy, None, Encoding::Bijoy, Mode::Safe);
        assert!(
            matches!(c.decision, Decision::AnsiBengali(_) | Decision::Ambiguous),
            "got {:?} p={}",
            c.decision,
            c.confidence
        );
        assert!(c.confidence > 0.50, "p={} too low for Bijoy text", c.confidence);
    }

    #[test]
    fn stage_as_str_matches_audit_log_contract() {
        assert_eq!(Stage::UnicodeRange.as_str(), "unicode_range");
        assert_eq!(Stage::AnsiFont.as_str(), "ansi_font");
        assert_eq!(Stage::UnicodeFont.as_str(), "unicode_font");
        assert_eq!(Stage::Heuristic.as_str(), "heuristic");
    }

    #[test]
    fn decision_as_str_and_encoding() {
        assert_eq!(Decision::AnsiBengali(Encoding::Bijoy).as_str(), "ansi_bengali");
        assert_eq!(Decision::UnicodeBengali.as_str(), "unicode_bengali");
        assert_eq!(Decision::Latin.as_str(), "latin");
        assert_eq!(Decision::Ambiguous.as_str(), "ambiguous");
        assert_eq!(
            Decision::AnsiBengali(Encoding::Bijoy).encoding(),
            Some(Encoding::Bijoy)
        );
        assert_eq!(Decision::Latin.encoding(), None);
    }

    #[test]
    fn mode_from_str_round_trips() {
        assert_eq!("safe".parse::<Mode>().unwrap(), Mode::Safe);
        assert_eq!("AGGRESSIVE".parse::<Mode>().unwrap(), Mode::Aggressive);
        assert!("yolo".parse::<Mode>().is_err());
    }
}
