# Google Workspace Add-on Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Google Workspace Add-on that converts Bijoy-encoded Bengali text to Unicode within Google Docs, Sheets, and Slides, using the existing banglakit-wasm module client-side.

**Architecture:** Apps Script reads the document model (paragraphs/cells/shapes), extracts `{text, font}` pairs, and passes them to an HTML modal dialog. The dialog loads `banglakit-wasm` from `https://banglakit.com/converter/pkg/`, runs classification and conversion client-side, then sends results back to Apps Script for write-back.

**Tech Stack:** Google Apps Script (V8 runtime), HTML/CSS/JS dialog, banglakit-wasm (existing, loaded from CDN)

---

## File Structure

```
google-addon/
├── appsscript.json           # Manifest: scopes, runtime, menu hooks
├── Common.gs                 # showConvertDialog(), applyConversions()
├── Docs.gs                   # onOpen() menu, extractRunsDocs()
├── Sheets.gs                 # onOpen() menu, extractRunsSheets()
├── Slides.gs                 # onOpen() menu, extractRunsSlides()
├── dialog.html               # Modal dialog: WASM load, scan, UI, convert
├── .clasp.json.example       # Template for clasp project config
└── README.md                 # Setup, dev workflow, testing guide
```

**Responsibilities:**
- `Common.gs` — shared logic: opens the dialog with run data, receives conversion results and dispatches to the correct editor's write-back function. The single entry point both Docs/Sheets/Slides menu handlers call.
- `Docs.gs` / `Sheets.gs` / `Slides.gs` — editor-specific: registers the menu in `onOpen()`, extracts `{id, text, font}` arrays from the document model, and applies conversions back. Each file is self-contained for its editor.
- `dialog.html` — the entire client-side app: loads WASM, scans runs, renders the convert dialog UI, runs conversion, sends results back via `google.script.run`. No external CSS/JS dependencies beyond the WASM module.

---

### Task 1: Project Scaffold

**Files:**
- Create: `google-addon/appsscript.json`
- Create: `google-addon/.clasp.json.example`
- Create: `google-addon/README.md`

- [ ] **Step 1: Create appsscript.json**

```json
{
  "timeZone": "Asia/Dhaka",
  "dependencies": {},
  "exceptionLogging": "STACKDRIVER",
  "runtimeVersion": "V8",
  "oauthScopes": [
    "https://www.googleapis.com/auth/documents.currentonly",
    "https://www.googleapis.com/auth/spreadsheets.currentonly",
    "https://www.googleapis.com/auth/presentations.currentonly"
  ]
}
```

- [ ] **Step 2: Create .clasp.json.example**

```json
{
  "scriptId": "YOUR_SCRIPT_ID_HERE",
  "rootDir": "."
}
```

- [ ] **Step 3: Create README.md**

```markdown
# Banglakit Converter — Google Workspace Add-on

Converts Bijoy-encoded Bengali text to Unicode in Google Docs, Sheets, and Slides.

## Prerequisites

- Node.js 18+
- [clasp](https://github.com/nicell/clasp) CLI: `npm install -g @nicell/clasp`
- A Google account with Apps Script API enabled

## Setup

1. Log in to clasp:
   ```bash
   clasp login
   ```

2. Create a new Apps Script project:
   ```bash
   cd google-addon
   clasp create --title "Banglakit Converter" --type standalone
   ```
   This creates `.clasp.json` with your script ID.

3. Push the code:
   ```bash
   clasp push
   ```

4. Open in browser:
   ```bash
   clasp open
   ```

## Testing

1. From the Apps Script editor, click **Deploy → Test deployments**
2. Select **Google Docs** (or Sheets/Slides) as the test application
3. Open a test document — the **Extensions → Banglakit Converter** menu appears
4. Add some Bijoy-encoded text (e.g., `Avwg evsjvq Mvb MvB|` in SutonnyMJ font)
5. Click **Extensions → Banglakit Converter → Convert to Unicode**
6. The dialog should scan, show font mappings, and convert on confirmation

## How it works

1. Apps Script reads the document's text runs with font metadata
2. A modal dialog loads `banglakit-wasm` from `https://banglakit.com/converter/pkg/`
3. The WASM classifier scans each run to identify Bijoy-encoded text
4. User confirms, WASM converts, results are written back to the document
```

- [ ] **Step 4: Commit**

```bash
git add google-addon/appsscript.json google-addon/.clasp.json.example google-addon/README.md
git commit -m "scaffold: Google Workspace Add-on project structure"
```

---

### Task 2: Google Docs — Extract Runs

**Files:**
- Create: `google-addon/Docs.gs`

Apps Script's `DocumentApp` gives us paragraphs, and within each paragraph we can enumerate character-level font families via `Text.getFontFamily(offset)`. We need to segment each paragraph into contiguous runs of the same font, producing `{id, text, font}` objects.

- [ ] **Step 1: Create Docs.gs with onOpen and extractRunsDocs**

```javascript
/**
 * Adds the Banglakit Converter menu when a Doc is opened.
 */
