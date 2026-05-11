# wasm-demo

50-line browser harness that loads the `banglakit-wasm` crate and round-trips a
paragraph of Bijoy text through the classifier + transliterator. Used to derisk
the WASM build independently of the Office.js scaffold (see
`examples/word-addin/`).

## Build

Requires [`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/) and the
`wasm32-unknown-unknown` target:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack    # one-time

wasm-pack build crates/banglakit-wasm \
    --target web \
    --release \
    --out-dir ../../examples/wasm-demo/pkg
```

This produces `examples/wasm-demo/pkg/banglakit_wasm.js` and
`banglakit_wasm_bg.wasm`, both of which `index.html` imports.

## Run

Any static HTTP server works (WASM cannot be loaded from `file://`):

```bash
python3 -m http.server --directory examples/wasm-demo 8000
# open http://localhost:8000/
```

Type or paste a Bijoy paragraph on the left; the right pane shows the Unicode
output and per-line classifier metadata.

## What this demonstrates

- The Rust core compiles to WASM with no `wasm32`-only `cfg` flags. The same
  source ships native (CLI), DOCX (via `quick-xml`/`zip`), and browser (via
  `wasm-bindgen`) without forks.
- The `convertRun(text, fontName, encoding, mode, unicodeFont)` API is the
  exact shape an Office Add-in's `Word.run` loop needs: one call per run,
  returns `{ text, changed, suggestedFont, … }`, no global state.
- The five-stage classifier short-circuits on a font hint (`SutonnyMJ` etc.)
  exactly as it does in the CLI — type a font name into the "font hint" box
  to see the stage flip from `heuristic` to `ansi_font`.

## Size

After `wasm-pack --release`, expect a 300–500 KB `.wasm`. `wasm-opt -Oz`
shrinks it further but is not run automatically; `wasm-pack` will invoke it if
the `binaryen` toolchain is on `PATH`.
