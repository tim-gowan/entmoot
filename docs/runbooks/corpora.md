# Repository campaigns

Use original upstream repositories in [repositories.json](../../evals/repositories.json).
The [SQLite harness](../../evals/harness/README.md) clones exact commits into
`.tmp/<task>/sources`, runs configured build commands and publishes with Entmoot.
Foreign sibling trees are not modified. This configuration is local orchestration,
not a new portable wire format.

```text
python -m evals.harness.repository_campaign run --config evals/repositories.json --run-dir .tmp/repository-probe --cli target/debug/entmoot --depth 1
```

Root depth is zero; the default dependency depth is one. Mappings select exact
locked package versions. Unmapped packages, unavailable or unsupported locks and
depth limits remain explicit. npm v2/v3, Cargo, uv, Go and NuGet locks are supported; npm
development dependencies are excluded by default. Cargo/uv results can include
resolved platform and feature variants. Installed build prerequisites beyond the
selected source set are not counted as analyzed repositories.

Native enrichment is automatic unless `--structural-only` is selected. Build
success, parsing and semantic binding are separate observations. A changed
configuration, selected adapter/profile contents, tool or storage layout requires
a new run directory. Relative configured tool paths resolve at invocation. Failed builds
can be retried; completed builds can be reused while Entmoot verifies publication
freshness. Outputs and useful logs stay under the task root.

Optional `checks` run Entmoot argument arrays after publication. They may include
typed `request` objects and reference selected `{root:id}`, `{graph:id}` and
`{store:id}` values. `{request}` names the written request file. Unavailable
publications leave the check unavailable. Process success is separate from the
returned semantic outcome, limits and pagination.

Grammar regeneration currently has its own machine-local checkout map. It is
build input, not an additional product corpus or a current qualification claim.
