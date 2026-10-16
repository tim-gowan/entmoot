# Wasm source-phase imports

Node 26.11.1 accepts and executes an authored minimal Wasm module imported with
`import source compiled from …`. The pinned JavaScript grammar rejects the import.
[grammar.patch](grammar.patch) adds a `phase` field while retaining the existing
import/identifier/string node structure. A default import literally named
`source` remains an ordinary binding and has no phase field.

[Corpus fixtures](fixtures/imports.txt) and the shared
[executable oracle](../qualify_features.py) use an authored Wasm byte sequence,
not a vendored repository or compiler-generated ground truth. This is a
Node-supported syntax control; it does not declare the proposal part of ES2026
or establish Wasm/native binding targets from import text.

The [shared receipt](../qualification_2026-10-10.json) records full corpus,
available bindings, examples, incremental repair and measured cost. No production
adoption or upstream acceptance is claimed. Missing platforms, visitor/cost
review and human review remain adoption gates. Remove temporary changes when equivalent
upstream support passes the same controls; reassess at dependency updates and
the repository move. Human submission only.
