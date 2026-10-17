---
name: entmoot-operate
description: Navigate Entmoot source knowledge, inspect open call boundaries, configure local storage location or diagnose native readiness and publication recovery. Use for operator workflows, not general code review or taint/security verdicts.
---

Read the repository README and the relevant runbook under docs/runbooks. Run from
the selected source or use -C. Ordinary operations prepare missing structural
inventory and manage storage automatically. Use --data-dir only to select a local
location. Existing history is not silently refreshed; init refreshes and bind
explicitly invokes configured native adapters. Never install toolchains implicitly.

Start with status and find, then inspect/callers/callees/read-source. Use boundaries
for callee syntax, normalized native outcome/reason and build context. A text match
is not a type claim. Count distinct sites separately from attempts/evaluations;
retain source digest, occurrence and provider selectors when reporting a subset.
Use Git for complete file-byte deltas; callable impact answers a different
question. Find results can include declarations and call stubs: select exact
subjects or boundaries, then verify the relevant source span and lexical scope.
Use compact output first, explain/export only for needed evidence. Preserve paging
receipts and distinguish search truncation from presentation limits.

Report repository, dependency and native-library completeness independently.
Unreported evidence, unsupported syntax, environment failure and runtime dispatch
need different next actions. Never interpret a missing edge or successful process
as absence, closure or a safe upgrade. Package compatibility without matching
import/export scope stays incomplete.

On fingerprint mismatch, use the matching historical reader or a fresh local data
directory and explicit rebuild. Do not edit fingerprints or delete active stores.
Use existing retention planning for intentional cleanup. Native setup belongs to
the resolver-adapters runbook and a trusted, isolated project environment.
