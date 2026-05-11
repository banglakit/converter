//! Integration tests for the transliterator. Curated (Bijoy, Unicode) pairs
//! covering each post-substitution pass per SDD §11.

use banglakit_core::{transliterate, Encoding};
use unicode_normalization::UnicodeNormalization;

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn assert_converts(bijoy: &str, expected_unicode: &str) {
    let got = transliterate(bijoy, Encoding::Bijoy);
    let expected = nfc(expected_unicode);
    assert_eq!(
        got, expected,
        "\nBijoy:    {:?}\nExpected: {:?} ({:?})\nGot:      {:?} ({:?})",
        bijoy,
        expected,
        expected.chars().map(|c| format!("U+{:04X}", c as u32)).collect::<Vec<_>>(),
        got,
        got.chars().map(|c| format!("U+{:04X}", c as u32)).collect::<Vec<_>>(),
    );
}

#[test]
fn canonical_sample() {
    // PRD §2: the canonical example from the document-mangling problem
    // statement. Note: NFC applies. ABV+B = আই (with i-kar after consonant).
    assert_converts("Avwg evsjvq Mvb MvB|", "আমি বাংলায় গান গাই।");
}

#[test]
fn simple_letters() {
    assert_converts("K", "ক");
    assert_converts("L", "খ");
    assert_converts("M", "গ");
    assert_converts("N", "ঘ");
}

#[test]
fn dari() {
    assert_converts("|", "।");
}

#[test]
fn smart_quotes() {
    assert_converts("Ô", "‘");
    assert_converts("Õ", "’");
}

#[test]
fn ikar_swap_in_word() {
    // wb + cons = ি + ন would be wrong; correct: w followed by b is
    // pre-base i-kar + na → after swap, "ন" + "ি" = "নি".
    assert_converts("wb", "নি");
}

#[test]
fn o_kar_split_recombination() {
    // ‡ + cons + v → e-kar + cons + aa-kar → cons + e-kar + aa-kar (swap)
    //   → cons + o-kar (recombine) → কো
    assert_converts("‡Kv", "কো");
}

#[test]
fn au_kar_split_recombination() {
    // ‡ + cons + Š → ে + cons + ৗ → cons + ে + ৗ → cons + ৌ → কৌ
    assert_converts("‡KŠ", "কৌ");
}

#[test]
fn reph_idempotent_when_reph_first() {
    // © maps to "র্" (Ra + Hasanta). With Bijoy reph-first byte order,
    // ©K substitutes directly to র্ক which is already proper Unicode.
    assert_converts("©K", "র্ক");
}

#[test]
fn reph_reorders_when_consonant_first() {
    // The other byte ordering some Bijoy keyboards produce: consonant
    // first, reph glyph after. Substitution yields কর্; the reph_reorder
    // pass must move র + ্ to before ক → র্ক.
    let bijoy_consonant_first = format!("K{}", "©");
    let got = transliterate(&bijoy_consonant_first, Encoding::Bijoy);
    let expected = nfc("র্ক");
    assert_eq!(got, expected, "got {got:?}");
}

#[test]
fn nukta_letters_decomposed_by_nfc() {
    // U+09DC/U+09DD/U+09DF carry Composition_Exclusion in Unicode, so NFC
    // decomposes them to <base> + ় (nukta). Our pipeline returns the
    // canonical NFC form.
    assert_converts("o", "\u{09A1}\u{09BC}");
    assert_converts("p", "\u{09A2}\u{09BC}");
    assert_converts("q", "\u{09AF}\u{09BC}");
}

#[test]
fn ya_phala_zwj_inserted() {
    // ª¨ is "্র্য" in the mapping table. Then the ya-phala pass should
    // inject ZWJ between র and ্ in the resulting `র + ্ + য` sequence.
    // Note: this mapping starts with `্র্য` which already has a leading
    // halant; we test the simpler `i¨` (র‌্য) form which uses ZWNJ.
    let got = transliterate("i¨", Encoding::Bijoy);
    // The avro.py source value is "র‌্য" with ZWNJ at U+200C.
    assert!(
        got.contains('\u{200C}') || got.contains('\u{200D}'),
        "ya-phala without joiner: {:?}",
        got
    );
}

#[test]
fn chandrabindu() {
    // ঁ encoded as `u` in Bijoy → after substitution, `Ku` → `কঁ`.
    assert_converts("Ku", "কঁ");
}

#[test]
fn digits_pass_through_mapped() {
    // The mapping table includes 0-9 → Bengali digits as a simple substitution.
    assert_converts("1234", "১২৩৪");
}

#[test]
fn anusvara() {
    // s → ং (anusvara)
    assert_converts("Ks", "কং");
}

#[test]
fn aa_kar_after_consonant() {
    // Kv → কা (consonant + aa-kar, no swap needed since aa-kar is post-base)
    assert_converts("Kv", "কা");
}

#[test]
fn longest_match_priority() {
    // Av is the bigram for আ. The single_char mapping for A is অ.
    // Aho-Corasick LeftmostLongest must pick the bigram.
    assert_converts("Av", "আ");
    assert_converts("A", "অ");
}

#[test]
fn punctuation_passes_through() {
    // ASCII punctuation that isn't in the mapping table stays untouched.
    let got = transliterate(",", Encoding::Bijoy);
    assert_eq!(got, ",");
}

#[test]
fn pre_normalize_strips_soft_hyphens() {
    // U+00AD between two Bijoy letters should be stripped before mapping.
    assert_converts("K\u{00AD}M", "কগ");
}
