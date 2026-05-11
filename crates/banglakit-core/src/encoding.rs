//! Encoding-family enum and the data registry that bundles each family's
//! mapping table, font allowlist, and classifier features.

use std::str::FromStr;

/// An ANSI/ASCII Bengali encoding family.
///
/// v0.1.0 only ships [`Encoding::Bijoy`]. Future variants (e.g. Boishakhi,
/// Lekhoni) plug in via [`registry`] without touching the transliterator or
/// classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    Bijoy,
}

impl Default for Encoding {
    fn default() -> Self {
        Encoding::Bijoy
    }
}

impl Encoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Bijoy => "bijoy",
        }
    }
}

impl FromStr for Encoding {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "bijoy" | "sutonnymj" => Ok(Encoding::Bijoy),
            other => Err(format!("unknown encoding family: {other}")),
        }
    }
}

/// Static data for a single encoding family.
///
/// All fields are `&'static`. Each [`Encoding`] variant maps to exactly one
/// `EncodingRegistry` via [`registry`].
pub struct EncodingRegistry {
    pub name: &'static str,
    /// The Bijoy→Unicode mapping table, in the schema documented at
    /// `data/<family>/mapping.toml`.
    pub mapping_toml: &'static str,
    /// The ANSI font allowlist for this family.
    pub fonts_toml: &'static str,
    /// Distinctive characters used by the heuristic classifier.
    pub distinctive_chars: &'static [char],
    /// Bigram patterns whose presence is strong evidence for this encoding.
    pub bigram_patterns: &'static [&'static str],
}

const BIJOY_REGISTRY: EncodingRegistry = EncodingRegistry {
    name: "bijoy",
    mapping_toml: include_str!("../data/bijoy/mapping.toml"),
    fonts_toml: include_str!("../data/bijoy/fonts.toml"),
    // Common Bijoy vowel-sign / conjunct-marker glyphs that rarely appear in
    // English prose. See PRD FR-3 and SDD §4 Stage 4.
    distinctive_chars: &[
        '†', '‡', '•', '‰', 'Š', '‹', 'Œ', '—', '¯', '¦', '§', '©', 'ª', '«', '¬', '®', 'µ', '¶',
        '·', '¸', '»', '¼', '½', '¾', 'À', 'Á', 'Â', 'Ã', 'Ä', 'Å', 'Æ', 'Ç', 'È', 'É', 'Ê', 'Ë',
        'Ì', 'Ï', '×', 'Ø', 'Ù', 'Ú', 'Û', 'Ü', 'Þ', 'ß', 'à', 'á', 'â', 'ã', 'ä', 'å', 'ç', 'é',
        'ê', 'ë', 'ì', 'í', 'î', 'ï', 'ð', 'ò', 'ó', 'ô', 'õ', 'ö', '÷', 'ù', 'û', 'ü', 'ý', 'þ',
        'š', '›', '¡', '¢', '£', '¥',
    ],
    // High-signal bigrams: aa-kar (`v`), i-kar (`w`), e-kar (`‡`), each
    // attached to a consonant. These near-uniquely identify Bijoy because
    // English text doesn't put `v`, `w`, `‡` in the position of a vowel
    // sign following a consonant-letter. Source: PRD FR-3 plus the avro.py
    // mapping output (the `Kv`, `Mv`, `Pv`, `Zv`, ... bigrams are extremely
    // common in any Bijoy paragraph).
    bigram_patterns: &[
        // aa-kar (v) attached to common consonants
        "Av", "Bv", "Kv", "Mv", "Nv", "Pv", "Rv", "Sv", "Tv", "Yv", "Zv", "av", "bv", "cv", "dv",
        "ev", "fv", "gv", "hv", "iv", "jv", "kv", "lv", "mv", "nv", "pv", "qv",
        // i-kar (w) prefix
        "wK", "wM", "wP", "wR", "wZ", "wa", "wb", "wc", "wd", "we", "wf", "wg", "wh", "wi", "wj",
        "wk", "wl", "wm", "wn", "wp", "wq", "wQ",
        // e-kar (‡) prefix (high-byte glyph; appears as U+2021)
        "‡K", "‡L", "‡M", "‡N", "‡P", "‡Q", "‡R", "‡S", "‡T", "‡U", "‡V", "‡W", "‡X", "‡Y", "‡Z",
        "‡b", "‡c", "‡d", "‡e", "‡f", "‡g", "‡h", "‡i", "‡j", "‡k", "‡l", "‡m", "‡n", "‡p", "‡q",
        // a few common standalone particles
        "Ges", "Aviv", "Zviv", "wQj",
    ],
};

/// Look up the static data registry for an encoding family.
pub fn registry(encoding: Encoding) -> &'static EncodingRegistry {
    match encoding {
        Encoding::Bijoy => &BIJOY_REGISTRY,
    }
}
