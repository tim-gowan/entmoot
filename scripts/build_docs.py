"""Build versioned operator docs from owners and executable interfaces.

Public SSOT: crate rustdoc, live CLI help, MCP tool schemas and contract JSON
Schema. README is the main operator entry; ARCHITECTURE is the meaning walkthrough;
runbooks stay short procedures. AGENTS is maintainer guidance (still rendered);
llms.txt is not part of this site. No second Markdown tree for CLI/MCP/API.
Output is disposable.
"""
import argparse
import html
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from scripts.package_smoke import fixture_repository
STYLE = "body{font:17px/1.6 system-ui,sans-serif;max-width:1080px;margin:3rem auto;padding:0 1.5rem;color:#172334}a{color:#1457a4}pre{overflow:auto;padding:1rem;background:#eff3f8}code{font-size:.9em}table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccd5e0;padding:.6rem;text-align:left}nav{border-bottom:1px solid #ccd5e0;padding-bottom:1rem}img{max-width:100%}"

def page(title, body, home="index.html"):
    body, diagrams = re.subn(r'<pre><code class="language-mermaid">(.*?)</code></pre>',
                            r'<pre class="mermaid">\1</pre>', body, flags=re.S)
    # GitHub renders Mermaid in README; the static site uses the same authored
    # diagrams. Escaped diagram text remains readable when JavaScript is disabled.
    renderer = ('<script type="module">import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11.12.0/dist/mermaid.esm.min.mjs";'
                'mermaid.initialize({startOnLoad:true,securityLevel:"strict"});</script>') if diagrams else ''
    return f'<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>{html.escape(title)} — Entmoot</title><style>{STYLE}</style><nav><a href="{home}">Entmoot documentation</a></nav><main>{body}</main>{renderer}</html>'

def qualify_examples(cli):
    # Executable operator examples on a fresh, explicit source root.
    with tempfile.TemporaryDirectory(prefix="docs-example-", dir=ROOT / ".tmp") as directory:
        source = Path(directory) / "source"
        data = Path(directory) / "data"
        env = fixture_repository(source)
        (source / "app.py").write_text("def work(object):\n    object.toString()\n", encoding="utf-8")
        for command in (["find", "work"], ["boundaries", "--callee", "*.toString()"], ["status"], ["inventory"]):
            result = json.loads(subprocess.check_output(
                [str(cli), "--data-dir", str(data), "-C", str(source), "--json", *command], env=env))
            if command[0] == "find":
                subjects = result.get("subjects") or {}
                callables = [
                    entry
                    for entry in subjects.values()
                    if entry.get("kind") == "callable" and entry.get("display_ref") == "app.py::work"
                ]
                assert len(callables) == 1, result
                assert result["result_bounds"]["total_matches"] == 1, result
            elif command[0] == "boundaries":
                assert result["result_bounds"]["total_matches"] == 1, result

def main():
    import markdown

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--mcp", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    cli, mcp = args.cli.resolve(), args.mcp.resolve()
    destination = args.out.resolve() / ("v" + version)
    destination.mkdir(parents=True, exist_ok=True)
    owners = [ROOT / name for name in ("README.md", "AGENTS.md", "PLANNING.md", "CHANGELOG.md")]
    owners += sorted((ROOT / "docs").rglob("*.md"))
    pages = {p.resolve(): p.relative_to(ROOT).with_suffix(".html") for p in owners}
    for source, relative in pages.items():
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        rendered = markdown.markdown(source.read_text(encoding="utf-8"), extensions=["tables", "fenced_code", "toc"])
        def link(match):
            url = match.group(1)
            parsed = urlsplit(html.unescape(url))
            if parsed.scheme or url.startswith(("#", "//")):
                return match.group(0)
            local = (source.parent / parsed.path).resolve()
            if not local.exists():
                raise ValueError(f"Broken documentation link: {source}: {url}")
            if local in pages:
                import os
                href = Path(os.path.relpath(destination / pages[local], target.parent)).as_posix()
            else:
                kind = "tree" if local.is_dir() else "blob"
                href = f"https://github.com/tim-gowan/entmoot/{kind}/v" + version + "/" + local.relative_to(ROOT).as_posix()
            if parsed.fragment:
                href += "#" + parsed.fragment
            return 'href="' + html.escape(href, quote=True) + '"'
        rendered = re.sub(r'href="([^"]+)"', link, rendered)
        import os
        home = Path(os.path.relpath(destination / "index.html", target.parent)).as_posix()
        target.write_text(page(source.stem, rendered, home), encoding="utf-8")
    # Help is obtained from Clap, including nested subcommands, never hand-copied.
    commands = [()]
    help_sections = []
    for command in commands:
        help_text = subprocess.check_output([str(cli), *command, "--help"], text=True, encoding="utf-8")
        label = "entmoot " + " ".join(command)
        help_sections.append(f'<h2>{html.escape(label.strip())}</h2><pre>{html.escape(help_text)}</pre>')
        if "Commands:\n" in help_text:
            section = help_text.split("Commands:\n", 1)[1].split("\nOptions:", 1)[0]
            for name in re.findall(r"^  ([a-z][a-z0-9-]+) {2,}", section, re.M):
                if name != "help":
                    commands.append((*command, name))
    (destination / "cli.html").write_text(page("CLI reference", "".join(help_sections)), encoding="utf-8")
    tools = json.loads(subprocess.check_output([str(mcp), "--tools-schema"]))
    (destination / "mcp-tools.json").write_text(json.dumps(tools, indent=2) + "\n", encoding="utf-8")
    tool_body = '<h1>MCP tools</h1><p><a href="mcp-tools.json">Generated input schemas</a></p>'
    tool_body += "".join(f'<h2>{html.escape(tool["name"])}</h2><p>{html.escape(tool.get("description", ""))}</p>' for tool in tools)
    (destination / "mcp.html").write_text(page("MCP reference", tool_body), encoding="utf-8")
    schemas = destination / "schemas"
    schemas.mkdir(exist_ok=True)
    for source in (ROOT / "crates/entmoot-contracts/generated").glob("*.json"):
        shutil.copyfile(source, schemas / source.name)
    if not (ROOT / "target/doc/entmoot_engine/index.html").is_file():
        raise SystemExit("Run cargo doc --workspace --no-deps first")
    shutil.copytree(ROOT / "target/doc", destination / "api", dirs_exist_ok=True)
    index = f'<h1>Entmoot {html.escape(version)}</h1><p>Structural evidence, attributable native observations and bounded change impact.</p><ul>'
    for label, url in [
        ("README (main entry)", "README.html"),
        ("Architecture walkthrough", "docs/architecture/ARCHITECTURE.html"),
        ("Rust API (rustdoc)", "api/entmoot_engine/index.html"),
        ("CLI reference (live help)", "cli.html"),
        ("MCP tools (live schemas)", "mcp.html"),
        ("Procedures", "docs/README.html"),
        ("Release and upgrade guide", "docs/runbooks/local-alpha.html"),
    ]:
        index += f'<li><a href="{url}">{label}</a></li>'
    (destination / "index.html").write_text(page("Documentation", index + "</ul>"), encoding="utf-8")
    (destination.parent / "index.html").write_text(page("Versions", f'<h1>Entmoot documentation</h1><a href="v{version}/index.html">{version}</a>'), encoding="utf-8")
    qualify_examples(cli)
    print(f"Generated {len(pages)} owner pages, {len(commands)} CLI help sections and {len(tools)} MCP schemas at {destination}")

if __name__ == "__main__":
    main()
