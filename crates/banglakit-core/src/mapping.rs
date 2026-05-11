//! Parses an encoding's `mapping.toml` into Aho-Corasick automata for
//! longest-match replacement.

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::collections::HashMap;

use crate::encoding::{registry, Encoding};

#[derive(Debug, Deserialize)]
struct RawMapping {
    #[serde(default)]
    quadgrams: HashMap<String, String>,
    #[serde(default)]
    trigrams: HashMap<String, String>,
    #[serde(default)]
    bigrams: HashMap<String, String>,
    #[serde(default)]
    single_char: HashMap<String, String>,
}

/// A single Aho-Corasick automaton plus its parallel replacement table.
pub struct GramAutomaton {
    automaton: AhoCorasick,
    replacements: Vec<String>,
}

impl GramAutomaton {
    fn build(entries: &HashMap<String, String>) -> Option<Self> {
        if entries.is_empty() {
            return None;
        }
        // Stable order by descending key length, then lexicographic, so that
        // ties break deterministically. The automaton uses LeftmostLongest,
        // but the parallel replacements slice must index in the same order
        // as the patterns we feed in.
        let mut pairs: Vec<(&String, &String)> = entries.iter().collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
        let patterns: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        let replacements: Vec<String> = pairs.iter().map(|(_, v)| (*v).clone()).collect();
        let automaton = AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
            .expect("aho-corasick build");
        Some(GramAutomaton { automaton, replacements })
    }

    /// Run the automaton over `input`, returning the rewritten string.
    pub fn replace(&self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut cursor = 0usize;
        for m in self.automaton.find_iter(input) {
            if m.start() > cursor {
                out.push_str(&input[cursor..m.start()]);
            }
            out.push_str(&self.replacements[m.pattern()]);
            cursor = m.end();
        }
        if cursor < input.len() {
            out.push_str(&input[cursor..]);
        }
        out
    }
}

/// The full set of compiled automata for one encoding, ordered for the
/// transliterator's longest-first replacement pipeline.
pub struct MappingSet {
    pub quadgrams: Option<GramAutomaton>,
    pub trigrams: Option<GramAutomaton>,
    pub bigrams: Option<GramAutomaton>,
    pub single_char: Option<GramAutomaton>,
}

/// Lazily-initialized cache, one slot per encoding family.
static BIJOY_MAPPING: OnceCell<MappingSet> = OnceCell::new();

pub fn mapping_for(encoding: Encoding) -> &'static MappingSet {
    match encoding {
        Encoding::Bijoy => BIJOY_MAPPING.get_or_init(|| load(registry(encoding).mapping_toml)),
    }
}

fn load(toml_src: &str) -> MappingSet {
    let raw: RawMapping = toml::from_str(toml_src).expect("mapping.toml parse");
    MappingSet {
        quadgrams: GramAutomaton::build(&raw.quadgrams),
        trigrams: GramAutomaton::build(&raw.trigrams),
        bigrams: GramAutomaton::build(&raw.bigrams),
        single_char: GramAutomaton::build(&raw.single_char),
    }
}
