// Banglakit Word Add-in — task pane entry point.
//
// Calls into the WASM-compiled banglakit-core for the actual classify +
// transliterate work. The Office.js loop here is just plumbing: iterate runs,
// pass each (text, fontName) through `convertRun`, write the output back.
//
// Build order:
//   1. `npm run build:wasm` produces `wasm-pkg/banglakit_wasm.js` + `.wasm`
//   2. webpack bundles this file plus `wasm-pkg/` into `dist/`
//   3. `npm start` sideloads `manifest.xml` into Word and serves `dist/` at
//      https://localhost:3000.

import init, { convertRun, coreVersion } from "../../wasm-pkg/banglakit_wasm";

type Mode = "safe" | "aggressive";

interface ConvertResult {
  text: string;
  changed: boolean;
  decision: string;
  encoding?: string;
  stage: string;
  confidence: number;
  suggestedFont?: string;
}

let wasmReady: Promise<void> | null = null;

function ensureWasm(): Promise<void> {
  if (!wasmReady) {
    wasmReady = init().then(() => {
      console.log(`banglakit-wasm ${coreVersion()} loaded`);
    });
  }
  return wasmReady;
}

function setStatus(msg: string) {
  const el = document.getElementById("status");
  if (el) el.textContent = msg;
}

// Convert every run in `range`. The function batches font-name reads via a
// single context.sync() before writing back, so the chatty Office.js network
// path between the iframe and Word is minimized — important on Word for the
// web where each sync round-trips to the server.
async function convertRange(
  context: Word.RequestContext,
  range: Word.Range,
  mode: Mode,
  unicodeFont: string,
): Promise<{ runs: number; changed: number }> {
  // Word.js doesn't expose a flat list of <w:r>; getTextRanges() with
  // splitting on whitespace approximates run boundaries well enough that
  // each result inherits a single font.
  const textRanges = range.getTextRanges([" ", "\t"], /* trimSpacing */ false);
  textRanges.load("items/text,items/font/name");
  await context.sync();

  const items = textRanges.items;
  let changed = 0;

  for (const r of items) {
    const text = r.text;
    if (!text) continue;
    const result = convertRun(
      text,
      r.font.name || undefined,
      "bijoy",
      mode,
      unicodeFont,
    ) as ConvertResult;
    if (result.changed) {
      r.insertText(result.text, Word.InsertLocation.replace);
      if (result.suggestedFont) {
        r.font.name = result.suggestedFont;
      }
      changed += 1;
    }
  }
  await context.sync();
  return { runs: items.length, changed };
}

async function convertDocument(scope: "document" | "selection") {
  await ensureWasm();
  const mode = (document.getElementById("mode") as HTMLSelectElement).value as Mode;
  const unicodeFont = (document.getElementById("unicodeFont") as HTMLSelectElement).value;

  setStatus("Converting…");
  try {
    await Word.run(async (context) => {
      const range =
        scope === "selection"
          ? context.document.getSelection()
          : context.document.body.getRange();
      const { runs, changed } = await convertRange(context, range, mode, unicodeFont);
      setStatus(`Done. ${changed} of ${runs} runs converted.`);
    });
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${(err as Error).message}`);
  }
}

Office.onReady(({ host }) => {
  if (host !== Office.HostType.Word) {
    setStatus("This add-in only runs in Word.");
    return;
  }
  document
    .getElementById("convert")
    ?.addEventListener("click", () => convertDocument("document"));
  document
    .getElementById("convertSelection")
    ?.addEventListener("click", () => convertDocument("selection"));
  ensureWasm().then(() => setStatus("Ready."));
});