function onOpen() {
  DocumentApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogDocs')
    .addToUi();
}

/**
 * Entry point for Docs conversion — called from the menu.
 */
function showConvertDialogDocs() {
  var doc = DocumentApp.getActiveDocument();
  var selection = doc.getSelection();
  var runs;
  var scope;

  if (selection) {
    runs = extractRunsFromSelection_(selection);
    scope = 'selection';
  } else {
    runs = extractRunsFromBody_(doc.getBody());
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'docs');
}

/**
 * Extracts {id, text, font} runs from the entire document body.
 * Segments each paragraph into contiguous runs of the same font family.
 *
 * ID format: "docs:P<paraIndex>:C<charStart>-<charEnd>"
 */
function extractRunsFromBody_(body) {
  var runs = [];
  var numChildren = body.getNumChildren();

  for (var i = 0; i < numChildren; i++) {
    var child = body.getChild(i);
    if (child.getType() === DocumentApp.ElementType.PARAGRAPH ||
        child.getType() === DocumentApp.ElementType.LIST_ITEM) {
      var textEl = child.editAsText();
      var paraRuns = segmentByFont_(textEl, i);
      runs = runs.concat(paraRuns);
    }
  }

  return runs;
}

/**
 * Extracts runs from the current selection only.
 */
function extractRunsFromSelection_(selection) {
  var runs = [];
  var elements = selection.getRangeElements();

  for (var i = 0; i < elements.length; i++) {
    var rangeEl = elements[i];
    var el = rangeEl.getElement();

    if (el.getType() === DocumentApp.ElementType.TEXT) {
      var textEl = el.editAsText();
      var start = rangeEl.isPartial() ? rangeEl.getStartOffset() : 0;
      var end = rangeEl.isPartial() ? rangeEl.getEndOffsetInclusive() : textEl.getText().length - 1;

      // Find the paragraph index for the ID
      var para = el.getParent();
      var paraIndex = para.getParent().getChildIndex(para);

      var segmented = segmentByFont_(textEl, paraIndex, start, end);
      runs = runs.concat(segmented);
    }
  }

  return runs;
}

/**
 * Segments a Text element into contiguous runs of the same font family.
 *
 * @param {GoogleAppsScript.Document.Text} textEl - The text element to segment.
 * @param {number} paraIndex - Paragraph index for building run IDs.
 * @param {number} [startOffset=0] - Start character offset (inclusive).
 * @param {number} [endOffset] - End character offset (inclusive). Defaults to end of text.
 * @returns {Array<{id: string, text: string, font: string|null}>}
 */
function segmentByFont_(textEl, paraIndex, startOffset, endOffset) {
  var fullText = textEl.getText();
  if (!fullText) return [];

  var start = (startOffset !== undefined) ? startOffset : 0;
  var end = (endOffset !== undefined) ? endOffset : fullText.length - 1;
  if (start > end) return [];

  var runs = [];
  var runStart = start;
  var currentFont = textEl.getFontFamily(start);

  for (var i = start + 1; i <= end + 1; i++) {
    var font = (i <= end) ? textEl.getFontFamily(i) : null;
    if (font !== currentFont || i > end) {
      var runText = fullText.substring(runStart, i);
      if (runText.length > 0) {
        runs.push({
          id: 'docs:P' + paraIndex + ':C' + runStart + '-' + (i - 1),
          text: runText,
          font: currentFont
        });
      }
      runStart = i;
      currentFont = font;
    }
  }

  return runs;
}
```

- [ ] **Step 2: Verify the file is syntactically valid**

Open the Apps Script editor via `clasp open` and confirm Docs.gs has no syntax errors (red underlines). Or run:

```bash
cd google-addon && clasp push
```

Expected: `Pushed 2 files.` (appsscript.json + Docs.gs) — no errors.

- [ ] **Step 3: Commit**

```bash
git add google-addon/Docs.gs
git commit -m "feat(google-addon): Docs run extraction with font segmentation"
```

---

### Task 3: Common — Dialog Launcher and Write-Back

**Files:**
- Create: `google-addon/Common.gs`

This file is the bridge between all three editors and the dialog. It opens the modal, and receives conversion results from the dialog's `google.script.run` callback.

- [ ] **Step 1: Create Common.gs**

```javascript
/**
 * Opens the convert dialog, passing extracted runs as template data.
 *
 * @param {Array<{id: string, text: string, font: string|null}>} runs - Extracted runs.
 * @param {string} scope - 'document' or 'selection'.
 * @param {string} editor - 'docs', 'sheets', or 'slides'.
 */
