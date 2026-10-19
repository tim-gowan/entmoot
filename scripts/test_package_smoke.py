"""Packaging controls must analyze their own source, even beneath a Git checkout."""
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tomllib
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

from scripts import package_smoke


class PackageSmokeTests(unittest.TestCase):
    def test_release_inputs_preserve_unchanged_files_and_reject_stale_check(self):
        from scripts import prepare_release_inputs
        with TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "package_fingerprint.rs"
            with patch.object(prepare_release_inputs, "ROOT", root), \
                    patch.object(prepare_release_inputs, "outputs", return_value=[(output, "// first\n")]), \
                    patch("sys.argv", ["prepare_release_inputs"]):
                prepare_release_inputs.main()
                os.utime(output, ns=(1_000_000_000, 1_000_000_000))
                before = output.stat().st_mtime_ns
                prepare_release_inputs.main()
                self.assertEqual(output.stat().st_mtime_ns, before,
                                 "identical generated Rust must not invalidate Cargo or need a write lock")
            with patch.object(prepare_release_inputs, "ROOT", root), \
                    patch.object(prepare_release_inputs, "outputs", return_value=[(output, "// changed\n")]), \
                    patch("sys.argv", ["prepare_release_inputs", "--check"]):
                with self.assertRaisesRegex(SystemExit, "Regenerate release inputs"):
                    prepare_release_inputs.main()
                self.assertEqual(output.read_text(), "// first\n")
            with patch.object(prepare_release_inputs, "ROOT", root), \
                    patch.object(prepare_release_inputs, "outputs", return_value=[(output, "// changed\n")]), \
                    patch("sys.argv", ["prepare_release_inputs"]):
                prepare_release_inputs.main()
                self.assertEqual(output.read_text(), "// changed\n")

    def test_prebuilt_baseline_still_builds_archives_and_rejects_fingerprint_drift(self):
        with TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "Cargo.toml").write_text('[workspace.package]\nversion="1.0.0"\n[profile.test]\nopt-level=1\n', encoding="utf-8")
            (root / "Cargo.lock").write_text("version=4\n", encoding="utf-8")
            target = root / "target"
            packages = target / "package"
            packages.mkdir(parents=True)
            source = root / "Cargo.toml"
            for name in package_smoke.publication_order(package_smoke.packages()):
                with tarfile.open(packages / f"{name}-1.0.0.crate", "w:gz") as archive:
                    archive.add(source, arcname=f"{name}-1.0.0/Cargo.toml")
            tools = [{"name": name} for name in ("boundaries", "find", "read_source", "status")]
            for actual, passes in (({"version": "same"}, True), ({"version": "different"}, False)):
                with self.subTest(passes=passes), \
                        patch.object(package_smoke, "ROOT", root), \
                        patch("sys.argv", ["package_smoke", "--target-dir", str(target), "--profile", "test", "--baseline-bin-dir", str(root / "qualified")]), \
                        patch.object(package_smoke, "run") as run, \
                        patch.object(package_smoke, "read", return_value=tools), \
                        patch.object(package_smoke, "fingerprint", side_effect=[{"version": "same"}, actual]):
                    if passes:
                        package_smoke.main()
                    else:
                        with self.assertRaises(AssertionError):
                            package_smoke.main()
                    builds = [call for call in run.call_args_list if call.args[0][:2] == ["cargo", "build"]]
                    self.assertEqual(len(builds), 1, "prebuilt input must not trigger another checkout build")
                    command, workspace = builds[0].args
                    self.assertNotEqual(workspace, root, "the one build must qualify extracted sources")
                    self.assertEqual(command[command.index("--profile") + 1], "test")
                    manifest = tomllib.loads((workspace / "Cargo.toml").read_text(encoding="utf-8"))
                    self.assertEqual(manifest["profile"]["test"]["opt-level"], 1)

    def test_extracted_workspace_keeps_profiles_without_checkout_dependencies(self):
        source = '''[workspace.dependencies]
internal = { path = "checkout-only" }
[profile.test]
opt-level = 1
[profile.test.package."some-dependency"]
opt-level = 2
[workspace.metadata]
secret = "not a build profile"
[profile.release]
lto = true
'''
        manifest = tomllib.loads(package_smoke.rehearsal_manifest(["example"], "1.0.0", source))
        self.assertEqual(manifest["profile"], tomllib.loads(source)["profile"])
        self.assertEqual(manifest["workspace"], {"resolver": "2", "members": ["example-1.0.0"]})
        self.assertEqual(manifest["patch"], {"crates-io": {"example": {"path": "example-1.0.0"}}})

    def test_fingerprint_fixture_is_a_separate_repository(self):
        with TemporaryDirectory() as directory:
            outer = Path(directory).resolve()
            env = {key: value for key, value in os.environ.items()
                   if not key.upper().startswith("GIT_")}
            env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
            subprocess.run(["git", "init", "--quiet", str(outer)], env=env, check=True)
            stage = outer / ".tmp" / "package-smoke"
            stage.mkdir(parents=True)
            source = stage / "baseline"

            def read(_binary, args, cwd, **_kwargs):
                root = subprocess.check_output(
                    ["git", "-C", str(cwd), "rev-parse", "--show-toplevel"],
                    env=env, text=True).strip()
                self.assertEqual(Path(root).resolve(), source,
                                 "package smoke escaped to the enclosing checkout")
                self.assertEqual(
                    (source / "app.py").read_text(encoding="utf-8"),
                    "def entmoot_smoke_fn(object):\n    object.toString()\n",
                )
                data = Path(args[args.index("--data-dir") + 1])
                store = data / "workspaces" / "fixture" / "store"
                store.mkdir(parents=True, exist_ok=True)
                graph = store / "graph.json"
                graph.write_text(json.dumps({"implementation": {"fixture": "matched"}}), encoding="utf-8")
                (store / "current.json").write_text(json.dumps({"current": {"graph": str(graph)}}), encoding="utf-8")
                return {"result_bounds": {"total_matches": 1}}

            with patch.object(package_smoke, "read", side_effect=read):
                self.assertEqual(package_smoke.fingerprint(Path("unused"), source), {"fixture": "matched"})


if __name__ == "__main__":
    unittest.main()
