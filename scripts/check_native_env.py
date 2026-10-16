#!/usr/bin/env python3
"""Optional host probe for Entmoot resolver adapters (no installs, no network).

Workspace SSOT is the home-scoped adapter profile from `entmoot init` (under
`~/.entmoot/workspaces/<key>/store/`); prefer `entmoot status`. This script does
not replace the profile and is not loaded by the CLI/MCP. Exit 0 when no hard
blockers; non-zero when a claimed family is hard-missing. Use --json for machine
receipts. See docs/runbooks/resolver-adapters.md and docs/runbooks/corpus-builds.md.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tomllib
import shutil
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = REPO_ROOT / "tools" / "resolver-adapters" / "catalog.toml"
ADAPTERS_ROOT_ENV = "ENTMOOT_ADAPTERS_ROOT"
DOTENV_PATH = REPO_ROOT / ".env"


def _entmoot_home() -> Path:
    home = os.environ.get("USERPROFILE") or os.environ.get("HOME") or str(Path.home())
    return Path(home) / ".entmoot"


def default_adapters_root() -> Path:
    override = os.environ.get(ADAPTERS_ROOT_ENV)
    if override:
        return Path(override)
    return _entmoot_home() / "adapters"


def load_repo_dotenv(path: Path = DOTENV_PATH) -> None:
    """Load repo-root `.env` into os.environ without overriding existing keys.

    Entmoot CLI/MCP do not read `.env`. Workspace SSOT is the home-scoped
    adapter profile from `entmoot init`; prefer `entmoot status`.
    """
    if not path.is_file():
        return
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if not key or key in os.environ:
            continue
        os.environ[key] = value


COMPDB_NAMES = (
    "compile_commands.json",
    "build/compile_commands.json",
    "out/compile_commands.json",
    "cmake-build-debug/compile_commands.json",
    "cmake-build-release/compile_commands.json",
)

@dataclass
class CheckResult:
    name: str
    status: str  # ok | missing | loud_limitation | info
    detail: str
    hard: bool = False


def _exists(path: str | Path | None) -> bool:
    if not path:
        return False
    return Path(path).exists()


def _first_existing(paths: list[str | Path]) -> Path | None:
    for p in paths:
        path = Path(p)
        if path.exists():
            return path
    return None


def load_catalog() -> list[dict]:
    data = tomllib.loads(CATALOG_PATH.read_text(encoding="utf-8"))
    return list(data.get("family", []))


def adapters_root() -> Path:
    return default_adapters_root()


def resolved_adapter(family: dict, environment: dict[str, str], root: Path = REPO_ROOT) -> Path | None:
    """Read-only catalog discovery; even an empty explicit override is exclusive."""
    if family["env"] in environment:
        candidates = [Path(environment[family["env"]])]
    else:
        home = Path(environment.get(ADAPTERS_ROOT_ENV, str(default_adapters_root())))
        checkout_build = root / ".tmp" / "adapter-build" / "resolver-adapters"
        bases = [checkout_build, home] if family.get("artifact_kind") != "repo_relative" else [
            root / ".tmp" / "adapter-build",
            home,
            root,
        ]
        candidates = [base / relative for base in bases for relative in family.get("relative_artifacts", [])]
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
        executable = str(Path(environment[home]) / ("" if home == "DOTNET_ROOT" else "bin") / (name + (".exe" if os.name == "nt" else "")))
    else:
        executable = shutil.which(name, path=environment.get("PATH", environment.get("Path")))
    if not executable or not Path(executable).is_file():
        return False
    if family["id"] == "clang":
        try:
            return subprocess.run([executable, "-c", "import clang.cindex; clang.cindex.Index.create()"], env=environment,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5, check=False).returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            return False
    return True


def check_adapter_env(family: dict) -> CheckResult:
    name = family["id"]
    env_var = family["env"]
    configured = os.environ.get(env_var)
    found = resolved_adapter(family, dict(os.environ))
    if configured is not None:
        if found:
            return CheckResult(
                name=name,
                status="ok",
                detail=f"{env_var}={configured}",
                hard=False,
            )
        return CheckResult(
            name=name,
            status="missing",
            detail=f"{env_var} set but path missing: {configured}",
            hard=True,
        )
    if found:
        return CheckResult(
            name=name,
            status="ok",
            detail=f"{env_var} unset; catalog candidate exists: {found}",
            hard=False,
        )
    return CheckResult(
        name=name,
        status="missing",
        detail=(
            f"{env_var} unset and no catalog candidate under {adapters_root()} "
            f"(build with scripts/build_resolver_adapters.py)"
        ),
        hard=True,
    )


def _compdb_at(root: Path) -> Path | None:
    for rel in COMPDB_NAMES:
        candidate = root / rel
        if candidate.is_file():
            return candidate
    return None


def _private_checkouts(prefix: str) -> list[Path]:
    base = REPO_ROOT / ".tmp" / "evals" / "checkouts"
    if not base.is_dir():
        return []
    return sorted(
        p for p in base.iterdir() if p.is_dir() and p.name.startswith(prefix)
    )


def check_libclang() -> CheckResult:
    configured = os.environ.get("LIBCLANG_PATH")
    if configured:
        path = Path(configured)
        if path.is_file() or path.is_dir():
            return CheckResult(
                name="libclang",
                status="ok",
                detail=f"LIBCLANG_PATH={configured}",
            )
        return CheckResult(
            name="libclang",
            status="missing",
            detail=f"LIBCLANG_PATH set but missing: {configured}",
            hard=True,
        )
    which = shutil.which("libclang") or shutil.which("libclang.dll")
    if which:
        return CheckResult(
            name="libclang",
            status="loud_limitation",
            detail=(
                f"LIBCLANG_PATH unset; PATH hit {which} "
                "(export LIBCLANG_PATH before clang produce)"
            ),
            hard=False,
        )
    return CheckResult(
        name="libclang",
        status="missing",
        detail="missing build or dependency: LIBCLANG_PATH unset and libclang not on PATH",
        hard=True,
    )


def check_pyright_program(catalog: list[dict]) -> CheckResult:
    """Bind-path Pyright Program batch adapter."""
    pyright = next((f for f in catalog if f["id"] == "pyright"), None)
    if pyright is None:
        return CheckResult(
            name="pyright_program",
            status="missing",
            detail="pyright missing from tools/resolver-adapters/catalog.toml",
            hard=True,
        )
    adapter = check_adapter_env(pyright)
    # Soften: missing pyright is loud for optional bind family
    if adapter.status == "missing" and not os.environ.get(pyright["env"]):
        adapter = CheckResult(
            name=adapter.name,
            status="loud_limitation",
            detail=adapter.detail,
            hard=False,
        )
    env_configured = os.environ.get("ENTMOOT_PYTHON_ENVIRONMENT")
    env_candidates = [
        REPO_ROOT / ".tmp/adapter-build/pyright-program/python_environment.json",
        default_adapters_root() / "pyright-program" / "python_environment.json",
    ]
    if env_configured:
        if _exists(env_configured):
            env_ok = CheckResult(
                name="python_environment",
                status="ok",
                detail=f"ENTMOOT_PYTHON_ENVIRONMENT={env_configured}",
            )
        else:
            env_ok = CheckResult(
                name="python_environment",
                status="missing",
                detail=f"ENTMOOT_PYTHON_ENVIRONMENT set but path missing: {env_configured}",
                hard=True,
            )
    else:
        found = _first_existing(env_candidates)
        if found:
            env_ok = CheckResult(
                name="python_environment",
                status="ok",
                detail=f"ENTMOOT_PYTHON_ENVIRONMENT unset; candidate exists: {found}",
            )
        else:
            env_ok = CheckResult(
                name="python_environment",
                status="loud_limitation",
                detail=(
                    "missing build or dependency: ENTMOOT_PYTHON_ENVIRONMENT unset "
                    "and no python_environment.json under adapter build/home; "
                    "write explicit JSON then bind"
                ),
                hard=False,
            )
    if adapter.status == "ok" and env_ok.status == "ok":
        return CheckResult(
            name="pyright_program",
            status="ok",
            detail=f"{adapter.detail}; {env_ok.detail}",
        )
    return CheckResult(
        name="pyright_program",
        status="loud_limitation",
        detail=f"{adapter.detail}; {env_ok.detail}",
        hard=False,
    )


def _compdb_kind(root: Path) -> str:
    marker = root / ".entmoot_compdb_kind"
    if marker.is_file():
        line = marker.read_text(encoding="utf-8").strip().splitlines()
        if line:
            return line[0].strip() or "unknown"
    src = root / "SOURCE_REVISION"
    if src.is_file():
        text = src.read_text(encoding="utf-8")
        if "kind=upstream" in text or "generator=cmake" in text or "generator=meson" in text:
            return "upstream"
        if "bootstrap" in text or "sparse_bootstrap" in text:
            return "bootstrap"
    return "unknown"


def _corpus_private_roots(corpus: str) -> list[Path]:
    roots = list(_private_checkouts(f"{corpus}-"))
    if corpus == "curl":
        roots.extend(_private_checkouts("curl-cmake-"))
    seen: set[Path] = set()
    out: list[Path] = []
    for root in roots:
        key = root.resolve()
        if key in seen:
            continue
        seen.add(key)
        out.append(root)
    return out


def _prepare_script(corpus: str) -> str:
    return {
        "curl": "scripts/prepare_curl_compdb.py --docker",
        "postgres": "scripts/prepare_postgres_compdb.py",
        "openssl": "scripts/prepare_openssl_compdb.py",
    }.get(corpus, f"scripts/prepare_{corpus}_compdb.py")


def check_docker_desktop() -> CheckResult:
    """Docker Desktop readiness for Go sidecar and curl CompDB materialization."""
    docker = shutil_which("docker")
    if not docker:
        return CheckResult(
            name="docker_desktop",
            status="loud_limitation",
            detail="docker not on PATH; install Docker Desktop for Go sidecar / curl CompDB",
            hard=False,
        )
    try:
        import subprocess

        subprocess.run(
            ["docker", "info"],
            check=True,
            capture_output=True,
            timeout=30,
        )
    except Exception as exc:  # noqa: BLE001 — readiness only
        return CheckResult(
            name="docker_desktop",
            status="loud_limitation",
            detail=f"docker present but daemon unreachable ({exc}); start Docker Desktop",
            hard=False,
        )
    return CheckResult(
        name="docker_desktop",
        status="ok",
        detail=f"docker reachable ({docker})",
    )


def check_curl_docker_compdb() -> CheckResult:
    """Prefer upstream curl CompDB prepared via Docker (generator=docker marker)."""
    compose = (
        REPO_ROOT
        / "tools"
        / "resolver-adapters"
        / "clang"
        / "compdb"
        / "docker-compose.yml"
    )
    if not compose.is_file():
        return CheckResult(
            name="curl_docker_compdb",
            status="missing",
            detail=f"missing compose recipe: {compose}",
            hard=True,
        )
    for root in _corpus_private_roots("curl"):
        hit = _compdb_at(root)
        if not hit:
            continue
        src = root / "SOURCE_REVISION"
        kind = _compdb_kind(root)
        generator = ""
        if src.is_file():
            for line in src.read_text(encoding="utf-8").splitlines():
                if line.startswith("generator="):
                    generator = line.split("=", 1)[1].strip()
        if kind == "upstream" and generator == "docker":
            return CheckResult(
                name="curl_docker_compdb",
                status="ok",
                detail=f"Docker CompDB at {hit} (root={root})",
            )
        if kind == "upstream":
            return CheckResult(
                name="curl_docker_compdb",
                status="ok",
                detail=(
                    f"upstream CompDB at {hit} (generator={generator or 'unknown'}); "
                    "Docker recipe available via prepare_curl_compdb.py --docker"
                ),
            )
    return CheckResult(
        name="curl_docker_compdb",
        status="loud_limitation",
        detail=(
            "no private curl CompDB; run python scripts/prepare_curl_compdb.py --docker "
            f"(compose={compose})"
        ),
        hard=False,
    )


def shutil_which(name: str) -> str | None:
    import shutil

    return shutil.which(name)


def check_corpus_compdb(corpus: str, hard_if_missing: bool) -> list[CheckResult]:
    """Report bootstrap vs upstream CompDB for a corpus (separate rows)."""
    upstream: tuple[Path, Path] | None = None
    bootstrap: tuple[Path, Path] | None = None
    other: tuple[Path, Path, str] | None = None

    for root in _corpus_private_roots(corpus):
        hit = _compdb_at(root)
        if not hit:
            continue
        kind = _compdb_kind(root)
        if kind == "upstream" and upstream is None:
            upstream = (root, hit)
        elif kind == "bootstrap" and bootstrap is None:
            bootstrap = (root, hit)
        elif other is None:
            other = (root, hit, kind)

    rows: list[CheckResult] = []
    if upstream:
        rows.append(
            CheckResult(
                name=f"{corpus}_upstream_compdb",
                status="ok",
                detail=f"upstream CompDB at {upstream[1]}",
            )
        )
    else:
        rows.append(
            CheckResult(
                name=f"{corpus}_upstream_compdb",
                status="loud_limitation",
                detail=f"no upstream CompDB; run {_prepare_script(corpus)}",
                hard=False,
            )
        )

    if bootstrap:
        rows.append(
            CheckResult(
                name=f"{corpus}_bootstrap_compdb",
                status="ok",
                detail=f"bootstrap CompDB at {bootstrap[1]} (discovery-gate only)",
            )
        )
    elif other and other[2] != "upstream":
        rows.append(
            CheckResult(
                name=f"{corpus}_bootstrap_compdb",
                status="ok",
                detail=f"CompDB at {other[1]} (kind={other[2]})",
            )
        )
    elif upstream:
        rows.append(
            CheckResult(
                name=f"{corpus}_bootstrap_compdb",
                status="info",
                detail="upstream present; bootstrap gate-only path optional",
            )
        )
    else:
        if corpus == "curl":
            hint = (
                f"run scripts/materialize_curl_compdb.py (gate) or "
                f"{_prepare_script(corpus)} (upstream Docker)"
            )
            hard = hard_if_missing
            status = "missing" if hard_if_missing else "loud_limitation"
        else:
            hint = f"run {_prepare_script(corpus)}"
            hard = False
            status = "loud_limitation"
        rows.append(
            CheckResult(
                name=f"{corpus}_bootstrap_compdb",
                status=status,
                detail=(
                    f"missing build or dependency: no CompDB under "
                    f".tmp/evals/checkouts for {corpus}; {hint}"
                ),
                hard=hard,
            )
        )
    return rows


def run_checks() -> list[CheckResult]:
    catalog = load_catalog()
    sidecar_ids = {"go", "roslyn", "java", "clang"}
    results: list[CheckResult] = []
    results.append(
        CheckResult(
            name="adapters_root",
            status="info",
            detail=f"{ADAPTERS_ROOT_ENV} → {adapters_root()}",
        )
    )
    for family in catalog:
        if family["id"] in sidecar_ids:
            results.append(check_adapter_env(family))
    results.append(check_libclang())
    results.append(check_pyright_program(catalog))
    results.append(check_docker_desktop())
    results.append(check_curl_docker_compdb())
    typescript = next((f for f in catalog if f["id"] == "typescript"), None)
    if typescript:
        ts = check_adapter_env(typescript)
        if ts.status == "missing" and not os.environ.get(typescript["env"]):
            ts = CheckResult(
                name=ts.name,
                status="loud_limitation",
                detail=ts.detail,
                hard=False,
            )
        results.append(ts)
    results.extend(check_corpus_compdb("curl", hard_if_missing=True))
    results.extend(check_corpus_compdb("postgres", hard_if_missing=False))
    results.extend(check_corpus_compdb("openssl", hard_if_missing=False))
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true")
    parser.add_argument(
        "--no-dotenv",
        action="store_true",
        help="do not load repo-root .env (use only the ambient process environment)",
    )
    args = parser.parse_args()
    if not args.no_dotenv:
        load_repo_dotenv()
    if not CATALOG_PATH.is_file():
        print(f"missing catalog: {CATALOG_PATH}", file=sys.stderr)
        return 2
    results = run_checks()
    if args.json:
        print(json.dumps([asdict(r) for r in results], indent=2))
    else:
        for row in results:
            flag = " HARD" if row.hard else ""
            print(f"[{row.status}]{flag} {row.name}: {row.detail}")
    hard = [r for r in results if r.hard]
    return 1 if hard else 0


if __name__ == "__main__":
    sys.exit(main())