function showConvertDialog_(runs, scope, editor) {
  var template = HtmlService.createTemplateFromFile('dialog');
  template.runs = JSON.stringify(runs);
  template.scope = scope;
  template.editor = editor;

  var html = template.evaluate()
    .setWidth(400)
    .setHeight(350)
    .setSandboxMode(HtmlService.SandboxMode.IFRAME);

  var ui;
  if (editor === 'docs') {
    ui = DocumentApp.getUi();
  } else if (editor === 'sheets') {
    ui = SpreadsheetApp.getUi();
  } else {
    ui = SlidesApp.getUi();
  }

  ui.showModalDialog(html, 'Banglakit Converter');
}

/**
 * Receives conversion results from the dialog and dispatches to the
 * correct editor's write-back function.
 *
 * Called from the dialog via google.script.run.applyConversions(results, editor).
 *
 * @param {Array<{id: string, newText: string, newFont: string}>} results
 * @param {string} editor - 'docs', 'sheets', or 'slides'
 */
function applyConversions(results, editor) {
  if (!results || results.length === 0) return;

  if (editor === 'docs') {
    applyConversionsDocs_(results);
  } else if (editor === 'sheets') {
    applyConversionsSheets_(results);
  } else if (editor === 'slides') {
    applyConversionsSlides_(results);
  }
}

/**
 * Applies conversion results to a Google Doc.
 * ID format: "docs:P<paraIndex>:C<charStart>-<charEnd>"
 */
function applyConversionsDocs_(results) {
  var doc = DocumentApp.getActiveDocument();
  var body = doc.getBody();

  // Process in reverse order so character offsets remain valid
  // (later offsets first, so earlier ones aren't shifted).
  var sorted = results.slice().sort(function(a, b) {
    var aParts = parseDocsId_(a.id);
    var bParts = parseDocsId_(b.id);
    if (bParts.para !== aParts.para) return bParts.para - aParts.para;
    return bParts.charStart - aParts.charStart;
  });

  for (var i = 0; i < sorted.length; i++) {
    var r = sorted[i];
    var parts = parseDocsId_(r.id);
    var child = body.getChild(parts.para);
    var textEl = child.editAsText();

    textEl.deleteText(parts.charStart, parts.charEnd);
    textEl.insertText(parts.charStart, r.newText);

    var newEnd = parts.charStart + r.newText.length - 1;
    textEl.setFontFamily(parts.charStart, newEnd, r.newFont);
  }
}

/**
 * Parses a Docs run ID into its components.
 * "docs:P5:C10-25" → {para: 5, charStart: 10, charEnd: 25}
 */
