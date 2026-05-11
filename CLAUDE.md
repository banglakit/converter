# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
# Build CLI (release)
cargo build --release -p banglakit-cli

# Run all tests
cargo test --workspace

# Run a single crate's tests
cargo test -p banglakit-core
cargo test -p banglakit-docx

# Run a specific test
cargo test -p banglakit-core -- transliterate::tests::test_name

# Check WASM target compiles
cargo check --target wasm32-unknown-unknown -p banglakit-wasm

# Build WASM for browser (requires wasm-pack)
wasm-pack build crates/banglakit-wasm --target web --release --out-dir ../../web/pkg

# Run browser converter locally
python3 -m http.server --directory web 8080
```

## Architecture

Rust workspace with 5 crates under `crates/`:

- **banglakit-core** — Pure-function library. No I/O. Houses the transliterator (Aho-Corasick multi-pass pipeline + normalization), five-stage classifier, `RunRef`/`RunVisitor` trait, and the canonical `convert_run` policy that every host calls.
- **banglakit-docx** — DOCX adapter. Walks `word/document.xml` via quick-xml event stream. Resolves run fonts through the full OOXML cascade: run → paragraph style chain → docDefaults → theme. Exposes both path-based and in-memory (`process_docx_bytes`) entry points.
- **banglakit-pptx** — PPTX adapter. Walks `ppt/slides/slideN.xml`, rewrites `<a:r>` runs. Same path/bytes dual API as DOCX.
- **banglakit-cli** — Binary crate (`banglakit-converter`). Dispatches on file extension. Has its own visitor with JSONL audit and `--explain` output (not in core because those are I/O concerns).
- **banglakit-wasm** — wasm-bindgen surface exposing `transliterateRun`, `classifyRun`, `convertRun`, `convertDocx`, `convertPptx`, `convertText` to JS.

**`web/`** — Vanilla JS browser app (no framework/bundler) deployed to GitHub Pages. Loads banglakit-wasm, provides drag-and-drop conversion.

### Key design pattern: `convert_run` as the single policy boundary

All hosts (CLI visitor, WASM bindings, future connectors) call `banglakit_core::convert_run(text, font_hint, &opts) -> ConvertedRun`. Hosts only differ in how they iterate runs and commit changes. Never duplicate classify-then-transliterate logic in a host.

### Mapping data

Transliteration tables live in `crates/banglakit-core/data/bijoy/mapping.toml` (derived from avro.py, inverted for Bijoy→Unicode). Font allowlists in `fonts.toml` files. These are compiled into the binary via `include_str!`.

### Adding a new ANSI encoding family

1. Add `data/<family>/mapping.toml` and `fonts.toml`
2. Add variant to `Encoding` enum in `encoding.rs`
3. Register in `EncodingRegistry`
4. Add to CLI's `--encoding` ValueEnum

## CI

- **ci.yml** — `cargo test --workspace` + wasm32 check on every push/PR. Builds Linux CLI artifact on main.
- **pages.yml** — Builds WASM and deploys `web/` to GitHub Pages on main push. Do NOT run `wasm-opt` — it breaks wasm-bindgen's externref-table init code.

## Design Documents

- **`docs/PRD.md`** — Product requirements. Defines functional requirements FR-1 through FR-10.
- **`docs/SDD.md`** — System design. Details the three-layer pipeline (Segmenter → Classifier → Transliterator), mapping table format, and cross-language packaging plan.

### Key design constraints (from PRD/SDD)

- **Per-run conversion only (FR-2):** Never operate on a document-level string. The unit of conversion is a single formatted run. Document-wide conversion is explicitly prohibited.
- **Transliterator is stateless and pure (SDD §5):** `transliterate()` takes `&str`, returns `String`. No I/O, no global state, no side effects.
- **Confidence-not-binary classification (SDD §2):** The classifier returns a calibrated probability. Binary decisions (convert vs. skip) are policy in `convert_run`, not mechanism in the classifier.
- **Never destroy input (SDD §2):** Original files are never modified in place. Every conversion produces a new output.
- **Transliteration pipeline order is non-negotiable (SDD §5.2):** Aho-Corasick (quad→tri→bi→single) → reph reorder → ikar swap → ekar recombine → ya-phala ZWJ → NFC. Later passes depend on earlier ones.
- **English false-positive rate < 0.5% (PRD §9):** The classifier must not convert English text. This is the primary guard against the core failure mode (document mangling).

## Exit codes (CLI)

- `0` — no changes made
- `1` — changes made
- `2` — error
