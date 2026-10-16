"""Independent inventory and boundary accounting for production revision pairs."""
from __future__ import annotations
import json
import re
from evals.harness.contracts import ContractError
from evals.harness.wire_schema import validate_wire

DIGEST = re.compile(r"blake3:[0-9a-f]{64}\Z")


def verify_inventory_delta(base: dict, head: dict, impact: dict) -> None:
    """Independent set accounting catches omitted and invented inventory deltas."""
    left = {r["path"]: r["content_digest"] for r in base["snapshot_files"]}
    right = {r["path"]: r["content_digest"] for r in head["snapshot_files"]}
    expected = {}
    for path in left.keys() | right.keys():
        if left.get(path) == right.get(path):
            continue
        kind = "added" if path not in left else "removed" if path not in right else "modified"
        expected[path] = kind
    actual = {r["path"]: r["kind"] for r in impact["files"]}
    if expected != actual or len(actual) != len(impact["files"]):
        raise ContractError("file delta is incomplete or contains invented changes")
    for row in impact["files"]:
        if row.get("base_content_digest") != left.get(row["path"]) or row.get("head_content_digest") != right.get(row["path"]):
            raise ContractError("file digest attribution mismatch")
    nodes = [{n["logic_address"]: n for n in graph["nodes"] if n["kind"] == "callable"}
             for graph in (base, head)]
    expected_callables = {}
    for address in nodes[0].keys() | nodes[1].keys():
        a, b = nodes[0].get(address), nodes[1].get(address)
        if a is None:
            kind = "introduced"
        elif b is None:
            kind = "removed"
        else:
            body = a.get("structural_hash") != b.get("structural_hash")
            boundary = a.get("signature_hash") != b.get("signature_hash")
            if not body and not boundary:
                continue
            kind = "body_and_boundary_changed" if body and boundary else "body_changed" if body else "boundary_changed"
        expected_callables[address] = kind
    actual_callables = {r["occurrence"]["logic_address"]: r["kind"] for r in impact["callables"]}
    if actual_callables != expected_callables or len(actual_callables) != len(impact["callables"]):
        raise ContractError("callable delta is incomplete or contains invented changes")
    for row in impact["callables"]:
        for side, by_address in zip(("base", "head"), nodes):
            node = by_address.get(row["occurrence"]["logic_address"])
            if node and row["occurrence"]["address_id"] != node.get("address_id"):
                raise ContractError("callable occurrence address_id mismatch")
            if row.get(f"{side}_node_id") != (node["id"] if node else None):
                raise ContractError("callable occurrence attribution mismatch")
            for key in ("structural_hash", "signature_hash"):
                if row.get(f"{side}_{key}") != (node.get(key) if node else None):
                    raise ContractError("callable digest attribution mismatch")
    # Every available binding record must be retained, even if not comparable.
    for side, graph in (("base", base), ("head", head)):
        manifests = {item["artifact_id"]: item for item in graph.get("artifacts", [])}
        if len(manifests) != len(graph.get("artifacts", [])):
            raise ContractError("duplicate artifact identity in binding evidence")
        for record in graph.get("resolved_identities", []):
            if not DIGEST.fullmatch(record["resolved_hash"]) or not re.fullmatch(r"[0-9a-f]{64}", record["resolution_policy_id"]):
                raise ContractError("invalid resolved identity digest/policy encoding")
            validate_wire(record, "ResolvedIdentityRecord")
            source = record["binding"].get("source")
            if source is not None:
                reference = source["artifact"]
                manifest = manifests.get(reference["artifact_id"])
                if manifest is None or manifest.get("content_digest") != reference["content_digest"]:
                    raise ContractError("resolved identity artifact reference mismatch")
        expected_rows = sorted(json.dumps(r, sort_keys=True) for r in graph.get("resolved_identities", []))
        actual_rows = sorted(json.dumps(r, sort_keys=True) for row in impact["bindings"] for r in row[side])
        if expected_rows != actual_rows:
            raise ContractError(f"{side} binding evidence omitted or invented")
    addresses = set()
    for row in impact["bindings"]:
        if row["address_id"] in addresses or any(r["address_id"] != row["address_id"] for r in row["base"] + row["head"]):
            raise ContractError("binding occurrence attribution mismatch")
        addresses.add(row["address_id"])
        comparable = bool(row["base"] and row["head"]) and all(
            r["binding"]["completeness"] == "complete" for r in row["base"] + row["head"])
        values = []
        for side, graph in (("base", base), ("head", head)):
            artifacts = {r["artifact_id"]: r["artifact_series_id"] for r in graph.get("artifacts", [])}
            bindings = {}
            for r in row[side]:
                source = r["binding"].get("source", {})
                series = artifacts.get(source.get("artifact", {}).get("artifact_id"))
                producer = source.get("producer", {})
                key = (series, producer.get("producer_id"), producer.get("version"), r.get("resolution_policy_id"))
                if series is None or key in bindings:
                    comparable = False
                bindings[key] = r["resolved_hash"]
            values.append(bindings)
        comparable = comparable and values[0].keys() == values[1].keys() and base.get("policy_ids") == head.get("policy_ids")
        expected_kind = ("unchanged" if values[0] == values[1] else "changed") if comparable else "not_comparable"
        if row["kind"] != expected_kind:
            raise ContractError("incomplete bindings or producer/policy/hash comparison mismatch")
        if not comparable and not row.get("reasons"):
            raise ContractError("non-comparable bindings require reasons")


