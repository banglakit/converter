//! Multi-pass transliterator (SDD §5.2).
//!
//! The pipeline is:
//! 1. `pre_normalize` (strip soft hyphens, NBSP→space, CRLF→LF).
//! 2. Longest-match Aho-Corasick replacement at quadgram, trigram, bigram,
//!    single_char levels.
//! 3. `ikar_swap` (move pre-base `ি`/`ী`/`ে`/`ৈ` after consonant cluster).
//! 4. `ekar_recombine` (fold `ে+া` → `ো`, `ে+ৗ` → `ৌ`).
//! 5. `reph_reorder` (move `র + ্` from post-cluster to pre-cluster).
//! 6. `ya_phala_zwj` (insert ZWJ in `র + ্ + য`).
//! 7. NFC normalization (also decomposes U+09DC/DD/DF as required by
//!    Unicode CompositionExclusions).

use crate::encoding::Encoding;
use crate::mapping::mapping_for;
use crate::normalize;

/// A single (Bijoy byte range → Unicode byte range) mapping in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanMapping {
    pub source_start: usize,
    pub source_end: usize,
    pub target_start: usize,
    pub target_end: usize,
}

/// Accumulated span map for one transliteration. Currently a coarse
/// whole-input span; per-token mapping is reserved for v0.2.0.
pub type SpanMap = Vec<SpanMapping>;

/// Transliterate an ANSI Bengali string to Unicode Bengali for the given
/// encoding family.
pub fn transliterate(input: &str, encoding: Encoding) -> String {
    let (out, _audit) = transliterate_with_audit(input, encoding);
    out
}

/// Transliterate and emit a [`SpanMap`] alongside the output.
pub fn transliterate_with_audit(input: &str, encoding: Encoding) -> (String, SpanMap) {
    let map = mapping_for(encoding);
    let mut s = normalize::pre_normalize(input);
    if let Some(a) = map.quadgrams.as_ref() {
        s = a.replace(&s);
    }
    if let Some(a) = map.trigrams.as_ref() {
        s = a.replace(&s);
    }
    if let Some(a) = map.bigrams.as_ref() {
        s = a.replace(&s);
    }
    if let Some(a) = map.single_char.as_ref() {
        s = a.replace(&s);
    }
    s = normalize::ikar_swap(&s);
    s = normalize::ekar_recombine(&s);
    s = normalize::reph_reorder(&s);
    s = normalize::ya_phala_zwj(&s);
    s = normalize::nfc(&s);

    let span = SpanMapping {
        source_start: 0,
        source_end: input.len(),
        target_start: 0,
        target_end: s.len(),
    };
    (s, vec![span])
}
