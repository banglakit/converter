#!/usr/bin/env python3
"""Audit Bijoy font cmaps against mapping.toml to find unmapped codepoints.

For each Bijoy font, extracts the cmap (codepoint→glyph mapping) and checks
whether every codepoint that has a Bengali-looking glyph is covered by our
mapping table. Reports unmapped codepoints grouped by frequency across fonts.
"""

import os
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

from fontTools.ttLib import TTFont

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
TOML_PATH = PROJECT_ROOT / "crates/banglakit-core/data/bijoy/mapping.toml"
FONT_DIR = Path("/tmp/bijoy-ekattor-files/Fonts Folder")

# Codepoints we know are passthrough (ASCII punctuation, digits, whitespace)
# and don't need Bijoy→Unicode mapping.
PASSTHROUGH = set()
# Standard ASCII control + whitespace
PASSTHROUGH.update(range(0x0000, 0x0020))  # control chars
PASSTHROUGH.add(0x0020)  # space
PASSTHROUGH.add(0x000D)  # CR
# Standard ASCII punctuation that maps to itself
for ch in '!#%()*+,-./:;<=>?[]{} \t\n\r':
    PASSTHROUGH.add(ord(ch))
# Codepoints that are clearly not Bijoy Bengali content
NON_BIJOY = {
    0xFEFB, 0xFEFC,  # Arabic ligatures
    0xFFFB, 0xFFFC,  # specials
    0x20AC,           # Euro sign
    0xFFFD,           # replacement char
}


def parse_mapping_toml(path: Path) -> set[str]:
    """Extract all Bijoy-side keys from mapping.toml (all sections)."""
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
            if not in_section:
                continue
            if "=" not in stripped or stripped.startswith("#"):
                continue
            key_part = stripped.split("=", 1)[0].strip().strip('"')
            # Handle TOML \uXXXX escapes
            key_part = re.sub(
                r"\\u([0-9A-Fa-f]{4})",
                lambda m: chr(int(m.group(1), 16)),
                key_part,
            )
            keys.add(key_part)
    return keys


def get_mapped_codepoints(keys: set[str]) -> set[int]:
    """Get the set of all codepoints that appear in any mapping key."""
    cps = set()
    for key in keys:
        for ch in key:
            cps.add(ord(ch))
    return cps


def scan_fonts(font_dir: Path) -> dict[int, list[str]]:
    """Scan all TTF fonts, return {codepoint: [font_names_that_have_it]}."""
    cp_fonts: dict[int, list[str]] = defaultdict(list)
    fonts = sorted(font_dir.glob("*.ttf")) + sorted(font_dir.glob("*.TTF"))
    for font_path in fonts:
        try:
            tt = TTFont(font_path, fontNumber=0)
            cmap = tt.getBestCmap() or {}
            for cp in cmap:
                cp_fonts[cp].append(font_path.name)
            tt.close()
        except Exception as e:
            print(f"  WARN: could not read {font_path.name}: {e}", file=sys.stderr)
    return cp_fonts


def classify_codepoint(cp: int) -> str:
    """Classify a codepoint into a category."""
    if cp in PASSTHROUGH:
        return "passthrough"
    if cp in NON_BIJOY:
        return "non-bijoy"
    if 0x0021 <= cp <= 0x007E:
        return "ascii-printable"
    if 0x00A0 <= cp <= 0x00FF:
        return "latin1-extended"
    if 0x0080 <= cp <= 0x009F:
        return "c1-control"
    if 0x0100 <= cp <= 0x024F:
        return "latin-extended"
    if 0x0900 <= cp <= 0x097F:
        return "devanagari"
    if 0x0980 <= cp <= 0x09FF:
        return "bengali-unicode"
    if 0x2000 <= cp <= 0x206F:
        return "general-punctuation"
    if 0x2070 <= cp <= 0x209F:
        return "superscripts"
    if 0x20A0 <= cp <= 0x20CF:
        return "currency"
    if 0x2100 <= cp <= 0x214F:
        return "letterlike"
    if 0x0250 <= cp <= 0x02FF:
        return "ipa-spacing-modifiers"
    if 0xF000 <= cp <= 0xF8FF:
        return "private-use"
    if 0xFB00 <= cp <= 0xFFFE:
        return "compat-specials"
    return "other"


