"""Ownership registry: path→cargo, sole-owner delete → obsolete, Cargo.toml coverage."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.pr_structural_diff import (
    DiffReport,
    FileResult,
    build_receipt,
    build_gate_decisions,
    map_path_to_selection,
)
from scripts.test_ownership import (
    CargoTarget,
    cargo_commands_for_targets,
    declared_core_tests,
    load_registry,
    validate_registry,
)


class OwnershipRegistryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.registry = load_registry()

    def test_campaign_owner_is_not_shadowed_by_a_skip_prefix(self) -> None:
        targets = self.registry.targets_for_path("evals/harness/revision_campaign.py")
        self.assertIn(CargoTarget("entmoot", "campaign_harness"), targets)

    def test_named_engine_target_does_not_widen_to_package(self) -> None:
        targets = self.registry.targets_for_path("crates/entmoot-engine/tests/joins_soundness.rs")
        keys = {target.key() for target in targets}
        self.assertIn("entmoot-engine:joins_soundness", keys)
        self.assertNotIn("entmoot-engine", keys)
        self.assertNotIn("entmoot", keys)
        commands = cargo_commands_for_targets(targets)
        self.assertTrue(any("--test" in cmd and "joins_soundness" in cmd for cmd in commands))
        self.assertFalse(any(cmd.count("-p") >= 1 and "--test" not in cmd and "--lib" not in cmd
                             and "--doc" not in cmd for cmd in commands))

    def test_claim_cohort_co_selects_named_targets(self) -> None:
        cohort = {
            "entmoot-engine:dispatch_expectations",
            "entmoot-engine:callsites_rust",
            "entmoot-engine:callsites_javascript",
            "entmoot-engine:callsites_go",
            "entmoot-engine:callsites_java",
            "entmoot-engine:callsites_csharp",
            "entmoot-engine:callsites_c_family",
            "entmoot-engine:joins_soundness",
            "entmoot:adapter_controls",
            "entmoot:program_adapters",
        }
        for path in (
            "crates/entmoot-engine/tests/dispatch_expectations.rs",
            "crates/entmoot-engine/tests/callsites_rust.rs",
            "crates/entmoot-cli/tests/adapter_controls.rs",
            "crates/entmoot-engine/tests/support/callsites_expect.rs",
            "crates/entmoot-engine/tests/fixtures/callsites/expectations_dispatch.json",
        ):
            with self.subTest(path=path):
                keys = {target.key() for target in self.registry.targets_for_path(path)}
                self.assertTrue(cohort <= keys, msg=f"{path} missing {sorted(cohort - keys)}")

    def test_shared_library_change_keeps_unit_doctest_and_downstream_controls(self) -> None:
        targets = self.registry.targets_for_path("crates/entmoot-contracts/src/code_similarity.rs")
        self.assertIn(CargoTarget("entmoot-engine"), targets)
        self.assertIn(CargoTarget("entmoot"), targets)
        self.assertIn(CargoTarget("entmoot-mcp"), targets)
        core = self.registry.targets_for_path("crates/entmoot-core/src/visitor.rs")
        commands = cargo_commands_for_targets(core)
        self.assertTrue(any("--lib" in c for c in commands))
        self.assertTrue(any("--doc" in c for c in commands))

    def test_workspace_configuration_keeps_core_unit_and_doctest_targets(self) -> None:
        targets = self.registry.targets_for_path("Cargo.toml")
        self.assertIn(CargoTarget("entmoot-core"), targets)

    def test_assessment_catalog_uses_the_same_ownership_floor(self) -> None:
        path = "crates/entmoot-engine/tests/joins_soundness.rs"
        catalog = self.registry.impact_request([path])
        ids = {target["id"] for target in catalog["targets"]}
        self.assertIn("entmoot-engine:joins_soundness", ids)
        self.assertIn("entmoot-engine:dispatch_expectations", ids)
        self.assertIn("entmoot:adapter_controls", ids)
        joins = next(t for t in catalog["targets"] if t["id"] == "entmoot-engine:joins_soundness")
        self.assertEqual(joins["owners"], [path])
        self.assertEqual(joins["runner"], "cargo")
        self.assertIn("joins_soundness", joins["command"])

    def test_file_revision_fixture_selects_its_consumer(self):
        self.assertIn(CargoTarget("entmoot-core", "file_revision_pairs"),
                      self.registry.targets_for_path("crates/entmoot-core/tests/fixtures/file_revision_pairs.json"))

    def test_manifest_cache_invalidates_on_new_dependency_bytes(self):
        from scripts.test_ownership import workspace_manifests
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "crates/entmoot-engine/Cargo.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("[dependencies]\n", encoding="utf-8")
            self.assertEqual(workspace_manifests(root)["entmoot-engine"]["dependencies"], {})
            manifest.write_text("[dependencies]\nentmoot-core={path='../entmoot-core'}\n", encoding="utf-8")
            self.assertIn("entmoot-core", workspace_manifests(root)["entmoot-engine"]["dependencies"])

    def test_target_specific_and_renamed_inherited_dependencies_keep_consumers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[workspace.dependencies]\nrenamed={package="entmoot-core",path="crates/entmoot-core"}\n', encoding="utf-8")
            for package, contents in {
                "entmoot-core": '[package]\nname="entmoot-core"\n',
                "entmoot-engine": '[target.\'cfg(windows)\'.dependencies]\nrenamed={workspace=true,optional=true}\n',
                "entmoot": '[build-dependencies]\nengine={package="entmoot-engine",path="../entmoot-engine"}\n',
            }.items():
                path = root / "crates" / package / "Cargo.toml"
                path.parent.mkdir(parents=True)
                path.write_text(contents, encoding="utf-8")
            self.registry.path = root / "scripts/test_ownership.json"
            targets = self.registry.targets_for_path("crates/entmoot-core/src/lib.rs")
            self.assertIn(CargoTarget("entmoot-engine"), targets)
            self.assertIn(CargoTarget("entmoot"), targets)

    def test_compatible_packages_share_one_cargo_invocation(self) -> None:
        commands = cargo_commands_for_targets([CargoTarget("entmoot-engine"), CargoTarget("entmoot")])
        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0].count("-p"), 2)

    def test_campaign_control_is_not_lost_when_its_package_is_also_selected(self):
        commands = cargo_commands_for_targets([CargoTarget("entmoot"), CargoTarget("entmoot", "campaign_harness")])
        self.assertTrue(any("campaign_harness" in cmd and "--include-ignored" in cmd for cmd in commands))

    def test_selected_native_targets_enable_their_semantic_producers(self):
        cases = [
            ("entmoot-engine", "callsites_rust", {"entmoot-engine/native-tests", "entmoot-engine/resolved-rust-analyzer"}),
            ("entmoot-engine", "callsites_javascript", {"entmoot-engine/native-tests", "entmoot-engine/resolved-oxc"}),
            ("entmoot-engine", "dispatch_expectations", {"entmoot-engine/native-tests", "entmoot-engine/resolved-rust-analyzer", "entmoot-engine/resolved-oxc"}),
            ("entmoot", "adapter_controls", {"entmoot/native-tests", "entmoot/producer"}),
            ("entmoot", "producer_cli", {"entmoot/producer"}),
        ]
        for package, name, required in cases:
            for whole_package in (False, True):
                with self.subTest(target=name, whole_package=whole_package):
                    targets = [CargoTarget(package, name)]
                    if whole_package:
                        targets.append(CargoTarget(package))
                    commands = cargo_commands_for_targets(targets)
                    command = next(c for c in commands if "--test" not in c or name in c)
                    enabled = set(command[command.index("--features") + 1].split(","))
                    self.assertLessEqual(required, enabled)

    def test_structural_package_selection_does_not_enable_native_tools(self):
        commands = cargo_commands_for_targets([CargoTarget("entmoot-engine"), CargoTarget("entmoot")])
        self.assertEqual(commands[0][commands[0].index("--features") + 1], "entmoot-core/full-tests")

    def test_registry_covers_all_declared_core_tests_and_packages(self) -> None:
        errors = validate_registry(self.registry)
        self.assertEqual(errors, [], msg="; ".join(errors))

    def test_python_plugin_path_selects_python_regressions(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        map_path_to_selection(
            "crates/entmoot-core/src/languages/python/mod.rs", report, force=True
        )
        self.assertEqual(report.selected_entmoot_core_tests, ["visitor_depth", "revision_pairs", "test_python", "python_inventory"])
        self.assertIn("entmoot-engine", report.selected_packages)
        self.assertTrue(report.selected_from_owners)

    def test_docs_path_skips_selection(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        map_path_to_selection("docs/architecture/ARCHITECTURE.md", report, force=True)
        self.assertEqual(report.selected_entmoot_core_tests, [])
        self.assertEqual(report.selected_packages, [])

    def test_revision_fixture_data_selects_both_visitor_and_identity_consumers(self) -> None:
        targets = self.registry.targets_for_path("crates/entmoot-core/tests/fixtures/revision_pairs.json")
        keys = {target.key() for target in targets}
        self.assertIn("entmoot-core:revision_pairs", keys)
        self.assertIn("entmoot-engine:resolved_identity_pyright_parity", keys)
        self.assertIn("entmoot-engine:joins_soundness", keys)

    def test_mcp_crate_selects_entmoot_mcp(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        map_path_to_selection("crates/entmoot-mcp/src/lib.rs", report, force=True)
        self.assertEqual(report.selected_packages, ["entmoot-mcp"])

    def test_c_family_plugin_file_selects_inventory_test(self) -> None:
        targets = self.registry.targets_for_path(
            "crates/entmoot-core/src/languages/c_family.rs"
        )
        keys = {t.key() for t in targets}
        self.assertIn("entmoot-core:c_family_inventory", keys)

    def test_deleted_integration_test_does_not_select_missing_stem(self) -> None:
        targets = self.registry.targets_for_path(
            "crates/entmoot-core/tests/test_utils.rs"
        )
        keys = {t.key() for t in targets}
        self.assertNotIn("entmoot-core:test_utils", keys)

    def test_shared_modules_and_fixtures_select_only_declared_consumers(self) -> None:
        declared = set(declared_core_tests())
        for path in ("common/mod.rs", "support/revision_pairs.rs", "fixtures/sample_granular.py"):
            with self.subTest(path=path):
                targets = self.registry.targets_for_path("crates/entmoot-core/tests/" + path)
                selected = {target.test for target in targets if target.package == "entmoot-core"}
                self.assertTrue(selected)
                self.assertLessEqual(selected, declared)
                if path == "support/revision_pairs.rs":
                    self.assertEqual(selected, {"revision_pairs", "test_config_comparison"})
                else:
                    self.assertIn("test_python", selected)
                    self.assertIn("revision_pairs", selected)

    def test_sole_owner_delete_marks_obsolete(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        report.files = [
            FileResult(
                path="crates/entmoot-core/src/languages/python/",
                classification="deleted",
                detail="plugin removed",
            ),
            FileResult(
                path="crates/entmoot-core/tests/test_python.rs",
                classification="deleted",
                detail="test removed",
            ),
        ]
        # head_exists empty → no owners remain
        report.obsolete_tests = self.registry.obsolete_targets(
            [
                "crates/entmoot-core/src/languages/python/",
                "crates/entmoot-core/tests/test_python.rs",
            ],
            head_exists=set(),
        )
        cargos = [entry["cargo"] for entry in report.obsolete_tests]
        self.assertIn({"package": "entmoot-core", "test": "test_python"}, cargos)
        receipt = build_receipt(report, build_gate_decisions(report))
        self.assertEqual(
            receipt["evidence"]["obsolete_tests"],
            report.obsolete_tests,
        )

    def test_remaining_owner_on_head_is_not_obsolete(self) -> None:
        obsolete = self.registry.obsolete_targets(
            ["crates/entmoot-core/src/languages/python/"],
            head_exists={"crates/entmoot-core/tests/test_python.rs"},
        )
        cargos = [entry["cargo"] for entry in obsolete]
        self.assertNotIn({"package": "entmoot-core", "test": "test_python"}, cargos)

    def test_ownerless_detects_missing_disk_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # Empty tree → every concrete owner is missing
            ownerless = self.registry.ownerless_targets(root)
            self.assertTrue(ownerless)

    def test_populate_wires_selected_from_owners_into_receipt(self) -> None:
        report = DiffReport(base="base", head="head", config="standard")
        map_path_to_selection("crates/entmoot-engine/src/lib.rs", report, force=True)
        decisions = build_gate_decisions(report)
        receipt = build_receipt(report, decisions)
        self.assertTrue(receipt["evidence"]["selected_from_owners"])
        self.assertEqual(receipt["evidence"]["obsolete_tests"], [])


if __name__ == "__main__":
    unittest.main()
