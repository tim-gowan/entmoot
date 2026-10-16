"""Tests for bounded SARIF repository-path projection."""

from __future__ import annotations

import json
import os
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.parse import quote

from scripts.normalize_sarif_repo_paths import (
    POLICY_ID,
    POLICY_VERSION,
    normalize_document,
    normalize_uri,
)

FIXTURES = Path(__file__).resolve().parent / "fixtures" / "sarif_repo_path"


def _load_fixture(name: str) -> dict:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def _file_uri(path: Path) -> str:
    return Path(path).resolve().as_uri()


class NormalizeSarifRepoPathsTest(unittest.TestCase):
    def test_strips_only_the_exact_repository_directory(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            (root / "src").mkdir()
            (root / "src" / "app.py").write_text("x", encoding="utf-8")
            self.assertEqual(normalize_uri("repo/src/app.py", root), "src/app.py")

    def test_preserves_original_uri_and_policy(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            (root / "src").mkdir()
            (root / "src" / "app.py").write_text("x", encoding="utf-8")
            document = _load_fixture("relative_ok.sarif")
            self.assertEqual(normalize_document(document, root), 1)
            artifact = document["runs"][0]["results"][0]["locations"][0][
                "physicalLocation"
            ]["artifactLocation"]
            self.assertEqual(artifact["uri"], "src/app.py")
            self.assertEqual(
                artifact["properties"]["entmootOriginalUri"], "repo/src/app.py"
            )
            self.assertEqual(
                artifact["properties"]["entmootPathProjectionPolicy"], POLICY_ID
            )
            self.assertEqual(
                artifact["properties"]["entmootPathProjectionVersion"], POLICY_VERSION
            )
            self.assertEqual(document["properties"]["entmootPathProjectionPolicy"], POLICY_ID)
            self.assertEqual(document["properties"]["entmootPathProjectionChanged"], 1)

    def test_already_relative_and_idempotent_repeat(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            (root / "src").mkdir()
            (root / "src" / "app.py").write_text("x", encoding="utf-8")
            document = _load_fixture("already_relative.sarif")
            self.assertEqual(normalize_document(document, root), 0)
            self.assertEqual(
                document["runs"][0]["results"][0]["locations"][0]["physicalLocation"][
                    "artifactLocation"
                ]["uri"],
                "src/app.py",
            )
            first = _load_fixture("relative_ok.sarif")
            self.assertEqual(normalize_document(first, root), 1)
            original = first["runs"][0]["results"][0]["locations"][0][
                "physicalLocation"
            ]["artifactLocation"]["properties"]["entmootOriginalUri"]
            self.assertEqual(normalize_document(first, root), 0)
            artifact = first["runs"][0]["results"][0]["locations"][0][
                "physicalLocation"
            ]["artifactLocation"]
            self.assertEqual(artifact["uri"], "src/app.py")
            self.assertEqual(artifact["properties"]["entmootOriginalUri"], original)
            self.assertEqual(first["properties"]["entmootPathProjectionChanged"], 0)

    def test_rejects_escape(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            with self.assertRaises(ValueError) as ctx:
                normalize_uri("../outside.py", root)
            self.assertIn("unsafe relative artifact URI:", str(ctx.exception))

    def test_rejects_unrelated_absolute_root(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            other = Path(directory, "other")
            other.mkdir()
            target = other / "leak.py"
            target.write_text("x", encoding="utf-8")
            with self.assertRaises(ValueError) as ctx:
                normalize_uri(str(target.resolve()), root)
            self.assertIn("artifact URI is outside repository root:", str(ctx.exception))

    def test_posix_absolute_under_root(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            nested = root / "src" / "app.py"
            nested.parent.mkdir()
            nested.write_text("x", encoding="utf-8")
            absolute = nested.resolve().as_posix()
            if os.name == "nt":
                # Drive-letter absolute is covered separately; POSIX-style /c:/...
                # is not produced by Path on Windows.
                self.assertEqual(normalize_uri(str(nested.resolve()), root), "src/app.py")
            else:
                self.assertTrue(absolute.startswith("/"))
                self.assertEqual(normalize_uri(absolute, root), "src/app.py")

    def test_file_uri_posix_or_host(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            nested = root / "src" / "app.py"
            nested.parent.mkdir()
            nested.write_text("x", encoding="utf-8")
            uri = _file_uri(nested)
            self.assertEqual(normalize_uri(uri, root), "src/app.py")
            localhost = uri.replace("file:///", "file://localhost/", 1)
            if "file://localhost/" in localhost:
                self.assertEqual(normalize_uri(localhost, root), "src/app.py")
            document = _load_fixture("file_uri_posix.sarif")
            document["runs"][0]["results"][0]["locations"][0]["physicalLocation"][
                "artifactLocation"
            ]["uri"] = uri
            self.assertEqual(normalize_document(document, root), 1)
            artifact = document["runs"][0]["results"][0]["locations"][0][
                "physicalLocation"
            ]["artifactLocation"]
            self.assertEqual(artifact["uri"], "src/app.py")
            self.assertEqual(artifact["properties"]["entmootOriginalUri"], uri)

    def test_percent_encoded_file_uri(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            nested = root / "src" / "my app.py"
            nested.parent.mkdir()
            nested.write_text("x", encoding="utf-8")
            # Build a percent-encoded file URI without relying on as_uri alone.
            resolved = nested.resolve()
            if os.name == "nt":
                drive = resolved.drive  # e.g. 'C:'
                rest = resolved.as_posix()[len(drive) :]
                encoded = "file:///" + drive + quote(rest, safe="/")
            else:
                encoded = "file://" + quote(resolved.as_posix(), safe="/")
            document = _load_fixture("percent_encoded.sarif")
            document["runs"][0]["results"][0]["locations"][0]["physicalLocation"][
                "artifactLocation"
            ]["uri"] = encoded
            self.assertEqual(normalize_document(document, root), 1)
            self.assertEqual(
                document["runs"][0]["results"][0]["locations"][0]["physicalLocation"][
                    "artifactLocation"
                ]["uri"],
                "src/my app.py",
            )

    def test_uri_base_srcroot_placeholder(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            (root / "src").mkdir()
            (root / "src" / "app.py").write_text("x", encoding="utf-8")
            document = _load_fixture("uri_base_srcroot.sarif")
            self.assertEqual(normalize_document(document, root), 1)
            artifact = document["runs"][0]["results"][0]["locations"][0][
                "physicalLocation"
            ]["artifactLocation"]
            self.assertEqual(artifact["uri"], "src/app.py")
            self.assertNotIn("uriBaseId", artifact)
            self.assertEqual(
                artifact["properties"]["entmootOriginalUriBaseId"], "%SRCROOT%"
            )
            self.assertEqual(artifact["properties"]["entmootOriginalUri"], "src/app.py")

    def test_uri_base_explicit_file_root(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            (root / "src").mkdir()
            (root / "src" / "app.py").write_text("x", encoding="utf-8")
            document = {
                "runs": [
                    {
                        "originalUriBaseIds": {
                            "ROOT": {"uri": _file_uri(root).rstrip("/") + "/"}
                        },
                        "results": [
                            {
                                "locations": [
                                    {
                                        "physicalLocation": {
                                            "artifactLocation": {
                                                "uri": "src/app.py",
                                                "uriBaseId": "ROOT",
                                            }
                                        }
                                    }
                                ]
                            }
                        ],
                    }
                ]
            }
            self.assertEqual(normalize_document(document, root), 1)
            artifact = document["runs"][0]["results"][0]["locations"][0][
                "physicalLocation"
            ]["artifactLocation"]
            self.assertEqual(artifact["uri"], "src/app.py")
            self.assertEqual(artifact["properties"]["entmootOriginalUriBaseId"], "ROOT")

    def test_rejects_unsupported_schemes_and_authorities(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            for uri, prefix in [
                ("https://example.com/a.py", "unsupported artifact URI scheme:"),
                ("git://example.com/a.py", "unsupported artifact URI scheme:"),
                ("data:text/plain,hi", "unsupported artifact URI scheme:"),
                (
                    "file://remote-host/share/a.py",
                    "unsupported file URI authority:",
                ),
            ]:
                with self.assertRaises(ValueError, msg=uri) as ctx:
                    normalize_uri(uri, root)
                self.assertIn(prefix, str(ctx.exception), uri)

    def test_windows_absolute_behavior(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            (root / "src").mkdir()
            (root / "src" / "app.py").write_text("x", encoding="utf-8")
            fake_windows = r"D:\unrelated\app.py"
            if os.name != "nt":
                with self.assertRaises(ValueError) as ctx:
                    normalize_uri(fake_windows, root)
                self.assertIn(
                    "windows absolute path unprovable on this host:", str(ctx.exception)
                )
                with self.assertRaises(ValueError) as ctx:
                    normalize_uri("file:///D:/unrelated/app.py", root)
                self.assertIn(
                    "windows absolute path unprovable on this host:", str(ctx.exception)
                )
                return

            nested = (root / "src" / "app.py").resolve()
            self.assertEqual(normalize_uri(str(nested), root), "src/app.py")
            self.assertEqual(normalize_uri(nested.as_uri(), root), "src/app.py")
            other_drive = None
            root_drive = nested.drive.casefold()
            for letter in "ABCDEFGHIJKLMNOPQRSTUVWXYZ":
                candidate = f"{letter}:\\nope\\app.py"
                if f"{letter}:".casefold() != root_drive:
                    other_drive = candidate
                    break
            assert other_drive is not None
            with self.assertRaises(ValueError) as ctx:
                normalize_uri(other_drive, root)
            self.assertIn("artifact URI is outside repository root:", str(ctx.exception))

    def test_symlink_escape_when_supported(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory, "repo")
            root.mkdir()
            outside = Path(directory, "outside")
            outside.mkdir()
            target = outside / "secret.py"
            target.write_text("x", encoding="utf-8")
            link = root / "leak.py"
            try:
                link.symlink_to(target)
            except OSError:
                self.skipTest("symlinks unavailable on this host")
            with self.assertRaises(ValueError) as ctx:
                normalize_uri("leak.py", root)
            self.assertIn("artifact URI escapes repository root:", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
