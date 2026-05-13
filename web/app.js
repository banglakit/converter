// Built by `wasm-pack build crates/banglakit-wasm --target web --release --out-dir ../../web/pkg`.
// The Pages workflow runs that command before publishing, so `./pkg/` exists
// in deployment but not in the repo checkout — see web/pkg/.gitignore.
import init, {
  convertDocx,
  convertPptx,
  convertText,
  coreVersion,
} from "./pkg/banglakit_wasm.js";

const $ = (id) => document.getElementById(id);

const DOCX_MIME = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const PPTX_MIME = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const SUPPORTED_EXT = new Set(["docx", "pptx", "txt", "md"]);

// Whether auto-map is active (toggled by the mapper UI)
let autoMap = true;

function options() {
  return {
    mode: $("mode").value,
    encoding: "bijoy",
    unicodeFont: $("unicodeFont").value,
    autoMatchFonts: autoMap,
  };
}

function setStatus(kind, html) {
  const el = $("filestatus");
  el.hidden = false;
  el.classList.remove("animate-in");
  void el.offsetWidth;
  el.className = `status ${kind} animate-in`;
  el.innerHTML = html;
}

function downloadBlob(bytes, mime, filename) {
  const blob = new Blob([bytes], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Revoke after a tick so Safari has finished kicking off the download.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function outputName(inputName, ext) {
  const base = inputName.replace(/\.[^.]+$/, "");
  return `${base}.unicode.${ext}`;
}

function fileExt(name) {
  const m = name.match(/\.([^.]+)$/);
  return m ? m[1].toLowerCase() : "";
}

function isSupported(name) {
  return SUPPORTED_EXT.has(fileExt(name));
}

/** Convert a single file. Returns { name, outputName, bytes, mime, changed, runs, error } */
async function convertSingleFile(file) {
  const name = file.name.toLowerCase();
  const opts = options();

  try {
    if (name.endsWith(".docx")) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = convertDocx(bytes, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      const changed = result.anyChange ?? result.any_change;
      const runs = result.runsConverted ?? result.runs_converted ?? 0;
      return { name: file.name, outputName: outputName(file.name, "docx"), bytes: result.bytes, mime: DOCX_MIME, changed, runs };
    } else if (name.endsWith(".pptx")) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = convertPptx(bytes, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      const changed = result.anyChange ?? result.any_change;
      const runs = result.runsConverted ?? result.runs_converted ?? 0;
      return { name: file.name, outputName: outputName(file.name, "pptx"), bytes: result.bytes, mime: PPTX_MIME, changed, runs };
    } else if (name.endsWith(".txt") || name.endsWith(".md") || file.type.startsWith("text/")) {
      const text = await file.text();
      const result = convertText(text, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      const changed = result.changed;
      const runs = result.runs_converted ?? 0;
      const encoder = new TextEncoder();
      return { name: file.name, outputName: outputName(file.name, "txt"), bytes: encoder.encode(result.text), mime: "text/plain;charset=utf-8", changed, runs };
    } else {
      return { name: file.name, error: "Unsupported file type", skipped: true };
    }
  } catch (err) {
    console.error(err);
    return { name: file.name, error: err.message ?? String(err) };
  }
}

/** Handle a single file drop/pick — direct download, no ZIP */
async function handleSingleFile(file) {
  setStatus("", `Converting <strong>${escapeHtml(file.name)}</strong>...`);
  const result = await convertSingleFile(file);

  if (result.error) {
    if (result.skipped) {
      setStatus("err", `Unsupported file type: <code>${escapeHtml(file.name)}</code>. Expected .docx, .pptx, or .txt.`);
    } else {
      setStatus("err", `Conversion failed: <code>${escapeHtml(result.error)}</code>`);
    }
    return;
  }

  downloadBlob(result.bytes, result.mime, result.outputName);
  reportFileResult(result.name, result);
}

/** Handle multiple files — show progress, bundle into ZIP */
async function handleMultipleFiles(files, zipName) {
  const zip = new JSZip();
  const results = [];
  let converted = 0, skipped = 0, errors = 0;

  for (let i = 0; i < files.length; i++) {
    const file = files[i];
    const pct = Math.round(((i) / files.length) * 100);
    setStatus("", `<div>Converting <strong>${i + 1}</strong> of <strong>${files.length}</strong>: ${escapeHtml(file.name)}</div><div class="progress-bar"><div class="progress-fill" style="width:${pct}%"></div></div>`);

    if (!isSupported(file.name)) {
      skipped++;
      results.push({ name: file.name, skipped: true });
      continue;
    }

    const result = await convertSingleFile(file);
    results.push(result);

    if (result.error) {
      errors++;
    } else {
      converted++;
      // Use webkitRelativePath for folder structure, else flat
      const path = file.webkitRelativePath
        ? outputName(file.webkitRelativePath, fileExt(result.outputName))
        : result.outputName;
      zip.file(path, result.bytes);
    }
  }

  // Generate and download ZIP
  if (converted > 0) {
    const zipBlob = await zip.generateAsync({ type: "blob" });
    downloadBlob(zipBlob, "application/zip", zipName);
  }

  // Show aggregate results
  showBatchResults(results, converted, skipped, errors);
}

function showBatchResults(results, converted, skipped, errors) {
  const parts = [];
  if (converted > 0) parts.push(`<strong>${converted}</strong> converted`);
  if (skipped > 0) parts.push(`<strong>${skipped}</strong> skipped`);
  if (errors > 0) parts.push(`<strong>${errors}</strong> failed`);

  let html = `<div class="batch-summary">${parts.join(", ")}</div>`;
  html += `<ul class="results-list">`;
  for (const r of results) {
    if (r.skipped) {
      html += `<li><span class="r-icon skip">&#9679;</span><span class="r-name">${escapeHtml(r.name)}</span><span class="r-detail">skipped</span></li>`;
    } else if (r.error) {
      html += `<li><span class="r-icon err">&#9679;</span><span class="r-name">${escapeHtml(r.name)}</span><span class="r-detail">${escapeHtml(r.error)}</span></li>`;
    } else {
      const detail = r.changed ? `${r.runs} run${r.runs === 1 ? "" : "s"}` : "no changes";
      html += `<li><span class="r-icon ok">&#9679;</span><span class="r-name">${escapeHtml(r.name)}</span><span class="r-detail">${detail}</span></li>`;
    }
  }
  html += `</ul>`;

  const kind = errors > 0 ? "err" : converted > 0 ? "ok" : "";
  setStatus(kind, html);
}

/** Entry point: dispatch single vs batch */
async function handleFiles(files, zipName) {
  const fileArray = Array.from(files);
  if (fileArray.length === 0) return;

  if (fileArray.length === 1) {
    await handleSingleFile(fileArray[0]);
  } else {
    await handleMultipleFiles(fileArray, zipName || "banglakit-converted.zip");
  }
}

function reportFileResult(name, result) {
  const changed = result.changed;
  const runs = result.runs ?? 0;
  const kind = changed ? "ok" : "";
  const summary = changed
    ? `Converted <strong>${escapeHtml(name)}</strong> — ${runs} run${runs === 1 ? "" : "s"} rewritten. Download started.`
    : `No Bijoy runs detected in <strong>${escapeHtml(name)}</strong>. Output is identical to the input.`;
  const fontLabel = autoMap ? "auto-matched" : $("unicodeFont").value;
  setStatus(kind, summary + `<span class="meta">mode=${$("mode").value} target=${fontLabel}</span>`);
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}

/** Recursively collect files from a DataTransfer directory entry, tracking paths */
function readEntryRecursive(entry, parentPath) {
  return new Promise((resolve) => {
    if (entry.isFile) {
      entry.file((f) => {
        // Attach the full relative path so ZIP can preserve folder layout
        f._relativePath = parentPath ? `${parentPath}/${f.name}` : f.name;
        resolve([f]);
      });
    } else if (entry.isDirectory) {
      const dirPath = parentPath ? `${parentPath}/${entry.name}` : entry.name;
      const reader = entry.createReader();
      const allFiles = [];
      const readBatch = () => {
        reader.readEntries((entries) => {
          if (entries.length === 0) {
            resolve(allFiles);
          } else {
            Promise.all(entries.map((e) => readEntryRecursive(e, dirPath))).then((nested) => {
              allFiles.push(...nested.flat());
              readBatch(); // directories can return entries in batches
            });
          }
        });
      };
      readBatch();
    } else {
      resolve([]);
    }
  });
}

/** Collect files from a drop event, handling folders via webkitGetAsEntry */
async function collectDroppedFiles(dataTransfer) {
  const items = dataTransfer.items;
  let folderName = null;

  // Try entry-based traversal for folder support
  if (items && items.length > 0 && items[0].webkitGetAsEntry) {
    const entries = [];
    for (let i = 0; i < items.length; i++) {
      const entry = items[i].webkitGetAsEntry();
      if (entry) {
        if (entry.isDirectory && !folderName) folderName = entry.name;
        entries.push(entry);
      }
    }
    const nested = await Promise.all(entries.map(readEntryRecursive));
    const files = nested.flat();
    return { files, folderName };
  }

  // Fallback: flat file list
  return { files: Array.from(dataTransfer.files), folderName: null };
}

function wireDropzone() {
  const dz = $("dropzone");
  ["dragenter", "dragover"].forEach((evt) =>
    dz.addEventListener(evt, (e) => {
      e.preventDefault();
      dz.classList.add("dragover");
    }),
  );
  ["dragleave", "drop"].forEach((evt) =>
    dz.addEventListener(evt, (e) => {
      e.preventDefault();
      dz.classList.remove("dragover");
    }),
  );

  dz.addEventListener("drop", async (e) => {
    const { files, folderName } = await collectDroppedFiles(e.dataTransfer);
    if (files.length === 0) return;
    const zipName = folderName ? `${folderName}-converted.zip` : "banglakit-converted.zip";
    await handleFiles(files, zipName);
  });

  $("filepicker").addEventListener("change", async (e) => {
    const files = e.target.files;
    if (files.length > 0) await handleFiles(files);
    e.target.value = "";
  });

  $("folderpicker").addEventListener("change", async (e) => {
    const files = e.target.files;
    if (files.length === 0) return;
    // Derive folder name from webkitRelativePath
    const firstPath = files[0].webkitRelativePath || "";
    const folderName = firstPath.split("/")[0] || "folder";
    await handleFiles(files, `${folderName}-converted.zip`);
    e.target.value = "";
  });
}

// ── Font Mapper ──────────────────────────────────────

const ARROW_SVG = '<svg width="16" height="16" viewBox="0 0 20 20" fill="none"><path d="M4 10h12M12 6l4 4-4 4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>';

const AUTO_EXAMPLES = [
  { from: "SutonnyMJ",  to: "SutonnyOMJ" },
  { from: "AnandaMJ",   to: "Kalpurush",  note: "fallback", fallback: true },
];

const MANUAL_EXAMPLES = ["SutonnyMJ"];

function renderMapRow(from, to, note, fallback) {
  return `<div class="map-row">
    <span class="font-from">${escapeHtml(from)}</span>
    <span class="map-arrow">${ARROW_SVG}</span>
    <span class="font-to${fallback ? " fallback" : ""}">${escapeHtml(to)}</span>
    ${note ? `<span class="map-note">${escapeHtml(note)}</span>` : ""}
  </div>`;
}

function renderAutoMap() {
  $("autoMapList").innerHTML = AUTO_EXAMPLES
    .map((e) => renderMapRow(e.from, e.to, e.note, e.fallback))
    .join("");
}

function renderManualMap() {
  const target = $("unicodeFont").value;
  $("manualMapList").innerHTML = MANUAL_EXAMPLES
    .map((f) => renderMapRow(f, target))
    .join("");
}

function wireMapper() {
  const btns = document.querySelectorAll(".toggle-btn");
  const indicator = document.querySelector(".toggle-indicator");

  btns.forEach((btn) => {
    btn.addEventListener("click", () => {
      const isAuto = btn.dataset.map === "auto";
      autoMap = isAuto;

      btns.forEach((b) => {
        b.classList.toggle("active", b === btn);
        b.setAttribute("aria-checked", b === btn);
      });
      indicator.classList.toggle("right", !isAuto);

      $("mapAuto").classList.toggle("active", isAuto);
      $("mapManual").classList.toggle("active", !isAuto);

      renderTextarea();
    });
  });

  $("unicodeFont").addEventListener("change", () => {
    renderManualMap();
    renderTextarea();
  });

  renderAutoMap();
  renderManualMap();
}

// ── Live text conversion ─────────────────────────────

function renderTextarea() {
  const opts = options();
  const lines = $("textin").value.split(/\r?\n/);
  const out = lines.map((line) => {
    const r = convertText(line, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
    return r.text;
  });
  $("textout").textContent = out.join("\n");
}

function wireTextarea() {
  $("textin").addEventListener("input", renderTextarea);
  $("mode").addEventListener("change", renderTextarea);
  renderTextarea();
}

(async () => {
  await init();
  $("version").textContent = `banglakit-wasm ${coreVersion()}`;
  wireDropzone();
  wireMapper();
  wireTextarea();
})();
