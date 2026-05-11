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

#[derive(Debug, Deserialize)]
struct FontFamilyEntry {
    stem: String,
    omj: Option<String>,
    class: String,
}

#[derive(Debug, Deserialize)]
struct FontFamiliesFile {
    family: Vec<FontFamilyEntry>,
}

struct ResolvedFamily {
    stem: String,
    omj: Option<&'static str>,
    is_sans: bool,
}

const UNICODE_FONTS_TOML: &str = include_str!("../data/unicode_fonts.toml");
const FONT_FAMILIES_TOML: &str = include_str!("../data/bijoy/font_families.toml");

static BIJOY_FONTS: OnceCell<Vec<String>> = OnceCell::new();
static UNICODE_FONTS: OnceCell<Vec<String>> = OnceCell::new();
static FONT_FAMILIES: OnceCell<Vec<ResolvedFamily>> = OnceCell::new();

fn bijoy_fonts() -> &'static [String] {
    BIJOY_FONTS.get_or_init(|| {
        let raw: AnsiFontFile =
            toml::from_str(registry(Encoding::Bijoy).fonts_toml).expect("bijoy/fonts.toml parse");
        raw.ansi_fonts
            .into_iter()
            .map(|s| s.to_ascii_lowercase())
            .collect()
    })
}

fn unicode_fonts() -> &'static [String] {
    UNICODE_FONTS.get_or_init(|| {
        let raw: UnicodeFontFile =
            toml::from_str(UNICODE_FONTS_TOML).expect("unicode_fonts.toml parse");
        raw.unicode_fonts
            .into_iter()
            .map(|s| s.to_ascii_lowercase())
            .collect()
    })
}

/// Strip the typical 6-letter subset prefix that PDF embedded fonts carry
/// (e.g. `ABCDEF+SutonnyMJ` → `SutonnyMJ`). Idempotent on names without a
/// prefix.
pub fn strip_subset_prefix(name: &str) -> &str {
    match name.split_once('+') {
        Some((prefix, rest))
            if prefix.len() == 6 && prefix.chars().all(|c| c.is_ascii_uppercase()) =>
        {
            rest
        }
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

fn font_families() -> &'static [ResolvedFamily] {
    FONT_FAMILIES.get_or_init(|| {
        let raw: FontFamiliesFile =
            toml::from_str(FONT_FAMILIES_TOML).expect("bijoy/font_families.toml parse");
        raw.family
            .into_iter()
            .map(|e| {
                // Leak the OMJ string so we can return &'static str from lookups.
                let omj: Option<&'static str> = e.omj.map(|s| &*s.leak());
                ResolvedFamily {
                    stem: e.stem.to_ascii_lowercase(),
                    omj,
                    is_sans: e.class == "sans",
                }
            })
            .collect()
    })
}

/// Known MJ-style suffixes to strip when extracting the family stem.
const MJ_SUFFIXES: &[&str] = &[
    "sushreemj",
    "matramj",
    "sreemj",
    "xmj",
    "cmj",
    "emj",
    "omj",
    "mj",
];

/// Extract the family stem from a Bijoy font name by stripping subset prefix,
/// weight/style suffixes, and MJ-variant suffixes.
fn extract_stem(font_name: &str) -> String {
    let name = strip_subset_prefix(font_name);
    let lower = name.to_ascii_lowercase();

    // Strip trailing weight/style modifiers (e.g. "-Bold", " Italic", "-BoldItalic")
    let base = lower
        .trim_end_matches(" bold")
        .trim_end_matches(" italic")
        .trim_end_matches(" bolditalic")
        .trim_end_matches("-bold")
        .trim_end_matches("-italic")
        .trim_end_matches("-bolditalic");

    // Strip MJ-variant suffix (longest first to avoid partial matches)
    for suffix in MJ_SUFFIXES {
        if let Some(stem) = base.strip_suffix(suffix) {
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    base.to_string()
}

/// Resolve the target Unicode font for a Bijoy ANSI font name.
///
/// Returns the OMJ family name if one exists for this font's family,
/// otherwise returns a serif/sans-serif-aware fallback:
/// - `"Kalpurush"` for serif families
/// - `"Siyam Rupali"` for sans-serif families
///
/// Returns `None` if the font stem is not recognized at all, letting the
/// caller fall back to `ConvertOptions::unicode_font`.
pub fn resolve_matched_font(font_name: &str, _encoding: Encoding) -> Option<&'static str> {
    let stem = extract_stem(font_name);
    let families = font_families();

    for fam in families {
        if stem == fam.stem {
            return Some(match fam.omj {
                Some(omj) => omj,
                None if fam.is_sans => "Siyam Rupali",
                None => "Kalpurush",
            });
        }
    }
    None
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

    #[test]
    fn resolve_sutonny_mj_to_omj() {
        assert_eq!(
            resolve_matched_font("SutonnyMJ", Encoding::Bijoy),
            Some("SutonnyOMJ")
        );
    }

    #[test]
    fn resolve_sutonny_xmj_bold_to_omj() {
        assert_eq!(
            resolve_matched_font("SutonnyXMJ-Bold", Encoding::Bijoy),
            Some("SutonnyOMJ")
        );
    }

    #[test]
    fn resolve_jajadi_sans_to_omj() {
        assert_eq!(
            resolve_matched_font("JaJaDiMJ", Encoding::Bijoy),
            Some("JaJaDiOMJ")
        );
    }

    #[test]
    fn resolve_ananda_no_omj_serif_fallback() {
        assert_eq!(
            resolve_matched_font("AnandaMJ", Encoding::Bijoy),
            Some("Kalpurush")
        );
    }

    #[test]
    fn resolve_unknown_font_returns_none() {
        assert_eq!(resolve_matched_font("Arial", Encoding::Bijoy), None);
    }

    #[test]
    fn resolve_with_subset_prefix() {
        assert_eq!(
            resolve_matched_font("ABCDEF+SutonnyMJ", Encoding::Bijoy),
            Some("SutonnyOMJ")
        );
    }
}
