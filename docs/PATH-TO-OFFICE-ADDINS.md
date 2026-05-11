# Path to Office Add-ins

A staged plan for getting `banglakit-converter` running inside Word (and
later Excel / PowerPoint) as a Microsoft Office Add-in, plus the
groundwork that this branch lays down.

## Why Office Add-ins

The CLI + DOCX adapter solves the batch-conversion case: you have a folder of
`.docx` files, you want them converted. But the dominant authoring workflow
for Bengali-language newsrooms is *live editing in Word*, often against
shared documents on OneDrive / SharePoint. An Office Add-in surfaces the
converter where the work actually happens — as a task-pane button — with
the same font-aware classifier preventing English/URL mangling that the CLI
already guarantees.

Office Add-ins are JavaScript running in a sandboxed iframe inside the host
app (Word / Excel / PowerPoint / Outlook), in both the web and desktop
clients. There is no Rust runtime; we get into the iframe via WebAssembly.

## Architecture

```
banglakit-core (Rust, pure functions)
    │
    ├── banglakit-cli      ← stdin/stdout, file I/O
    ├── banglakit-docx     ← zip + quick-xml adapter
    └── banglakit-wasm     ← wasm-bindgen surface         (NEW, this branch)
            │
            └── examples/word-addin/  ← Office.js task pane (scaffold, this branch)
```

The core is unchanged. Each adapter is a thin shell that ferries `(text,
fontName)` pairs into `classify` + `transliterate` and writes the results
back into a host-specific document model.

## Stages

### Stage 1 — WASM bindings ✅ (this branch)

`crates/banglakit-wasm` exposes three functions to JS:

- `transliterateRun(text, encoding?)` — the raw transliterator.
- `classifyRun(text, fontName?, encoding?, mode?)` — five-stage classifier,
  returns `{ decision, stage, confidence, signals }`.
- `convertRun(text, fontName?, encoding?, mode?, unicodeFont?)` — the
  combo call the Word.run loop wants. Returns `{ text, changed,
  suggestedFont, … }`.

Built with `wasm-pack build crates/banglakit-wasm --target web --release`.
Raw `.wasm` from `cargo build --target wasm32-unknown-unknown --release`
is ~2.1 MB; after `wasm-pack` + `wasm-opt -Oz` the shipped artifact is
expected in the 300–500 KB range.

### Stage 2 — Browser harness ✅ (this branch)

`examples/wasm-demo/` is a 50-line HTML page that loads the WASM, lets you
paste Bijoy text, and shows the converted output plus classifier metadata.
Used to derisk the WASM build before pulling in the Office.js toolchain.

### Stage 3 — Word Task Pane add-in 🟡 (scaffold in this branch)

`examples/word-addin/` is a scaffold: `manifest.xml`, webpack config,
TypeScript task-pane entry point, README explaining how to run it. The
Rust → WASM half is built and tested; the Office.js half compiles against
the listed dependencies but has not been validated against a live Word
host (no Word in CI). The README enumerates the gaps.

The conversion loop, copied verbatim from `src/taskpane/taskpane.ts`:

```ts
await Word.run(async (context) => {
  const range = context.document.body.getRange();
  const runs = range.getTextRanges([" ", "\t"], false);
  runs.load("items/text,items/font/name");
  await context.sync();
  for (const r of runs.items) {
    const result = convertRun(r.text, r.font.name, "bijoy", "safe", "Kalpurush");
    if (result.changed) {
      r.insertText(result.text, Word.InsertLocation.replace);
      r.font.name = result.suggestedFont!;
    }
  }
  await context.sync();
});
```

### Stage 4 — Excel / PowerPoint (not started)

Same WASM engine. Different host APIs:
- **Excel:** `Excel.Range.values` is 2D; font is cell-level via
  `Range.format.font.name`. The classifier's font-hint signal helps here
  because spreadsheet cells often have explicit font metadata.
- **PowerPoint:** `PowerPoint.TextRange` on shape `textFrame`s. Closer to
  Word's model than Excel's.

### Stage 5 — Distribution (not started)

1. **Sideload** for dev — already wired in the scaffold.
2. **AppSource** for public listing — requires a real GUID, hosted bundle,
   Microsoft Partner Center submission, and a privacy policy URL.
3. **Centralized deployment** for newsrooms — the most realistic
   distribution channel; M365 admin uploads `manifest.xml` once for the
   whole org.

## Open decisions

1. **`getTextRanges` granularity.** Word.js doesn't expose `<w:r>` directly;
   `getTextRanges` with whitespace delimiters is a workable approximation
   but will occasionally fuse two runs that share a whitespace boundary but
   not a font. v0.2 of the add-in should investigate the paragraph-level
   walk pattern.
2. **WASM size budget.** A 300–500 KB add-in load is fine for desktop Word
   but noticeable on Word for the web's first launch. If we want < 100 KB,
   we'd have to ditch `regex` (only used for one daggers-in-Bijoy match)
   and shrink the embedded English wordlist.
3. **Cache strategy.** Add-ins can fetch resources from their `AppDomains`.
   The `.wasm` blob can sit on a CDN with long cache headers; only
   `taskpane.html` needs to revalidate.
4. **Audit log surfacing.** The CLI writes JSONL via `--audit`. In the
   add-in, the equivalent would be a "review changes" panel — a `Suspense`
   step before committing the writes. Worth designing once Stage 3 is
   validated.

## Building everything

```bash
# Rust unit tests (unchanged from main)
cargo test --workspace

# Native type-check of the new crate
cargo check -p banglakit-wasm

# wasm32 build
rustup target add wasm32-unknown-unknown
cargo build -p banglakit-wasm --target wasm32-unknown-unknown --release

# Bundled JS package (requires wasm-pack)
wasm-pack build crates/banglakit-wasm --target web --release \
    --out-dir ../../examples/wasm-demo/pkg

# Word add-in (requires Node 18+)
cd examples/word-addin && npm install && npm run build
```
