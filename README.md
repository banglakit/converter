# banglakit-converter

A lossless, font-aware converter for legacy ANSI/ASCII-encoded Bengali text
(Bijoy / SutonnyMJ family) to standard Unicode Bengali (U+0980–U+09FF).

The defining design choice is **per-run, font-aware classification**: every
existing tool surveyed in the PRD applies a Bijoy-to-Unicode substitution
table indiscriminately to the entire document, mangling English text and
URLs in the process. `banglakit-converter` looks at run-level font metadata
(in DOCX), and falls back to a calibrated heuristic classifier for plain
text, so a sentence like `Gas price is $5 today` is preserved byte-for-byte
even when it sits next to Bijoy-encoded Bengali in the same file.

This release (v0.1.0) ships:

- A pure-Rust core (`banglakit-core`) — transliterator + classifier with no
  I/O dependencies.
- A DOCX adapter (`banglakit-docx`) that streams runs through a visitor and
  preserves all non-run zip entries byte-identically.
- A CLI (`banglakit-converter`) that handles plain text on stdin/stdout
  and `.docx` files end-to-end.

Encodings beyond Bijoy (e.g. Boishakhi, Lekhoni) plug in via the
`Encoding` enum without architectural changes.

## Quick start

Build the release binary:

```bash
cargo build --release -p banglakit-cli
```

Plain text (stdin → stdout):

```bash
echo 'Avwg evsjvq Mvb MvB|' | ./target/release/banglakit-converter -i - -o -
# → আমি বাংলায় গান গাই।
```

DOCX:

```bash
./target/release/banglakit-converter \
    -i article.docx \
    -o article_unicode.docx \
    --audit article.audit.jsonl
```

The audit file is newline-delimited JSON, one entry per processed run.

## How it works

Pipeline per run:

1. **Classifier** (`banglakit-core::classify`) routes the run through five
   stages, short-circuiting at the first conclusive signal:
   - Unicode range pre-check — if the run already contains U+0980–U+09FF,
     skip.
   - Bijoy font allowlist — `SutonnyMJ`, `AdorshoLipi`, `JugantorMJ`, or
     any `*MJ`-suffix variant (case-insensitive). Subset prefixes like
     `ABCDEF+SutonnyMJ` are stripped first.
   - Unicode Bengali font allowlist — `Kalpurush`, `Nikosh`, `SolaimanLipi`,
     `Noto Sans Bengali`, etc. Skip.
   - Heuristic scorer — high-byte density + Bijoy distinctive characters +
     Bijoy bigram hits − English wordlist coverage, combined into a sigmoid
     probability.
   - Threshold policy — `safe` (default) requires P ≥ 0.95 to convert;
     `aggressive` requires P ≥ 0.85; below 0.50 is `Latin`; in between is
     `Ambiguous` (kept under safe mode, reviewable via `--explain`).

