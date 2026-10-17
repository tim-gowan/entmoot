"""Documentation rendering and example isolation do not require a Rust build."""
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

from scripts import build_docs


class DocumentationTests(unittest.TestCase):
    def test_examples_own_their_repository_and_data_lifetimes(self):
        with TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / ".tmp").mkdir()
            env = {key: value for key, value in os.environ.items() if not key.upper().startswith("GIT_")}
            env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
            subprocess.run(["git", "init", "--quiet", str(root)], env=env, check=True)
            git_output = subprocess.check_output
            sources, stores = set(), set()

            def cli(args, **kwargs):
                self.assertIn("--data-dir", args, "docs examples must never write to the caller's home store")
                source = Path(args[args.index("-C") + 1]).resolve()
                store = Path(args[args.index("--data-dir") + 1]).resolve()
                self.assertFalse(store.is_relative_to(source))
                actual = git_output(["git", "-C", str(source), "rev-parse", "--show-toplevel"],
                                    env=kwargs["env"], text=True).strip()
                self.assertEqual(Path(actual).resolve(), source)
                sources.add(source)
                stores.add(store)
                store.mkdir(exist_ok=True)
                (store / "sentinel").write_text("fixture", encoding="utf-8")
                return json.dumps({"subjects": {"f": {"kind": "callable", "display_ref": "app.py::work"}},
                                   "result_bounds": {"total_matches": 1}}).encode()

            with patch.object(build_docs, "ROOT", root), patch.object(build_docs.subprocess, "check_output", side_effect=cli):
                build_docs.qualify_examples(Path("unused"))
            self.assertEqual(len(sources), 1)
            self.assertEqual(len(stores), 1)
            self.assertTrue(all(not path.exists() for path in sources | stores))

    def test_mermaid_is_renderable_and_ordinary_code_remains_code(self):
        body = '<pre><code class="language-mermaid">flowchart LR\n A--&gt;B\n</code></pre><pre><code class="language-text">plain\n</code></pre>'
        page = build_docs.page("fixture", body)
        self.assertIn('<pre class="mermaid">flowchart LR', page)
        self.assertIn('<pre><code class="language-text">plain', page)
        self.assertEqual(page.count('mermaid.initialize('), 1)
        self.assertNotIn('mermaid.initialize(', build_docs.page("ordinary", "<p>text</p>"))


if __name__ == "__main__":
    unittest.main()
