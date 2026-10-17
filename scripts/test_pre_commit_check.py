from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
import os
from unittest.mock import patch

from scripts.pre_commit_check import check_crate_boundaries, check_local_markdown_links


class NativeLaneTests(unittest.TestCase):
    def test_catalog_controls_respect_owners_and_exclusive_overrides(self):
        from scripts.adapter_catalog import resolved_adapter
        from scripts.pre_commit_check import sidecar_controls
        with TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "adapter.exe"
            binary.touch()
            family = {"id":"go", "env":"ENTMOOT_GO_ADAPTER", "relative_artifacts":["adapter.exe"], "artifact_kind":"repo_relative",
                      "control":{"owners":["tools/resolver-adapters/go/"],"target":"adapter_controls","filter":"go_control"}}
            self.assertEqual(resolved_adapter(family, {}, root), binary)
            self.assertIsNone(resolved_adapter(family, {"ENTMOOT_GO_ADAPTER":""}, root))
            self.assertIsNone(resolved_adapter(family, {"ENTMOOT_GO_ADAPTER":str(root/'missing')}, root))
            with patch("scripts.adapter_catalog.load_catalog", return_value=[family]), patch("scripts.adapter_catalog.control_runtime_available", return_value=True):
                env={"ENTMOOT_GO_ADAPTER":str(binary)}
                self.assertEqual(sidecar_controls(env, ["unrelated.rs"]), [])
                commands = sidecar_controls(env, ["tools/resolver-adapters/go/main.go"])
                self.assertEqual(len(commands), 1)
                self.assertIn("--include-ignored", commands[0][1], "ordinary controls must actually execute")
                command = commands[0][1]
                features = set(command[command.index("--features") + 1].split(","))
                self.assertLessEqual({"entmoot/native-tests", "entmoot/producer"}, features)
                self.assertEqual(sidecar_controls({**env,"ENTMOOT_NATIVE_CHECK":"off"}, None), [])

    def test_catalog_discovers_checkout_then_path_then_home(self):
        from scripts.adapter_catalog import resolved_adapter
        with TemporaryDirectory() as directory:
            root = Path(directory)
            checkout = root / ".tmp/adapter-build/resolver-adapters/go/adapter.exe"
            on_path = root / "bin/adapter.exe"
            installed = root / "home/go/adapter.exe"
            for path in (checkout, on_path, installed):
                path.parent.mkdir(parents=True)
                path.touch()
            family = {"env":"ENTMOOT_GO_ADAPTER", "relative_artifacts":["go/adapter.exe"]}
            environment = {"PATH":str(on_path.parent), "ENTMOOT_ADAPTERS_ROOT":str(root / "home")}
            self.assertEqual(resolved_adapter(family, environment, root), checkout)
            checkout.unlink()
            self.assertEqual(resolved_adapter(family, environment, root), on_path)
            on_path.unlink()
            self.assertEqual(resolved_adapter(family, environment, root), installed)
    def test_native_lane_is_opt_in_or_preserves_an_existing_producer(self) -> None:
        from scripts.pre_commit_check import native_lane_enabled
        with TemporaryDirectory() as directory:
            producer = Path(directory) / "entmoot-producer.exe"
            self.assertFalse(native_lane_enabled({}, producer))
            self.assertTrue(native_lane_enabled({"ENTMOOT_NATIVE_CHECK": "on"}, producer))
            producer.touch()
            self.assertTrue(native_lane_enabled({}, producer))
            self.assertFalse(native_lane_enabled({"ENTMOOT_NATIVE_CHECK": "off"}, producer))
            with self.assertRaises(ValueError):
                native_lane_enabled({"ENTMOOT_NATIVE_CHECK": "typo"}, producer)

    def test_installed_native_lane_follows_affected_owners(self) -> None:
        from scripts.pre_commit_check import native_lane_enabled
        with TemporaryDirectory() as directory:
            producer = Path(directory) / "producer.exe"
            producer.touch()
            self.assertFalse(native_lane_enabled({}, producer, ["crates/entmoot-engine/tests/knowledge_store.rs"]))
            self.assertTrue(native_lane_enabled({}, producer, ["crates/entmoot-core/src/visitor.rs"]))
            self.assertTrue(native_lane_enabled({}, producer, ["tools/resolver-adapters/python/adapter.py"]))
            self.assertTrue(native_lane_enabled({"ENTMOOT_NATIVE_CHECK": "on"}, producer, []))