2. **Transliterator** (`banglakit-core::transliterate`) runs an
   Aho-Corasick longest-match pipeline (quadgrams → trigrams → bigrams →
   single chars) followed by:
   - `ikar_swap` — move pre-base vowel signs (`ি`, `ী`, `ে`, `ৈ`) after
     the consonant cluster.
   - `ekar_recombine` — fold `ে+া` → `ো`, `ে+ৗ` → `ৌ`.
   - `reph_reorder` — move `র + ্` from the post-cluster Bijoy position
     to the pre-cluster Unicode position (per Unicode L2/2003/03233 and
     avro.py's `rearrange_bijoy_text`).
   - `ya_phala_zwj` — insert ZWJ in `র + ্ + য` to force ya-phala
     rendering (W3C IIP #14).
   - NFC normalization. (NFC also decomposes the composition-excluded
     `ড়`/`ঢ়`/`য়` to `<base> + ়`, which is the canonical Bengali nukta
     encoding.)

## CLI reference

```
banglakit-converter [OPTIONS] --input <INPUT> --output <OUTPUT>

  -i, --input <PATH>          Input path (`-` for stdin)
  -o, --output <PATH>         Output path (`-` for stdout; not allowed for DOCX)
  --mode <MODE>               safe (default) | aggressive
  --threshold <FLOAT>         Override the mode's default convert threshold
  --encoding <ENCODING>       Encoding family (currently only `bijoy`)
  --unicode-font <NAME>       Target Unicode font written into DOCX runs
                              (default: Kalpurush)
  --audit <PATH>              Write a JSONL audit log to PATH
  --audit-stdout              Write the audit log to stdout
  --explain                   Print per-run classifier signals to stderr
  --dry-run                   Don't write output (plain text only in v0.1.0)
```

Exit codes follow PRD FR-9:

- `0` — no changes were made.
- `1` — changes were made.
- `2` — error.

## Architecture

```
crates/
├── banglakit-core/        # Pure Rust: transliterator + classifier
│   ├── data/
│   │   ├── bijoy/
│   │   │   ├── mapping.toml      # Derived from avro.py (MIT)
│   │   │   └── fonts.toml        # ANSI font allowlist
│   │   ├── unicode_fonts.toml    # Unicode Bengali font allowlist
│   │   └── english_words.txt     # Embedded English wordlist
│   └── src/
│       ├── encoding.rs           # Encoding enum + Registry
│       ├── mapping.rs            # TOML loader + Aho-Corasick automata
│       ├── transliterate.rs      # Multi-pass pipeline
│       ├── normalize.rs          # reph, i-kar, e-kar, ya-phala
│       ├── classifier.rs         # Five-stage classifier
│       ├── fonts.rs              # Font allowlist matching
│       └── english.rs            # English dictionary feature
├── banglakit-docx/        # DOCX zip + quick-xml adapter
├── banglakit-cli/         # `banglakit-converter` binary
└── banglakit-wasm/        # wasm-bindgen surface for browsers / Office Add-ins
```

## Office Add-in path

For the Word / Excel / PowerPoint Add-in roadmap, the WASM bindings, and a
scaffold of the Word task-pane project, see
[`docs/PATH-TO-OFFICE-ADDINS.md`](docs/PATH-TO-OFFICE-ADDINS.md). The browser
demo lives at [`examples/wasm-demo/`](examples/wasm-demo/) and the Office
scaffold at [`examples/word-addin/`](examples/word-addin/).

## Extending to other ANSI encodings

To add Boishakhi or Lekhoni support:

1. Drop a new mapping table at `crates/banglakit-core/data/<family>/mapping.toml`.
2. Drop a new font allowlist at `crates/banglakit-core/data/<family>/fonts.toml`.
3. Add a variant to `Encoding` in `src/encoding.rs` and an
   `EncodingRegistry` constant pointing at the new files.
4. Add the variant to the CLI's `--encoding` `ValueEnum`.

The transliterator and classifier are encoding-parameterized; no other
changes are required.

## Out of scope (v0.1.0)

These items are documented in the PRD/SDD and deferred:

- Python wheel via PyO3/maturin.
- ~~WASM build~~ — see `crates/banglakit-wasm/` (Stage 1 of the Office
  Add-in path). npm package and mobile (UniFFI) bindings still deferred.
- PPTX, RTF, HTML, PDF, clipboard adapters.
- ANSI encoding families beyond Bijoy.
- Trained logistic-regression / fastText LID fallback (Stage 5 of SDD §4).
  Replaced with a rule-based weighted-sum sigmoid using PRD-documented
  per-feature thresholds.
- Full DOCX style-cascade resolution (run inherits font from paragraph
  style / docDefaults). v0.1.0 reads run-level font only; missing-font
  runs fall through to heuristic scoring.
- DOCX `--dry-run`.

## Acknowledgements & licenses

- **Mapping table**: derived from `hitblast/avro.py` (MIT OR Apache-2.0),
  `src/avro/resources/dictionary.py`. avro.py's mapping is
  Unicode→Bijoy; we invert it for the Bijoy→Unicode direction. The reph
  reorder algorithm is also adapted from avro.py's `rearrange_bijoy_text`.
- **English wordlist**: top ~3,000 entries from
  `first20hours/google-10000-english` (MIT).
- **Design source**: the PRD and System Design Document supplied by the
  user; see Unicode L2/2003/03233 (Bengali) and W3C IIP #14 for the
  encoding-rule citations.

## License

MIT OR Apache-2.0.
