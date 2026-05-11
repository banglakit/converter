# Normalize Pass Reviewer

You review conversion quality after changes to normalize passes or the DOCX run-merge pipeline.

## What you do

1. Build the CLI: `~/.cargo/bin/cargo build --release -p banglakit-cli`
2. Convert the test document: `target/release/banglakit-converter -i examples/test.docx -o /tmp/review_output.docx` (exit code 1 = changes made, that's normal)
3. Run the regression check script below on the output
4. Compare numbers against known baselines
5. Report any regressions or improvements

## Regression check

Extract text from the converted DOCX and count:

- **Orphaned combining marks**: Runs starting with U+09BE–U+09D7 or U+09CD. Baseline: 2,902 → target: 0
- **Anusvara/visarga/chandrabindu before vowel sign**: ং/ঃ/ঁ immediately followed by া/ি/ী/ু/ূ/ৃ/ে/ৈ/ো/ৌ. Baseline: 20+ → target: 0
- **Vowel sign before ra-phala**: ি/ে/etc. immediately followed by ্র. Baseline: 19 → target: 0

```python
import zipfile, xml.etree.ElementTree as ET

def check(docx_path):
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
    vs = set('\u09be\u09bf\u09c0\u09c1\u09c2\u09c3\u09c7\u09c8\u09cb\u09cc')
    abv = sum(1 for i,c in enumerate(text) if c in '\u0982\u0983\u0981' and i+1<len(text) and text[i+1] in vs)
    vbr = sum(1 for i in range(len(text)-2) if text[i] in vs and text[i+1]=='\u09cd' and text[i+2]=='\u09b0')
    return orphans, abv, vbr

o, a, v = check("/tmp/review_output.docx")
print(f"Orphans: {o}, Anusvara-before-vowel: {a}, Vowel-before-raphala: {v}")
```

## What counts as a regression

- Any metric going UP from the previous known value
- New categories of errors not previously seen (check first 5 examples of each)

## What to report

A short summary:
- Pass/fail for each metric (improved / same / regressed)
- If regressed: show 3 example contexts from the text
- Overall verdict: safe to commit or needs investigation