class AccelerationSkipTests(unittest.TestCase):
    def plan_for(self, path):
        from scripts.pre_commit_check import _acceleration_plan
        from scripts.pr_structural_diff import ChangedPath, path_preflight_from_changes
        report = path_preflight_from_changes([ChangedPath(path, path, path, "M")], base="HEAD", head="INDEX", config="v1")
        with patch("scripts.pr_structural_diff.path_preflight_index", return_value=report), patch("scripts.pr_structural_diff.hash_blob", side_effect=AssertionError("classifier must not authorize skips")):
            return _acceleration_plan()

    def test_docs_keep_cheap_checks_without_cargo(self):
        self.assertEqual(self.plan_for("docs/architecture/ARCHITECTURE.md").mode, "skip")

    def test_hook_unit_tests_keep_the_always_run_python_lane_without_cargo(self):
        self.assertEqual(self.plan_for("scripts/test_pre_commit_check.py").mode, "skip")

    def test_eval_configuration_and_python_units_need_no_cargo_build(self):
        from scripts import pre_commit_check as hook
        for path in ("evals/experiments/local-trials.json", "evals/harness/test_measurement.py",
                     "evals/harness/agent_provider.py", "evals/harness/test_campaign_configuration.py"):
            with self.subTest(path=path):
                plan = self.plan_for(path)
                self.assertEqual(plan.mode, "skip")
                with patch.multiple(hook, check_banned_patterns=lambda: 0, check_dylint=lambda: 0,
                                    check_cargo_deny=lambda: 0, check_cargo_machete=lambda: 0,
                                    check_crate_boundaries=lambda: 0, check_local_markdown_links=lambda: 0,
                                    check_test_ownership_registry=lambda: 0), \
                        patch.object(hook, "_acceleration_plan", return_value=plan), \
                        patch.object(hook, "native_lane_enabled", return_value=False), \
                        patch.object(hook, "run", return_value=0) as run, patch("builtins.print"):
                    self.assertEqual(hook.main(), 0)
                commands = [call.args[0] for call in run.call_args_list]
                self.assertTrue(any("evals/harness" in cmd and "discover" in cmd for cmd in commands))
                self.assertFalse(any(cmd[:2] in (["cargo", "build"], ["cargo", "test"], ["cargo", "clippy"])
                                     for cmd in commands))

    def test_source_uses_owners_without_building_or_trusting_stale_binary(self):
        plan = self.plan_for("crates/entmoot-cli/src/main.rs")
        self.assertEqual(plan.mode, "selected")
        self.assertEqual(plan.clippy_packages, ["entmoot"])

    def test_unmapped_paths_widen(self):
        self.assertEqual(self.plan_for("new-build-config.unknown").mode, "workspace")

    def test_mixed_unmapped_and_known_paths_widen(self):
        from scripts.pre_commit_check import _acceleration_plan
        from scripts.pr_structural_diff import ChangedPath, path_preflight_from_changes
        paths = ["crates/entmoot-cli/src/main.rs", "new-build-config.unknown"]
        report = path_preflight_from_changes([ChangedPath(p,p,p,"M") for p in paths],base="HEAD",head="INDEX",config="v1")
        with patch("scripts.pr_structural_diff.path_preflight_index", return_value=report):
            self.assertEqual(_acceleration_plan().mode, "workspace")


