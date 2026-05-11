---
name: docx-visual-diff
description: Visually compare original and converted DOCX files by rendering them as PDF pages and inspecting side-by-side. Use whenever the user reports a conversion bug, wants to verify DOCX output quality, says "the output looks wrong", or asks to compare before/after documents. Also trigger on "visual diff", "render docx", "compare documents", or when debugging conversion artifacts like orphaned combining marks, garbled text, or missing characters.
---

# DOCX Visual Diff

Render DOCX files as PDF pages via LibreOffice for visual inspection, then drill into the XML run structure to diagnose conversion bugs.

## Why this matters

Conversion bugs in Bengali DOCX files are often invisible in plain text extraction — they only show up when rendered (orphaned combining marks display as dotted circles, wrong run boundaries break ligatures, etc.). Rendering to PDF and reading the pages visually is the fastest way to spot problems.

## Workflow

### 1. Render both documents as PDF

```bash
mkdir -p /tmp/docx_diff/original /tmp/docx_diff/converted

# Render original
/Applications/LibreOffice.app/Contents/MacOS/soffice --headless \
  --convert-to pdf --outdir /tmp/docx_diff/original "<original.docx>"

# Convert with CLI
cargo run --release -p banglakit-cli -- \
  -i "<original.docx>" -o /tmp/docx_diff/converted.docx

# Render converted
/Applications/LibreOffice.app/Contents/MacOS/soffice --headless \
  --convert-to pdf --outdir /tmp/docx_diff/converted /tmp/docx_diff/converted.docx
```

### 2. Read the PDFs visually

Use the Read tool on the PDF files with `pages` parameter to view specific pages:

```
Read /tmp/docx_diff/original/file.pdf  pages="1-3"
Read /tmp/docx_diff/converted/converted.pdf  pages="1-3"
```

Claude can read PDFs as rendered images. Compare the pages side by side looking for:
- **Dotted circles (◌)** — orphaned combining marks without a base character
- **Garbled Latin text** — English/URLs incorrectly converted as Bengali
- **Missing vowel signs** — ে, ি, ো etc. detached from their consonants
- **Wrong character order** — reph, ikar in wrong position
- **Font rendering differences** — missing glyphs, wrong ligatures

### 3. Drill into XML run structure

Once you spot a visual bug, examine the actual run boundaries in both files:

```python
import zipfile, xml.etree.ElementTree as ET

def show_runs(docx_path, max_paras=10):
    with zipfile.ZipFile(docx_path) as z:
        with z.open("word/document.xml") as f:
            tree = ET.parse(f)
    ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
    for pi, p in enumerate(tree.findall(".//w:p", ns)[:max_paras]):
        for ri, r in enumerate(p.findall(".//w:r", ns)):
            font = None
            rpr = r.find(".//w:rFonts", ns)
            if rpr is not None:
                font = rpr.get(f'{{{ns["w"]}}}ascii') or rpr.get(f'{{{ns["w"]}}}cs')
            for t in r.findall(".//w:t", ns):
                if t.text:
                    print(f"  p{pi}/r{ri} font={font}: {t.text!r}")
```

### 4. Find orphaned combining marks

A combining mark (vowel sign, hasanta, etc.) at the start of a run means it got separated from its base consonant — the most common DOCX conversion bug:

```python
def find_orphans(docx_path):
    with zipfile.ZipFile(docx_path) as z:
        with z.open("word/document.xml") as f:
            tree = ET.parse(f)
    ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
    for pi, p in enumerate(tree.findall(".//w:p", ns)):
        for ri, r in enumerate(p.findall(".//w:r", ns)):
            for t in r.findall(".//w:t", ns):
                if t.text and (0x09BE <= ord(t.text[0]) <= 0x09D7
                              or t.text[0] == '\u09CD'):
                    print(f"p{pi}/r{ri}: orphaned {t.text[0]!r} "
                          f"(U+{ord(t.text[0]):04X}) text={t.text!r}")
```

### 5. Cross-run analysis

When Bijoy splits pre-base characters across runs (common in Word), check if the converter needs to merge adjacent runs:

```python
def show_cross_run_splits(original_docx):
    """Show where Bijoy pre-base chars are in separate runs from their consonants."""
    # Pre-base Bijoy chars: † = ে, ‡ = ে, ˆ = ৈ, ‰ = ৈ, © = র্
    PREBASE_BIJOY = {'†', '‡', 'ˆ', '‰', '©', 'Š'}
    # ... extract runs and flag any run that is entirely prebase chars
    # followed by a run starting with a consonant
```

## Common bug patterns

| Visual symptom | Root cause | Where to fix |
|---|---|---|
| Dotted circle before vowel sign | Orphaned combining mark in its own run | DOCX walker: merge pre-base runs with next run |
| English URLs converted to Bengali | Run has Bijoy font but contains English | Classifier or per-run language detection |
| Wrong vowel sign position | ikar_swap/reph_reorder only works within a run | DOCX walker: cross-run reordering |
| Missing ো / ৌ | ে in one run, া/ৗ in another, ekar_recombine can't see both | DOCX walker: merge split vowel runs |

## Dependencies

- LibreOffice (for PDF rendering): `/Applications/LibreOffice.app/Contents/MacOS/soffice`
- On Linux: `libreoffice --headless`