def main():
    if not FONT_DIR.exists():
        print(f"ERROR: {FONT_DIR} not found. Extract BijoyEkattor.msi first.")
        sys.exit(1)

    print("Parsing mapping.toml...")
    keys = parse_mapping_toml(TOML_PATH)
    mapped_cps = get_mapped_codepoints(keys)
    print(f"  {len(keys)} mapping keys using {len(mapped_cps)} unique codepoints")

    print(f"\nScanning {FONT_DIR}...")
    cp_fonts = scan_fonts(FONT_DIR)
    all_font_cps = set(cp_fonts.keys())
    print(f"  {len(cp_fonts)} unique codepoints across {len(set().union(*cp_fonts.values()))} fonts")

    # Exclude passthrough and non-Bijoy
    skip = PASSTHROUGH | NON_BIJOY
    # Codepoints in fonts but NOT in any mapping key
    unmapped = all_font_cps - mapped_cps - skip

    # Group unmapped by category
    by_category: dict[str, list[tuple[int, int]]] = defaultdict(list)
    for cp in sorted(unmapped):
        cat = classify_codepoint(cp)
        font_count = len(cp_fonts[cp])
        by_category[cat].append((cp, font_count))

    # ===== Report =====
    print("\n" + "=" * 70)
    print("UNMAPPED CODEPOINTS FOUND IN BIJOY FONTS")
    print("=" * 70)

    # Interesting categories (likely Bijoy Bengali content)
    interesting = [
        "ascii-printable", "latin1-extended", "ipa-spacing-modifiers",
        "general-punctuation", "other",
    ]
    # Less interesting (probably not Bijoy content)
    boring = [
        "passthrough", "non-bijoy", "bengali-unicode", "devanagari",
        "latin-extended", "c1-control", "private-use", "compat-specials",
        "currency", "superscripts", "letterlike",
    ]

    total_interesting = 0
    for cat in interesting:
        if cat not in by_category:
            continue
        entries = by_category[cat]
        total_interesting += len(entries)
        print(f"\n--- {cat} ({len(entries)} codepoints) ---")
        # Sort by font count descending (most common = most important)
        for cp, fc in sorted(entries, key=lambda x: -x[1]):
            ch = chr(cp)
            safe = repr(ch)
            # Show sample fonts
            sample_fonts = cp_fonts[cp][:3]
            more = f" +{len(cp_fonts[cp])-3} more" if len(cp_fonts[cp]) > 3 else ""
            print(f"  U+{cp:04X} {safe:8s}  in {fc:3d} fonts  e.g. {', '.join(sample_fonts)}{more}")

    print(f"\n--- Less interesting categories (summary) ---")
    for cat in boring:
        if cat in by_category:
            print(f"  {cat}: {len(by_category[cat])} codepoints")

    # ===== Mapped codepoints NOT in any font (dead mappings?) =====
    mapped_not_in_fonts = mapped_cps - all_font_cps - skip
    if mapped_not_in_fonts:
        print(f"\n{'='*70}")
        print(f"MAPPED CODEPOINTS NOT IN ANY FONT ({len(mapped_not_in_fonts)})")
        print(f"{'='*70}")
        for cp in sorted(mapped_not_in_fonts):
            # Find which keys use this codepoint
            using_keys = [k for k in keys if chr(cp) in k]
            print(f"  U+{cp:04X} {chr(cp)!r:8s}  used in keys: {using_keys[:5]}")

    # ===== Summary =====
    print(f"\n{'='*70}")
    print("SUMMARY")
    print(f"{'='*70}")
    print(f"  Font codepoints (excl. passthrough): {len(all_font_cps - skip)}")
    print(f"  Mapped codepoints:                   {len(mapped_cps)}")
    print(f"  Unmapped (interesting):               {total_interesting}")
    print(f"  Unmapped (boring/non-Bijoy):          {len(unmapped) - total_interesting}")
    coverage = len(mapped_cps & (all_font_cps - skip)) / len(all_font_cps - skip) * 100
    print(f"  Codepoint coverage:                   {coverage:.1f}%")


if __name__ == "__main__":
    main()
