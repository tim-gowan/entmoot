# entmoot-engine

Compatibility re-export of `entmoot-workspace` for one release. Production code
now lives in topology, store, evidence, query, native, ops and workspace crates.
New integrations should depend on their owning crate. Existing module paths
remain available; repository consumers import `KnowledgeRead` for reads and
`KnowledgeRepository` for publication.

Alpha software: pin the exact prerelease and lockfile; incompatible derivations require rebuilding disposable local publications.

Public API meaning: crate rustdoc (`cargo doc -p entmoot-engine`). Operator entry:
[README](https://github.com/tim-gowan/entmoot#readme); walkthrough:
[ARCHITECTURE](https://github.com/tim-gowan/entmoot/blob/main/docs/architecture/ARCHITECTURE.md);
generated site: [docs](https://tim-gowan.github.io/entmoot/). Structural evidence
and provenance only—not security verdicts or taint analysis.

Licensed under MIT OR Apache-2.0; bundled third-party notices retain their upstream terms.
