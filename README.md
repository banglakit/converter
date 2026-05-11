# banglakit-converter

A lossless, font-aware converter for legacy ANSI/ASCII-encoded Bengali text
(Bijoy / SutonnyMJ family) to standard Unicode Bengali (U+0980–U+09FF).

**Try it in your browser:** <https://banglakit.com/converter/> — drag a
`.docx`, `.pptx`, or plain-text file onto the page and download the
converted result. All conversion runs client-side via WebAssembly; files
never leave your machine.

The defining design choice is **per-run, font-aware classification**: every
existing tool surveyed in the PRD applies a Bijoy-to-Unicode substitution
table indiscriminately to the entire document, mangling English text and
URLs in the process. `banglakit-converter` looks at run-level font metadata
(in DOCX), and falls back to a calibrated heuristic classifier for plain
text, so a sentence like `Gas price is $5 today` is preserved byte-for-byte
even when it sits next to Bijoy-encoded Bengali in the same file.

This release ships:

- A pure-Rust core (`banglakit-core`) — transliterator + classifier with no
  I/O dependencies. Hosts the shared `RunRef` / `RunVisitor` types used by
  every format adapter, plus the canonical `convert_run` policy and the
  `DefaultRunVisitor` reused by every host.
- A DOCX adapter (`banglakit-docx`) with full OOXML font resolution: a
  run inherits its font from `pPr/rPr`, then the paragraph style chain
  (`pStyle` → `basedOn` → default paragraph style), then `docDefaults`.
  Each `<w:rFonts>` element is theme-aware: `w:asciiTheme="minorHAnsi"`
  references resolve through `word/theme/theme1.xml`, which is what
  modern Word output uses for its default font. When a converted run has
  no `w:rFonts`, one is injected so the new font survives. Both path-based
  (`process_docx`) and in-memory (`process_docx_bytes`) entry points are
  exposed so the same code converts files on disk and in a browser tab.
- A PPTX adapter (`banglakit-pptx`) that walks every `ppt/slides/slideN.xml`
  and rewrites `<a:r>` runs in place. Slide masters, layouts, theme, and
  media are copied byte-for-byte. Same path / bytes pair as the DOCX
  adapter.
- A CLI (`banglakit-converter`) that dispatches on file extension and
  handles plain text on stdin/stdout, `.docx`, and `.pptx`.
- A WASM crate (`banglakit-wasm`) exposing `transliterateRun`,
  `classifyRun`, `convertRun`, `convertDocx`, `convertPptx`, and
  `convertText` to JavaScript — the same JS surface the browser
  converter and a future Office Add-in build on.
