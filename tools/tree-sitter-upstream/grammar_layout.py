"""Resolve every generated parser in a single- or multi-grammar package."""
import json
from pathlib import Path
import re

from grammar_runtime import digest


def layout_digest():
    return digest(Path(__file__).read_bytes())


def parser_roots(grammar):
    metadata = json.loads((grammar / "tree-sitter.json").read_text(encoding="utf-8"))
    roots = []
    for item in metadata["grammars"]:
        path = (grammar / item["path"]).resolve(strict=True)
        if not path.is_relative_to(grammar):
            raise ValueError("grammar path escapes package")
        if path not in roots:
            roots.append(path)
    if not roots or any(not (root / "src/parser.c").is_file() for root in roots):
        raise ValueError("package has no complete tracked parser layout")
    return roots


def parser_digests(grammar):
    return {str((root / "src/parser.c").relative_to(grammar).as_posix()): digest((root / "src/parser.c").read_bytes())
            for root in parser_roots(grammar)}


def parser_commitment(grammar):
    files = parser_digests(grammar)
    # Preserve the existing single-parser receipt representation.
    return next(iter(files.values())) if len(files) == 1 else digest(json.dumps(files, sort_keys=True).encode())


def parser_abi(grammar):
    abis = set()
    for root in parser_roots(grammar):
        match = re.search(r"^#define LANGUAGE_VERSION (\d+)$", (root / "src/parser.c").read_text(encoding="utf-8"), re.MULTILINE)
        if match is None:
            raise ValueError("generated parser ABI is missing")
        abis.add(int(match.group(1)))
    if len(abis) != 1:
        raise ValueError("package parsers use different ABIs")
    return abis.pop()


def generated_digests(grammar):
    return {(root / name).relative_to(grammar).as_posix(): digest((root / name).read_bytes())
            for root in parser_roots(grammar)
            for name in ("src/parser.c", "src/grammar.json", "src/node-types.json")}


def lint_sources(grammar):
    # The TypeScript package's upstream script lints its shared implementation.
    if (grammar / "common/define-grammar.js").is_file():
        return ["common/define-grammar.js"]
    return [(root / "grammar.js").relative_to(grammar).as_posix() for root in parser_roots(grammar)]
