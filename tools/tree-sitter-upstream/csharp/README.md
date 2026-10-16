# tree-sitter-c-sharp

Upstream: <https://github.com/tree-sitter/tree-sitter-c-sharp>. License: MIT.
Entmoot crate pin: **0.23.5** (Cargo.lock is authoritative).
Inspected upstream revision: `8c0abe0b84a3681d3e6852ce2e2a1eecb4f731c5`. The upstream workflow metadata below is
research at the October 10, 2026 cutoff, not a claim that the installed crate has
already passed that workflow against new syntax.

The upstream lock pins generator **0.26.5** (declaration `^0.26.5`); measured
parser ABI is **15**. Upstream tracks generated C/JSON files. Two baseline
generations match those tracked bytes, and baseline/candidate generation matches
between Windows and Linux. The full corpus has 179 baseline groups and 181 with
the local directive supplement, plus 892 highlighting assertions each.

The platform matrix is Windows, Linux and macOS. Metadata enables C, Go, Node,
Python, Rust and Swift bindings; the inspected parser-test action invocation has
no explicit binding-test booleans. Local available-platform results are in the
[directive packet](file-based-directives/README.md), with failed/unrun lanes kept
separate. Node uses 22.23.3 and the locked tree-sitter peer 0.25.0. Cargo, Node and
Go dependency locks are upstream-owned and unchanged. Scratch Cargo manifests
only add workspace isolation. The Linux generator is a locked local build of the
published CLI 0.26.5 crate, not a substituted release.

The example workflow names Newtonsoft.Json, NUnit, Orchard and PowerToys, with
130 exact exclusions. The local 10,284-file comparison parses both included and
excluded sources, records the seven exclusions unmatched at these pins, and
retains all 95 existing parse failures. Exclusion is never a successful parse.
Use the packet's commands for corpus/generation, explicit bindings, example
oracles, incremental recovery and parser measurements. Fresh output namespaces
and unique parser-library caches are required.

No grammar-specific CONTRIBUTING/AGENTS file was present in the inspected original
repository. Review its README, package scripts and .github/workflows again before
human submission. Core's maintainer-only contribution rule is separately scoped in
the [parent record](../README.md); it is not automatically a rule for this grammar.

Upstream is maintained at the cutoff: extension support is merged in PR #429;
file-based directive issue #431 remains open. This observation is not a maintainer
commitment or an acceptance claim. No external interaction was performed.

No patch/fork is adopted by this record. Any temporary adoption must be removed
when upstream supplies qualified equivalent behavior, or reassessed at dependency
updates and the repository move. Maintainer acceptance is an external outcome.
