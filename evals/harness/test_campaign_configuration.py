"""Campaign configuration and persistence controls; no Entmoot binary required."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest

from evals.harness.npm_lock_inventory import dependency_rows
from evals.harness.repository_campaign import native_inputs

ROOT = Path(__file__).resolve().parents[2]


class CampaignConfigurationTests(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.env = {key: value for key, value in os.environ.items()
                    if not key.upper().startswith("GIT_")}
        self.env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)

    def write(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value if isinstance(value, str) else json.dumps(value), encoding="utf-8")
        return path

    def invoke(self, module, *args, success=True):
        result = subprocess.run([sys.executable, "-X", "utf8", "-m", f"evals.harness.{module}",
                                 *map(str, args)], cwd=ROOT, env=self.env,
                                capture_output=True, text=True, timeout=30)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_sqlite_reopen_retry_context_fence_and_atomic_batch(self):
        db = self.root / "campaign.sqlite3"
        context = self.write("context.json", {"repository": "authored/repo", "revision": "a", "tool": "one"})
        def invoke(operation, *args, **kwargs):
            return self.invoke("revision_campaign", operation, "--db", db, "--context", context, *args, **kwargs)
        invoke("create")
        rows = self.write("rows.json", {"build": {"status": "failed", "reason": "missing tool"}})
        invoke("put", "--rows", rows)
        self.write("rows.json", {"build": {"status": "complete"}})
        invoke("put", "--rows", rows)
        self.write("context.json", '{\n "tool":"one", "revision":"a", "repository":"authored/repo"\n}')
        self.assertEqual(invoke("get", "--key", "build"), {"status": "complete"})
        connection = sqlite3.connect(db)
        try:
            connection.execute("CREATE TRIGGER refuse_bad BEFORE INSERT ON receipts WHEN NEW.receipt_key = 'bad' BEGIN SELECT RAISE(ABORT, 'authored failure'); END;")
            connection.commit()
            self.write("rows.json", {"a-first": {"value": 1}, "bad": {"value": 2}})
            invoke("put", "--rows", rows, success=False)
            self.assertIsNone(invoke("get", "--key", "a-first"))
            self.assertEqual(connection.execute("SELECT count(*) FROM receipts").fetchone()[0], 1)
        finally:
            connection.close()
        self.write("context.json", {"repository": "authored/repo", "revision": "b", "tool": "one"})
        invoke("get", "--key", "build", success=False)
        invoke("put", "--rows", rows, success=False)

    def test_foreign_sqlite_database_is_not_adopted_or_migrated(self):
        db = self.root / "foreign.sqlite3"
        context = self.write("context.json", {"run": "foreign control"})
        connection = sqlite3.connect(db)
        try:
            connection.executescript("CREATE TABLE owned_elsewhere(value TEXT); INSERT INTO owned_elsewhere VALUES ('keep');")
            for operation in ("create", "get"):
                self.invoke("revision_campaign", operation, "--db", db, "--context", context,
                            "--key", "missing", success=False)
            self.assertEqual(connection.execute("SELECT value FROM owned_elsewhere").fetchone()[0], "keep")
            self.assertEqual(connection.execute("SELECT count(*) FROM sqlite_master WHERE name='campaign_context'").fetchone()[0], 0)
        finally:
            connection.close()

    def test_direct_parent_selection_does_not_substitute_previous_selected_revision(self):
        def git(*args):
            result = subprocess.run(["git", "-C", str(self.root), *args], env=self.env,
                                    capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.strip()
        git("init", "--quiet")
        git("config", "user.name", "Campaign fixture")
        git("config", "user.email", "fixture@example.invalid")
        git("config", "core.hooksPath", "empty-hooks")
        revisions = []
        for value in (1, 2, 3):
            self.write("source.py", f"def value():\n    return {value}\n")
            git("add", ".")
            git("commit", "--no-gpg-sign", "--quiet", "-m", "authored change")
            revisions.append(git("rev-parse", "HEAD"))
        def compare(*args, **kwargs):
            return self.invoke("revision_campaign", "comparisons", "--repository", self.root, *args, **kwargs)
        parent = compare("--head", revisions[2])["comparisons"][0]
        self.assertEqual(parent["base_revision"], revisions[1])
        self.assertEqual(parent["relation"]["kind"], "direct_parent")
        window = compare("--window", revisions[0], revisions[2], "skip middle intentionally")["comparisons"][0]
        self.assertEqual(window["base_revision"], revisions[0])
        self.assertEqual(window["relation"]["kind"], "selected_window")
        for args in (("--head", revisions[0]), ("--head", "HEAD"), ("--head", revisions[2], "--head", revisions[2])):
            with self.subTest(args=args):
                compare(*args, success=False)

    def test_lock_inventory_preserves_versions_ambiguity_and_unsupported_scope(self):
        def locks(name, **kwargs):
            return self.invoke("repository_campaign", "locks", "--root", self.root, "--lock", name, **kwargs)
        self.write("package-lock.json", {"lockfileVersion": 3, "packages": {
            "": {"dependencies": {"lib": "1", "alias": "npm:lib@2"}, "devDependencies": {"tool": "3"}},
            "node_modules/lib": {"version": "1.0.0"}, "node_modules/alias": {"name": "lib", "version": "2.0.0"},
            "node_modules/tool": {"version": "3.0.0"}}})
        self.assertEqual(locks("package-lock.json")["packages"], ["pkg:npm/lib@1.0.0", "pkg:npm/lib@2.0.0"])
        self.write("Cargo.toml", "[package]\nname='authored'\nversion='1.0.0'\n")
        self.write("Cargo.lock", "version=4\n[[package]]\nname='authored'\nversion='1.0.0'\ndependencies=['lib']\n[[package]]\nname='lib'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\n[[package]]\nname='lib'\nversion='2.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\n")
        ambiguous = locks("Cargo.lock")
        self.assertEqual(ambiguous["packages"], [])
        self.assertIn("lib:ambiguous_or_missing_lock_entry", ambiguous["limitations"])
        self.write("uv.lock", "version=1\n[[package]]\nname='authored'\nversion='1.0.0'\nsource={editable='.'}\ndependencies=[{name='lib',version='1.0.0'},{name='lib',version='2.0.0'}]\n[[package]]\nname='lib'\nversion='1.0.0'\nsource={registry='https://pypi.org/simple'}\n[[package]]\nname='lib'\nversion='2.0.0'\nsource={registry='https://pypi.org/simple'}\n")
        self.assertEqual(locks("uv.lock")["packages"], ["pkg:pypi/lib@1.0.0", "pkg:pypi/lib@2.0.0"])
        self.assertEqual(locks("unavailable.lock")["limitations"], ["lock_unavailable:unavailable.lock"])
        self.write("unknown.lock", "")
        self.assertEqual(locks("unknown.lock")["limitations"], ["unsupported_lock_format:unknown.lock"])
        locks("../outside.lock", success=False)
        self.write("package-lock.json", {"lockfileVersion": 3, "packages": {"node_modules/lib": {"version": "1.0.0"}}})
        self.assertEqual(locks("package-lock.json")["limitations"], ["lock_root_unavailable"])
        self.write("go.mod", "module authored.test/root\ngo 1.24\nrequire (\n example.org/lib v1.2.3\n example.org/indirect v2.0.0 // indirect\n example.org/replaced v1.0.0\n)\nreplace example.org/replaced => ../local\n")
        go = locks("go.mod")
        self.assertEqual(go["packages"], ["pkg:golang/example.org/lib@v1.2.3"])
        self.assertIn("example.org/replaced:replacement_not_qualified", go["limitations"])
        self.write("packages.lock.json", {"version": 1, "dependencies": {
            "net8.0": {"Example": {"type": "Direct", "resolved": "1.0.0"}, "Hidden": {"type": "Transitive", "resolved": "2.0.0"}},
            "net9.0": {"Example": {"type": "Direct", "resolved": "2.0.0"}}}})
        self.assertEqual(locks("packages.lock.json")["packages"], ["pkg:nuget/example@1.0.0", "pkg:nuget/example@2.0.0"])

    def test_npm_locations_preserve_nested_instances_aliases_and_optional_precedence(self):
        rows = dependency_rows({"lockfileVersion": 3, "packages": {
            "": {"dependencies": {"a": "^1"}}, "node_modules/a": {"version": "1.2.0", "dependencies": {"b": "^2", "@x/c": "3"}},
            "node_modules/b": {"version": "1.0.0"}, "node_modules/a/node_modules/b": {"version": "2.4.0"},
            "node_modules/@x/c": {"version": "3.0.0"}}})
        self.assertEqual([(row["name"], row["target_path"], row["version"]) for row in rows], [
            ("a", "node_modules/a", "1.2.0"), ("@x/c", "node_modules/@x/c", "3.0.0"), ("b", "node_modules/a/node_modules/b", "2.4.0")])
        rows = dependency_rows({"lockfileVersion": 3, "packages": {"": {
            "dependencies": {"a": "1"}, "optionalDependencies": {"a": "2"},
            "peerDependencies": {"b": "^1"}, "peerDependenciesMeta": {"b": {"optional": True}}}}})
        self.assertEqual([(r["kind"], r["requested"], r["status"]) for r in rows], [("optional", "2", "missing"), ("optional_peer", "^1", "missing")])
        rows = dependency_rows({"lockfileVersion": 3, "packages": {"": {"dependencies": {"a": "*"}},
            "node_modules/a": {"link": True, "resolved": "packages/a"}, "packages/a": {"version": "1.0.0"}}})
        self.assertEqual(rows[0]["status"], "unsupported_link")
        rows = dependency_rows({"lockfileVersion": 3, "packages": {"": {"dependencies": {"alias": "npm:real@1"}},
            "node_modules/alias": {"name": "real", "version": "1.0.0"}}})
        self.assertEqual((rows[0]["name"], rows[0]["package_name"]), ("alias", "real"))
        for lock in ({"lockfileVersion": 1, "dependencies": {}}, {"lockfileVersion": 3, "packages": {"../escape": {}}}):
            with self.subTest(lock=lock), self.assertRaises(ValueError):
                dependency_rows(lock)

    def test_native_input_identity_fences_adapter_and_environment_contents(self):
        self.write("tools/resolver-adapters/catalog.toml", "[[family]]\nid='fixture'\nenv='ENTMOOT_FIXTURE_ADAPTER'\nrelative_artifacts=['fixture/adapter.dll']\n")
        adapter = self.write(".tmp/adapter-build/resolver-adapters/fixture/adapter.dll", "version one")
        profile = self.write("profile.json", {"python_version": "3.13"})
        env = {"ENTMOOT_PYTHON_ENVIRONMENT": str(profile)}
        original = native_inputs(env, self.root)
        self.assertNotEqual(original, native_inputs({}, self.root))
        self.write("profile.json", {"python_version": "3.14"})
        self.assertNotEqual(original, native_inputs(env, self.root))
        before = native_inputs(env, self.root)
        adapter.write_text("version two", encoding="utf-8")
        self.assertNotEqual(before, native_inputs(env, self.root))
        before = native_inputs(env, self.root)
        adapter.with_name("dependency.dll").write_text("new runtime dependency", encoding="utf-8")
        self.assertNotEqual(before, native_inputs(env, self.root))
        exclusive = {"ENTMOOT_FIXTURE_ADAPTER": str(self.root / "missing")}
        before = native_inputs(exclusive, self.root)
        adapter.write_text("unused fallback change", encoding="utf-8")
        self.assertEqual(before, native_inputs(exclusive, self.root))


if __name__ == "__main__":
    unittest.main()
