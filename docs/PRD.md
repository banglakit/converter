# PRD: Cross-Platform Bijoy-to-Unicode Bengali Converter

**Author:** Aniruddha Adhikary  
**Status:** Draft  
**Last updated:** 2025

---

## 1. TL;DR

This product is a **lossless, font-aware converter** that transforms Bijoy/SutonnyMJ-family ANSI-encoded Bengali text into standard Unicode Bengali (U+0980–U+09FF) across every major document format and platform. Every existing open-source tool either converts document-wide without font awareness — mangling English text alongside Bengali — or is Windows-only, unmaintained, or limited to plain-text strings. This converter wins by classifying text at the run level using font metadata, token-level heuristics, and a configurable confidence pipeline, then shipping as a static Rust binary, Python wheel, WASM module, and mobile bindings.

---

## 2. Problem Statement

**Bijoy encoding is not an encoding in the standard sense.** It is a font-substitution trick: the text `Avwg evsjvq Mvb MvB|` stored in a file is byte-identical to random Latin characters; what makes it Bengali is solely the presence of the SutonnyMJ (or compatible) font applied to those bytes. As documented in the [TLDP Bangla PDF HOWTO](https://tldp.org/HOWTO/archived/Bangla-PDF-HOWTO/fonts.html), Type1 versions of SutonnyMJ use `adobe-fontspecific` encoding and TrueType versions use `apple-roman`, neither of which carries a BOM or encoding marker. The Symbol-encoded TrueType cmap (platform 3/encoding 0) places Bengali glyphs in the Unicode Private Use Area (0xF000–0xF0FF), [as documented on Stack Overflow](https://stackoverflow.com/questions/56186947/why-does-getglyphindicesw-add-seemingly-non-existent-mappings-for-certain-charac), making the bytes completely opaque to any tool that does not inspect font metadata.

The core failure mode in every current tool is **document-wide conversion**: they apply the Bijoy-to-Unicode substitution table to every character in the input, including English words, URLs, and numerals. The [primary user complaint](https://thecustomizewindows.com/2025/03/unicode-and-ansi-format-in-bangla-language/) is that a sentence like `"Gas price is $5"` embedded in a Bijoy document emerges as Bengali gibberish. The [`bijoy-to-unicode-file-converter`](https://github.com/almehady/Bijoy-to-Unicode-File-Converter) attempts to mitigate this with `langdetect`, but `langdetect` is trained on Unicode text — it sees Bijoy bytes as Latin and returns English with high confidence, providing no protection at all. [BanglaText.com](https://www.banglatext.com/uni2bijoy.html) explicitly acknowledges this: "simple text editor can't handle mixed character," and recommends rich text for mixed documents, but provides no programmatic solution.

---

## 3. Goals / Non-Goals

**Goals:**

- Lossless conversion of Bijoy-encoded runs to Unicode Bengali, with zero semantic loss in the character mapping and ligature normalization passes.
- Preserve English, Latin, and numeric text entirely untouched; the English false-positive rate must be below 0.5% on standard corpora.
- Cross-platform delivery: CLI binary, Python library, WASM module, npm package, and optional mobile bindings — no Windows-only runtime dependency.
- Handle DOCX, PPTX, RTF, HTML, PDF (read), plain text, and clipboard in a single unified tool.
- Safe-by-default operation: skip ambiguous spans unless the operator explicitly opts into aggressive mode.
- Emit a reversibility audit log mapping every converted span to its original bytes, enabling full undo.

**Non-goals (v1):**

- Phonetic input conversion (Avro/romanized → Unicode Bengali). That is a separate product; see [avro.py](https://github.com/hitblast/avro.py).
- OCR of scanned documents containing Bijoy-printed text.
- Machine translation between Bengali and any other language.
- Support for other Indic scripts (Devanagari, Tamil, etc.) in v1.
- PDF write-back (produce a modified PDF with Unicode text in place). Deferred to M4; v1 outputs to a sidecar text file.

---

## 4. Target Users and Use Cases

**Journalists and newspaper editors** maintain decade-scale archives in Bijoy. Bangladeshi news portals including [Jagonews24](https://www.jagonews24.com/bangla-converter) and [DhakaPost](https://www.dhakapost.com/unicode-to-bijoy-converter) have embedded web converters precisely because their editorial staff produce DOCX files in SutonnyMJ. The web tools work for single pastes but fail on batch DOCX archives and on mixed-language articles with English brand names and statistics.

**Government and academic typists** hold corpora of DOCX and RTF files produced on Windows XP–era systems where Bijoy was the only available Bengali input method. These files contain formal legal and academic text; conversion errors are high-cost.

**NLP pipeline developers** need Unicode-normalized Bengali as input to tokenizers, language models, and corpus tools. Libraries like [bnUnicodeNormalizer](https://github.com/mnansary/bnUnicodeNormalizer) (cited in LREC-COLING 2024) operate only on already-Unicode Bengali; they cannot accept Bijoy-encoded input.

**Designers** have legacy print assets (Illustrator/InDesign files, PDFs) using embedded SutonnyMJ. They need the text content extracted and converted so it can be re-set in Unicode fonts like [Noto Sans Bengali](https://fontsource.org/fonts/noto-sans-bengali) or [Kalpurush](https://www.omicronlab.com/bangla-fonts.html).

### User Stories

**Story 1 — Batch archive migration**

*As a newspaper archive manager, I want to convert 10,000 DOCX files from SutonnyMJ to Unicode Bengali in a single CLI command, so that the archive can be indexed by a full-text search engine.*

Acceptance criteria:
- Given a directory of DOCX files, when `bijoy-convert --input-dir ./archive --output-dir ./unicode --format docx` is run, then each output DOCX has all SutonnyMJ-font runs replaced with Unicode Bengali in Kalpurush, all non-Bijoy runs untouched, and an audit JSON file emitted per document.
- Given a file where fewer than 5% of runs are classified as Bijoy, when run in safe mode, then the tool exits with a warning and converts nothing, requiring `--mode aggressive` to proceed.
- Throughput: at least 1 DOCX page per second on a modern laptop.

**Story 2 — Mixed-language document**

*As a government typist, I want to convert a report that has Bengali paragraphs in SutonnyMJ and English section headings in Times New Roman, so that neither the Bengali content nor the English headings are corrupted.*

Acceptance criteria:
- Given a DOCX with alternating runs of SutonnyMJ (Bengali) and Times New Roman (English), when conversion runs, then Bengali runs are converted to Unicode and English runs are byte-for-byte identical to the input.
- Given a run with font metadata missing (inherited from style), when the statistical classifier scores it below 0.15 on the Bijoy scale, then it is left unconverted.

**Story 3 — NLP pipeline integration**

*As an NLP engineer, I want to call the converter as a Python function on a string with an explicit font hint, so that I can normalize a corpus without shelling out.*

Acceptance criteria:
- Given `from bijoy_unicode import convert; result = convert(text, font="SutonnyMJ")`, the function returns Unicode Bengali.
- Given `convert(text, font=None, mode="safe")`, the function returns `(converted_text, confidence_score, list_of_unchanged_spans)`.
- The Python wheel imports without error on Python 3.9+ on Linux (manylinux), macOS arm64/x86_64, and Windows x64.

---

## 5. Differentiators vs Existing Tools

Existing tools fail in five distinct ways:

- **Windows-only runtime:** The [Avro Keyboard](https://github.com/omicronlab/Avro-Keyboard) converter GUI is built with Delphi 2010 and depends on NativeXml, PCRE, DISQLite3, and ICS. There is no cross-platform package and no public API.
- **No font metadata awareness:** Every web tool — [BanglaConverter.org](https://www.banglaconverter.org), [FreeBanglaConverter.com](https://freebanglaconverter.com), [bijoy.converteraz.com](https://bijoy.converteraz.com/en) — operates on raw text strings with no access to DOCX run font properties.
- **Unmaintained or broken:** [Mad-FOX/bijoy2unicode](https://github.com/Mad-FOX/bijoy2unicode) has had no meaningful commits since 2023; its Unicode→Bijoy direction was broken until v0.1.1. [BanglaString by MirazMac](https://github.com/MirazMac/BanglaString) is unmaintained since 2021.
- **Unreliable language detection:** [`bijoy-to-unicode-file-converter`](https://github.com/almehady/Bijoy-to-Unicode-File-Converter) wraps `langdetect`, which — as the detection research confirms — misidentifies Bijoy-encoded text as English or another Latin language because all five major LID libraries (`langid.py`, fastText `lid.176`, gcld3, lingua-py) expect UTF-8 Unicode input and have never seen Bijoy-encoded Bengali.
- **No document-format awareness:** No open-source tool reads DOCX `w:rFonts/@w:ascii`, RTF `\fonttbl`, or PDF `fontname` character attributes to drive per-run conversion decisions.

This product addresses all five gaps: **font-aware run extraction** is the primary classifier, statistical scoring and a multi-stage confidence pipeline handle the fontless plain-text case, and multi-format document adapters are first-class rather than an afterthought.

---

## 6. Functional Requirements

**FR-1 — Font allowlist classification.** Maintain a built-in allowlist of 128+ Bijoy-family ANSI fonts (SutonnyMJ, AdorshoLipi, Kalpurush ANSI, Siyam Rupali ANSI, JugantorMJ, and all `*MJ`-suffix variants catalogued at [hindityping.info](https://hindityping.info/download/bangla-fonts-bijoy)) and 16+ Unicode Bengali fonts (Kalpurush Unicode, Nikosh, Vrinda, Noto Sans Bengali, SolaimanLipi, etc.). Any run whose font name matches the ANSI allowlist is classified Bijoy; any run whose font name matches the Unicode list is left untouched; all others proceed to FR-3.

**FR-2 — Per-run conversion only.** The converter must never operate on a document-level string. The unit of conversion is a single formatted run (a `<w:r>` in DOCX, a run in PPTX, a font-span in RTF, an element with a resolved `font-family` in HTML, or a character sequence sharing a `fontname` in PDF). Document-wide conversion is explicitly prohibited in all modes.

**FR-3 — Token-level heuristic scoring for fontless text.** When font metadata is absent or ambiguous, score each run using:

- **High-byte density:** Count characters in the Windows-1252 high range (0x80–0xFF) that are Bijoy kars (†, ‡, ˆ, ‰, •, Š, etc.); normalize by run length. In Bengali Bijoy text, this reaches 0.15–0.35 per character; in English text it is below 0.01.
- **Bijoy bigram detection:** Match the regex `[†‡][A-Za-z]` (e-kar immediately followed by a consonant letter). A single match is suspicious; three or more is near-certain Bijoy. This pattern is diagnostic because English daggers appear only as isolated footnote marks, never mid-word.
- **English dictionary lookup:** Tokenize on whitespace; for tokens of 3+ characters, check against the top-50k English wordlist via [wordfreq](https://pypi.org/project/wordfreq/) (CC BY-SA 4.0). English paragraphs yield coverage of 0.75–0.95; Bijoy paragraphs yield below 0.05 because sequences like `evsjvq`, `Zviv`, `†k‡l` are not English words.

**FR-4 — Multi-stage classifier pipeline.** Classification proceeds through these stages in order, stopping at the first conclusive result:

1. Font allowlist lookup (FR-1) — near-certain signal when font metadata present.
2. Unicode range check: if the run already contains characters in U+0980–U+09FF, it is Unicode Bengali; skip.
3. Statistical scoring: high-byte density + Bijoy bigrams + English dictionary (FR-3).
4. LID fallback via fastText `lid.176.ftz` (917 KB quantized model) for runs ≥ 100 characters. As documented in [detection research](https://rnd.ultimate.ai/blog/language-detection-tips-tricks), fastText achieves medium accuracy on runs of this length; it cannot classify Bijoy by itself but can confirm when a run is definitively non-Bijoy Unicode Bengali.
5. Human review queue for all spans that remain uncertain after stages 1–4.

**FR-5 — Three operating modes.**

- **Safe (default):** Skip any span classified as UNCERTAIN. Emit a warning listing skipped spans and their scores. Convert only AUTO-BIJOY spans (Bijoy score > 0.25 and English coverage < 0.10).
- **Aggressive:** Convert all spans where Bijoy confidence exceeds 0.85. Suitable for bulk archives where the operator has verified the document type.
- **Interactive:** Prompt the operator at the terminal for each UNCERTAIN span, showing a side-by-side render of the span in SutonnyMJ interpretation vs. Latin interpretation. Batch mode for CI pipelines should reject this mode and exit with a non-zero status if uncertain spans exist.

**FR-6 — Ligature normalization pass.** After the byte-substitution table is applied, a normalization pass corrects Unicode logical ordering:

- **Reph reordering:** In Bijoy, the reph glyph byte appears after the consonant cluster it tops. The converter must detect `reph_glyph + consonant_cluster` and rewrite it as `Ra (U+09B0) + Hasanta (U+09CD) + consonant_cluster`. Per the [Unicode Bengali document L2/2003/03233](https://www.unicode.org/L2/L2003/03233-bengali.pdf): "Ra + Hasanta + Anything = Reph."
- **i-kar / ii-kar visual-to-logical swap:** Bijoy stores short i-kar (ি, U+09BF) and long ii-kar (ী, U+09C0) after the consonant in byte order (visual convention). Unicode requires the logical order: consonant first, then vowel sign. The normalizer must swap these.
- **Ya-phala ZWJ insertion (র্য):** The sequence Ra + Hasanta + Ya is ambiguous between reph-over-ya and ya-phala. Per [W3C IIP issue #14](https://github.com/w3c/iip/issues/14), correct encoding requires `Ra + ZWJ (U+200D) + Hasanta + Ya` to force ya-phala. The [Bangla Type Foundry](https://banglatypefoundry.com/reph-in-indesign/) documents that HarfBuzz renders this correctly while Adobe InDesign's Lipika engine (pre-October 2022) does not; the converter emits ZWJ to give compliant renderers the best chance.
- **o-kar / e-kar split-glyph recombination:** In Bijoy, the vowel sign ো (U+09CB) is split across two glyph bytes: the e-part (†, 0x86) appears before the consonant in byte order and the aa-part (া) appears after. The converter must detect this pre/post-base bracket pattern and emit `consonant + U+09CB` (or `consonant + U+09C7 + U+09BE`). The same applies to ৌ (ou-vowel).
- **Nukta letter ordering:** Map Bijoy nukta letter glyphs to the correct precomposed Unicode forms: ড় (U+09DC), ঢ় (U+09DD), য় (U+09DF). The [bnUnicodeNormalizer](https://github.com/mnansary/bnUnicodeNormalizer) documents a `BrokenNukta` failure mode — base consonant + nukta in wrong order — that conversion tools commonly produce; this product must not introduce it.
- **Chandrabindu placement:** Chandrabindu (ঁ, U+0981) must be placed after the complete nukta/vowel sequence for the base consonant, not inside a conjunct sequence.

**FR-7 — Format adapters.** Each adapter extracts runs, calls the Rust core for byte-level conversion and ligature normalization, then writes the result back into the document structure:

- **DOCX** via [python-docx](https://python-docx.readthedocs.io): read `w:ascii` and `w:hAnsi` from `<w:rFonts>` (and resolve inheritance from paragraph style, `docDefaults`, and Normal style). Replace font with the Unicode equivalent (Kalpurush by default, configurable). Font resolution must traverse the full cascade: run `rPr` → paragraph style → document defaults.
- **PPTX** via python-pptx: `run.font.name` reads `<a:latin typeface>`. Font inheritance traverses shape → layout → master → theme.
- **RTF**: Parse `\fonttbl` to build a `{font_number: font_name}` map; track active font number as `\fN` control words change. Use [rtfparse](https://pypi.org/project/rtfparse/) for production parsing rather than hand-rolled regex.
- **HTML**: Extract `font-family` from inline styles and `<style>` blocks; rewrite matching selectors and inline styles to a Unicode font. Handle Word-pasted CF_HTML clipboard fragments, which carry inline `style="font-family:'SutonnyMJ',serif"` per the [Microsoft CF_HTML spec](https://learn.microsoft.com/en-us/windows/win32/dataxchg/html-clipboard-format).
- **PDF (read)**: Extract character-level font names via [pdfplumber](https://pypi.org/project/pdfplumber/) (`char['fontname']`) or pdfminer.six (`LTChar.fontname`). Strip the 6-letter subset prefix (e.g., `ABCDEF+SutonnyMJ`) before allowlist matching. Legacy Bijoy PDFs frequently lack a `ToUnicode` CMap or have one that maps Bijoy glyph IDs to Latin codepoints; trust `fontname` over `char.get_text()` in this case and apply the conversion table to the raw byte values. PDF output in v1 is a sidecar `.txt` file; write-back is deferred to M4.
- **Plain text**: No font metadata available; apply the full FR-4 classifier pipeline. Emit a warning if the document-level Bijoy score is ambiguous.
- **Clipboard**: On Windows, read `CF_HTML` format to recover font metadata. On macOS, read `NSPasteboardTypeHTML`. Plain-text clipboard formats (`CF_TEXT`, `CF_UNICODETEXT`) strip font metadata; fall back to FR-4 statistical scoring.

**FR-8 — Reversibility audit log.** For every converted span, emit a JSON entry containing:

- Original byte sequence (base64-encoded)
- Source location: file path, paragraph/run index, character offset
- Detected font name (or `null` if inferred statistically)
- Classifier stage that made the decision (FR-4 stages 1–5)
- Confidence score
- Converted Unicode string

The audit log enables full undo: apply original bytes back to the source location to restore the Bijoy version. The log is emitted to a sidecar file (e.g., `document.docx.bijoy-audit.json`) or to stdout with `--audit-stdout`.

**FR-9 — Dry-run mode.** `--dry-run` performs all classification and conversion steps but writes no output files. Instead, it emits a unified diff to stdout showing which spans would change. Exit code 0 means no changes would be made; exit code 1 means changes would be made; exit code 2 means errors occurred.

**FR-10 — Configurable confidence threshold.** `--threshold <float>` overrides the default 0.85 aggressive-mode cutoff. Per-run overrides can be specified in a YAML config file mapping font names or span patterns to thresholds.

---

## 7. Non-Functional Requirements

**Performance.** Plain-text throughput must be ≥ 10 MB/s on a single core (a typical 1 MB Bijoy text file converts in under 100 ms). DOCX throughput must be ≥ 1 page/second including DOM parse and write-back. These targets are achievable with a Rust core and Python-layer document parsing.

**Memory.** Files larger than 100 MB must be processed as streams; the entire document must never be loaded into memory at once. DOCX paragraph iteration with python-docx already streams; PDF processing with pdfminer.six supports page-level iteration.

**Determinism.** Given identical input bytes and configuration, the output must be bit-for-bit identical across runs, platforms, and versions within a major version. The audit log must be reproducible.

**Security.** No network calls at runtime. No telemetry. No external font server lookups. The font allowlist and conversion table are embedded in the binary. The tool must be safe to run in air-gapped government environments.

**Accessibility.** The CLI must be fully scriptable: all output goes to stdout/stderr (never interactive prompts unless `--mode interactive` is set), exit codes are meaningful, and all options are expressible as flags (no interactive wizards in batch mode). The Python library must be embeddable with no global state.

---

## 8. Platform / Packaging Matrix

The **core engine** is implemented in Rust. Rust is preferred over Go for this project because:

- WASM compilation via `wasm-pack` is mature and well-tested in the Rust ecosystem.
- Python bindings via [PyO3](https://pyo3.rs) produce `manylinux`-compatible wheels with minimal friction.
- The character mapping table, ligature normalization automata, and confidence scoring are pure computation with no I/O — a natural fit for Rust's zero-cost abstractions.

Document format adapters (DOCX, PPTX, PDF extraction) remain in Python for v1 because the Python ecosystem dominates here and the I/O cost dwarfs the conversion cost.

| Artifact | Technology | Targets |
|---|---|---|
| CLI binary | Rust (static, `cargo build --release`) | macOS (arm64, x86_64), Linux (x86_64, arm64), Windows x64 |
| Python wheel | Rust core + PyO3 | manylinux2014 x86_64/arm64, macOS arm64/x86_64, Windows x64; Python 3.9–3.13 |
| WASM module | `wasm-pack` + `wasm-bindgen` | Browser (ESM), Node.js (CJS + ESM) |
| npm package | Wraps WASM module | Published to npmjs.com |
| Mobile bindings (optional) | [UniFFI](https://github.com/mozilla/uniffi-rs) | Swift (iOS/macOS), Kotlin (Android) |

Document segmenters (DOCX/PPTX/PDF adapters) are distributed as the `bijoy-unicode[docs]` Python extra. The WASM build exposes only the string-level conversion API (FR-3 / FR-6); document parsing runs in the host environment and calls into WASM for per-run conversion.

---

## 9. Success Metrics

**Conversion accuracy ≥ 99% on a held-out Bijoy corpus.**  
Measured as: character-level accuracy on a corpus of 10,000 Bijoy sentences with ground-truth Unicode pairs, drawn from existing Bangladeshi newspaper archives and cross-validated against known-good conversions from [Mad-FOX/bijoy2unicode](https://github.com/Mad-FOX/bijoy2unicode) and [avro.py](https://github.com/hitblast/avro.py). Evaluated at each milestone; regressions gate release.

**English false-positive rate < 0.5%.**  
Measured as: the fraction of English tokens in the [Brown corpus](https://www.nltk.org/book/ch02.html) (1 million words, diverse genres) and a 500,000-word Wikipedia English sample that are incorrectly converted by the tool when processed in aggressive mode with no font hints. This directly quantifies the document-mangling failure mode documented in §2. Target < 0.5% token-level error.

**1,000 weekly active CLI users within 12 months of M1 release.**  
Measured via opt-in anonymous usage counter (disabled by default; users must run `bijoy-convert --enable-stats` to participate) or via GitHub release download counts as a lower bound.

**50 npm packages listing `bijoy-unicode-wasm` as a dependency within 12 months of WASM release.**  
Measured via the [npm dependents API](https://registry.npmjs.org/-/v1/search?text=bijoy-unicode-wasm).

---

## 10. Open Questions / Risks

**PDF write-back complexity.** Reconstructing a PDF with Unicode Bengali in place of Bijoy glyphs requires either rewriting the `ToUnicode` CMap for the embedded subset font or reflowing the entire page through a new PDF renderer (reportlab, fpdf2, or WeasyPrint). Neither approach is trivial: CMap rewriting with [pymupdf](https://github.com/pymupdf/PyMuPDF/issues/530) or [fonttools](https://fonttools.readthedocs.io) is low-level and fragile; reflowing loses the original layout. This is deferred to M4 and the v1 PDF output is a sidecar text file.

**SutonnyMJ font licensing.** SutonnyMJ is a proprietary font owned by Ananda Computers / Mustafa Jabbar. The converter must never ship the font file itself. The conversion table — a mapping of byte values to Unicode codepoints — is factual/mathematical data and is independently derived by the community (as evidenced by [bahar/BijoyToUnicode](https://github.com/bahar/BijoyToUnicode), [Mad-FOX/bijoy2unicode](https://github.com/Mad-FOX/bijoy2unicode), and [avro.py](https://github.com/hitblast/avro.py) all implementing the same table under open licenses). The table in this product must be sourced from MIT-licensed references (avro.py is MIT) and documented as clean-room.

**Bijoy Bayanno legal pressure.** Mustafa Jabbar and Ananda Computers have historically pursued IP claims related to the Bijoy keyboard layout. The [Wikipedia article on Bengali input methods](https://en.wikipedia.org/wiki/Bengali_input_methods) documents that Bijoy Bayanno targeted the [Avro Keyboard](https://github.com/omicronlab/Avro-Keyboard) project with legal pressure. This project must:
- Avoid shipping any Ananda Computers-owned font files.
- Source the conversion table exclusively from MIT-licensed open projects.
- Not use the trademark "Bijoy" in package names; use "ANSI Bengali" or "Bijoy-family" in documentation only.
- Maintain legal review before each public release, particularly for the npm/PyPI distribution.

**Font name ambiguity.** The font name `SutonnyOMJ` is shared between the ANSI variant and the Unicode-output Bijoy layout's target font. The allowlist must distinguish these by checking whether the run contains any codepoints in U+0980–U+09FF (Unicode Bengali block); if so, it is the Unicode variant and should not be converted.

**Minimum viable corpus.** The 99% accuracy metric requires a labelled corpus of Bijoy↔Unicode sentence pairs. No public dataset currently exists at sufficient scale. Creating or curating this corpus is a prerequisite for M1 acceptance testing and should begin immediately, potentially by converting known-good texts through multiple existing tools and taking the intersection.

---

## 11. Milestones

**M1 — 8 weeks.** Rust core with static character substitution table, reph and i-kar/o-kar normalization, and confidence scoring. PyO3 Python bindings with `manylinux` wheel. CLI binary handling plain-text and DOCX with font-allowlist classifier (FR-1 through FR-6, FR-8, FR-9). Accuracy ≥ 99% on held-out corpus. English FPR < 0.5% on Brown corpus.

**M2 — 16 weeks.** PPTX, RTF, and HTML format adapters (FR-7). N-gram statistical classifier for fontless text integrated into the pipeline (FR-3 / FR-4 stages 3–4). Interactive review mode with side-by-side diff output (FR-5). Full ligature normalization including chandrabindu placement and ya-phala ZWJ (FR-6 complete). Configurable threshold (FR-10).

**M3 — 24 weeks.** PDF read adapter via pdfplumber/pdfminer.six (FR-7 PDF). WASM build via `wasm-pack`; npm package published. Optional UniFFI Swift/Kotlin mobile bindings. CLI binary published for all three platforms via GitHub Releases and Homebrew tap.

**M4 — 32 weeks.** PDF write-back (experimental): CMap rewriting via fonttools for PDFs with intact embedded Bijoy fonts; reflow-based output for others. VS Code extension providing right-click "Convert Bijoy to Unicode" on selected text. LibreOffice macro wrapper calling the Python wheel.

---

## 12. Appendix

### Source Material

- [research_converters.md](/home/user/workspace/research_converters.md) — landscape of existing tools, encoding details, font families, conversion bugs
- [research_detection.md](/home/user/workspace/research_detection.md) — detection heuristics, LID library analysis, document format font extraction APIs

### Key References

- [Bengali input methods — Wikipedia](https://en.wikipedia.org/wiki/Bengali_input_methods) — canonical overview of Bijoy history and encoding
- [Mad-FOX/bijoy2unicode](https://github.com/Mad-FOX/bijoy2unicode) — Python MPL-2.0 reference implementation
- [hitblast/avro.py](https://github.com/hitblast/avro.py) — MIT-licensed Python library; canonical open-source conversion table
- [bahar/BijoyToUnicode](https://github.com/bahar/BijoyToUnicode) — PHP AGPL-3.0 implementation with full mapping table
- [mnansary/bnUnicodeNormalizer](https://github.com/mnansary/bnUnicodeNormalizer) — post-conversion Unicode normalization; covers BrokenNukta, invalid hosonto, chandrabindu mis-placement
- [omicronlab/Avro-Keyboard](https://github.com/omicronlab/Avro-Keyboard) — MPL-1.1 Delphi source; reference for Windows-only converter GUI
- [sarim/ibus-avro](https://github.com/sarim/ibus-avro) — Linux ibus input method; Unicode output only
- [Unicode L2/2003/03233 — Bengali](https://www.unicode.org/L2/L2003/03233-bengali.pdf) — canonical reph and ya-phala encoding rules
- [W3C IIP issue #14 — র্য ZWJ/ZWNJ](https://github.com/w3c/iip/issues/14) — Ra + Hasanta + Ya encoding ambiguity
- [Bangla Type Foundry — Reph in InDesign](https://banglatypefoundry.com/reph-in-indesign/) — documents HarfBuzz vs. Lipika rendering differences
- [TLDP Bangla PDF HOWTO](https://tldp.org/HOWTO/archived/Bangla-PDF-HOWTO/fonts.html) — adobe-fontspecific and apple-roman cmap encoding in SutonnyMJ
- [python-docx font analysis](https://python-docx.readthedocs.io/en/latest/dev/analysis/features/text/font.html) — `w:rFonts` attribute structure
- [pdfplumber PyPI](https://pypi.org/project/pdfplumber/) — character-level `fontname` extraction from PDF
- [wordfreq PyPI](https://pypi.org/project/wordfreq/) — CC BY-SA 4.0 English word frequency list for dictionary lookup
- [HindiTyping.info — Bijoy font list](https://hindityping.info/download/bangla-fonts-bijoy) — exhaustive enumeration of 128+ Bijoy-family fonts
- [OmicronLab Bangla Fonts](https://www.omicronlab.com/bangla-fonts.html) — Kalpurush, Siyam Rupali, and other Unicode/ANSI variants
- [pymupdf/fonttools — ToUnicode CMap recovery](https://github.com/pymupdf/PyMuPDF/issues/530) — approach for Bijoy PDFs without correct ToUnicode CMap
- [langid.py ACL 2012 paper](https://aclanthology.org/P12-3005.pdf) — byte n-gram LID; explains why Bijoy is misidentified as Latin
- [CEUR FIRE shared task — code-switched LID](https://ceur-ws.org/Vol-1587/T2-5.pdf) — token-level Bengali/English classification accuracy benchmarks