class ValidationIsolationTests(unittest.TestCase):
    def test_mcp_http_prepares_cli_once_before_testing_without_affecting_stdio(self):
        from scripts import pre_commit_check as hook
        for target, needs_cli in (("http_e2e", True), ("stdio_e2e", False)):
            plan = hook.AccelerationPlan("selected", "fixture", clippy_packages=["entmoot-mcp"],
                cargo_commands=[["cargo", "test", "-p", "entmoot-mcp", "--test", target]])
            with self.subTest(target=target), \
                    patch.multiple(hook, check_banned_patterns=lambda: 0, check_dylint=lambda: 0,
                                   check_cargo_deny=lambda: 0, check_cargo_machete=lambda: 0,
                                   check_crate_boundaries=lambda: 0, check_local_markdown_links=lambda: 0,
                                   check_test_ownership_registry=lambda: 0), \
                    patch.object(hook, "_acceleration_plan", return_value=plan), \
                    patch.object(hook, "native_lane_enabled", return_value=False), \
                    patch.object(hook, "sidecar_controls", return_value=[]), \
                    patch.object(hook, "run", return_value=0) as run, patch("builtins.print"):
                self.assertEqual(hook.main(), 0)
            commands = [call.args[0] for call in run.call_args_list]
            builds = [cmd for cmd in commands if cmd[:2] == ["cargo", "build"]]
            self.assertEqual(len(builds), int(needs_cli))
            if needs_cli:
                build = builds[0]
                self.assertEqual(build[build.index("--profile") + 1], "test")
                self.assertLess(commands.index(build), next(i for i, cmd in enumerate(commands) if cmd[:2] == ["cargo", "test"]))

    def test_sidecar_features_survive_without_the_in_process_native_lane(self):
        from scripts import pre_commit_check as hook
        command = ["cargo", "test", "-p", "entmoot", "--features", "entmoot/native-tests", "--test", "adapter_controls", "go_control", "--", "--include-ignored", "--exact"]
        with patch.multiple(hook, check_banned_patterns=lambda: 0, check_dylint=lambda: 0,
                            check_cargo_deny=lambda: 0, check_cargo_machete=lambda: 0,
                            check_crate_boundaries=lambda: 0, check_local_markdown_links=lambda: 0,
                            check_test_ownership_registry=lambda: 0), \
                patch.object(hook, "_acceleration_plan", return_value=hook.AccelerationPlan("workspace", "fixture")), \
                patch.object(hook, "native_lane_enabled", return_value=False), \
                patch.object(hook, "sidecar_controls", return_value=[("go", command)]), \
                patch.object(hook, "run", return_value=0) as run, patch("builtins.print"):
            self.assertEqual(hook.main(), 0)
        selected = next(call.args[0] for call in run.call_args_list if "go_control" in call.args[0])
        self.assertIn("entmoot/native-tests", selected[selected.index("--features") + 1])
        self.assertNotIn("entmoot/producer", selected[selected.index("--features") + 1])

    def test_main_runs_cheap_checks_then_one_feature_aligned_workspace_lane(self):
        from scripts import pre_commit_check as hook
        with patch.multiple(hook, check_banned_patterns=lambda: 0, check_dylint=lambda: 0,
                            check_cargo_deny=lambda: 0, check_cargo_machete=lambda: 0,
                            check_crate_boundaries=lambda: 0,
                            check_local_markdown_links=lambda: 0, check_test_ownership_registry=lambda: 0), \
                patch.object(hook, "_acceleration_plan", return_value=hook.AccelerationPlan("workspace", "fixture")), \
                patch.object(hook, "native_lane_enabled", return_value=True), patch.object(hook, "sidecar_controls", return_value=[]), \
                patch.object(hook, "run", return_value=0) as run, patch("builtins.print"):
            self.assertEqual(hook.main(), 0)
        commands = [call.args[0] for call in run.call_args_list]
        self.assertTrue(any("unittest" in cmd for cmd in commands))
        self.assertTrue(any("fmt" in cmd for cmd in commands))
        tests = [cmd for cmd in commands if cmd[:2] == ["cargo", "test"]]
        self.assertEqual(len(tests), 1)
        self.assertIn("--workspace", tests[0])
        self.assertIn("entmoot/producer", tests[0][tests[0].index("--features") + 1])
        self.assertFalse(any(cmd[:2] == ["cargo", "build"] for cmd in commands),
                         "CLI integration tests already build both native executables")

    def test_only_actual_cli_integration_builds_can_replace_the_binary_build(self):
        from scripts.pre_commit_check import selects_integration_tests
        for command, expected in [
            (["cargo", "test", "--workspace"], True),
            (["cargo", "test", "-p", "entmoot", "--test", "help"], True),
            (["cargo", "test", "-p", "entmoot-engine"], False),
            (["cargo", "test", "-p", "entmoot", "--lib"], False),
            (["cargo", "test", "-p", "entmoot", "--doc"], False),
            (["cargo", "test", "-p", "entmoot", "--bin", "entmoot"], False),
            (["cargo", "test", "-p", "entmoot", "--bins"], False),
            (["cargo", "test", "-p", "entmoot", "--examples"], False),
            (["cargo", "test", "-p", "entmoot", "--tests"], True),
            (["cargo", "test", "-p", "entmoot", "--lib", "--all-targets"], True),
            (["cargo", "test", "--workspace", "--exclude", "entmoot"], False),
        ]:
            with self.subTest(command=command):
                self.assertEqual(selects_integration_tests([command], "entmoot"), expected)

    def test_validation_and_eval_use_same_isolated_target(self) -> None:
        from scripts.pre_commit_check import validation_environment, ROOT
        from scripts.cargo_paths import debug_directory

        env = validation_environment({'CARGO_TARGET_DIR': str(ROOT / 'target')})
        self.assertEqual(Path(env['CARGO_TARGET_DIR']), ROOT / 'target/commit-check')
        with patch.dict(os.environ, env), patch.object(Path, 'is_file', return_value=True):
            self.assertEqual(debug_directory(), ROOT / 'target/commit-check/debug')

    def test_windows_validation_bounds_default_linker_concurrency_and_preserves_override(self):
        from scripts.pre_commit_check import validation_environment

        # Replace the consumer's platform view, not Python's shared sys module:
        # shutil.which must keep using the actual host's platform APIs.
        with patch('scripts.pre_commit_check.sys', SimpleNamespace(platform='win32')), \
                patch('scripts.pre_commit_check.os.cpu_count', return_value=64):
            self.assertEqual(validation_environment({}).get('CARGO_BUILD_JOBS'), '6')
            self.assertEqual(validation_environment({'CARGO_BUILD_JOBS': '12'})['CARGO_BUILD_JOBS'], '12')
        with patch('scripts.pre_commit_check.sys', SimpleNamespace(platform='win32')), \
                patch('scripts.pre_commit_check.os.cpu_count', return_value=2):
            self.assertEqual(validation_environment({}).get('CARGO_BUILD_JOBS'), '2')
        with patch('scripts.pre_commit_check.sys', SimpleNamespace(platform='win32')), \
                patch('scripts.pre_commit_check.os.cpu_count', return_value=None):
            self.assertEqual(validation_environment({}).get('CARGO_BUILD_JOBS'), '1')
        with patch('scripts.pre_commit_check.sys', SimpleNamespace(platform='linux')):
            self.assertNotIn('CARGO_BUILD_JOBS', validation_environment({}))

    def test_child_git_commands_do_not_inherit_the_committing_index(self) -> None:
        from scripts.pre_commit_check import validation_environment

        source = {'GIT_DIR': '.git', 'GIT_COMMON_DIR': 'another/.git',
                  'GIT_INDEX_FILE': 'staged-index', 'GIT_PREFIX': 'subdir/',
                  'GIT_CONFIG_COUNT': '1', 'GIT_CONFIG_KEY_0': 'core.worktree',
                  'GIT_CONFIG_VALUE_0': 'wrong-worktree', 'PATH': 'tools',
                  'ENTMOOT_CAMPAIGN_PYTHON': 'selected-python'}
        clean = validation_environment(source)
        self.assertFalse(any(key.startswith('GIT_') for key in clean))
        self.assertEqual(clean['PATH'], 'tools')
        self.assertEqual(clean['ENTMOOT_CAMPAIGN_PYTHON'], 'selected-python')
        self.assertEqual(source['GIT_INDEX_FILE'], 'staged-index', 'keep staged-selection context')

    def test_campaign_python_uses_path_without_overriding_explicit_selection(self) -> None:
        from scripts.pre_commit_check import validation_environment

        with patch('scripts.pre_commit_check.shutil.which', return_value='tools/python'):
            self.assertEqual(validation_environment({})['ENTMOOT_CAMPAIGN_PYTHON'], 'tools/python')
            self.assertEqual(validation_environment({'ENTMOOT_CAMPAIGN_PYTHON': 'chosen'})
                             ['ENTMOOT_CAMPAIGN_PYTHON'], 'chosen')



