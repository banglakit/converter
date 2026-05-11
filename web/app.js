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

function options() {
  return {
    mode: $("mode").value,
    encoding: "bijoy",
    unicodeFont: $("unicodeFont").value,
    autoMatchFonts: $("autoMatchFonts").checked,
  };
}

function setStatus(kind, html) {
  const el = $("filestatus");
  el.hidden = false;
  el.className = `status ${kind}`;
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

async function handleFile(file) {
  const name = file.name.toLowerCase();
  const opts = options();
  setStatus("", `Converting <strong>${escapeHtml(file.name)}</strong>…`);

  try {
    if (name.endsWith(".docx")) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = convertDocx(bytes, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      downloadBlob(result.bytes, DOCX_MIME, outputName(file.name, "docx"));
      reportFileResult(file.name, result);
    } else if (name.endsWith(".pptx")) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = convertPptx(bytes, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      downloadBlob(result.bytes, PPTX_MIME, outputName(file.name, "pptx"));
      reportFileResult(file.name, result);
    } else if (name.endsWith(".txt") || name.endsWith(".md") || file.type.startsWith("text/")) {
      const text = await file.text();
      const result = convertText(text, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      downloadBlob(result.text, "text/plain;charset=utf-8", outputName(file.name, "txt"));
      reportFileResult(file.name, { anyChange: result.changed, runsConverted: result.runs_converted });
    } else {
      setStatus("err", `Unsupported file type: <code>${escapeHtml(file.name)}</code>. Expected .docx, .pptx, or .txt.`);
    }
  } catch (err) {
    console.error(err);
    setStatus("err", `Conversion failed: <code>${escapeHtml(err.message ?? String(err))}</code>`);
  }
}

function reportFileResult(name, result) {
  const changed = result.anyChange ?? result.any_change;
  const runs = result.runsConverted ?? result.runs_converted ?? 0;
  const kind = changed ? "ok" : "";
  const summary = changed
    ? `Converted <strong>${escapeHtml(name)}</strong> — ${runs} run${runs === 1 ? "" : "s"} rewritten. Download started.`
    : `No Bijoy runs detected in <strong>${escapeHtml(name)}</strong>. Output is identical to the input.`;
  setStatus(kind, summary + `<span class="meta">mode=${$("mode").value} target=${$("unicodeFont").value}</span>`);
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
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
  dz.addEventListener("drop", (e) => {
    const file = e.dataTransfer?.files?.[0];
    if (file) handleFile(file);
  });
  $("filepicker").addEventListener("change", (e) => {
    const file = e.target.files?.[0];
    if (file) handleFile(file);
    e.target.value = "";
  });
}

function wireTextarea() {
  const render = () => {
    const opts = options();
    const lines = $("textin").value.split(/\r?\n/);
    const out = lines.map((line) => {
      const r = convertText(line, opts.mode, opts.encoding, opts.unicodeFont, opts.autoMatchFonts);
      return r.text;
    });
    $("textout").textContent = out.join("\n");
  };
  ["textin", "mode", "unicodeFont"].forEach((id) =>
    $(id).addEventListener("input", render),
  );
  $("mode").addEventListener("change", render);
  $("unicodeFont").addEventListener("change", render);
  $("autoMatchFonts").addEventListener("change", render);
  render();
}

(async () => {
  await init();
  $("version").textContent = `banglakit-wasm ${coreVersion()}`;
  wireDropzone();
  wireTextarea();
})();
