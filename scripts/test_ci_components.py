"""Guard component coverage when workspace members or native targets change."""
from pathlib import Path
import re
import unittest

from scripts.crate_graph import dependencies, packages
from scripts.run_native_ci import SUITES, command

ROOT = Path(__file__).resolve().parents[1]


class ComponentCiTests(unittest.TestCase):
    def test_feature_guarded_test_targets_have_a_native_job(self):
        guard = re.compile(r'^#!?\[cfg\(feature = "(?:producer|resolved-oxc|resolved-rust-analyzer)"\)\]', re.MULTILINE)
        required = set()
        for package, (directory, _) in packages().items():
            for source in (directory / "tests").glob("*.rs"):
                if guard.search(source.read_text(encoding="utf-8")):
                    required.add((package, source.stem))
        covered = {(package, suite) for suite, (package, _, _) in SUITES.items()}
        self.assertLessEqual(required, covered)

    def test_structural_matrix_covers_every_workspace_member(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        matrix = re.search(r"package: \[([^\]]+)\]", workflow)[1]
        self.assertEqual({name.strip() for name in matrix.split(",")}, set(packages()))
        self.assertIn("cache-on-failure: true", workflow)
        self.assertIn("fail-fast: false", workflow)

    def test_native_suites_resolve_to_declared_cargo_targets(self):
        graph = packages()
        workflow = (ROOT / ".github/workflows/native.yml").read_text()
        matrix = {name.strip() for name in re.search(r"suite: \[([^\]]+)\]", workflow)[1].split(",")}
        self.assertEqual(matrix - {"historical_rust", "packages"}, set(SUITES))
        for suite, (package, _, _) in SUITES.items():
            directory, manifest = graph[package]
            source = directory / "src/lib.rs" if suite == "native_library" else directory / "tests" / f"{suite}.rs"
            self.assertTrue(source.is_file(), (package, suite))
            argv = command(suite)
            selected = set(argv[argv.index("--features") + 1].split(","))
            self.assertLessEqual(selected, set(manifest["features"]), suite)
            self.assertIn("--test-threads=1", argv)

    def test_transport_production_dependencies_use_workspace_facade(self):
        graph = packages()
        for name in ("entmoot", "entmoot-mcp"):
            deps = dependencies(graph[name][1])
            self.assertIn("entmoot-workspace", deps)
            self.assertNotIn("entmoot-engine", deps)


if __name__ == "__main__":
    unittest.main()
