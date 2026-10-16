"""Resolver-adapter catalog helpers for pre-commit and maintainer tooling.

Read-only discovery against `tools/resolver-adapters/catalog.toml`. Does not
install tools or replace `entmoot init` / `entmoot status` readiness.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = REPO_ROOT / "tools" / "resolver-adapters" / "catalog.toml"
ADAPTERS_ROOT_ENV = "ENTMOOT_ADAPTERS_ROOT"


def _entmoot_home() -> Path:
    home = os.environ.get("USERPROFILE") or os.environ.get("HOME") or str(Path.home())
    return Path(home) / ".entmoot"


def default_adapters_root() -> Path:
    override = os.environ.get(ADAPTERS_ROOT_ENV)
    if override:
        return Path(override)
    return _entmoot_home() / "adapters"


def load_catalog() -> list[dict]:
    data = tomllib.loads(CATALOG_PATH.read_text(encoding="utf-8"))
    return list(data.get("family", []))


def resolved_adapter(
    family: dict, environment: dict[str, str], root: Path = REPO_ROOT
) -> Path | None:
    """Read-only catalog discovery; even an empty explicit override is exclusive."""
    if family["env"] in environment:
        candidates = [Path(environment[family["env"]])]
    else:
        home = Path(environment.get(ADAPTERS_ROOT_ENV, str(default_adapters_root())))
        checkout_build = root / ".tmp" / "adapter-build" / "resolver-adapters"
        if family.get("artifact_kind") == "repo_relative":
            bases = [root / ".tmp" / "adapter-build", home, root]
        else:
            bases = [checkout_build, home]
        candidates = [
            base / relative
            for base in bases
            for relative in family.get("relative_artifacts", [])
        ]
    return next((path for path in candidates if path.is_file()), None)


def control_runtime_available(family: dict, environment: dict[str, str]) -> bool:
    """Bounded readiness only. The actual control establishes qualification."""
    name, override, home = {
        "go": ("go", None, None),
        "roslyn": ("dotnet", "ENTMOOT_DOTNET", "DOTNET_ROOT"),
        "java": ("java", "ENTMOOT_JAVA", "JAVA_HOME"),
        "clang": ("python", "ENTMOOT_PYTHON_EXECUTABLE", None),
        "typescript": ("node", "ENTMOOT_NODE", None),
        "pyright": ("node", "ENTMOOT_NODE", None),
    }[family["id"]]
    if override and override in environment:
        executable = environment[override]
    elif home and home in environment:
        executable = str(
            Path(environment[home])
            / ("" if home == "DOTNET_ROOT" else "bin")
            / (name + (".exe" if os.name == "nt" else ""))
        )
    else:
        executable = shutil.which(
            name, path=environment.get("PATH", environment.get("Path"))
        )
    if not executable or not Path(executable).is_file():
        return False
    if family["id"] == "clang":
        try:
            return (
                subprocess.run(
                    [
                        executable,
                        "-c",
                        "import clang.cindex; clang.cindex.Index.create()",
                    ],
                    env=environment,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=5,
                    check=False,
                ).returncode
                == 0
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
    return True
