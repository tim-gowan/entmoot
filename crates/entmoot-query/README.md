# entmoot-query

Read-only bounded navigation over retained publications. No producer dependencies.

`QueryEngine::with_backend` accepts a `RepositoryReader`; durable traversal
checkpoints use a separate optional capability. The SQLite convenience
constructors preserve the existing CLI/MCP behavior. Native tools, discovery,
canon production and publication orchestration cannot link into this crate.

Run its tests with `cargo test -p entmoot-query`. Exercise the standalone
read benchmark with:

```sh
cargo run -p entmoot-query --profile test --example query_scale_bench -- --chain-fixture .tmp/query-chain
```

The authored chain checks scan/index/lazy result equivalence and reports timings;
it does not qualify representative repository scale. Supply a retained graph,
store and request file to measure a real workload. See the repository testing
runbook for the measurement protocol.
