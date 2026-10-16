#!/usr/bin/env python3
"""Export a normalized contract taxonomy from generated JSON Schema catalogs.

Repo-local review utility (not a product CLI). Discovers wire types, property
keys, and enum/const variants for human and machine comparison. Scratch outputs
default under ``.tmp/contract-taxonomy/`` (gitignored).

```text
python scripts/export_contract_taxonomy.py
python scripts/export_contract_taxonomy.py --regen
python scripts/export_contract_taxonomy.py --dogfood-self
```

``--dogfood-self`` regenerates emit into scratch, diffs against checked-in
goldens, runs full ``entmoot init`` on ``crates/entmoot-contracts`` (native bind
when the CLI has producer features), captures ``status`` /
``assess compatibility`` / ``assess impact`` receipts plus MCP ``tools/list``
when available, and reports coverage in ``dogfood_compare.json``.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
GENERATED = ROOT / "crates" / "entmoot-contracts" / "generated"
CORE_SCHEMA = GENERATED / "entmoot_contracts.schema.json"
QUERY_SCHEMA = GENERATED / "entmoot_query_contracts.schema.json"
VOLATILE_KEYS = frozenset(
    {
        "publication",
        "provider_publication",
        "artifact_id",
        "record_id",
        "provider_pointer",
        "snapshot_digest",
        "content_digest",
        "observed_document_set_digest",
        "build_context_digest",
        "wall_seconds",
        "peak_rss_sampled_bytes",
        "graph",
        "store",
        "source_root",
        "input_key",
        "freshness_key",
    }
)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def sort_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: sort_value(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        items = [sort_value(item) for item in value]
        try:
            return sorted(items, key=lambda item: json.dumps(item, sort_keys=True))
        except TypeError:
            return items
    return value


def collect_enums(node: Any, out: set[str]) -> None:
    if isinstance(node, dict):
        if "enum" in node and isinstance(node["enum"], list):
            for item in node["enum"]:
                if isinstance(item, str):
                    out.add(item)
        if "const" in node and isinstance(node["const"], str):
            out.add(node["const"])
        for child in node.values():
            collect_enums(child, out)
    elif isinstance(node, list):
        for child in node:
            collect_enums(child, out)


def property_keys(node: dict[str, Any]) -> list[str]:
    props = node.get("properties")
    if isinstance(props, dict):
        return sorted(props)
    return []


def flatten_catalog(document: dict[str, Any], catalog: str) -> dict[str, Any]:
    types: dict[str, Any] = {}

    def visit(defs: dict[str, Any]) -> None:
        for name, schema in defs.items():
            if not isinstance(schema, dict):
                continue
            nested = schema.get("$defs")
            if isinstance(nested, dict):
                visit(nested)
            enums: set[str] = set()
            collect_enums(schema, enums)
            # Prefer the richest definition when the same name appears nested.
            prior = types.get(name)
            row = {
                "keys": property_keys(schema),
                "enums": sorted(enums),
                "required": sorted(schema.get("required") or []),
                "type": schema.get("type"),
            }
            if prior is None or len(row["enums"]) + len(row["keys"]) >= len(prior["enums"]) + len(
                prior["keys"]
            ):
                types[name] = row

    visit(document.get("$defs") or document.get("definitions") or {})
    return {
        "catalog": catalog,
        "id": document.get("$id"),
        "title": document.get("title"),
        "types": {name: types[name] for name in sorted(types)},
    }


def export_taxonomy(out: Path, core: dict[str, Any], query: dict[str, Any]) -> dict[str, Any]:
    taxonomy = sort_value(
        {
            "schema": "entmoot.contract_taxonomy.v1",
            "core": flatten_catalog(core, "entmoot_contracts"),
            "query": flatten_catalog(query, "entmoot_query_contracts"),
        }
    )
    write_json(out / "taxonomy.normalized.json", taxonomy)
    lines = [
        "# Contract taxonomy export",
        "",
        f"Core types: {len(taxonomy['core']['types'])}",
        f"Query types: {len(taxonomy['query']['types'])}",
        "",
        "## Highlighted stitch / judgment enums",
        "",
    ]
    for name in (
        "CompatibilityVerdict",
        "CompatibilityReason",
        "BoundaryKind",
        "PackageApiEcosystem",
        "ScopeCompleteness",
        "QueryStatus",
        "BindingImpactReason",
        "BindingImpactKind",
    ):
        for side in ("query", "core"):
            row = taxonomy[side]["types"].get(name)
            if row:
                lines.append(f"### {name} ({side})")
                lines.append("")
                if row["enums"]:
                    lines.append("- enums: " + ", ".join(f"`{item}`" for item in row["enums"]))
                if row["keys"]:
                    lines.append("- keys: " + ", ".join(f"`{item}`" for item in row["keys"]))
                lines.append("")
    (out / "taxonomy.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return taxonomy


def regen_schemas(scratch: Path) -> tuple[Path, Path]:
    scratch.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "CARGO_TARGET_DIR": str(scratch / "target")}
    for example, name in (
        ("emit_json_schema", "entmoot_contracts.schema.json"),
        ("emit_query_json_schema", "entmoot_query_contracts.schema.json"),
    ):
        subprocess.run(
            ["cargo", "run", "-q", "-p", "entmoot-contracts", "--example", example],
            cwd=ROOT,
            env=env,
            check=True,
        )
        # Examples write into the crate generated/ directory; copy for scratch compare.
        source = GENERATED / name
        dest = scratch / "emitted" / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(source.read_bytes())
    return scratch / "emitted" / CORE_SCHEMA.name, scratch / "emitted" / QUERY_SCHEMA.name


def compare_emit_to_golden(out: Path) -> dict[str, Any]:
    emitted_core, emitted_query = regen_schemas(out / "regen")
    report = {
        "core_equal": load_json(emitted_core) == load_json(CORE_SCHEMA),
        "query_equal": load_json(emitted_query) == load_json(QUERY_SCHEMA),
        "emitted_core": str(emitted_core.relative_to(ROOT)).replace("\\", "/"),
        "emitted_query": str(emitted_query.relative_to(ROOT)).replace("\\", "/"),
    }
    write_json(out / "emit_vs_golden.json", report)
    return report


def find_cli(explicit: Path | None) -> Path:
    if explicit:
        return explicit.resolve(strict=True)
    for candidate in (
        ROOT / "target" / "debug" / "entmoot.exe",
        ROOT / "target" / "debug" / "entmoot",
        ROOT / "target" / "release" / "entmoot.exe",
        ROOT / "target" / "release" / "entmoot",
    ):
        if candidate.is_file():
            return candidate
    raise SystemExit("entmoot CLI not found; pass --cli or build -p entmoot")


def run_cli(cli: Path, args: list[str], cwd: Path) -> Any:
    completed = subprocess.run(
        [str(cli), *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    )
    text = completed.stdout.strip()
    return json.loads(text) if text.startswith("{") or text.startswith("[") else text


def scrub(value: Any) -> Any:
    if isinstance(value, dict):
        cleaned = {}
        for key, child in value.items():
            if key in VOLATILE_KEYS:
                cleaned[key] = "<volatile>"
            else:
                cleaned[key] = scrub(child)
        return cleaned
    if isinstance(value, list):
        return [scrub(item) for item in value]
    if isinstance(value, str):
        text = value.replace("\\", "/")
        root = str(ROOT).replace("\\", "/")
        if text.startswith(root):
            return "<repo>/" + text[len(root) :].lstrip("/")
        if text.startswith("blake3:") or (len(text) == 64 and all(c in "0123456789abcdef" for c in text)):
            return "<digest>"
        return text
    return value


def observed_paths(value: Any, prefix: str = "") -> set[str]:
    paths: set[str] = set()
    if isinstance(value, dict):
        for key, child in value.items():
            path = f"{prefix}.{key}" if prefix else key
            paths.add(path)
            paths |= observed_paths(child, path)
    elif isinstance(value, list):
        for child in value[:3]:
            paths |= observed_paths(child, prefix + "[]")
    return paths


def schema_property_paths(
    schema: dict[str, Any],
    defs: dict[str, Any],
    prefix: str = "",
    seen: set[str] | None = None,
) -> set[str]:
    """Walk property paths, resolving nested schemars $defs/$ref scopes."""
    paths: set[str] = set()
    seen = seen if seen is not None else set()
    local_defs = dict(defs)
    nested = schema.get("$defs")
    if isinstance(nested, dict):
        local_defs.update(nested)

    def resolve(node: dict[str, Any]) -> dict[str, Any]:
        ref = node.get("$ref")
        if not isinstance(ref, str):
            return node
        name = ref.rsplit("/", 1)[-1]
        target = local_defs.get(name)
        return target if isinstance(target, dict) else node

    node = resolve(schema)
    nested = node.get("$defs")
    if isinstance(nested, dict):
        local_defs.update(nested)
    identity = f"{prefix}:{id(node)}"
    if identity in seen:
        return paths
    seen.add(identity)

    props = node.get("properties")
    if isinstance(props, dict):
        for key, child in props.items():
            path = f"{prefix}.{key}" if prefix else key
            paths.add(path)
            if isinstance(child, dict):
                paths |= schema_property_paths(child, local_defs, path, seen)
    for key in ("oneOf", "anyOf", "allOf"):
        options = node.get(key)
        if isinstance(options, list):
            for option in options:
                if isinstance(option, dict):
                    paths |= schema_property_paths(option, local_defs, prefix, seen)
    items = node.get("items")
    if isinstance(items, dict):
        paths |= schema_property_paths(items, local_defs, prefix + "[]", seen)
    return paths


def schema_enums_in(schema: dict[str, Any], defs: dict[str, Any]) -> set[str]:
    local = dict(defs)
    nested = schema.get("$defs")
    if isinstance(nested, dict):
        local.update(nested)
    ref = schema.get("$ref")
    if isinstance(ref, str):
        target = local.get(ref.rsplit("/", 1)[-1])
        if isinstance(target, dict):
            schema = target
            nested = schema.get("$defs")
            if isinstance(nested, dict):
                local.update(nested)
    enums: set[str] = set()
    collect_enums(schema, enums)
    return enums


def field_enums(
    root_schema: dict[str, Any], defs: dict[str, Any], field: str
) -> set[str]:
    """Collect allowed string consts/enums for a named property under root_schema."""
    local = dict(defs)
    nested = root_schema.get("$defs")
    if isinstance(nested, dict):
        local.update(nested)
    props = root_schema.get("properties") or {}
    node = props.get(field)
    if not isinstance(node, dict):
        # Nested under common wrappers (health, suggested_selectors, sites, …).
        found: set[str] = set()

        def walk(schema: dict[str, Any], scope: dict[str, Any]) -> None:
            scope = dict(scope)
            more = schema.get("$defs")
            if isinstance(more, dict):
                scope.update(more)
            ref = schema.get("$ref")
            if isinstance(ref, str):
                target = scope.get(ref.rsplit("/", 1)[-1])
                if isinstance(target, dict):
                    walk(target, scope)
                    return
            props_local = schema.get("properties") or {}
            if field in props_local and isinstance(props_local[field], dict):
                found.update(schema_enums_in(props_local[field], scope))
            for key in ("oneOf", "anyOf", "allOf"):
                for option in schema.get(key) or []:
                    if isinstance(option, dict):
                        walk(option, scope)
            items = schema.get("items")
            if isinstance(items, dict):
                walk(items, scope)

        walk(root_schema, local)
        return found
    return schema_enums_in(node, local)


def collect_string_leaves(value: Any, key: str | None = None) -> set[tuple[str, str]]:
    found: set[tuple[str, str]] = set()
    if isinstance(value, dict):
        for child_key, child in value.items():
            found |= collect_string_leaves(child, child_key)
    elif isinstance(value, list):
        for child in value:
            found |= collect_string_leaves(child, key)
    elif isinstance(value, str) and key in {
        "verdict",
        "reason",
        "boundary_kind",
        "source_state",
        "facet",
        "operation",
    }:
        found.add((key, value))
    return found


def find_mcp() -> Path | None:
    for candidate in (
        ROOT / "target" / "debug" / "entmoot-mcp.exe",
        ROOT / "target" / "debug" / "entmoot-mcp",
        ROOT / "target" / "release" / "entmoot-mcp.exe",
        ROOT / "target" / "release" / "entmoot-mcp",
    ):
        if candidate.is_file():
            return candidate
    return None


def mcp_tools_list(mcp: Path, cwd: Path, log: Path) -> list[dict[str, Any]]:
    import queue
    import threading

    log.parent.mkdir(parents=True, exist_ok=True)
    process = subprocess.Popen(
        [str(mcp)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=log.open("wb"),
        text=True,
        encoding="utf-8",
        cwd=cwd,
    )
    messages: queue.Queue = queue.Queue()

    def read() -> None:
        assert process.stdout is not None
        for line in process.stdout:
            messages.put(json.loads(line))
        messages.put({"transport_closed": process.poll()})

    threading.Thread(target=read, daemon=True).start()
    serial = 0

    def send(value: dict[str, Any]) -> None:
        assert process.stdin is not None
        process.stdin.write(json.dumps(value) + "\n")
        process.stdin.flush()

    def rpc(method: str, params: dict[str, Any] | None = None) -> Any:
        nonlocal serial
        serial += 1
        send({"jsonrpc": "2.0", "id": serial, "method": method, "params": params or {}})
        while True:
            result = messages.get(timeout=120)
            if "transport_closed" in result:
                raise RuntimeError(result)
            if result.get("id") == serial:
                if "error" in result:
                    raise RuntimeError(result["error"])
                return result["result"]

    try:
        rpc(
            "initialize",
            {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "contract-taxonomy", "version": "1"},
            },
        )
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        tools = rpc("tools/list").get("tools") or []
        return tools
    finally:
        if process.stdin:
            process.stdin.close()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def dogfood_self(out: Path, cli: Path, source_root: Path, taxonomy: dict[str, Any]) -> dict[str, Any]:
    emit_report = compare_emit_to_golden(out)
    self_dir = out / "self"
    self_dir.mkdir(parents=True, exist_ok=True)
    # Full init binds available producers (Rust on this crate). Writes .entmoot under source_root.
    init = run_cli(
        cli,
        [
            "init",
            "--root",
            str(source_root),
            "--repo",
            "local/entmoot-contracts",
        ],
        cwd=ROOT,
    )
    write_json(self_dir / "init.json", scrub(init))
    status = run_cli(cli, ["status"], cwd=source_root)
    write_json(self_dir / "status.normalized.json", scrub(status))
    compatibility = run_cli(cli, ["assess", "compatibility"], cwd=source_root)
    write_json(self_dir / "compatibility.normalized.json", scrub(compatibility))
    impact = run_cli(cli, ["assess", "impact"], cwd=source_root)
    write_json(self_dir / "impact.normalized.json", scrub(impact))

    mcp_report: dict[str, Any] | None = None
    mcp = find_mcp()
    if mcp is not None:
        tools = mcp_tools_list(mcp, source_root, self_dir / "mcp.stderr")
        names = sorted(tool.get("name") for tool in tools if isinstance(tool, dict))
        write_json(self_dir / "mcp_tools.normalized.json", scrub({"tools": tools}))
        expected_read_tools = {
            "status",
            "find",
            "inspect",
            "read_source",
            "query",
            "traverse",
            "callers",
            "callees",
            "neighborhood",
            "trace_seeded_paths",
            "impact",
            "compatibility",
        }
        mcp_report = {
            "binary": str(mcp.relative_to(ROOT)).replace("\\", "/")
            if mcp.is_relative_to(ROOT)
            else str(mcp),
            "tool_count": len(names),
            "tools": names,
            "missing_read_tools": sorted(expected_read_tools - set(names)),
            "extra_tools": sorted(set(names) - expected_read_tools),
        }

    query_schema = load_json(QUERY_SCHEMA)
    top_defs = query_schema.get("$defs") or {}
    status_schema = top_defs.get("StatusReport") or {}
    compat_schema = top_defs.get("CompatibilityResult") or {}
    impact_schema = top_defs.get("RevisionImpactManifest") or {}
    expected_status = schema_property_paths(status_schema, top_defs)
    expected_compat = schema_property_paths(compat_schema, top_defs)
    expected_impact = schema_property_paths(impact_schema, top_defs)
    observed_status = observed_paths(status)
    observed_compat = observed_paths(compatibility)
    observed_impact = observed_paths(impact)

    def coverage(observed: set[str], expected: set[str]) -> dict[str, Any]:
        unknown = []
        for path in sorted(observed):
            if path in expected:
                continue
            if any(
                path == item
                or path.startswith(item + ".")
                or path.startswith(item + "[]")
                or (item.endswith("[]") and path.startswith(item[:-2] + "[]"))
                for item in expected
            ):
                continue
            unknown.append(path)
        return {
            "observed": len(observed),
            "expected": len(expected),
            "unknown_paths": unknown,
            "unused_top_level": sorted(
                path
                for path in expected
                if "." not in path and "[]" not in path and path not in observed
            ),
        }

    def enum_rows(receipt: str, value: Any, schema: dict[str, Any]) -> list[dict[str, Any]]:
        rows = []
        exact = {
            "source_state": "SourceState",
            "boundary_kind": "BoundaryKind",
            "verdict": "CompatibilityVerdict" if receipt == "compatibility" else None,
        }
        for field, item in sorted(collect_string_leaves(value)):
            allowed = field_enums(schema, top_defs, field)
            if not allowed:
                type_name = exact.get(field)
                if type_name:
                    for side in ("query", "core"):
                        row = taxonomy[side]["types"].get(type_name)
                        if row:
                            allowed |= set(row["enums"])
                elif receipt == "compatibility" and field == "reason":
                    allowed |= set(
                        taxonomy["query"]["types"].get("CompatibilityReason", {}).get("enums")
                        or []
                    )
                elif receipt == "impact" and field == "kind":
                    allowed |= set(
                        taxonomy["query"]["types"].get("BindingImpactKind", {}).get("enums")
                        or []
                    )
                elif receipt == "impact" and field == "reason":
                    allowed |= set(
                        taxonomy["query"]["types"].get("BindingImpactReason", {}).get("enums")
                        or []
                    )
            rows.append(
                {
                    "receipt": receipt,
                    "field": field,
                    "value": item,
                    "allowed": (item in allowed) if allowed else None,
                    "catalog_size": len(allowed),
                }
            )
        return rows

    enum_check = (
        enum_rows("compatibility", compatibility, compat_schema)
        + enum_rows("status", status, status_schema)
        + enum_rows("impact", impact, impact_schema)
    )

    # Catalog enum exercise for stitch/impact highlights (observed vs declared).
    highlight_exercise = {}
    observed_values = {field: set() for field, _ in collect_string_leaves(compatibility)}
    for field, item in collect_string_leaves(compatibility):
        observed_values.setdefault(field, set()).add(item)
    for field, item in collect_string_leaves(impact):
        observed_values.setdefault(field, set()).add(item)
    for name in (
        "CompatibilityVerdict",
        "CompatibilityReason",
        "BoundaryKind",
        "BindingImpactKind",
        "BindingImpactReason",
    ):
        declared = set(taxonomy["query"]["types"].get(name, {}).get("enums") or [])
        # Map type name to receipt fields roughly.
        field = {
            "CompatibilityVerdict": "verdict",
            "CompatibilityReason": "reason",
            "BoundaryKind": "boundary_kind",
            "BindingImpactKind": "kind",
            "BindingImpactReason": "reason",
        }[name]
        seen = observed_values.get(field, set())
        # Binding impact reasons live under nested impact fields named reason/kind.
        if name.startswith("Binding"):
            seen = {
                item
                for key, item in collect_string_leaves(impact)
                if key in {"kind", "reason"} or key.endswith("_kind")
            }
            if name == "BindingImpactKind":
                seen = {item for key, item in collect_string_leaves(impact) if key == "kind"}
            if name == "BindingImpactReason":
                seen = {
                    item
                    for key, item in collect_string_leaves(impact)
                    if key == "reason" and item in declared
                }
        highlight_exercise[name] = {
            "declared": sorted(declared),
            "observed": sorted(seen & declared) if declared else sorted(seen),
            "missing": sorted(declared - seen),
        }

    status_cov = coverage(observed_status, expected_status)
    compat_cov = coverage(observed_compat, expected_compat)
    impact_cov = coverage(observed_impact, expected_impact)
    package_api = {
        "init_stages": scrub(init.get("stages") if isinstance(init, dict) else init),
        "compatibility_sites": len(compatibility.get("sites") or [])
        if isinstance(compatibility, dict)
        else 0,
        "compatibility_verdict": compatibility.get("verdict")
        if isinstance(compatibility, dict)
        else None,
        "limitations": compatibility.get("limitations")
        if isinstance(compatibility, dict)
        else None,
    }
    report = {
        "schema": "entmoot.contract_taxonomy_dogfood.v1",
        "source_root": str(source_root.relative_to(ROOT)).replace("\\", "/"),
        "mode": "full_bind",
        "emit_vs_golden": emit_report,
        "status_keys": status_cov,
        "compatibility_keys": compat_cov,
        "impact_keys": impact_cov,
        "enum_values": enum_check,
        "highlight_enum_exercise": highlight_exercise,
        "package_api": package_api,
        "mcp": mcp_report,
        "taxonomy_type_counts": {
            "core": len(taxonomy["core"]["types"]),
            "query": len(taxonomy["query"]["types"]),
        },
        "ok": bool(
            emit_report["core_equal"]
            and emit_report["query_equal"]
            and not status_cov["unknown_paths"]
            and not compat_cov["unknown_paths"]
            and not impact_cov["unknown_paths"]
            and all(row["allowed"] in (True, None) for row in enum_check)
            and (mcp_report is None or not mcp_report["missing_read_tools"])
        ),
    }
    write_json(out / "dogfood_compare.json", report)
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / ".tmp" / "contract-taxonomy",
        help="Scratch directory for normalized exports (default .tmp/contract-taxonomy)",
    )
    parser.add_argument(
        "--regen",
        action="store_true",
        help="Re-run emit_* schema examples before exporting (updates crate generated/)",
    )
    parser.add_argument(
        "--dogfood-self",
        action="store_true",
        help="Compare emit vs golden and receipts from full self init/bind (+ impact, MCP tools)",
    )
    parser.add_argument("--cli", type=Path, help="Path to entmoot binary")
    parser.add_argument(
        "--source-root",
        type=Path,
        default=ROOT / "crates" / "entmoot-contracts",
        help="Tree for structural self dogfood (default crates/entmoot-contracts)",
    )
    args = parser.parse_args()
    out = args.out if args.out.is_absolute() else (ROOT / args.out)
    out.mkdir(parents=True, exist_ok=True)

    if args.regen and not args.dogfood_self:
        regen_schemas(out / "regen")

    if not CORE_SCHEMA.is_file() or not QUERY_SCHEMA.is_file():
        raise SystemExit(f"missing schema catalogs under {GENERATED}")

    core = load_json(CORE_SCHEMA)
    query = load_json(QUERY_SCHEMA)
    write_json(out / "entmoot_contracts.schema.json", sort_value(core))
    write_json(out / "entmoot_query_contracts.schema.json", sort_value(query))
    taxonomy = export_taxonomy(out, core, query)
    print(json.dumps({"out": str(out), "core_types": len(taxonomy["core"]["types"]), "query_types": len(taxonomy["query"]["types"])}))

    if args.dogfood_self:
        report = dogfood_self(out, find_cli(args.cli), args.source_root.resolve(strict=True), taxonomy)
        print(json.dumps({"dogfood_ok": report["ok"], "report": str(out / "dogfood_compare.json")}))
        if not report["ok"]:
            sys.exit(1)


if __name__ == "__main__":
    main()