def verify_breaking_change_signals(base: dict, head: dict, impact: dict) -> None:
    """Independently reconstruct the complete boundary-review facet, failing on omission/invention."""
    facet = impact.get("breaking_change_signals")
    if not isinstance(facet, dict) or facet.get("derivation_policy") != "entmoot.impact.boundary_signals.v1":
        raise ContractError("missing or unsupported breaking-change signal facet")
    inventories = [{node["logic_address"]: node for node in graph["nodes"] if node["kind"] == "callable"}
                   for graph in (base, head)]
    failures = [{d.get("provider_pointer") for d in graph.get("diagnostics", [])
                 if d.get("code", "").startswith("source.")}
                for graph in (base, head)]
    policies = bool(base.get("policy_ids")) and base.get("policy_ids") == head.get("policy_ids")
    expected = {}
    for address, old in inventories[0].items():
        new = inventories[1].get(address)
        path = (new or old)["path"]
        if not policies or path in failures[0] or path in failures[1]:
            continue
        if not old.get("signature_hash"):
            continue
        if new is None:
            expected[address] = ("callable_removed", old, None)
        elif new.get("signature_hash") and old["signature_hash"] != new["signature_hash"]:
            expected[address] = ("boundary_changed", old, new)
    rows = facet.get("signals")
    if not isinstance(rows, list) or len(rows) != len(expected):
        raise ContractError("breaking-change signal set incomplete or invented")
    seen = []
    for signal in rows:
        row = signal["callable"]
        address = row["occurrence"]["logic_address"]
        if address not in expected or address in seen:
            raise ContractError("unexpected or duplicate breaking-change signal")
        seen.append(address)
        kind, old, new = expected[address]
        if signal.get("kind") != kind or not signal.get("summary"):
            raise ContractError("breaking-change signal reason missing or wrong")
        if row not in impact["callables"]:
            raise ContractError("signal lacks exact callable partition evidence")
        for side, node in (("base", old), ("head", new)):
            for field in ("node_id", "structural_hash", "signature_hash"):
                expected_value = node.get("id" if field == "node_id" else field) if node else None
                if row.get(f"{side}_{field}") != expected_value:
                    raise ContractError("signal evidence attribution mismatch")
    if seen != sorted(seen):
        raise ContractError("breaking-change signals are not deterministic")
    non_signals = {
        "body_only_changes": sum(row["kind"] == "body_changed" for row in impact["callables"]),
        "binding_changes": sum(row["kind"] == "changed" for row in impact["bindings"]),
        "bindings_not_comparable": sum(row["kind"] == "not_comparable" for row in impact["bindings"]),
        "stale_evidence_records": sum(row["disposition"] != "retained" for row in impact["evidence"]),
        "base_unresolved_attempts": len(base.get("unresolved_relations", [])),
        "head_unresolved_attempts": len(head.get("unresolved_relations", [])),
    }
    if facet.get("non_signals") != non_signals or not facet.get("limitations"):
        raise ContractError("non-signal accounting or limitations missing")
