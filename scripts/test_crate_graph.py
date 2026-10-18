import tempfile
from pathlib import Path
import unittest

from scripts.crate_graph import cache_key, closure, packages, publication_order


class CrateGraphTests(unittest.TestCase):
    def test_upstream_invalidates_downstream_but_unrelated_crates_do_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            graph = {}
            for name, deps in (("model", {}), ("query", {"model": {}}), ("native", {"model": {}})):
                directory = root / name
                directory.mkdir()
                (directory / "lib.rs").write_text(name)
                graph[name] = (directory, {"dependencies": deps})
            before = cache_key("query", graph, root)
            (root / "native/lib.rs").write_text("native changed")
            self.assertEqual(before, cache_key("query", graph, root))
            (root / "model/lib.rs").write_text("model changed")
            self.assertNotEqual(before, cache_key("query", graph, root))
            self.assertEqual(closure("query", graph), {"query", "model"})

    def test_build_target_and_dev_dependencies_are_in_cache_closure(self):
        graph = {name: (Path(name), {}) for name in ("a", "b", "c", "d")}
        graph["a"][1].update({"build-dependencies": {"b": {}}, "dev-dependencies": {"c": {}}, "target": {"cfg(windows)": {"dependencies": {"alias": {"package": "d"}}}}})
        self.assertEqual(closure("a", graph, True), set(graph))
        self.assertEqual(closure("a", graph), {"a", "b", "d"})
        order = publication_order(graph)
        self.assertLess(order.index("b"), order.index("a"))
        self.assertLess(order.index("d"), order.index("a"))

    def test_production_cycles_fail_explicitly(self):
        graph = {"a": (Path("a"), {"dependencies": {"b": {}}}), "b": (Path("b"), {"dependencies": {"a": {}}})}
        with self.assertRaisesRegex(ValueError, "cycle"):
            publication_order(graph)

    def test_opaque_inputs_do_not_normalize_away_changed_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            directory = root / "model"
            directory.mkdir()
            artifact = directory / "grammar.packet"
            artifact.write_bytes(b"\x00header\r\npayload")
            graph = {"model": (directory, {})}
            before = cache_key("model", graph, root)
            artifact.write_bytes(b"\x00header\npayload")
            self.assertNotEqual(before, cache_key("model", graph, root))

    def test_inherited_renamed_dependency_invalidates_the_consumer(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text('[workspace]\nmembers=["model", "query"]\n[workspace.dependencies]\nalias={package="model", path="model"}\n')
            for name in ("model", "query"):
                directory = root / name
                directory.mkdir()
                extra = '\n[dependencies]\nalias.workspace=true\n' if name == "query" else ''
                (directory / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="1.0.0"\n{extra}')
                (directory / "lib.rs").write_text(name)
            graph = packages(root)
            self.assertEqual(closure("query", graph), {"query", "model"})
            before = cache_key("query", graph, root)
            (root / "model/lib.rs").write_text("changed upstream")
            self.assertNotEqual(before, cache_key("query", graph, root))


if __name__ == "__main__":
    unittest.main()
