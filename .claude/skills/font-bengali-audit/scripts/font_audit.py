#!/usr/bin/env python3
"""Audit legacy ANSI Bengali font files to find Bengali glyphs at non-Bengali codepoints.

Usage:
    python font_audit.py <font-or-directory> [--mapping mapping.toml]

Outputs:
    - Console report of mapped/unmapped/passthrough/empty codepoints
    - /tmp/font_audit_unmapped.png — rendered grid of unmapped Bengali glyphs
    - /tmp/font_audit_all_real.png — rendered grid of ALL codepoints with real glyphs
"""

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

from fontTools.ttLib import TTFont

# ASCII printable chars that are typically passthrough (punctuation rendered as-is)
PASSTHROUGH_CHARS = set(
    '!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~ \t\n\r'
)
PASSTHROUGH = {ord(ch) for ch in PASSTHROUGH_CHARS}
PASSTHROUGH.update(range(0x0000, 0x0020))  # control chars

# Codepoints that are clearly not legacy Bengali content
NON_BENGALI = {0xFEFB, 0xFEFC, 0xFFFB, 0xFFFC, 0x20AC, 0xFFFD}

# Bengali Unicode range (these are output chars, not legacy input)
BENGALI_RANGE = range(0x0980, 0x09FF + 1)


def parse_mapping_toml(path: Path) -> set[str]:
    """Extract all legacy-side keys from mapping.toml."""
    keys = set()
    in_section = False
    with open(path, encoding="utf-8") as f:
        for line in f:
            stripped = line.strip()
            if stripped.startswith("[") and stripped.endswith("]"):
                section = stripped[1:-1]
                in_section = section in (
                    "single_char", "bigrams", "trigrams", "quadgrams"
                )
                continue
            if not in_section or "=" not in stripped or stripped.startswith("#"):
                continue
            key = stripped.split("=", 1)[0].strip().strip('"')
            key = re.sub(
                r"\\u([0-9A-Fa-f]{4})",
                lambda m: chr(int(m.group(1), 16)),
                key,
            )
            keys.add(key)
    return keys


def get_mapped_codepoints(keys: set[str]) -> set[int]:
    """Get all codepoints appearing in any mapping key."""
    cps = set()
    for key in keys:
        for ch in key:
            cps.add(ord(ch))
    return cps


def scan_font(font_path: Path) -> tuple[dict[int, str], set[int]]:
    """Return (cmap, set of codepoints with actual contours)."""
    tt = TTFont(font_path)
    cmap = tt.getBestCmap() or {}

    has_contours = set()
    if "glyf" in tt:
        glyf = tt["glyf"]
        for cp, glyph_name in cmap.items():
            try:
                g = glyf[glyph_name]
                if g.numberOfContours != 0:  # 0 = empty, -1 = composite
                    has_contours.add(cp)
            except Exception:
                pass
    elif "CFF " in tt or "CFF2" in tt:
        # CFF fonts: assume all cmap entries have contours (can't easily check)
        has_contours = set(cmap.keys())

    tt.close()
    return cmap, has_contours


def scan_fonts(font_dir: Path) -> tuple[dict[int, list[str]], set[int]]:
    """Scan all fonts in a directory. Return (cp->font_names, all_with_contours)."""
    cp_fonts: dict[int, list[str]] = defaultdict(list)
    all_contours: set[int] = set()
    fonts = sorted(font_dir.glob("*.ttf")) + sorted(font_dir.glob("*.TTF"))
    fonts += sorted(font_dir.glob("*.otf")) + sorted(font_dir.glob("*.OTF"))
    for fp in fonts:
        try:
            _, contours = scan_font(fp)
            for cp in contours:
                cp_fonts[cp].append(fp.name)
            all_contours |= contours
        except Exception as e:
            print(f"  WARN: {fp.name}: {e}", file=sys.stderr)
    return cp_fonts, all_contours


def classify(cp: int) -> str:
    if cp in PASSTHROUGH:
        return "passthrough"
    if cp in NON_BENGALI:
        return "non-bengali"
    if cp in BENGALI_RANGE:
        return "bengali-unicode"
    if 0x0021 <= cp <= 0x007E:
        return "ascii-printable"
    if 0x0080 <= cp <= 0x00FF:
        return "latin1"
    if 0x0100 <= cp <= 0x024F:
        return "latin-extended"
    if 0x0250 <= cp <= 0x02FF:
        return "ipa-modifiers"
    if 0x0370 <= cp <= 0x03FF:
        return "greek"
    if 0x0400 <= cp <= 0x04FF:
        return "cyrillic"
    if 0x1E00 <= cp <= 0x1EFF:
        return "vietnamese"
    if 0x2000 <= cp <= 0x206F:
        return "general-punctuation"
    return "other"


def u_repr(cp: int) -> str:
    return f"U+{cp:04X} {chr(cp)!r}"


