#!/usr/bin/env python3
"""Render the reviewed CHANGELOG section for a stable GitHub release."""
import re
import sys
from pathlib import Path


def render(tag: str, changelog: str) -> str:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError("expected a stable version tag such as v0.1.8")
    sections = re.split(r"^## ", changelog, flags=re.MULTILINE)
    matches = [section.split("\n", 1)[1].strip() for section in sections[1:]
               if section.split("\n", 1)[0].strip() == tag]
    if len(matches) != 1 or not re.search(r"^### (Features|Fixes)\n", matches[0], re.MULTILINE):
        raise ValueError(f"{tag} needs one reviewed Features/Fixes section in CHANGELOG.md")
    if not re.search(r"^- \S", matches[0], re.MULTILINE):
        raise ValueError(f"{tag} has no described changes")
    return matches[0] + "\n\n" + (
        "Linux x86_64 and ARM64 builds; Ubuntu, Rocky Linux (RHEL-compatible) and Arch installation checks.\n\n"
        "cx is under active development. See [validation](https://github.com/1unarzDev/cx/blob/"
        + tag + "/VALIDATION.md) for tested behavior and known limitations.\n\n"
        "Downloads are signed: verify against the pinned release-key.pem. SHA256SUMS alone is not an authenticity check.\n"
    )


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: release_notes.py vMAJOR.MINOR.PATCH")
        print(render(sys.argv[1], Path(__file__).resolve().parents[1].joinpath("CHANGELOG.md").read_text()), end="")
    except ValueError as error:
        sys.exit(str(error))
