# Repository campaigns

The [SQLite harness](harness/README.md) consumes production Entmoot operations.
[Original upstream pins and build recipes](repositories.json) drive isolated
repository runs. [The runbook](../docs/runbooks/corpora.md) describes dependency
scope and limitations.

Correctness fixtures belong to Cargo suites. Native qualification is opt-in;
missing tools or incomplete bindings remain visible. Checkouts, builds, databases
and useful logs belong under `.tmp/<task>/`. Historical measurements are bounded
observations, not current release gates.
