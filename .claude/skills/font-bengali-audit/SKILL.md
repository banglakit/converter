---
name: font-bengali-audit
description: Audit a TTF/OTF font file to find Bengali glyphs stored at non-Bengali codepoints (ASCII, Latin Extended, etc.) and generate mapping entries for a legacy ANSI Bengali to Unicode conversion table. Use whenever the user provides a font file and wants to identify unmapped Bengali characters, complete a translation table, find what glyphs a legacy Bengali font contains, or cross-reference font cmap data against an existing mapping.toml. Also trigger on "font audit", "cmap", "unmapped glyphs", "missing mappings", or questions about completing a legacy ANSI Bengali conversion table from font data.
---

# Font Bengali Audit

Identify Bengali glyphs stored at borrowed (non-Bengali) Unicode codepoints in legacy ANSI Bengali fonts, then generate mapping table entries for conversion to proper Unicode Bengali.

## Why this matters

Legacy ANSI Bengali fonts (SutonnyMJ, AnandaMJ, Kalpurush, etc.) store Bengali characters at ASCII/Latin/Greek/Cyrillic codepoints instead of the Unicode Bengali block (U+0980–U+09FF). A complete conversion table needs to map every one of these borrowed codepoints back to proper Unicode Bengali. Fonts are the ground truth — if a codepoint has a Bengali glyph contour, it needs a mapping.

## Workflow

### 1. Run the audit script

The bundled script handles font parsing, empty-glyph filtering, and classification:

```bash
python3 .claude/skills/font-bengali-audit/scripts/font_audit.py \
  <font-path-or-directory> \
  --mapping <path-to-mapping.toml>
```

Arguments:
- First arg: path to a single `.ttf`/`.otf` file, or a directory of fonts
- `--mapping`: (optional) path to an existing mapping.toml to cross-reference against
- `--xml`: (optional) path to a reference XML mapping file (e.g., CharCombiC2U.xml)
- `--output-dir`: (optional) directory for output images, defaults to /tmp

The script outputs:
- A summary of codepoints with real Bengali glyphs vs empty/passthrough
- A list of unmapped codepoints (in font but not in mapping.toml)
- Rendered grid images for visual identification

### 2. Visually identify the unmapped glyphs

Read the rendered PNG (`/tmp/font_audit_unmapped.png`) to see what Bengali characters each unmapped codepoint represents. The grid shows each glyph rendered from the font with its codepoint label.

For detailed rendering techniques — rendering specific codepoint ranges, zooming into individual glyphs, comparing across fonts — read `references/rendering.md`.

### 3. Generate mapping entries

Once glyphs are identified, add entries to the appropriate section of `mapping.toml`:
- Single input char → Unicode: `[single_char]`
- Two-char sequence → Unicode: `[bigrams]`
- Three-char: `[trigrams]`
- Four-char: `[quadgrams]`

Group new entries with comments explaining their origin (e.g., "font cmap audit", "alternate vowel sign codes").

### 4. Verify

After adding entries, run tests to ensure no regressions.

## Key distinctions

- **Passthrough codepoints**: ASCII punctuation (`!`, `#`, `(`, etc.) rendered as standard punctuation. No mapping entry needed.
- **Empty glyphs**: Codepoints with cmap entries but zero contours (common in Greek/Cyrillic ranges when fonts include placeholder entries). Ignore these.
- **Alternate codes**: Multiple legacy codepoints mapping to the same Unicode character (e.g., several codepoints all rendering ু). All alternates need entries.
- **Encoding families**: A single font may contain glyphs for multiple encoding schemes (e.g., Classic and Ekattor). Identify which encoding each codepoint belongs to before adding it to a mapping table.

## Dependencies

- `fontTools` (Python): `pip install fonttools`
- `Pillow` (Python): `pip install Pillow`
