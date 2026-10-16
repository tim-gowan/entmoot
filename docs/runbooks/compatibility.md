# Compatibility controls

Optional Bound/Fits rung on `package_api` (not overlay presence, not impact).
Semantics and limits: [ARCHITECTURE](../architecture/ARCHITECTURE.md) and engine
rustdoc (`compatibility`). Prefer `entmoot assess overlay` for inventory presence
and impact with subject correspondence for same-package revision deltas.

Owners: `package_compatibility`, `joins_soundness`, opt-in CLI `program_adapters`.
When no consumer `package_api` importer references match a requested provider
coordinate, the receipt includes
`consumer_missing_importer_refs:<ecosystem:name>` alongside
`no_evaluable_package_sites`. Python bind may emit seeking `package_api`
references from package-qualified native call/reference facts; `uv.lock` can
assert `LockedPackageInstance` rows when lock bindings are absent (strengthens
Fits only).

Setup: [adapter runbook](resolver-adapters.md). Campaigns:
[SQLite harness](../../evals/harness/README.md).