def render_grid(codepoints: list[int], font_path: Path, output: Path, title: str):
    """Render a grid of glyphs from the font."""
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        print(f"  WARN: Pillow not installed, skipping grid render", file=sys.stderr)
        return

    if not codepoints:
        print(f"  No codepoints to render for {title}")
        return

    pil_font = ImageFont.truetype(str(font_path), 64)
    try:
        label_font = ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", 12)
    except OSError:
        label_font = ImageFont.load_default()

    cols = 10
    rows = (len(codepoints) + cols - 1) // cols
    cell_w, cell_h = 100, 95
    img = Image.new("RGB", (cols * cell_w, rows * cell_h + 30), "white")
    draw = ImageDraw.Draw(img)
    draw.text((10, 5), f"{title} ({len(codepoints)} glyphs)", fill="black", font=label_font)

    for idx, cp in enumerate(codepoints):
        col = idx % cols
        row = idx // cols
        x = col * cell_w
        y = row * cell_h + 30
        try:
            draw.text((x + 10, y + 2), chr(cp), font=pil_font, fill="black")
        except Exception:
            pass
        draw.text((x + 10, y + 72), f"U+{cp:04X}", fill="#666", font=label_font)
        draw.rectangle([x, y, x + cell_w - 1, y + cell_h - 1], outline="#ccc")

    img.save(str(output))
    print(f"  Saved: {output}")


def main():
    parser = argparse.ArgumentParser(
        description="Audit legacy ANSI Bengali font cmap for unmapped glyphs"
    )
    parser.add_argument("font", help="Path to a .ttf/.otf file or directory of fonts")
    parser.add_argument("--mapping", help="Path to mapping.toml to cross-reference")
    parser.add_argument("--output-dir", default="/tmp", help="Directory for output images")
    args = parser.parse_args()

    font_path = Path(args.font)
    output_dir = Path(args.output_dir)

    # --- Scan fonts ---
    if font_path.is_dir():
        print(f"Scanning directory: {font_path}")
        cp_fonts, all_contours = scan_fonts(font_path)
        render_font = next(
            (f for f in sorted(font_path.glob("*.ttf")) + sorted(font_path.glob("*.TTF"))),
            None,
        )
        font_count = len(set().union(*cp_fonts.values())) if cp_fonts else 0
        print(f"  {len(all_contours)} codepoints with contours across {font_count} fonts")
    else:
        print(f"Scanning font: {font_path}")
        cmap, all_contours = scan_font(font_path)
        cp_fonts = {cp: [font_path.name] for cp in all_contours}
        render_font = font_path
        print(f"  {len(all_contours)} codepoints with contours")

    # --- Load mapping ---
    mapped_cps: set[int] = set()
    if args.mapping:
        mapping_path = Path(args.mapping)
        keys = parse_mapping_toml(mapping_path)
        mapped_cps = get_mapped_codepoints(keys)
        print(f"\nMapping: {mapping_path}")
        print(f"  {len(keys)} keys using {len(mapped_cps)} unique codepoints")

    # --- Classify ---
    skip = PASSTHROUGH | NON_BENGALI
    unmapped = sorted(cp for cp in all_contours if cp not in mapped_cps and cp not in skip)

    by_cat: dict[str, list[int]] = defaultdict(list)
    for cp in unmapped:
        by_cat[classify(cp)].append(cp)

    # --- Report ---
    print(f"\n{'='*70}")
    print("UNMAPPED CODEPOINTS WITH REAL GLYPHS")
    print(f"{'='*70}")

    interesting_cats = ["ascii-printable", "latin1", "ipa-modifiers", "general-punctuation", "other"]
    info_cats = ["latin-extended", "greek", "cyrillic", "vietnamese", "bengali-unicode"]

    total_interesting = 0
    for cat in interesting_cats:
        cps = by_cat.get(cat, [])
        if not cps:
            continue
        total_interesting += len(cps)
        print(f"\n--- {cat} ({len(cps)}) ---")
        for cp in sorted(cps, key=lambda c: -len(cp_fonts.get(c, []))):
            fc = len(cp_fonts.get(cp, []))
            print(f"  {u_repr(cp):30s}  {fc:3d} fonts")

    print(f"\n--- Other ranges (may belong to a different encoding family) ---")
    for cat in info_cats:
        cps = by_cat.get(cat, [])
        if cps:
            print(f"  {cat}: {len(cps)} codepoints")

    # --- Mapped but not in font ---
    if mapped_cps:
        missing = sorted(mapped_cps - all_contours - skip)
        if missing:
            print(f"\n{'='*70}")
            print(f"MAPPED CODEPOINTS NOT IN ANY FONT ({len(missing)})")
            print(f"{'='*70}")
            for cp in missing:
                print(f"  {u_repr(cp)}")

    # --- Summary ---
    real_cps = all_contours - skip
    print(f"\n{'='*70}")
    print("SUMMARY")
    print(f"{'='*70}")
    print(f"  Codepoints with real glyphs (excl. passthrough): {len(real_cps)}")
    print(f"  Mapped in mapping.toml:                          {len(mapped_cps & real_cps)}")
    print(f"  Unmapped (interesting):                          {total_interesting}")
    print(f"  Unmapped (other ranges):                         {len(unmapped) - total_interesting}")
    if real_cps:
        cov = len(mapped_cps & real_cps) / len(real_cps) * 100
        print(f"  Coverage:                                        {cov:.1f}%")

    # --- Render grids ---
    if render_font:
        all_real = sorted(cp for cp in all_contours if cp >= 0x20)
        render_grid(
            all_real, render_font,
            output_dir / "font_audit_all_real.png",
            "All codepoints with real glyphs",
        )

        interesting_unmapped = []
        for cat in interesting_cats:
            interesting_unmapped.extend(by_cat.get(cat, []))
        interesting_unmapped.sort()
        render_grid(
            interesting_unmapped, render_font,
            output_dir / "font_audit_unmapped.png",
            "Unmapped codepoints (interesting ranges)",
        )


if __name__ == "__main__":
    main()