function parseDocsId_(id) {
  var match = id.match(/^docs:P(\d+):C(\d+)-(\d+)$/);
  return {
    para: parseInt(match[1], 10),
    charStart: parseInt(match[2], 10),
    charEnd: parseInt(match[3], 10)
  };
}
```

- [ ] **Step 2: Push and verify no syntax errors**

```bash
cd google-addon && clasp push
```

Expected: `Pushed 3 files.` — no errors.

- [ ] **Step 3: Commit**

```bash
git add google-addon/Common.gs
git commit -m "feat(google-addon): shared dialog launcher and Docs write-back"
```

---

### Task 4: Dialog HTML — WASM Scan, UI, and Convert

**Files:**
- Create: `google-addon/dialog.html`

This is the core of the add-on — the modal dialog that loads WASM, scans runs, shows the font mapping UI, and sends results back. All conversion logic runs client-side in the iframe.

- [ ] **Step 1: Create dialog.html**

```html
<!DOCTYPE html>
<html>
<head>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: 'Google Sans', Roboto, Arial, sans-serif;
    font-size: 14px;
    color: #202124;
    padding: 20px;
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  .header {
    text-align: center;
    margin-bottom: 16px;
  }
  .header h1 {
    font-size: 18px;
    font-weight: 500;
    color: #202124;
  }
  .header .subtitle {
    font-size: 12px;
    color: #5f6368;
    margin-top: 2px;
  }

  .scope {
    padding: 8px 12px;
    border-radius: 4px;
    font-size: 13px;
    margin-bottom: 12px;
  }
  .scope.document {
    background: #e8f0fe;
    border-left: 3px solid #1a73e8;
    color: #174ea6;
  }
  .scope.selection {
    background: #f3e8fd;
    border-left: 3px solid #8430ce;
    color: #5b2c8a;
  }

  .font-map {
    flex: 1;
    overflow-y: auto;
    margin-bottom: 16px;
  }
  .font-map-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #5f6368;
    margin-bottom: 8px;
  }
  .font-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    background: #f8f9fa;
    border-radius: 4px;
    margin-bottom: 4px;
    font-size: 13px;
  }
  .font-row .from { color: #202124; min-width: 110px; }
  .font-row .arrow { color: #9aa0a6; }
  .font-row .to { color: #137333; }

  .empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #5f6368;
    font-size: 14px;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .btn {
    padding: 8px 20px;
    border-radius: 4px;
    font-size: 14px;
    font-family: inherit;
    cursor: pointer;
    border: none;
  }
  .btn-cancel {
    background: transparent;
    color: #1a73e8;
  }
  .btn-cancel:hover { background: #f0f4ff; }
  .btn-convert {
    background: #1a73e8;
    color: white;
  }
  .btn-convert:hover { background: #1557b0; }
  .btn-convert:disabled {
    background: #dadce0;
    color: #80868b;
    cursor: default;
  }

  .loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #5f6368;
  }
  .spinner {
    width: 24px; height: 24px;
    border: 3px solid #dadce0;
    border-top-color: #1a73e8;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    margin-right: 10px;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .status {
    text-align: center;
    margin-top: 8px;
    font-size: 12px;
    color: #5f6368;
  }
  .status.error { color: #c5221f; }
</style>
</head>
<body>

<div class="header">
  <h1>Banglakit Converter</h1>
  <div class="subtitle">Bijoy to Unicode</div>
</div>

<div id="loading" class="loading">
  <div class="spinner"></div>
  <span>Loading converter...</span>
</div>

<div id="content" style="display:none; flex:1; display:none; flex-direction:column;">
  <div id="scopeBar" class="scope"></div>
  <div id="fontMapSection" class="font-map">
    <div class="font-map-label">Font Mapping</div>
    <div id="fontMapRows"></div>
  </div>
  <div id="emptyState" class="empty" style="display:none;">
    No Bijoy text found.
  </div>
  <div class="actions">
    <button class="btn btn-cancel" onclick="closeDialog()">Cancel</button>
    <button class="btn btn-convert" id="convertBtn" onclick="doConvert()">Convert</button>
  </div>
</div>

<div id="statusMsg" class="status"></div>

<script>
// Template data injected by Apps Script
var RUNS = <?!= runs ?>;
var SCOPE = '<?= scope ?>';
var EDITOR = '<?= editor ?>';

// State
var convertRun = null;  // WASM function reference
var scanResults = [];    // [{id, text, font, result: {text, changed, suggestedFont, ...}}]

async function initialize() {
  try {
    var mod = await import('https://banglakit.com/converter/pkg/banglakit_wasm.js');
    await mod.default();
    convertRun = mod.convertRun;

    scan();
  } catch (err) {
    showError('Failed to load converter: ' + err.message);
  }
}

function scan() {
  var fontMap = {};  // {bijoyFont: omjFont}
  scanResults = [];

  for (var i = 0; i < RUNS.length; i++) {
    var run = RUNS[i];
    var result = convertRun(
      run.text,
      run.font || undefined,  // fontName
      'bijoy',                // encoding
      'safe',                 // mode
      undefined,              // unicodeFont (let auto-match handle it)
      true                    // autoMatchFonts
    );

    if (result.changed) {
      scanResults.push({
        id: run.id,
        text: run.text,
        font: run.font,
        newText: result.text,
        newFont: result.suggested_font
      });

      var fromFont = run.font || 'Unknown';
      var toFont = result.suggested_font || 'Kalpurush';
      if (!fontMap[fromFont]) {
        fontMap[fromFont] = toFont;
      }
    }
  }

  renderUI(fontMap);
}

function renderUI(fontMap) {
  document.getElementById('loading').style.display = 'none';
  var content = document.getElementById('content');
  content.style.display = 'flex';

  // Scope bar
  var scopeBar = document.getElementById('scopeBar');
  scopeBar.className = 'scope ' + (SCOPE === 'selection' ? 'selection' : 'document');
  scopeBar.innerHTML = '<strong>Scope:</strong> ' +
    (SCOPE === 'selection' ? 'Current selection' : 'Entire document');

  var fontKeys = Object.keys(fontMap);

  if (scanResults.length === 0) {
    document.getElementById('fontMapSection').style.display = 'none';
    document.getElementById('emptyState').style.display = 'flex';
    document.getElementById('convertBtn').disabled = true;
    document.getElementById('convertBtn').textContent = 'Nothing to convert';
    return;
  }

  // Font mapping rows
  var rowsHtml = '';
  for (var i = 0; i < fontKeys.length; i++) {
    rowsHtml += '<div class="font-row">' +
      '<span class="from">' + escapeHtml(fontKeys[i]) + '</span>' +
      '<span class="arrow">&rarr;</span>' +
      '<span class="to">' + escapeHtml(fontMap[fontKeys[i]]) + '</span>' +
      '</div>';
  }
  document.getElementById('fontMapRows').innerHTML = rowsHtml;
}

function doConvert() {
  var btn = document.getElementById('convertBtn');
  btn.disabled = true;
  btn.textContent = 'Converting...';

  var results = scanResults.map(function(r) {
    return { id: r.id, newText: r.newText, newFont: r.newFont };
  });

  google.script.run
    .withSuccessHandler(function() {
      showStatus('Converted ' + results.length + ' run' + (results.length === 1 ? '' : 's') + '.');
      setTimeout(function() { google.script.host.close(); }, 1200);
    })
    .withFailureHandler(function(err) {
      showError('Write-back failed: ' + err.message);
      btn.disabled = false;
      btn.textContent = 'Convert';
    })
    .applyConversions(results, EDITOR);
}

function closeDialog() {
  google.script.host.close();
}

function showStatus(msg) {
  var el = document.getElementById('statusMsg');
  el.className = 'status';
  el.textContent = msg;
}

function showError(msg) {
  document.getElementById('loading').style.display = 'none';
  var el = document.getElementById('statusMsg');
  el.className = 'status error';
  el.textContent = msg;
}

function escapeHtml(s) {
  var div = document.createElement('div');
  div.textContent = s || '';
  return div.innerHTML;
}

initialize();
</script>
</body>
</html>
```

- [ ] **Step 2: Push and verify**

```bash
cd google-addon && clasp push
```

Expected: `Pushed 4 files.` — no errors.

- [ ] **Step 3: Smoke test in Google Docs**

1. Run `clasp open` to open the Apps Script editor
2. Click **Deploy → Test deployments → Google Docs**
3. Open a test document with Bijoy text (e.g., `Avwg evsjvq Mvb MvB|` in SutonnyMJ font)
4. Click **Extensions → Banglakit Converter → Convert to Unicode**
5. Verify: dialog opens, shows scope "Entire document", shows font mapping (SutonnyMJ → SutonnyOMJ), Convert button works

- [ ] **Step 4: Commit**

```bash
git add google-addon/dialog.html
git commit -m "feat(google-addon): dialog with WASM scan, font mapping UI, and convert"
```

---

### Task 5: Google Sheets — Extract Runs and Write-Back

**Files:**
- Create: `google-addon/Sheets.gs`
- Modify: `google-addon/Common.gs` (add `applyConversionsSheets_`)

Sheets is simpler than Docs — each cell is one run. Font is per-cell via `Range.getFontFamily()`.

- [ ] **Step 1: Create Sheets.gs**

```javascript
/**
 * Adds the Banglakit Converter menu when a Sheet is opened.
 */
function onOpen() {
  SpreadsheetApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogSheets')
    .addToUi();
}

/**
 * Entry point for Sheets conversion.
 */
function showConvertDialogSheets() {
  var sheet = SpreadsheetApp.getActiveSpreadsheet();
  var selection = sheet.getSelection();
  var activeRange = selection.getActiveRange();
  var runs;
  var scope;

  // If user selected specific cells, use those; otherwise use all data
  if (activeRange && !isEntireSheet_(activeRange, sheet.getActiveSheet())) {
    runs = extractRunsFromRange_(activeRange);
    scope = 'selection';
  } else {
    var dataRange = sheet.getActiveSheet().getDataRange();
    runs = extractRunsFromRange_(dataRange);
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'sheets');
}

/**
 * Checks if the active range covers the entire sheet (i.e., no meaningful selection).
 */
function isEntireSheet_(range, sheet) {
  return range.getNumRows() >= sheet.getMaxRows() &&
         range.getNumColumns() >= sheet.getMaxColumns();
}

/**
 * Extracts {id, text, font} from each non-empty cell in a range.
 * ID format: "sheets:R<row>:C<col>" (1-based)
 */
function extractRunsFromRange_(range) {
  var runs = [];
  var values = range.getValues();
  var fonts = range.getFontFamilies();
  var startRow = range.getRow();
  var startCol = range.getColumn();

  for (var r = 0; r < values.length; r++) {
    for (var c = 0; c < values[r].length; c++) {
      var val = values[r][c];
      if (val === '' || val === null || val === undefined) continue;
      // Only process string values
      if (typeof val !== 'string') continue;

      runs.push({
        id: 'sheets:R' + (startRow + r) + ':C' + (startCol + c),
        text: val,
        font: fonts[r][c]
      });
    }
  }

  return runs;
}
```

- [ ] **Step 2: Add applyConversionsSheets_ to Common.gs**

Add this function at the end of `google-addon/Common.gs`:

```javascript
/**
 * Applies conversion results to a Google Sheet.
 * ID format: "sheets:R<row>:C<col>"
 */
function applyConversionsSheets_(results) {
  var sheet = SpreadsheetApp.getActiveSpreadsheet().getActiveSheet();

  for (var i = 0; i < results.length; i++) {
    var r = results[i];
    var match = r.id.match(/^sheets:R(\d+):C(\d+)$/);
    var row = parseInt(match[1], 10);
    var col = parseInt(match[2], 10);

    var cell = sheet.getRange(row, col);
    cell.setValue(r.newText);
    cell.setFontFamily(r.newFont);
  }
}
```

- [ ] **Step 3: Push and verify**

```bash
cd google-addon && clasp push
```

Expected: `Pushed 5 files.` — no errors.

- [ ] **Step 4: Smoke test in Google Sheets**

1. Click **Deploy → Test deployments → Google Sheets**
2. Open a test spreadsheet, type Bijoy text in a few cells, set font to SutonnyMJ
3. Click **Extensions → Banglakit Converter → Convert to Unicode**
4. Verify: dialog opens, shows font mapping, converts cells on confirm

- [ ] **Step 5: Commit**

```bash
git add google-addon/Sheets.gs google-addon/Common.gs
git commit -m "feat(google-addon): Sheets cell extraction and write-back"
```

---

### Task 6: Google Slides — Extract Runs and Write-Back

**Files:**
- Create: `google-addon/Slides.gs`
- Modify: `google-addon/Common.gs` (add `applyConversionsSlides_`)

Slides uses `SlidesApp` → shapes → text ranges. Each shape's text body can have multiple runs with different fonts, similar to Docs paragraphs.

- [ ] **Step 1: Create Slides.gs**

```javascript
/**
 * Adds the Banglakit Converter menu when a Presentation is opened.
 */
function onOpen() {
  SlidesApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogSlides')
    .addToUi();
}

/**
 * Entry point for Slides conversion.
 */
function showConvertDialogSlides() {
  var presentation = SlidesApp.getActivePresentation();
  var selection = presentation.getSelection();
  var runs;
  var scope;

  if (selection.getSelectionType() === SlidesApp.SelectionType.TEXT) {
    var textRange = selection.getTextRange();
    var pageElement = selection.getCurrentPage()
      ? null  // we'll get it from the text range's parent
      : null;
    // For text selection, extract from the selected text range
    runs = extractRunsFromTextRange_(textRange, 0, 0);
    scope = 'selection';
  } else {
    runs = extractRunsFromAllSlides_(presentation);
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'slides');
}

/**
 * Extracts runs from all slides in the presentation.
 */
function extractRunsFromAllSlides_(presentation) {
  var runs = [];
  var slides = presentation.getSlides();

  for (var s = 0; s < slides.length; s++) {
    var shapes = slides[s].getShapes();
    for (var sh = 0; sh < shapes.length; sh++) {
      var textRange;
      try {
        textRange = shapes[sh].getText();
      } catch (e) {
        continue;  // Shape has no text frame
      }
      var shapeRuns = extractRunsFromTextRange_(textRange, s, sh);
      runs = runs.concat(shapeRuns);
    }
  }

  return runs;
}

/**
 * Segments a Slides TextRange into contiguous runs of the same font.
 * ID format: "slides:S<slideIdx>:SH<shapeIdx>:C<charStart>-<charEnd>"
 */
function extractRunsFromTextRange_(textRange, slideIdx, shapeIdx) {
  var runs = [];
  var textRuns = textRange.getRuns();

  var offset = 0;
  for (var i = 0; i < textRuns.length; i++) {
    var run = textRuns[i];
    var text = run.asString();
    // Strip trailing newline that Slides appends
    if (text.endsWith('\n')) {
      text = text.substring(0, text.length - 1);
    }
    if (text.length === 0) {
      offset += run.asString().length;
      continue;
    }

    var font = run.getTextStyle().getFontFamily();
    var endOffset = offset + text.length - 1;

    runs.push({
      id: 'slides:S' + slideIdx + ':SH' + shapeIdx + ':C' + offset + '-' + endOffset,
      text: text,
      font: font
    });

    offset += run.asString().length;
  }

  return runs;
}
```

- [ ] **Step 2: Add applyConversionsSlides_ to Common.gs**

Add this function at the end of `google-addon/Common.gs`:

```javascript
/**
 * Applies conversion results to a Google Slides presentation.
 * ID format: "slides:S<slideIdx>:SH<shapeIdx>:C<charStart>-<charEnd>"
 */
function applyConversionsSlides_(results) {
  var presentation = SlidesApp.getActivePresentation();
  var slides = presentation.getSlides();

  // Process in reverse character-offset order within each shape
  var sorted = results.slice().sort(function(a, b) {
    var aP = parseSlidesId_(a.id);
    var bP = parseSlidesId_(b.id);
    if (bP.slide !== aP.slide) return bP.slide - aP.slide;
    if (bP.shape !== aP.shape) return bP.shape - aP.shape;
    return bP.charStart - aP.charStart;
  });

  for (var i = 0; i < sorted.length; i++) {
    var r = sorted[i];
    var parts = parseSlidesId_(r.id);
    var shape = slides[parts.slide].getShapes()[parts.shape];
    var textRange = shape.getText();

    // Get the specific range and replace
    var range = textRange.getRange(parts.charStart, parts.charEnd + 1);
    var style = range.getTextStyle();

    range.setText(r.newText);

    // Re-acquire the range after setText (offsets may shift)
    var newRange = textRange.getRange(parts.charStart, parts.charStart + r.newText.length);
    newRange.getTextStyle().setFontFamily(r.newFont);
  }
}

/**
 * Parses a Slides run ID into its components.
 * "slides:S0:SH2:C10-25" → {slide: 0, shape: 2, charStart: 10, charEnd: 25}
 */
function parseSlidesId_(id) {
  var match = id.match(/^slides:S(\d+):SH(\d+):C(\d+)-(\d+)$/);
  return {
    slide: parseInt(match[1], 10),
    shape: parseInt(match[2], 10),
    charStart: parseInt(match[3], 10),
    charEnd: parseInt(match[4], 10)
  };
}
```

- [ ] **Step 3: Push and verify**

```bash
cd google-addon && clasp push
```

Expected: `Pushed 6 files.` — no errors.

- [ ] **Step 4: Smoke test in Google Slides**

1. Click **Deploy → Test deployments → Google Slides**
2. Open a test presentation, add text boxes with Bijoy text in SutonnyMJ
3. Click **Extensions → Banglakit Converter → Convert to Unicode**
4. Verify: dialog opens, shows font mapping, converts text in shapes on confirm

- [ ] **Step 5: Commit**

```bash
git add google-addon/Slides.gs google-addon/Common.gs
git commit -m "feat(google-addon): Slides text range extraction and write-back"
```

---

### Task 7: Duplicate onOpen Fix and Final Polish

**Files:**
- Modify: `google-addon/Docs.gs`
- Modify: `google-addon/Sheets.gs`
- Modify: `google-addon/Slides.gs`

Apps Script only allows one `onOpen()` function per project. Since all three editor files define `onOpen()`, only one will run. The fix: use a single `onOpen()` that detects which editor is active.

- [ ] **Step 1: Replace onOpen in all three files with a single one in Common.gs**

Add to the top of `google-addon/Common.gs`:

```javascript
/**
 * Single onOpen handler that detects the active editor and adds the menu.
 */
function onOpen() {
  var ui;
  try {
    ui = DocumentApp.getUi();
    ui.createAddonMenu()
      .addItem('Convert to Unicode', 'showConvertDialogDocs')
      .addToUi();
    return;
  } catch (e) {}

  try {
    ui = SpreadsheetApp.getUi();
    ui.createAddonMenu()
      .addItem('Convert to Unicode', 'showConvertDialogSheets')
      .addToUi();
    return;
  } catch (e) {}

  try {
    ui = SlidesApp.getUi();
    ui.createAddonMenu()
      .addItem('Convert to Unicode', 'showConvertDialogSlides')
      .addToUi();
    return;
  } catch (e) {}
}
```

- [ ] **Step 2: Remove onOpen from Docs.gs, Sheets.gs, Slides.gs**

Delete the `onOpen()` function from each of these three files. Keep only the `showConvertDialog*` entry points and the extract/segment helper functions.

From `Docs.gs`, remove:
```javascript
function onOpen() {
  DocumentApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogDocs')
    .addToUi();
}
```

From `Sheets.gs`, remove:
```javascript
function onOpen() {
  SpreadsheetApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogSheets')
    .addToUi();
}
```

From `Slides.gs`, remove:
```javascript
function onOpen() {
  SlidesApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogSlides')
    .addToUi();
}
```

- [ ] **Step 3: Push and verify**

```bash
cd google-addon && clasp push
```

Expected: `Pushed 6 files.` — no errors.

- [ ] **Step 4: Test in all three editors**

Test in each editor via **Deploy → Test deployments**:
1. **Google Docs** — menu appears, conversion works
2. **Google Sheets** — menu appears, conversion works
3. **Google Slides** — menu appears, conversion works

- [ ] **Step 5: Commit**

```bash
git add google-addon/Common.gs google-addon/Docs.gs google-addon/Sheets.gs google-addon/Slides.gs
git commit -m "fix(google-addon): unify onOpen into Common.gs for multi-editor support"
```

---

### Task 8: End-to-End Verification

**Files:** None (testing only)

- [ ] **Step 1: Prepare test documents**

Create three test documents:

**Google Doc:**
1. Open a new Google Doc
2. Type: `Avwg evsjvq Mvb MvB|` — set font to SutonnyMJ
3. On the next line type: `Hello world` — keep in Arial
4. On the next line type: `GB‡K GKwU Mvb` — set font to NikoshMJ

**Google Sheet:**
1. Open a new Google Sheet
2. Cell A1: `Avwg evsjvq` in SutonnyMJ
3. Cell A2: `Hello world` in Arial
4. Cell B1: `GB‡K GKwU` in NikoshMJ

**Google Slides:**
1. Open a new Google Slides
2. Text box 1: `Avwg evsjvq Mvb MvB|` in SutonnyMJ
3. Text box 2: `English text` in Arial

- [ ] **Step 2: Test full-document conversion in each editor**

For each test document:
1. Open the document
2. Click **Extensions → Banglakit Converter → Convert to Unicode**
3. Verify the dialog shows:
   - Scope: "Entire document"
   - Font mapping: SutonnyMJ → SutonnyOMJ (and NikoshMJ → NikoshOMJ if present)
4. Click **Convert**
5. Verify:
   - Bijoy text is converted to Unicode Bengali
   - English text is untouched
   - Font is changed to the OMJ variant
   - Ctrl+Z undoes the changes

- [ ] **Step 3: Test selection-only conversion**

1. In the Google Doc, select only the first line
2. Run the converter
3. Verify only the selected line is converted, other lines unchanged

1. In the Google Sheet, select only cell A1
2. Run the converter
3. Verify only A1 is converted, A2 and B1 unchanged

- [ ] **Step 4: Test "nothing to convert" state**

1. Open a document with only English text
2. Run the converter
3. Verify dialog shows "No Bijoy text found" and Convert button is disabled

- [ ] **Step 5: Final commit**

```bash
git add -A google-addon/
git commit -m "feat(google-addon): complete Google Workspace Add-on for Docs, Sheets, Slides"
```
