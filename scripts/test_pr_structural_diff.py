from __future__ import annotations

import unittest

from scripts.pr_structural_diff import (
    DiffReport,
    FileResult,
    GATE_SCHEMA_VERSION,
    boundary_change_count,
    build_gate_decisions,
    build_receipt,
    callable_delta,
    force_full_workspace_selection,
    map_path_to_selection,
    needs_retest,
    parse_name_status,
    selection_boundary,
    should_skip_cli_build,
)


def inventory(
    *structural_hashes: str,
    shell: str = "blake3:shell",
    signatures: tuple[str, ...] | None = None,
) -> dict[str, object]:
    signatures = signatures or tuple("blake3:sig" for _ in structural_hashes)
    return {
        "file_hash": "blake3:file",
        "module_shell_hash": shell,
        "occurrences": [
            {
                "structural_hash": structural_hash,
                "address_id": f"address-{index}",
                "logic_address": f"logic|test|{index}",
            }
            for index, structural_hash in enumerate(structural_hashes)
        ],
        "callable_boundaries": [
            {
                "address_id": f"address-{index}",
                "signature_hash": signatures[index],
                "syntactic_signature": "function_signature",
            }
            for index in range(len(structural_hashes))
        ],
    }


class CallableInvalidationTests(unittest.TestCase):
    def test_workflow_configuration_is_accepted_by_the_real_parser(self) -> None:
        import re
        from pathlib import Path
        from unittest.mock import patch
        from scripts import pr_structural_diff

        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/pr-gate-decision.yml").read_text(encoding="utf-8")
        config = re.search(r"--config\s+(\S+)", workflow).group(1)
        report = DiffReport(base="base", head="head", config="v1")
        with patch("sys.argv", ["pr_structural_diff", "--base", "base", "--head", "head", "--path-preflight", "--config", config]), \
                patch.object(pr_structural_diff, "path_preflight_report", return_value=report) as collect, \
                patch.object(pr_structural_diff, "write_outputs"):
            self.assertEqual(pr_structural_diff.main(), 0)
        collect.assert_called_once_with("base", "head", "v1")

    def test_structural_hash_multiset_preserves_moves_and_duplicates(self) -> None:
        invalidated, introduced, retained = callable_delta(
            inventory("blake3:a", "blake3:a", "blake3:b"),
            inventory("blake3:a", "blake3:b", "blake3:c"),
        )
        self.assertEqual(invalidated, ["blake3:a"])
        self.assertEqual(introduced, ["blake3:c"])
        self.assertEqual(retained, 2)

    def test_unchanged_structural_hashes_do_not_trigger_callable_retest(self) -> None:
        invalidated, introduced, retained = callable_delta(
            inventory("blake3:a", "blake3:b"),
            inventory("blake3:b", "blake3:a"),
        )
        self.assertEqual((invalidated, introduced, retained), ([], [], 2))
        self.assertFalse(needs_retest("noop"))

    def test_boundary_change_is_detected_when_body_survives(self) -> None:
        base = inventory("blake3:a", signatures=("blake3:sig-a",))
        head = inventory("blake3:a", signatures=("blake3:sig-b",))
        self.assertEqual(boundary_change_count(base, head), 1)
        self.assertTrue(needs_retest("boundary_changed"))

    def test_boundary_matching_is_multiset_safe(self) -> None:
        base = inventory(
            "blake3:a",
            "blake3:a",
            signatures=("blake3:sig-a", "blake3:sig-b"),
        )
        head = inventory(
            "blake3:a",
            "blake3:a",
            signatures=("blake3:sig-b", "blake3:sig-c"),
        )
        self.assertEqual(boundary_change_count(base, head), 1)

    def test_boundary_join_rejects_missing_record(self) -> None:
        base = inventory("blake3:a")
        head = inventory("blake3:a")
        head["callable_boundaries"] = []
        with self.assertRaisesRegex(ValueError, "omitted boundary record"):
            boundary_change_count(base, head)

    def test_changed_component_selects_only_its_package(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        map_path_to_selection("crates/entmoot-cli/src/main.rs", report, force=False)
        self.assertEqual(report.selected_packages, [])
        map_path_to_selection("crates/entmoot-cli/src/main.rs", report, force=True)
        self.assertEqual(report.selected_packages, ["entmoot"])

    def test_module_and_analysis_failures_widen(self) -> None:
        self.assertTrue(needs_retest("module_changed"))
        self.assertTrue(needs_retest("error"))

    def test_git_rename_preserves_both_blob_paths(self) -> None:
        changes = parse_name_status(b"R100\0old/path.rs\0new/path.rs\0M\0same.rs\0")
        self.assertEqual(len(changes), 2)
        self.assertEqual(changes[0].base_path, "old/path.rs")
        self.assertEqual(changes[0].head_path, "new/path.rs")
        self.assertEqual(changes[0].path, "new/path.rs")
        self.assertEqual(changes[1].base_path, "same.rs")
        self.assertEqual(changes[1].head_path, "same.rs")

    def test_ownership_boundary_distinguishes_cross_component_moves(self) -> None:
        self.assertEqual(
            selection_boundary("crates/entmoot-cli/src/main.rs"), "crate:entmoot-cli"
        )
        self.assertNotEqual(
            selection_boundary("crates/entmoot-engine/src/lib.rs"),
            selection_boundary("crates/entmoot-cli/src/main.rs"),
        )


class GateDecisionPolicyTests(unittest.TestCase):
    def test_docs_only_path_preflight_skips_cli_build_and_tests(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        report.files = [
            FileResult(path="docs/architecture/ARCHITECTURE.md", classification="skip"),
            FileResult(path="docs/runbooks/impact.md", classification="skip"),
        ]
        decisions = build_gate_decisions(report, path_preflight_only=True)
        actions = [d.action for d in decisions]
        self.assertEqual(actions, ["skip_cli_build", "skip_cargo_tests"])
        self.assertTrue(should_skip_cli_build(decisions))
        receipt = build_receipt(report, decisions)
        self.assertEqual(receipt["schema_version"], GATE_SCHEMA_VERSION)
        self.assertTrue(receipt["skip_cli_build"])
        self.assertEqual(receipt["cargo_commands"], [])
        self.assertTrue(receipt["evidence"]["docs_only"])

    def test_structural_noop_skips_cli_build_and_tests(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        report.files = [
            FileResult(
                path="crates/entmoot-core/src/languages/python/mod.rs",
                classification="noop",
                language="rust",
                base_hash="blake3:a",
                head_hash="blake3:a",
            ),
            FileResult(path="README.md", classification="skip"),
        ]
        decisions = build_gate_decisions(report, path_preflight_only=False)
        self.assertEqual(
            [d.action for d in decisions],
            ["skip_cli_build", "skip_cargo_tests"],
        )
        self.assertEqual(build_receipt(report, decisions)["cargo_commands"], [])

    def test_rust_core_change_runs_cli_build_and_selected_tests(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        report.files = [
            FileResult(
                path="crates/entmoot-cli/src/main.rs",
                classification="path_changed",
                detail="non-source content changed",
            )
        ]
        map_path_to_selection("crates/entmoot-cli/src/main.rs", report, force=True)
        decisions = build_gate_decisions(report, path_preflight_only=False)
        actions = [d.action for d in decisions]
        self.assertEqual(actions, ["run_cli_build", "run_cargo_tests"])
        self.assertFalse(should_skip_cli_build(decisions))
        receipt = build_receipt(report, decisions)
        self.assertTrue(any("-p entmoot" in command for command in receipt["cargo_commands"]))

    def test_fail_closed_forces_full_workspace_selection(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        report.files = [
            FileResult(
                path="crates/entmoot-core/src/hasher.rs",
                classification="error",
                detail="invalid callable inventory",
            )
        ]
        force_full_workspace_selection(report)
        decisions = build_gate_decisions(report, path_preflight_only=False)
        self.assertEqual(decisions[0].action, "fail_closed")
        self.assertIn("run_cli_build", [d.action for d in decisions])
        self.assertIn("run_cargo_tests", [d.action for d in decisions])
        receipt = build_receipt(report, decisions)
        self.assertTrue(receipt["evidence"]["fail_closed"])
        self.assertTrue(any("entmoot-core" in cmd for cmd in receipt["cargo_commands"]))
        self.assertTrue(any("entmoot-engine" in cmd for cmd in receipt["cargo_commands"]))

    def test_path_preflight_non_doc_requests_cli_build(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        report.files = [
            FileResult(
                path="crates/entmoot-engine/src/lib.rs",
                classification="path_changed",
                detail="path-preflight: requires structural classification",
            )
        ]
        decisions = build_gate_decisions(report, path_preflight_only=True)
        self.assertEqual([d.action for d in decisions], ["run_cli_build"])
        self.assertFalse(should_skip_cli_build(decisions))

    def test_skip_listed_py_scripts_are_skip_despite_ext_lang(self) -> None:
        """Skip prefixes win over language extension (scripts/*.py must not fail-closed)."""
        from scripts.pr_structural_diff import (
            ChangedPath,
            path_preflight_from_changes,
        )

        report = path_preflight_from_changes(
            [
                ChangedPath(
                    path="scripts/pre_commit_check.py",
                    base_path="scripts/pre_commit_check.py",
                    head_path="scripts/pre_commit_check.py",
                    status="M",
                ),
                ChangedPath(
                    path="scripts/test_ownership.py",
                    base_path="scripts/test_ownership.py",
                    head_path="scripts/test_ownership.py",
                    status="M",
                ),
                ChangedPath(
                    path="docs/architecture/ARCHITECTURE.md",
                    base_path="docs/architecture/ARCHITECTURE.md",
                    head_path="docs/architecture/ARCHITECTURE.md",
                    status="M",
                ),
            ],
            base="base",
            head="head",
            config="standard",
        )
        self.assertTrue(all(f.classification == "skip" for f in report.files))
        decisions = build_gate_decisions(report, path_preflight_only=True)
        self.assertEqual(
            [d.action for d in decisions],
            ["skip_cli_build", "skip_cargo_tests"],
        )


class ClassifyIndexTests(unittest.TestCase):
    def test_equal_structural_inventory_does_not_erase_line_sensitive_change(self) -> None:
        from pathlib import Path
        from unittest.mock import patch
        from scripts.pr_structural_diff import ChangedPath, classify_file

        source = b"fn value() -> u32 { line!() }\n"
        path = "crates/entmoot-cli/src/main.rs"
        with patch("scripts.pr_structural_diff.hash_blob", return_value=(inventory("body"), None)):
            result = classify_file(ChangedPath(path, path, path, "M"), "base", "head", Path("entmoot"), "v1", base_blob_override=source, head_blob_override=b"\n" + source)
        self.assertTrue(needs_retest(result.classification))

    def test_classify_index_structural_noop_skips_selection(self) -> None:
        from pathlib import Path
        from unittest.mock import patch

        from scripts.pr_structural_diff import ChangedPath, classify_index

        change = ChangedPath(
            path="crates/entmoot-core/src/languages/python/mod.rs",
            base_path="crates/entmoot-core/src/languages/python/mod.rs",
            head_path="crates/entmoot-core/src/languages/python/mod.rs",
            status="M",
        )
        stable = inventory("blake3:a")

        with patch(
            "scripts.pr_structural_diff.list_index_changed_paths",
            return_value=[change],
        ), patch(
            "scripts.pr_structural_diff.blobs_for_local_change",
            return_value=(b"same bytes", b"same bytes"),
        ), patch(
            "scripts.pr_structural_diff.hash_blob",
            return_value=(stable, None),
        ):
            report = classify_index(Path("entmoot"), staged_only=True)
        self.assertEqual(len(report.files), 1)
        self.assertEqual(report.files[0].classification, "noop")
        self.assertEqual(report.selected_packages, [])

    def test_classify_index_maps_invalidation_to_owners(self) -> None:
        from pathlib import Path
        from unittest.mock import patch

        from scripts.pr_structural_diff import ChangedPath, classify_index

        change = ChangedPath(
            path="crates/entmoot-cli/src/main.rs",
            base_path="crates/entmoot-cli/src/main.rs",
            head_path="crates/entmoot-cli/src/main.rs",
            status="M",
        )
        base_inv = inventory("blake3:a")
        head_inv = inventory("blake3:b")
        base_inv["file_hash"] = "blake3:file-base"
        head_inv["file_hash"] = "blake3:file-head"

        def fake_hash(_cli, _path, _lang, _config, content: bytes):
            if content == b"base":
                return base_inv, None
            if content == b"head":
                return head_inv, None
            return None, "unexpected content"

        with patch(
            "scripts.pr_structural_diff.list_index_changed_paths",
            return_value=[change],
        ), patch(
            "scripts.pr_structural_diff.blobs_for_local_change",
            return_value=(b"base", b"head"),
        ), patch("scripts.pr_structural_diff.hash_blob", side_effect=fake_hash):
            report = classify_index(Path("entmoot"), staged_only=True)
        self.assertEqual(report.files[0].classification, "callable_changed")
        self.assertEqual(report.selected_packages, ["entmoot"])


if __name__ == "__main__":
    unittest.main()
