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
