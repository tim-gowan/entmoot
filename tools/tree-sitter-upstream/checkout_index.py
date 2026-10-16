"""Fetch pinned grammar/example remotes into gitignored scratch — no sibling host paths."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
INDEX_PATH = Path(__file__).resolve().parent / "checkouts.json"
SCRATCH = REPO / ".tmp" / "grammar-checkouts"


def load_index(path: Path | None = None) -> dict:
    return json.loads((path or INDEX_PATH).read_text(encoding="utf-8"))


def checkout_row(checkout_id: str, index: dict | None = None) -> dict:
    index = index or load_index()
    return next(entry for entry in index["checkouts"] if entry["id"] == checkout_id)


def _run(command: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def ensure_checkout(checkout_id: str, index: dict | None = None) -> Path:
    """Clone or update ``checkout_id`` under ``.tmp/grammar-checkouts/<id>`` at the pinned commit."""
    row = checkout_row(checkout_id, index)
    url = row.get("url") or row.get("upstream")
    revision = row["commit_sha"]
    if not url:
        raise ValueError(f"checkout {checkout_id!r} missing url/upstream")
    if len(revision) != 40:
        raise ValueError(f"checkout {checkout_id!r} commit_sha must be a full peeled commit id")

    SCRATCH.mkdir(parents=True, exist_ok=True)
    destination = SCRATCH / checkout_id
    if not (destination / ".git").is_dir():
        if destination.exists():
            raise FileNotFoundError(f"scratch path exists but is not a git checkout: {destination}")
        _run(["git", "clone", "--no-checkout", "--no-hardlinks", "--", url, str(destination)])

    def has_commit() -> bool:
        return (
            subprocess.run(
                ["git", "-C", str(destination), "cat-file", "-e", f"{revision}^{{commit}}"],
                capture_output=True,
            ).returncode
            == 0
        )

    if not has_commit():
        _run(["git", "-C", str(destination), "fetch", "--depth=1", "origin", revision])
    if not has_commit():
        raise FileNotFoundError(f"pinned commit {revision} unavailable for {checkout_id} from {url}")

    peeled = subprocess.check_output(
        ["git", "-C", str(destination), "rev-parse", f"{revision}^{{commit}}"],
        text=True,
    ).strip()
    if peeled != revision:
        raise ValueError(f"checkout {checkout_id!r} pin must be a peeled commit id, not a tag")

    _run(["git", "-C", str(destination), "checkout", "--detach", "--force", revision])
    _run(["git", "-C", str(destination), "reset", "--hard", revision])
    return destination.resolve()
