//! Post-substitution Unicode normalization passes (SDD §5.2 steps 6–11),
//! adapted to match the actual Bijoy byte-ordering convention used by
//! existing converters (avro.py `rearrange_bijoy_text`, MIT) and the
//! Unicode encoding rules for reph (L2/2003/03233).
//!
//! Each pass is a pure `&str -> String` transformation and runs in the
//! order documented by [`crate::transliterate`].

use unicode_normalization::UnicodeNormalization;

const REPH_LEAD_RA: char = '\u{09B0}'; // র
const HASANTA: char = '\u{09CD}'; // ্
const YA: char = '\u{09AF}'; // য
const ZWJ: char = '\u{200D}';
const ZWNJ: char = '\u{200C}';

const I_KAR: char = '\u{09BF}'; // ি
const II_KAR: char = '\u{09C0}'; // ী
const E_KAR: char = '\u{09C7}'; // ে
const AI_KAR: char = '\u{09C8}'; // ৈ
const AA_KAR: char = '\u{09BE}'; // া
const TAIL_AU_KAR: char = '\u{09D7}'; // ৗ
const O_KAR: char = '\u{09CB}'; // ো
const AU_KAR: char = '\u{09CC}'; // ৌ

#[allow(dead_code)]
const NUKTA: char = '\u{09BC}'; // ় — kept for future BrokenNukta safety pass

/// Strip soft hyphens, fold NBSP → space, collapse CRLF → LF.
pub fn pre_normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_cr = false;
    for c in input.chars() {
        match c {
            '\u{00AD}' => continue,
            '\u{00A0}' => out.push(' '),
            '\r' => {
                prev_cr = true;
                continue;
            }
            '\n' => {
                out.push('\n');
                prev_cr = false;
                continue;
            }
            other => {
                if prev_cr {
                    out.push('\n');
                }
                prev_cr = false;
                out.push(other);
            }
        }
    }
    if prev_cr {
        out.push('\n');
    }
    out
}

fn is_bengali_consonant(c: char) -> bool {
    matches!(c, '\u{0995}'..='\u{09B9}' | '\u{09DC}' | '\u{09DD}' | '\u{09DF}' | '\u{09CE}')
}

fn is_prekar(c: char) -> bool {
    matches!(c, I_KAR | II_KAR | E_KAR | AI_KAR)
}

fn is_kar(c: char) -> bool {
    matches!(
        c,
        AA_KAR | I_KAR | II_KAR | '\u{09C1}' | '\u{09C2}' | '\u{09C3}'
            | E_KAR | AI_KAR | O_KAR | AU_KAR | TAIL_AU_KAR
    )
}

/// Move reph (`র + ্`) from its post-cluster Bijoy byte position to the
/// Unicode logical position (before the cluster).
///
/// Bijoy keyboards produce byte order `<cluster>©` where `©` substitutes to
/// `র + ্`. Substitution alone yields `<cluster> + র + ্`; Unicode requires
/// `র + ্ + <cluster>` (see Unicode L2/2003/03233: "Ra + Hasanta + Anything
/// = Reph"). When the reph is already at the start of the string (Bijoy
/// byte order was reph-first), no reorder is needed.
///
/// Cluster walk follows avro.py's `rearrange_bijoy_text` algorithm: starting
/// at `i - 1`, walk backward over `(halant consonant)*` and an optional
/// trailing kar.
pub fn reph_reorder(input: &str) -> String {
    let mut text: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < text.len() {
        let has_reph_here = i + 1 < text.len()
            && text[i] == REPH_LEAD_RA
            && text[i + 1] == HASANTA;
        let preceded_by_consonant = i > 0 && is_bengali_consonant(text[i - 1]);
        let preceded_by_halant = i >= 2 && text[i - 2] == HASANTA;
        if has_reph_here && preceded_by_consonant && !preceded_by_halant {
            // Walk backwards from i-1 collecting the consonant cluster.
            let mut j: usize = 1;
            loop {
                if j > i {
                    break;
                }
                let pos = i - j;
                if pos >= 1
                    && is_bengali_consonant(text[pos])
                    && text[pos - 1] == HASANTA
                {
                    // halant + consonant in cluster
                    j += 2;
                    continue;
                }
                if j == 1 && is_kar(text[pos]) {
                    j += 1;
                    continue;
                }
                break;
            }
            let cluster_start = i - j;
            let mut new_text: Vec<char> = Vec::with_capacity(text.len());
            new_text.extend_from_slice(&text[..cluster_start]);
            new_text.push(REPH_LEAD_RA);
            new_text.push(HASANTA);
            new_text.extend_from_slice(&text[cluster_start..i]);
            new_text.extend_from_slice(&text[i + 2..]);
            let advance = 2 + (i - cluster_start);
            text = new_text;
            i = cluster_start + advance;
            continue;
        }
        i += 1;
    }
    text.into_iter().collect()
}

