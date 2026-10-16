"""Protect multi-parser receipt boundaries, including TypeScript's Flow alias."""
import json
from pathlib import Path
import tempfile
import unittest

from grammar_layout import generated_digests, parser_abi, parser_commitment, parser_roots


class GrammarLayoutTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="entmoot-grammar-layout-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()

    def package(self, roots):
        (self.root / "tree-sitter.json").write_text(json.dumps({"grammars": [
            {"name": name, "path": path} for name, path in roots]}), encoding="utf-8")
        for _, path in roots:
            parser = self.root / path / "src"
            parser.mkdir(parents=True, exist_ok=True)
            (parser / "parser.c").write_bytes(b"#define LANGUAGE_VERSION 15\n")
            for name in ("grammar.json", "node-types.json"):
                (parser / name).write_bytes(b"{}\n")

    def test_second_dialect_changes_the_package_commitment(self):
        self.package([("typescript", "typescript"), ("tsx", "tsx"), ("flow", "tsx")])
        self.assertEqual(len(parser_roots(self.root)), 2)
        self.assertEqual(len(generated_digests(self.root)), 6)
        before = parser_commitment(self.root)
        (self.root / "tsx/src/parser.c").write_bytes(b"#define LANGUAGE_VERSION 15\n/* changed */\n")
        self.assertNotEqual(before, parser_commitment(self.root))

    def test_mixed_parser_abis_are_rejected(self):
        self.package([("typescript", "typescript"), ("tsx", "tsx")])
        (self.root / "tsx/src/parser.c").write_bytes(b"#define LANGUAGE_VERSION 14\n")
        with self.assertRaisesRegex(ValueError, "different ABIs"):
            parser_abi(self.root)

    def test_package_escape_is_rejected_before_reading_parser(self):
        (self.root / "tree-sitter.json").write_text(json.dumps({"grammars": [
            {"name": "outside", "path": ".."}]}), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "escapes package"):
            parser_roots(self.root)

    def test_missing_second_parser_is_not_silently_skipped(self):
        self.package([("typescript", "typescript"), ("tsx", "tsx")])
        (self.root / "tsx/src/parser.c").unlink()
        with self.assertRaisesRegex(ValueError, "complete tracked parser"):
            parser_commitment(self.root)


if __name__ == "__main__":
    unittest.main()
