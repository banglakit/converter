# Google Workspace Add-on for Banglakit Converter

**Author:** Aniruddha Adhikary
**Date:** 2026-05-11
**Status:** Approved

---

## Overview

A Google Workspace Add-on that converts Bijoy-encoded Bengali text to Unicode within Google Docs, Sheets, and Slides. The add-on runs the existing `banglakit-wasm` module client-side in a modal dialog iframe, communicating with Apps Script for document read/write.

## Architecture

```
Extensions menu → "Banglakit Converter" → "Convert to Unicode"
        │
        ▼
   Apps Script (server-side)
   ├── Reads document structure
   ├── Extracts (text, fontFamily) pairs per run
   └── Opens modal dialog, passes run data
        │
        ▼
   HTML Dialog (client-side iframe)
   ├── Loads banglakit-wasm from https://banglakit.com/converter/pkg/
   ├── Scan phase: classifyRun() on each run
   ├── Shows convert dialog (scope + font mapping)
   └── Convert phase: convertRun() on each run
        │
        ▼
   Apps Script (server-side)
   └── Writes converted text + font changes back to document
```

Apps Script can read/write the document but can't run WASM. The dialog iframe can run WASM but can't touch the document. Data is shuttled between them: runs go to the dialog for classification/conversion, results come back for write-back.

## Editor Support

| App | API | Unit of conversion |
|-----|-----|--------------------|
| Docs | `DocumentApp` → paragraphs → text runs | `Text.getFontFamily(offset)` per character range |
| Sheets | `SpreadsheetApp` → cells | `Range.getFontFamily()` per cell |
| Slides | `SlidesApp` → shapes → text ranges | `TextRange.getTextStyle().getFontFamily()` per run |

One add-on, three editors. The only difference is the Apps Script code that reads/writes the document model — the dialog and WASM engine are shared.

## File Structure

```
google-addon/
├── appsscript.json           # Scopes for docs + sheets + slides
├── Common.gs                 # Shared: applyConversions(), showDialog()
├── Docs.gs                   # onOpen(), extractRuns() for Docs
├── Sheets.gs                 # onOpen(), extractRuns() for Sheets
├── Slides.gs                 # onOpen(), extractRuns() for Slides
├── dialog.html               # Shared modal dialog + WASM
└── (WASM loaded from CDN)
```

## Conversion Flow

```
User clicks Extensions → Banglakit Converter → Convert to Unicode
        │
        ▼
   Apps Script: detect active app + check selection
        │
        ├── Docs:   selection → RangeElements, or body → all paragraphs
        ├── Sheets: selected range, or active sheet → all cells with content
        └── Slides: selected shapes, or all slides → all shapes with text
        │
        ▼
   Extract runs: [{id, text, font}]
   (id encodes location: para+offset / row+col / slide+shape+offset)
        │
        ▼
   Open modal dialog, pass runs as template data
        │
        ▼
   Dialog (client-side):
   ├── Load WASM from https://banglakit.com/converter/pkg/
   ├── Scan: classifyRun() on each run
   ├── No Bijoy found? → "Nothing to convert" + Close
   ├── Found Bijoy → show font mapping table + Convert/Cancel
   └── Convert: convertRun() on each Bijoy run
        │
        ▼
   Send [{id, newText, newFont}] back via google.script.run
        │
        ▼
   Apps Script: walk document again, apply changes by id
   ├── Docs:   setText() + setFontFamily() per range
   ├── Sheets: setValue() + setFontFamily() per cell
   └── Slides: setText() + getTextStyle().setFontFamily() per range
```

### Selection behavior

- If text/cells/shapes are selected, convert only the selection
- If nothing is selected, convert the entire document/sheet/presentation

### Font replacement

- Smart OMJ mapping using `font_families.toml`: SutonnyMJ → SutonnyOMJ, NikoshMJ → NikoshOMJ, etc.
- Fallback by font class when no OMJ variant exists: serif → Kalpurush, sans → Noto Sans Bengali
- Fallback fonts are not configurable in the add-on (no options dialog — keep it simple)

### Sheets special case

No "runs" within a cell — each cell is one unit. Font is per-cell, so classification uses the cell's font family.

## Dialog UI

Modal dialog, 400x350px, centered over the editor.

**Contents:**
- Scope label: "Entire document" / "Current selection" (color-coded)
- Font mapping table: lists each Bijoy font found → its OMJ replacement
- Convert / Cancel buttons
- "Nothing to convert" state when no Bijoy text is found

Same design as the LibreOffice extension's convert dialog. Plain HTML/CSS, no framework.

## WASM Loading

```html
<script type="module">
  import init, { convertRun, classifyRun } from
    'https://banglakit.com/converter/pkg/banglakit_wasm.js';
  await init();
</script>
```

No WASM bundled in the add-on. Loaded from the existing GitHub Pages deployment at `banglakit.com/converter/pkg/`. WASM updates are automatic: push to main → Pages deploys → add-on picks up new WASM on next dialog open.

## Manifest

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

Minimal scopes — `.currentonly` variants so the add-on only accesses the document the user has open.

## Distribution

| Stage | Access | How |
|-------|--------|-----|
| Development | Just you | `clasp push` + test as editor add-on |
| Unlisted | Share link | Publish in Google Workspace Marketplace as unlisted |
| Public | Anyone | Publish as public listing (requires Google review) |

Managed via `clasp` CLI. No new CI job needed — the add-on is pure Apps Script + HTML, and the only build dependency (WASM) is already handled by `pages.yml`.

## What this does NOT include

- Options/settings dialog — mode is hardcoded to safe, fallback fonts are hardcoded defaults
- Audit log — no equivalent of the CLI's `--audit` JSONL output
- Undo batching — Google editors handle undo natively per write call
- Offline support — requires network to load WASM from CDN
