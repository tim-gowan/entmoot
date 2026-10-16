# Repository campaigns

The [SQLite harness](harness/README.md) consumes production Entmoot operations.
[Original upstream pins and build recipes](repositories.json) drive isolated
repository runs (clone into `.tmp/<task>/`; see harness README for depth, locks
and limitations).

Correctness fixtures belong to Cargo suites. Native qualification is opt-in;
missing tools or incomplete bindings remain visible. Checkouts, builds, databases
and useful logs belong under `.tmp/<task>/`. Historical measurements are bounded
observations, not current release gates.

## Dispatch demo

[`demo/getrandom-structural.json`](demo/getrandom-structural.json) is a one-root
structural-only pin (`getrandom-0.1.16`) for Actions → **Corpus demo**
(`.github/workflows/demo.yml`). It reuses the CI release CLI artifact when
present. Not a release gate; structural/repo CI must not clone this corpus.
