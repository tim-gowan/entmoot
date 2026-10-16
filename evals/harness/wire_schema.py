"""Read Rust-generated schemas for portable payloads; do not duplicate wire shape."""
from functools import lru_cache
import json
from pathlib import Path

from evals.harness.contracts import ContractError

CATALOG = Path(__file__).resolve().parents[2] / "crates/entmoot-contracts/generated/entmoot_contracts.schema.json"
QUERY_CATALOG = CATALOG.with_name("entmoot_query_contracts.schema.json")


@lru_cache(maxsize=16)
def _validator(root_type: str):
    try:
        from jsonschema import Draft202012Validator
    except ImportError as error:
        raise ContractError("wire validation requires evals/harness/requirements.txt") from error
    schema = None
    for path in (CATALOG, QUERY_CATALOG):
        catalog = json.loads(path.read_text(encoding="utf-8"))
        if root_type in catalog["$defs"]:
            schema = catalog["$defs"][root_type]
            break
    if schema is None:
        raise ContractError(f"no Rust-owned wire schema for {root_type}")
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def validate_wire(value: object, root_type: str) -> None:
    error = next(_validator(root_type).iter_errors(value), None)
    if error is not None:
        path = "/".join(map(str, error.absolute_path))
        raise ContractError(f"{root_type} violates generated schema at /{path}: {error.message}")
