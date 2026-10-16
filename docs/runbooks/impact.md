# Revision impact controls

Procedure for revision compare, subject correspondence and inventory overlay.
Meaning (subjects, correspondence, missing ≠ absence): [ARCHITECTURE](../architecture/ARCHITECTURE.md).
CLI shapes: `entmoot assess --help` (and generated docs CLI reference). Cargo
owners: `revision_pairs`, `revision_receipts`, `impact_relay`, `artifact_cli`,
MCP `stdio_e2e`; harness: [evals/harness](../../evals/harness/README.md).

## Subject correspondence and inventory overlay

Cross-materialization compares (zip extracts, fork checkouts, PyPI trees) declare
a subject with `entmoot init --subject pkg:pypi/name` or
`entmoot deps materialize --subject pkg:pypi/name --version <v> --from <tree>`.
Impact accepts `--correspondence-mode relative_occurrence|explicit_alias`,
`--subject`, and `--alias from=to`. Same declared subject joins without equal
`repository_id`. Keep prior generations addressable via `entmoot storage-plan`
(`keep_recent`); do not compact away the base used for impact.

Inventory-only presence deltas (no Bound/Fits):  
`entmoot assess overlay --base-graph <a> --head-graph <b> [--name model_validate]`.

Package-downgrade recipe: materialize both versions under the workspace volume's `deps/`, init
each with `--subject`, run impact with PURL subject correspondence, overlay
consumer×provider inventories, then optional `entmoot assess compatibility`.

`entmoot assess impact --tests --base-graph <base> --head-graph <head>
--test-targets <request.json>` combines both sides and explicit target ownership.
Export this repository's ownership floor with `python -m scripts.test_ownership
--base-graph <base> --head-graph <head> --out .tmp/<task>/targets.json`. The request
can also declare pytest/unittest, Node, Go, JUnit, dotnet, CTest or explicit runner
commands and fixture dependencies; the read operation does not execute them.
Without ownership, or with an unmapped change/budget gap, the report requires the
full suite. Reaching a test does not establish that its assertions cover a change.

`entmoot assess clones` requires a compatible graph produced by
`entmoot bind --experimental canon` and emits that command as a nudge when absent.
It reports exact, syntax, alpha and parameterized candidates; a request with
`"canonical": true` selects complete scalar canon forms under their assumptions.
Reads use the retained snapshot even if the working checkout has since changed.
`entmoot assess unused` ranks supplied compiler diagnostics, scoped declarations
without observed references, and recursive islands for manual review. Exactly joined
non-call uses, escapes and exposure facts suppress unreferenced candidates. Compiler
diagnostics retain their producer/build context; none establishes a global verdict.
It prominently reports completeness limits; `--require-complete`
fails when inventory, references, dispatch or entrypoints remain unqualified.
Neither command changes identities or deletes source.

Assessment evidence enters through the existing manifest/attachment path. Declaration
facts require matching snapshot, path, digest, occurrence and full span. Stale or
ambiguous attachment stays diagnostic. Instrumented hits may add an exactly matching
test target on either revision side; ownership remains the floor. Explicit execution
is described in [testing](testing.md), separately from these read-only operations.
