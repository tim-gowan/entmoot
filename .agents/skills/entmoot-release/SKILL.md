---
name: entmoot-release
description: Prepare, verify and hand off Entmoot alpha/beta packages, generated docs and GitHub release workflows. Use for release readiness or an authorized publish, not routine source navigation.
---

Read AGENTS.md and docs/runbooks/local-alpha.md. Keep the documented crate dependency DAG
and exact prerelease dependencies. README maturity must describe demonstrated
utility and remaining limits; a historical corpus process success is not a gate.

Run focused regressions before broad tests, fmt, strict Clippy, doctests, schema
goldens and policy checks. Regenerate schemas with the contracts examples and
package inputs with scripts/prepare_release_inputs.py. Maintain test ownership.
Run scripts/check_release.py and scripts/package_smoke.py; extracted packages must
work outside the source checkout with equal fingerprints. Build docs through
scripts/build_docs.py; do not duplicate CLI/MCP schemas in prose.

Native qualification uses explicit matching adapters, clean selected source/build
scopes and semantic positive/negative controls. Keep missing tools unknown. Read
receipts and retained evidence, including failed/partial runs. Preserve historical
stores rather than relabeling them with a new implementation.

Prepare a clean payload without secrets, scratch, databases, build products or Git
metadata. Follow the user's requested copy/commit/push boundary. Sole CD is
`release.yml` on tag `v*` (GitHub Release + crates.io). `publish.yml` is dry-run /
bootstrap only. Publish only when authorized and the exact tag passes structural,
native and docs workflows. Trusted Publishing must list `release.yml` and
environment `crates-io`. Check ownership and protected-environment configuration
without printing credentials. A local dry run is not publication; report actual
remote outcomes. Partial registry publication is immutable and needs reconciliation
before another attempt.
