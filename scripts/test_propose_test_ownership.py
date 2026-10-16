"""Tests for ownership proposals and skip-prefix vs ext_lang."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.propose_test_ownership import (
    cargo_proposals,
    diff_proposals,
    propose_core_test,
    propose_package,
    write_draft,
    _local_import_owners,
)
from scripts.test_ownership import load_registry


class ProposeOwnershipTests(unittest.TestCase):
    def test_python_convention_suggests_plugin_and_test_file(self) -> None:
        proposal = propose_core_test("test_python")
        paths = [o["path"] for o in proposal.owners]
        self.assertIn("crates/entmoot-core/src/languages/python/", paths)
        self.assertIn("crates/entmoot-core/tests/test_python.rs", paths)
        self.assertEqual(proposal.extensions, [".py"])

    def test_package_convention_suggests_crate_path(self) -> None:
        proposal = propose_package("entmoot-mcp")
        self.assertEqual(proposal.cargo, {"package": "entmoot-mcp"})
        self.assertEqual(
            proposal.owners,
            [{"kind": "crate", "path": "crates/entmoot-mcp/"}],
        )

    def test_convention_proposals_match_current_registry(self) -> None:
        """Registry is complete relative to Cargo.toml conventions (INV-GATE-02)."""
        report = diff_proposals(cargo_proposals())
        self.assertEqual(
            report["missing_from_registry"],
            [],
            msg=json.dumps(report["missing_from_registry"], indent=2),
        )
        self.assertEqual(report["extra_in_registry"], [])

    def test_write_draft_is_review_only_schema(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "draft.json"
            write_draft(out, [propose_core_test("test_go")])
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertIn("DRAFT", payload["notes"])
            self.assertEqual(len(payload["targets"]), 1)
            self.assertEqual(payload["targets"][0]["cargo"]["test"], "test_go")

    def test_write_draft_refuses_authoritative_registry_path(self) -> None:
        registry = Path(__file__).resolve().parent / "test_ownership.json"
        with self.assertRaises(ValueError) as ctx:
            write_draft(registry, [propose_core_test("test_go")])
        self.assertIn("refusing to overwrite", str(ctx.exception))

    def test_local_import_owners_finds_sibling_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "demo_pkg"
            pkg.mkdir()
            (pkg / "__init__.py").write_text("x = 1\n", encoding="utf-8")
            test_file = root / "test_demo.py"
            test_file.write_text("import demo_pkg\n\ndef test_ok():\n    assert True\n", encoding="utf-8")
            # Paths must be under repo for relative_to — skip if outside.
            # Exercise AST helper with a repo-relative fixture under scripts/.
        fixture_dir = Path(__file__).resolve().parent / "_ownership_propose_fixture"
        fixture_dir.mkdir(exist_ok=True)
        pkg = fixture_dir / "own_fixture_pkg"
        pkg.mkdir(exist_ok=True)
        (pkg / "__init__.py").write_text("VALUE = 1\n", encoding="utf-8")
        test_file = fixture_dir / "test_own_fixture.py"
        test_file.write_text(
            "import own_fixture_pkg\n\ndef test_ok():\n    assert own_fixture_pkg.VALUE == 1\n",
            encoding="utf-8",
        )
        try:
            owners = _local_import_owners(test_file, fixture_dir)
            paths = [o["path"] for o in owners]
            self.assertTrue(any(p.endswith("test_own_fixture.py") for p in paths))
            self.assertTrue(
                any("own_fixture_pkg" in p for p in paths),
                msg=paths,
            )
        finally:
            test_file.unlink(missing_ok=True)
            (pkg / "__init__.py").unlink(missing_ok=True)
            pkg.rmdir()
            # leave fixture_dir if other files; try rmdir
            try:
                fixture_dir.rmdir()
            except OSError:
                pass


class RegistryStillAuthoritativeTests(unittest.TestCase):
    def test_validate_registry_still_clean(self) -> None:
        from scripts.test_ownership import validate_registry

        errors = validate_registry(load_registry())
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
