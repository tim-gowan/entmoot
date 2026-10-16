"""Protected CI publishing, followed by clean registry installation. Never called by local checks."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import urllib.error
import urllib.request
from package_smoke import fingerprint

def main():
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not os.environ.get("CARGO_REGISTRY_TOKEN"):
        raise SystemExit("Configure trusted publishing or the protected environment's scoped CARGO_REGISTRY_TOKEN")
    names = ["entmoot-contracts", "entmoot-core", "entmoot-engine", "entmoot", "entmoot-mcp"]
    for name in names:
        # A version is immutable: an already published version must be reconciled
        # by the maintainer, never silently treated as this build's success.
        subprocess.run(["cargo", "publish", "--locked", "-p", name], cwd=root, check=True)
        for attempt in range(30):
            request = urllib.request.Request(f"https://crates.io/api/v1/crates/{name}/{version}", headers={"User-Agent": "Entmoot release qualification (github.com/tim-gowan/entmoot)"})
            try:
                with urllib.request.urlopen(request, timeout=20) as response:
                    if json.load(response)["version"]["num"] == version:
                        break
            except urllib.error.HTTPError as error:
                if error.code != 404:
                    raise
            time.sleep(10)
        else:
            raise SystemExit(f"Published {name} but registry visibility timed out")
    with tempfile.TemporaryDirectory(prefix="entmoot-install-") as directory:
        env = dict(os.environ, CARGO_HOME=str(Path(directory) / "cargo"))
        env.pop("CARGO_REGISTRY_TOKEN", None)
        for name in ("entmoot", "entmoot-mcp"):
            subprocess.run(["cargo", "install", name, "--version", version, "--locked", "--root", directory], env=env, cwd=directory, check=True)
        subprocess.run([str(Path(directory) / "bin/entmoot"), "--version"], check=True)
        fingerprint(Path(directory) / "bin/entmoot", Path(directory) / "source")
        tools = json.loads(subprocess.check_output([str(Path(directory) / "bin/entmoot-mcp"), "--tools-schema"]))
        assert {"boundaries", "find", "read_source", "status"}.issubset({tool["name"] for tool in tools})

if __name__ == "__main__":
    main()
