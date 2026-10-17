"""Execute the docs parameter step with direct and reusable-workflow inputs."""
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]


class DocsWorkflowTests(unittest.TestCase):
    def resolve_parameters(self, event, inputs):
        # Keep the executable workflow as the owner; extract only this literal
        # run block, without interpreting YAML or reimplementing its defaults.
        workflow = (ROOT / ".github/workflows/docs.yml").read_text(encoding="utf-8")
        step = workflow.split("      - name: Resolve docs parameters\n", 1)[1]
        run = step.split("        run: |\n", 1)[1].split("      - ", 1)[0]
        script = textwrap.dedent(run)
        if os.name == "nt":
            git = shutil.which("git")
            self.assertIsNotNone(git, "workflow controls require Git for Windows")
            # Hooks prepend Git's internal exec-path; shells commonly use cmd.
            bash = next((directory / "bin/bash.exe" for directory in Path(git).parents
                         if (directory / "bin/bash.exe").is_file()), None)
        else:
            bash = shutil.which("bash")
        self.assertTrue(bash and Path(bash).is_file(), "workflow controls require Git Bash/bash")
        with TemporaryDirectory() as directory:
            output = Path(directory) / "outputs"
            env = {key: value for key, value in os.environ.items() if not key.startswith("IN_")}
            env.update(EVENT_NAME=event, GITHUB_OUTPUT=output.as_posix())
            env.update(inputs)
            result = subprocess.run([str(bash), "--noprofile", "--norc", "-c", script],
                                    cwd=directory, env=env, capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
            return dict(line.split("=", 1) for line in output.read_text(encoding="utf-8").splitlines())

    def test_release_call_uses_inputs_even_when_caller_event_is_push(self):
        self.assertEqual(self.resolve_parameters("push", {
            "IN_OUT": ".tmp/site", "IN_UPLOAD_PAGES": "true", "IN_DEPLOY": "true",
            "IN_STAGING": "false", "IN_ARTIFACT": "release-docs", "IN_CACHE": "docs-release",
        }), {
            "out_dir": ".tmp/site", "upload_pages": "true", "deploy": "true",
            "staging_artifact": "false", "artifact_name": "release-docs", "rust_cache_key": "docs-release",
        })

    def test_direct_push_and_pull_request_stage_without_deploying(self):
        for event in ("push", "pull_request"):
            with self.subTest(event=event):
                self.assertEqual(self.resolve_parameters(event, {}), {
                    "out_dir": ".tmp/staging", "upload_pages": "false", "deploy": "false",
                    "staging_artifact": "true", "artifact_name": "docs-staging", "rust_cache_key": "docs-ubuntu",
                })

    def test_explicit_false_input_is_not_replaced_by_true_default(self):
        self.assertEqual(self.resolve_parameters("workflow_dispatch", {
            "IN_STAGING": "false", "IN_UPLOAD_PAGES": "false", "IN_DEPLOY": "false",
        })["staging_artifact"], "false")


if __name__ == "__main__":
    unittest.main()