/// Swap pre-base vowel signs (`ি`, `ী`, `ে`, `ৈ`) from their Bijoy visual
/// byte position (before the consonant cluster) to the Unicode logical
/// position (after).
pub fn ikar_swap(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_prekar(c) && i + 1 < chars.len() {
            let cluster_start = i + 1;
            if is_bengali_consonant(chars[cluster_start]) {
                let mut j = cluster_start + 1;
                while j + 1 < chars.len()
                    && chars[j] == HASANTA
                    && is_bengali_consonant(chars[j + 1])
                {
                    j += 2;
                }
                out.extend(&chars[cluster_start..j]);
                out.push(c);
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out.into_iter().collect()
}

/// Fold split e-kar/o-kar glyphs into the composed vowel signs:
/// `ে + া` → `ো`; `ে + ৗ` → `ৌ`.
pub fn ekar_recombine(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == E_KAR && i + 1 < chars.len() {
            match chars[i + 1] {
                AA_KAR => {
                    out.push(O_KAR);
                    i += 2;
                    continue;
                }
                TAIL_AU_KAR => {
                    out.push(AU_KAR);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.into_iter().collect()
}

/// Insert ZWJ in `র + ্ + য` to force ya-phala rendering (W3C IIP #14).
/// If the sequence is already preceded by ZWJ or ZWNJ, leave it alone.
pub fn ya_phala_zwj(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len()
            && chars[i] == REPH_LEAD_RA
            && chars[i + 1] == HASANTA
            && chars[i + 2] == YA
        {
            let preceded_by_joiner = !out.is_empty()
                && matches!(out.last().copied(), Some(ZWJ) | Some(ZWNJ));
            if !preceded_by_joiner {
                out.push(REPH_LEAD_RA);
                out.push(ZWJ);
                out.push(HASANTA);
                out.push(YA);
                i += 3;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.into_iter().collect()
}

/// Apply Unicode NFC normalization. For Bengali this DECOMPOSES the
/// composition-excluded letters U+09DC/U+09DD/U+09DF into their
/// `<base> + ্` (nukta) pairs — the canonical encoding (see Unicode
/// CompositionExclusions.txt).
pub fn nfc(input: &str) -> String {
    input.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_normalize_strips_soft_hyphen() {
        assert_eq!(pre_normalize("a\u{00AD}b"), "ab");
    }

    #[test]
    fn pre_normalize_folds_crlf() {
        assert_eq!(pre_normalize("a\r\nb"), "a\nb");
    }

    #[test]
    fn ikar_swap_moves_i_kar_after_consonant() {
        let input = format!("{}{}", I_KAR, '\u{0995}');
        let expected = format!("{}{}", '\u{0995}', I_KAR);
        assert_eq!(ikar_swap(&input), expected);
    }

    #[test]
    fn ikar_swap_handles_e_kar() {
        let input = format!("{}{}", E_KAR, '\u{0995}');
        let expected = format!("{}{}", '\u{0995}', E_KAR);
        assert_eq!(ikar_swap(&input), expected);
    }

    #[test]
    fn ekar_recombine_folds_o_kar() {
        let input = format!("{}{}{}", '\u{0995}', E_KAR, AA_KAR);
        let expected = format!("{}{}", '\u{0995}', O_KAR);
        assert_eq!(ekar_recombine(&input), expected);
    }

    #[test]
    fn ya_phala_inserts_zwj() {
        let input = format!("{}{}{}", REPH_LEAD_RA, HASANTA, YA);
        let expected = format!("{}{}{}{}", REPH_LEAD_RA, ZWJ, HASANTA, YA);
        assert_eq!(ya_phala_zwj(&input), expected);
    }

    #[test]
    fn reph_reorder_consonant_first_moves_reph_before() {
        // ক + র + ্  →  র + ্ + ক
        let input = format!("{}{}{}", '\u{0995}', REPH_LEAD_RA, HASANTA);
        let expected = format!("{}{}{}", REPH_LEAD_RA, HASANTA, '\u{0995}');
        assert_eq!(reph_reorder(&input), expected);
    }

    #[test]
    fn reph_reorder_idempotent_when_reph_first() {
        // র + ্ + ক  →  unchanged
        let input = format!("{}{}{}", REPH_LEAD_RA, HASANTA, '\u{0995}');
        assert_eq!(reph_reorder(&input), input);
    }

    #[test]
    fn nfc_decomposes_precomposed_nukta() {
        // U+09DC has composition exclusion; NFC decomposes to U+09A1 U+09BC.
        let composed = "\u{09DC}";
        assert_eq!(nfc(composed), "\u{09A1}\u{09BC}");
    }
}
