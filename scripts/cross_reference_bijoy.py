#!/usr/bin/env python3
"""Cross-reference BijoyEkattor's CharCombiC2U.xml against our mapping.toml.

Parses both mapping files and reports:
  1. Mappings in CharCombiC2U.xml missing from mapping.toml
  2. Mappings in mapping.toml missing from CharCombiC2U.xml
  3. Conflicts (same Bijoy input, different Unicode output)
  4. Summary statistics
"""

import xml.etree.ElementTree as ET
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent

XML_PATH = Path("/tmp/bijoy-ekattor-files/Windows Folder/BijoyUt1/CharCombiC2U.xml")
TOML_PATH = PROJECT_ROOT / "crates/banglakit-core/data/bijoy/mapping.toml"


def parse_c2u_xml(path: Path) -> dict[str, str]:
    """Parse CharCombiC2U.xml. <price> = Bijoy input, <title> = Unicode output."""
    tree = ET.parse(path)
    root = tree.getroot()
    mapping = {}
    for book in root.findall("book"):
        title = book.findtext("title") or ""
        price = book.findtext("price") or ""
        bijoy_in = price.strip()
        unicode_out = title.strip()
        if bijoy_in and unicode_out:
            # XML has duplicates (multiple Bijoy codes -> same Unicode).
            # Keep all unique bijoy_in keys; last wins for duplicates.
            mapping[bijoy_in] = unicode_out
    return mapping


def parse_mapping_toml(path: Path) -> dict[str, str]:
    """Parse mapping.toml manually (no toml dependency needed).

    Extracts key = value pairs from [single_char], [bigrams], [trigrams],
    [quadgrams] sections. Keys/values are TOML-quoted strings.
    """
    mapping = {}
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
            # Parse "key" = "value"
            parts = stripped.split("=", 1)
            key = parts[0].strip().strip('"')
            val = parts[1].strip().strip('"')
            # Unescape TOML basic strings
            key = key.replace("\\\\", "\x00").replace('\\"', '"').replace("\\\\", "\\").replace("\x00", "\\")
            val = val.replace("\\\\", "\x00").replace('\\"', '"').replace("\\\\", "\\").replace("\x00", "\\")
            mapping[key] = val
    return mapping


def u_repr(s: str) -> str:
    """Show string with Unicode codepoints for clarity."""
    cps = " ".join(f"U+{ord(c):04X}" for c in s)
    return f"'{s}' [{cps}]"


def main():
    if not XML_PATH.exists():
        print(f"ERROR: {XML_PATH} not found. Extract BijoyEkattor.msi first.")
        sys.exit(1)
    if not TOML_PATH.exists():
        print(f"ERROR: {TOML_PATH} not found.")
        sys.exit(1)

    xml_map = parse_c2u_xml(XML_PATH)
    toml_map = parse_mapping_toml(TOML_PATH)

    xml_keys = set(xml_map.keys())
    toml_keys = set(toml_map.keys())

    # --- Passthrough entries (Bijoy in == Unicode out, e.g. punctuation) ---
    xml_passthrough = {k for k, v in xml_map.items() if k == v}
    xml_functional = {k: v for k, v in xml_map.items() if k != v}
    xml_func_keys = set(xml_functional.keys())

    # --- Conflicts ---
    common = xml_func_keys & toml_keys
    conflicts = []
    matches = []
    for k in sorted(common):
        xv = xml_functional[k]
        tv = toml_map[k]
        if xv != tv:
            conflicts.append((k, xv, tv))
        else:
            matches.append(k)

    # --- Only in XML (not passthrough) ---
    only_xml = sorted(xml_func_keys - toml_keys)

    # --- Only in TOML ---
    only_toml = sorted(toml_keys - xml_keys)

    # ===== Report =====
    print("=" * 70)
    print("Cross-reference: CharCombiC2U.xml vs mapping.toml")
    print("=" * 70)

    print(f"\nCharCombiC2U.xml : {len(xml_map)} total entries "
          f"({len(xml_passthrough)} passthrough, {len(xml_functional)} functional)")
    print(f"mapping.toml     : {len(toml_map)} entries")
    print(f"Exact matches    : {len(matches)}")

    # --- Conflicts ---
    print(f"\n{'='*70}")
    print(f"CONFLICTS ({len(conflicts)}) — same Bijoy input, different Unicode output")
    print(f"{'='*70}")
    if conflicts:
        for bijoy, xml_uni, toml_uni in conflicts:
            print(f"\n  Bijoy input : {u_repr(bijoy)}")
            print(f"  XML output  : {u_repr(xml_uni)}")
            print(f"  TOML output : {u_repr(toml_uni)}")
    else:
        print("  (none)")

    # --- Only in XML ---
    print(f"\n{'='*70}")
    print(f"ONLY IN XML ({len(only_xml)}) — missing from mapping.toml")
    print(f"{'='*70}")
    for k in only_xml:
        print(f"  {u_repr(k):50s} -> {u_repr(xml_functional[k])}")

    # --- Only in TOML ---
    print(f"\n{'='*70}")
    print(f"ONLY IN TOML ({len(only_toml)}) — missing from CharCombiC2U.xml")
    print(f"{'='*70}")
    for k in only_toml:
        print(f"  {u_repr(k):50s} -> {u_repr(toml_map[k])}")

    # --- Summary ---
    print(f"\n{'='*70}")
    print("SUMMARY")
    print(f"{'='*70}")
    print(f"  Exact matches : {len(matches)}")
    print(f"  Conflicts     : {len(conflicts)}")
    print(f"  Only in XML   : {len(only_xml)}")
    print(f"  Only in TOML  : {len(only_toml)}")

    coverage = len(matches) / len(xml_functional) * 100 if xml_functional else 0
    print(f"  TOML coverage of XML functional mappings: {coverage:.1f}%")


if __name__ == "__main__":
    main()
