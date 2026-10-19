"""Digest-bearing fixtures must survive Git's Windows checkout conversion."""
import json
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory
import unittest

import blake3


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = Path("crates/entmoot-contracts/tests/fixtures/artifacts")


class FixtureCheckoutTests(unittest.TestCase):
    def test_artifact_digests_survive_autocrlf_checkout(self):
        manifests = sorted((ROOT / FIXTURES).rglob("*.manifest.json"))
        self.assertTrue(manifests, "the checkout control must exercise real artifact fixtures")
        # Run a real checkout in a disposable repository, without inheriting the
        # committing index or the maintainer's Git filters/configuration.
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith("GIT_")}
        env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        with TemporaryDirectory() as directory:
            checkout = Path(directory)

            def git(*args):
                subprocess.run(["git", "-C", directory, *args], env=env, check=True,
                               capture_output=True, timeout=30)

            git("init")
            git("config", "core.autocrlf", "true")
            shutil.copyfile(ROOT / ".gitattributes", checkout / ".gitattributes")
            expected = []
            for manifest_path in manifests:
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
                relative = manifest_path.parent.relative_to(ROOT) / manifest["artifact_path"]
                artifact = checkout / relative
                artifact.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, artifact)
                expected.append((artifact, manifest["content_digest"]))
            git("add", "--", ".")
            for artifact, _ in expected:
                artifact.unlink()
            git("checkout-index", "--all", "--force")
            for artifact, digest in expected:
                with self.subTest(artifact=artifact.relative_to(checkout)):
                    self.assertEqual("blake3:" + blake3.blake3(artifact.read_bytes()).hexdigest(), digest)


if __name__ == "__main__":
    unittest.main()
