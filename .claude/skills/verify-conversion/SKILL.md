---
name: verify-conversion
description: Convert a DOCX file, render original and converted as PDF, run automated quality checks, and present results for visual inspection. Use when testing conversion quality, after fixing normalize passes, or when the user says "verify", "check conversion", "test the docx", or "how does it look".
---

# Verify Conversion

End-to-end conversion verification: convert a DOCX, render to PDF, run regression checks, present results.

## Usage

`/verify-conversion [path]` — defaults to `examples/test.docx` if no path given.

## Steps

### 1. Convert

```bash
~/.cargo/bin/cargo build --release -p banglakit-cli
target/release/banglakit-converter -i <input> -o /tmp/verify_output.docx
```

Exit code 1 = changes made (normal). Exit code 2 = error.

### 2. Render both to PDF

```bash
/Applications/LibreOffice.app/Contents/MacOS/soffice --headless \
  --convert-to pdf --outdir /tmp/verify_original <input>
/Applications/LibreOffice.app/Contents/MacOS/soffice --headless \
  --convert-to pdf --outdir /tmp /tmp/verify_output.docx
```

### 3. Visual comparison

Read both PDFs with the Read tool (use `pages` parameter for multi-page docs). Compare looking for:
- Dotted circles (◌) = orphaned combining marks
- Garbled Latin = English text incorrectly converted
- Missing vowel signs = cross-run split not merged
- Wrong character order = normalize pass bug

### 4. Automated regression checks

Run this Python script to count known bug patterns:

```python
import zipfile, xml.etree.ElementTree as ET

def verify(docx_path):
    with zipfile.ZipFile(docx_path) as z:
        with z.open("word/document.xml") as f:
            tree = ET.parse(f)
    ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}

    text = ""
    orphans = 0
    for p in tree.findall(".//w:p", ns):
        for r in p.findall(".//w:r", ns):
            for t in r.findall(".//w:t", ns):
                if t.text:
                    text += t.text
                    if 0x09BE <= ord(t.text[0]) <= 0x09D7 or t.text[0] == '\u09CD':
                        orphans += 1

    vowel_signs = set('\u09be\u09bf\u09c0\u09c1\u09c2\u09c3\u09c7\u09c8\u09cb\u09cc')
    anusvara_before_vowel = sum(
        1 for i, ch in enumerate(text)
        if ch in '\u0982\u0983\u0981' and i+1 < len(text) and text[i+1] in vowel_signs
    )
    vowel_before_raphala = sum(
        1 for i in range(len(text)-2)
        if text[i] in vowel_signs and text[i+1] == '\u09cd' and text[i+2] == '\u09b0'
    )

    print(f"Orphaned combining marks at run starts: {orphans}")
    print(f"Anusvara/visarga/chandrabindu before vowel: {anusvara_before_vowel}")
    print(f"Vowel sign before ra-phala: {vowel_before_raphala}")
    return orphans, anusvara_before_vowel, vowel_before_raphala

verify("/tmp/verify_output.docx")
```

### 5. Report

Present a summary table comparing current numbers against known baselines:

| Metric | Baseline (pre-fix) | Current | Target |
|--------|-------------------|---------|--------|
| Orphaned combining marks | 2,902 | ? | 0 |
| Anusvara before vowel | 20+ | ? | 0 |
| Vowel before ra-phala | 19 | ? | 0 |

Then show the first 1-2 pages of the converted PDF for visual review.
