"""Enforce production dependency boundaries without compiling native providers."""
from pathlib import Path
import tomllib
import unittest
from scripts.crate_graph import packages, dependencies, closure as dependency_closure

ROOT = Path(__file__).resolve().parents[1]


class CrateArchitectureTests(unittest.TestCase):
    def manifest(self, name):
        return tomllib.loads((ROOT / "crates" / name / "Cargo.toml").read_text(encoding="utf-8"))

    def closure(self, name):
        return dependency_closure(name, packages()) - {name}

    def test_query_cannot_link_producers_or_orchestration(self):
        dependencies = self.closure("entmoot-query")
        self.assertFalse(dependencies & {
            "entmoot-engine", "entmoot-native", "entmoot-ops", "entmoot-workspace",
        })

    def test_query_tests_cannot_pull_in_producers(self):
        self.assertFalse(dependency_closure("entmoot-query", packages(), include_dev=True) & {"entmoot-native", "entmoot-ops", "entmoot-workspace", "entmoot-engine"})

    def test_topology_depends_only_on_structural_model(self):
        self.assertEqual(self.closure("entmoot-topology"), {
            "entmoot-contracts", "entmoot-core",
        })

    def test_store_cannot_call_queries_or_producers(self):
        self.assertFalse(self.closure("entmoot-store") & {
            "entmoot-engine", "entmoot-native", "entmoot-ops",
            "entmoot-workspace", "entmoot-query",
        })

    def test_vendor_dependencies_have_one_owner(self):
        owners = {"rusqlite": "entmoot-store", "gix": "entmoot-store",
                  "oxc": "entmoot-native", "cap-std": "entmoot-native"}
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        for member in workspace["workspace"]["members"]:
            manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text(encoding="utf-8"))
            for dependency in dependencies(manifest):
                owner = "entmoot-native" if dependency.startswith("ra_ap_") else owners.get(dependency)
                if owner:
                    self.assertEqual(manifest["package"]["name"], owner, dependency)


if __name__ == "__main__":
    unittest.main()
