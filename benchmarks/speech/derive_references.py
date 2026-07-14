#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "indic-transliteration==2.3.82",
# ]
# ///
"""Create an explicitly machine-derived Roman proxy track from a mixed-script manifest."""

from __future__ import annotations

import argparse
import json
import unicodedata
from pathlib import Path

from indic_transliteration import sanscript
from indic_transliteration.sanscript import transliterate


def roman_proxy(text: str) -> str:
    """Transliterate Devanagari to IAST, then remove diacritics while preserving existing Latin text."""
    iast = transliterate(text, sanscript.DEVANAGARI, sanscript.IAST)
    decomposed = unicodedata.normalize("NFKD", iast)
    ascii_proxy = "".join(char for char in decomposed if not unicodedata.combining(char))
    return ascii_proxy.replace("~", "n").casefold()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    source = args.input.expanduser().resolve()
    output = args.output.expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with source.open(encoding="utf-8") as reader, output.open("w", encoding="utf-8") as writer:
        for line in reader:
            record = json.loads(line)
            record["referenceStyle"] = "roman"
            record["referenceQuality"] = "machine-derived-proxy"
            record["referenceDerivation"] = "indic-transliteration-2.3.82-devanagari-iast-ascii"
            if record.get("type") != "corpus_header":
                record["ref_text"] = roman_proxy(record["ref_text"])
            writer.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
