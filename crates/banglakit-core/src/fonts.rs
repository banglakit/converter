//! Font-name allowlist matching. Case-insensitive substring against curated
//! lists per encoding family; an `*MJ`-suffix fallback rule catches the long
//! tail of Bijoy-family variants without exhaustive enumeration.

use once_cell::sync::OnceCell;
use serde::Deserialize;

use crate::encoding::{registry, Encoding};

#[derive(Debug, Deserialize)]
struct AnsiFontFile {
    ansi_fonts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UnicodeFontFile {
    unicode_fonts: Vec<String>,
}

const UNICODE_FONTS_TOML: &str = include_str!("../data/unicode_fonts.toml");

static BIJOY_FONTS: OnceCell<Vec<String>> = OnceCell::new();
static UNICODE_FONTS: OnceCell<Vec<String>> = OnceCell::new();

fn bijoy_fonts() -> &'static [String] {
    BIJOY_FONTS.get_or_init(|| {
        let raw: AnsiFontFile = toml::from_str(registry(Encoding::Bijoy).fonts_toml)
            .expect("bijoy/fonts.toml parse");
        raw.ansi_fonts.into_iter().map(|s| s.to_ascii_lowercase()).collect()
    })
}

fn unicode_fonts() -> &'static [String] {
    UNICODE_FONTS.get_or_init(|| {
        let raw: UnicodeFontFile = toml::from_str(UNICODE_FONTS_TOML)
            .expect("unicode_fonts.toml parse");
        raw.unicode_fonts.into_iter().map(|s| s.to_ascii_lowercase()).collect()
    })
}

/// Strip the typical 6-letter subset prefix that PDF embedded fonts carry
/// (e.g. `ABCDEF+SutonnyMJ` → `SutonnyMJ`). Idempotent on names without a
/// prefix.
pub fn strip_subset_prefix(name: &str) -> &str {
    match name.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 && prefix.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

/// Return true if `font_name` is on the ANSI Bengali allowlist for the
/// given encoding family. Substring match is case-insensitive. The
/// `*MJ`-suffix fallback applies only to [`Encoding::Bijoy`].
pub fn is_ansi_font(font_name: &str, encoding: Encoding) -> bool {
    let name = strip_subset_prefix(font_name).to_ascii_lowercase();
    let list = match encoding {
        Encoding::Bijoy => bijoy_fonts(),
    };
    if list.iter().any(|f| name.contains(f.as_str())) {
        return true;
    }
    if matches!(encoding, Encoding::Bijoy) && name.ends_with("mj") {
        return true;
    }
    false
}

/// Return true if `font_name` is a known Unicode Bengali font. Such runs
/// already render Unicode correctly and must not be re-converted.
pub fn is_unicode_bengali_font(font_name: &str) -> bool {
    let name = strip_subset_prefix(font_name).to_ascii_lowercase();
    unicode_fonts().iter().any(|f| name.contains(f.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_font_matches_named() {
        assert!(is_ansi_font("SutonnyMJ", Encoding::Bijoy));
        assert!(is_ansi_font("sutonnyomj", Encoding::Bijoy));
        assert!(is_ansi_font("ABCDEF+SutonnyMJ", Encoding::Bijoy));
        assert!(is_ansi_font("NikoshMJ", Encoding::Bijoy));
    }

    #[test]
    fn ansi_font_mj_suffix_fallback() {
        assert!(is_ansi_font("RandomUnknownMJ", Encoding::Bijoy));
    }

    #[test]
    fn ansi_font_rejects_unicode_and_latin() {
        // Kalpurush ANSI variant is not currently handled separately: the
        // name resolution here returns it as "not ANSI" because the
        // *MJ-suffix rule does not catch it. A v0.2.0 enhancement would
        // disambiguate by inspecting run text content.
        assert!(!is_ansi_font("Arial", Encoding::Bijoy));
        assert!(!is_ansi_font("Times New Roman", Encoding::Bijoy));
        assert!(!is_ansi_font("Calibri", Encoding::Bijoy));
    }

    #[test]
    fn unicode_font_allowlist() {
        assert!(is_unicode_bengali_font("Kalpurush"));
        assert!(is_unicode_bengali_font("Nikosh"));
        assert!(is_unicode_bengali_font("Noto Sans Bengali"));
        assert!(!is_unicode_bengali_font("SutonnyMJ"));
    }
}