- A static browser app (`web/`) deployed to GitHub Pages on every push to
  `main`. See [Continuous integration](#continuous-integration) below.

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

PPTX:

```bash
./target/release/banglakit-converter \
    -i deck.pptx \
    -o deck_unicode.pptx \
    --audit deck.audit.jsonl
```

The audit file is newline-delimited JSON, one entry per processed run.
PPTX entries also carry a `slide_index` field.

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
├── banglakit-core/        # Pure Rust: transliterator + classifier + visitor
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
│       ├── english.rs            # English dictionary feature
│       ├── visitor.rs            # RunRef / RunAction / RunVisitor
│       └── policy.rs             # convert_run — the cross-host boundary
├── banglakit-docx/        # DOCX adapter: word/document.xml + word/styles.xml
│   └── src/styles.rs              # Style-cascade resolution
├── banglakit-pptx/        # PPTX adapter: walks ppt/slides/slideN.xml
├── banglakit-cli/         # `banglakit-converter` binary
└── banglakit-wasm/        # wasm-bindgen surface for browsers / Office Add-ins

web/                      # Static browser app deployed to GitHub Pages
├── index.html              # Drag-and-drop drop zone + paste-text demo
├── app.js                  # File dispatch: .docx → convertDocx, .pptx → convertPptx, .txt → convertText
└── style.css

.github/workflows/
├── ci.yml                  # cargo test + wasm32 check on every push/PR;
│                           # uploads Linux CLI artifact on main
└── pages.yml               # Builds banglakit-wasm and deploys web/ to Pages
```

### Cross-platform run policy

Every host — the CLI's DOCX/PPTX visitor, the WASM bindings used by
Office.js (Word, PowerPoint, Excel Add-ins), and any future LibreOffice /
Apache OpenOffice connector — calls one canonical function per run:
`banglakit_core::convert_run(text, font_hint, &opts) -> ConvertedRun`.
The classifier, the transliterator, and the safe / aggressive / threshold
policy live behind that one call. Hosts differ only in *how they iterate
runs* and *how they commit changes*:

```
                   ┌────────────────────────────────┐
                   │  banglakit-core::policy        │
                   │                                │
                   │  convert_run(text, font, opts) │
                   │    → ConvertedRun {            │
                   │        text, font, changed,    │
                   │        classification          │
                   │    }                           │
                   └────────────┬───────────────────┘
                                │ called once per run
     ┌──────────────────────────┼────────────────────────────┐
     │                          │                            │
┌────▼──────────┐  ┌────────────▼──────────┐    ┌────────────▼──────────┐
│ CLI visitor   │  │ WASM convertRun       │    │ LibreOffice UNO       │
│ (file path)   │  │ (JS / Office.js)      │    │ (future, Java/Python) │
│               │  │                       │    │                       │
│ quick-xml     │  │ Office.js Word.Range  │    │ UNO TextPortion       │
│ event stream  │  │ iteration             │    │ enumeration           │
└───────────────┘  └───────────────────────┘    └───────────────────────┘
```

Adding a new host means writing the small layer below the dashed line —
iterate the host's native runs, call `convert_run`, apply the
`ConvertedRun`. The XML state machines in `banglakit-docx` and
`banglakit-pptx` deliberately stay format-specific; Office.js and UNO
don't see XML, so the OOXML walker isn't worth generalizing across them.

## Browser converter

The static site at [`web/`](web/) is what's deployed to
<https://banglakit.com/converter/>. It is a vanilla-JS module — no
framework, no bundler — that loads `banglakit-wasm` via `wasm-pack --target
web` and exposes drag-and-drop file conversion for DOCX, PPTX, and plain
text. All work happens in the user's tab; nothing is uploaded.

Run it locally:

```bash
wasm-pack build crates/banglakit-wasm --target web --release \
    --out-dir ../../web/pkg
python3 -m http.server --directory web 8080
# open http://localhost:8080/
```

The minimal text-only harness at
[`examples/wasm-demo/`](examples/wasm-demo/) still exists for hacking on
the classifier in isolation.

## Office Add-in path

For the Word / Excel / PowerPoint Add-in roadmap, the WASM bindings, and a
scaffold of the Word task-pane project, see
[`docs/PATH-TO-OFFICE-ADDINS.md`](docs/PATH-TO-OFFICE-ADDINS.md). The Office
scaffold lives at [`examples/word-addin/`](examples/word-addin/) and shares
the same `convertRun` entry point the browser app calls per text run.

## Continuous integration

Two GitHub Actions workflows under [`.github/workflows/`](.github/workflows/):

- **`ci.yml`** — runs `cargo test --workspace` and
  `cargo check --target wasm32-unknown-unknown -p banglakit-wasm` on every
  push and pull request. On pushes to `main`, additionally builds a
  release of `banglakit-converter` for Linux x86_64 and uploads it as a
  workflow artifact (`banglakit-converter-linux-x86_64`).
- **`pages.yml`** — on pushes to `main`, builds `banglakit-wasm` with
  `wasm-pack --target web --release`, then deploys `web/` (with the
  generated `web/pkg/`) to GitHub Pages. `wasm-opt` is intentionally
  *not* run: binaryen's `-Oz` pass strips the externref-table grow
  capability that the wasm-bindgen 0.2.121 init code depends on.

To enable Pages on a fresh fork: **Settings → Pages → Source: GitHub
Actions**.

## Extending to other ANSI encodings

To add Boishakhi or Lekhoni support:

1. Drop a new mapping table at `crates/banglakit-core/data/<family>/mapping.toml`.
2. Drop a new font allowlist at `crates/banglakit-core/data/<family>/fonts.toml`.
3. Add a variant to `Encoding` in `src/encoding.rs` and an
   `EncodingRegistry` constant pointing at the new files.
4. Add the variant to the CLI's `--encoding` `ValueEnum`.

The transliterator and classifier are encoding-parameterized; no other
changes are required.

## Out of scope (current release)

These items are documented in the PRD/SDD and deferred:

- Python wheel via PyO3/maturin.
- ~~WASM build~~ — `crates/banglakit-wasm/` ships text + DOCX + PPTX
  entry points; the browser app at `web/` consumes them. npm package
  publication and mobile (UniFFI) bindings still deferred.
- ~~Browser file conversion~~ — live at <https://banglakit.com/converter/>.
- RTF, HTML, PDF, clipboard adapters.
- ANSI encoding families beyond Bijoy.
- Trained logistic-regression / fastText LID fallback (Stage 5 of SDD §4).
  Replaced with a rule-based weighted-sum sigmoid using PRD-documented
  per-feature thresholds.
- PPTX style cascade (shape → layout → master → theme). The PPTX adapter
  reads run-level fonts only; missing-font runs fall through to heuristic
  scoring. Theme-font references like `+mn-lt` are also not resolved.
- DOCX / PPTX `--dry-run`.

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