class MarkdownLinkCheckTests(unittest.TestCase):
    @staticmethod
    def run_check(root: Path) -> int:
        with redirect_stdout(StringIO()), redirect_stderr(StringIO()):
            return check_local_markdown_links(root)

    def test_accepts_existing_relative_targets_and_external_links(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs").mkdir()
            (root / "target.md").write_text("# Target\n", encoding="utf-8")
            (root / "docs" / "source.md").write_text(
                "[local](../target.md) [anchor](#part) [web](https://example.com)\n",
                encoding="utf-8",
            )

            self.assertEqual(self.run_check(root), 0)

    def test_rejects_missing_relative_target(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.md").write_text("[missing](docs/nope.md)\n", encoding="utf-8")

            self.assertEqual(self.run_check(root), 1)

    def test_excludes_local_adapter_data_but_checks_repository_docs(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            vendor = root / ".entmoot" / "toolchains"
            vendor.mkdir(parents=True)
            (vendor / "README.md").write_text("[upstream](missing.md)\n", encoding="utf-8")
            self.assertEqual(self.run_check(root), 0)
            (root / "README.md").write_text("[broken](missing.md)\n", encoding="utf-8")
            self.assertEqual(self.run_check(root), 1)

    def test_excludes_isolated_python_dependencies_but_checks_eval_docs(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            for name in (".venv", "venv"):
                vendor = root / "evals" / name / "Lib"
                vendor.mkdir(parents=True)
                (vendor / "README.md").write_text("[upstream](missing.md)\n", encoding="utf-8")
            self.assertEqual(self.run_check(root), 0)
            (root / "evals" / "README.md").write_text("[broken](missing.md)\n", encoding="utf-8")
            self.assertEqual(self.run_check(root), 1)


class ManifestScanTests(unittest.TestCase):
    def test_excludes_local_adapter_manifests_but_checks_repository_manifests(self) -> None:
        from scripts.pre_commit_check import check_banned_patterns

        with TemporaryDirectory() as directory:
            root = Path(directory)
            vendor = root / ".entmoot" / "sources"
            vendor.mkdir(parents=True)
            (vendor / "Cargo.toml").write_text('[dependencies]\nsha2 = "0.10"\n', encoding="utf-8")
            with patch("scripts.pre_commit_check.ROOT", root), redirect_stdout(StringIO()), redirect_stderr(StringIO()):
                self.assertEqual(check_banned_patterns(), 0)
                (root / "Cargo.toml").write_text('[dependencies]\nsha2 = "0.10"\n', encoding="utf-8")
                self.assertEqual(check_banned_patterns(), 1)


class CrateBoundaryCheckTests(unittest.TestCase):
    @staticmethod
    def run_check(root: Path) -> int:
        with redirect_stdout(StringIO()), redirect_stderr(StringIO()):
            return check_crate_boundaries(root)

    def test_accepts_canonical_inward_dependency_graph(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates").mkdir()
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/entmoot-contracts", '
                '"crates/entmoot-core", "crates/entmoot-engine", '
                '"crates/entmoot-cli", "crates/entmoot-mcp"]\n',
                encoding="utf-8",
            )
            manifests = {
                "entmoot-contracts": "",
                "entmoot-core": 'entmoot-contracts = { path = "../entmoot-contracts" }',
                "entmoot-engine": (
                    'entmoot-contracts = { path = "../entmoot-contracts" }\n'
                    'entmoot-core = { path = "../entmoot-core" }'
                ),
                "entmoot-mcp": 'entmoot-engine = { path = "../entmoot-engine" }',
                "entmoot": 'entmoot-engine = { path = "../entmoot-engine" }',
            }
            for name, dependencies in manifests.items():
                crate = root / "crates" / ("entmoot-cli" if name == "entmoot" else name)
                crate.mkdir()
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\nversion = "1.0.0"\n'
                    f"[dependencies]\n{dependencies}\n",
                    encoding="utf-8",
                )
            self.assertEqual(self.run_check(root), 0)

    def test_rejects_contract_engine_dependency_and_retired_crate(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates" / "entmoot-contracts").mkdir(parents=True)
            (root / "crates" / "entmoot-query").mkdir()
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/entmoot-contracts"]\n',
                encoding="utf-8",
            )
            (root / "crates" / "entmoot-contracts" / "Cargo.toml").write_text(
                '[package]\nname = "entmoot-contracts"\nversion = "1.0.0"\n'
                '[dependencies]\nentmoot-engine = { path = "../entmoot-engine" }\n',
                encoding="utf-8",
            )
            self.assertEqual(self.run_check(root), 1)

if __name__ == "__main__":
    unittest.main()
