# Rendering Font Glyphs for Visual Inspection

## Prerequisites

```python
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont
```

## Checking which codepoints have real glyphs

Not all cmap entries have visible glyphs — many are empty placeholders (zero contours).
Always filter before rendering.

```python
tt = TTFont("font.ttf")
cmap = tt.getBestCmap()
glyf = tt["glyf"]

real_cps = []
for cp, glyph_name in cmap.items():
    g = glyf[glyph_name]
    # numberOfContours: 0 = empty, -1 = composite (real), >0 = simple (real)
    if g.numberOfContours != 0:
        real_cps.append(cp)
tt.close()
```

## Rendering a grid of glyphs

Use this to see all glyphs at a glance with their codepoint labels.

```python
def render_grid(codepoints, font_path, output_path, title=""):
    pil_font = ImageFont.truetype(font_path, 64)
    label_font = ImageFont.load_default()

    cols = 10
    rows = (len(codepoints) + cols - 1) // cols
    cell_w, cell_h = 100, 95

    img = Image.new("RGB", (cols * cell_w, rows * cell_h + 30), "white")
    draw = ImageDraw.Draw(img)
    draw.text((10, 5), title, fill="black", font=label_font)

    for idx, cp in enumerate(codepoints):
        col, row = idx % cols, idx // cols
        x, y = col * cell_w, row * cell_h + 30

        draw.text((x + 10, y + 2), chr(cp), font=pil_font, fill="black")
        draw.text((x + 10, y + 72), f"U+{cp:04X}", fill="#666", font=label_font)
        draw.rectangle([x, y, x + cell_w - 1, y + cell_h - 1], outline="#ccc")

    img.save(output_path)
```

## Rendering a single glyph large

When a glyph in the grid is ambiguous, render it at high resolution for identification.

```python
def render_single(cp, font_path, output_path):
    font = ImageFont.truetype(font_path, 120)
    img = Image.new("RGB", (200, 180), "white")
    draw = ImageDraw.Draw(img)
    draw.text((30, 10), chr(cp), font=font, fill="black")
    draw.text((30, 145), f"U+{cp:04X}", fill="#666")
    img.save(output_path)
```

## Rendering by codepoint range

To inspect a specific encoding range (e.g., Latin Extended U+0100–U+024F):

```python
tt = TTFont("font.ttf")
cmap = tt.getBestCmap()
glyf = tt["glyf"]

# Filter to range + non-empty
range_cps = [
    cp for cp in range(0x0100, 0x0250)
    if cp in cmap and glyf[cmap[cp]].numberOfContours != 0
]
tt.close()

render_grid(range_cps, "font.ttf", "/tmp/latin_ext.png", "Latin Extended")
```

Common ranges to check in legacy ANSI Bengali fonts:

| Range | Codepoints | What you'll find |
|---|---|---|
| ASCII printable | U+0021–U+007E | Base consonants, vowels, digits, punctuation |
| Latin-1 Supplement | U+0080–U+00FF | Conjuncts, vowel signs, modifiers |
| Latin Extended | U+0100–U+024F | More conjuncts (may be a different encoding family) |
| IPA / Modifiers | U+0250–U+02FF | Occasional conjunct variants |
| General Punctuation | U+2000–U+206F | Smart quotes, dashes, special marks |
| Greek | U+0370–U+03FF | Usually empty glyphs (check contours!) |
| Cyrillic | U+0400–U+04FF | Usually empty glyphs (check contours!) |

## Comparing glyphs across fonts

To check if different fonts in the same family use the same codepoints:

```python
from collections import defaultdict

def survey_fonts(font_dir):
    """Return {codepoint: [font_names]} for all non-empty glyphs."""
    cp_fonts = defaultdict(list)
    for fp in sorted(Path(font_dir).glob("*.ttf")):
        tt = TTFont(fp)
        cmap = tt.getBestCmap() or {}
        glyf = tt["glyf"]
        for cp, name in cmap.items():
            if glyf[name].numberOfContours != 0:
                cp_fonts[cp].append(fp.name)
        tt.close()
    return cp_fonts
```

Codepoints present in most fonts are core to the encoding.
Codepoints in only a few fonts may be font-specific extras.

## Reading rendered images

After saving a PNG, use the Read tool to view it — Claude can read images directly.
This is how you visually identify what Bengali character each codepoint represents.

```
Read /tmp/font_audit_unmapped.png
```

If a glyph is unclear from the grid, render it larger with `render_single()` and read again.
