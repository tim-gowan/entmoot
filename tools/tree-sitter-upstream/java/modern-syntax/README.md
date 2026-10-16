# Java 27 and Orchard 0.5.22 qualification

This packet compares the original MIT-licensed Java grammar at
`e10607b45ff745f5f876bfa3e94fbcc6b44bdc11` with the existing MIT-licensed
[Orchard 0.5.22](https://codeberg.org/grammar-orchard/tree-sitter-java-orchard)
at `7dc7faa53d68c9455c6f35f0d0f7583332127452`. It proposes no duplicate patch.
[proposal.json](proposal.json) pins both sources and Orchard's shared workflow.
The [qualification receipt](qualification_2026-10-10.json) retains the evidence
and failed/unrun lanes. No production dependency is changed by this packet.

The [authored controls](cases.py) separately test module imports, ordinary
`module` identifiers, import ambiguity, constructor prologues, delegation,
early field writes/reads, compact source files, records, qualified/modified
patterns, guards, unnamed patterns, primitive previews, historical syntax,
Unicode/CRLF coordinates and preservation of following declarations. Explicit
CST selectors include fields, byte spans, named modifier children and vararg
types. They are not generated from parser output. Every accepted program executes
an assertion and an exact success marker; every negative requires the intended
compiler diagnostic. Incremental damage and repair compare all node kinds,
fields and spans to a fresh parse.

All 33 candidate CST expectations and compiler-mode expectations pass, versus
18/33 CST expectations for the original grammar. Positive programs execute under
the verified OpenJDK 27 build. Historical `--release` controls include 8, 11, 14,
15, 16, 17, 21, 22 and 25. These target modes do not represent an exhaustive set
of old installed JDKs. The initial Java-8 harness incorrectly passed
`--module-path`, which javac rejects for that target; the corrected lane omits
that inapplicable option while preserving isolated class/source paths.

[Module imports (JEP 511)](https://openjdk.org/jeps/511),
[compact source files (JEP 512)](https://openjdk.org/jeps/512), and
[flexible constructors (JEP 513)](https://openjdk.org/jeps/513) are stable from
Java 25. [Primitive patterns in Java 27 (JEP 532)](https://openjdk.org/jeps/532)
remain preview: the enabled and disabled modes have distinct compiler outcomes
even when their CSTs match. A syntactically valid import cannot establish a unique
bind; two module imports deliberately produce a compiler ambiguity. Likewise,
syntax acceptance of an early field read or repeated constructor invocation is
not a claim that Java permits that behavior.

Both complete upstream corpora pass: 108 original groups and 133 Orchard groups,
plus 17 highlighting assertions each. Repeated generation is deterministic and
the generated C/JSON bytes match between Windows and Linux. The original CLI is
0.24.4; Orchard's pinned shared workflow uses 0.26.12 and ABI 14. The original
repository tracks generated files; Orchard generates bindings with `init` and
parser files during its build/release workflow. Its generated CMake project
defaults to ABI 15. The qualification runners explicitly pass the selected ABI
and verify that every binding lane preserves the same parser digest. Preliminary
CMake results that regenerated ABI 15 are superseded, not counted as ABI-14 passes.

The official CLI 0.26.12 Linux binary requires glibc 2.39, unavailable on this
host. A local build of the exact published crate and its lockfile succeeds with
Rust 1.95.0, selected by the repository's toolchain file. Linux qualification uses
that binary; it does not silently substitute another CLI version. Node binding
tests use 22.23.3 with the declared runtime peers: 0.21.1 for the original grammar
and 0.25.0 for Orchard. Generated Orchard dependency locks are retained as build
inputs. The generated lint config reports five pre-existing trailing-comma errors;
the packet does not rewrite foreign source to hide those failures.

The full example comparison streams 61,705 regular Java Git objects from the
six repositories in the original workflow: Elasticsearch, Guava, RxJava, Flink,
Log4j2 and Cassandra. Their exact pins live in the checkout index. Neither grammar
has its own selected `examples/**/*.java` files at these pins. No upstream
exclusion is added. The scope contains 534,190,214 source bytes. Errors fall from
34 to six with 28 recoveries and no introduced errors. All six remaining errors
contain Flink's literal `package ${package};` archetype placeholder; both grammars
and the JDK parse oracle reject them. They remain explicit template inputs.

[ParseOracle.java](ParseOracle.java) uses `JavacTask.parse` at source 27 with
preview enabled, without attribution, annotation processing or corpus execution.
It accepts 61,694 files and rejects 11. Those results are syntax evidence only,
not successful builds, stable-language acceptance or historical runtime evidence.
The upstream example lane itself is `continue-on-error`.

Orchard changes 60,668 named CST signatures. New named `visibility` and `modifier`
nodes explain the broad change; `spread_parameter` also gains explicit
`annotations`, `modifiers` and `type` fields. Four node contracts change field
alternatives. These changes require an Entmoot visitor/identity migration review;
a lower parse-error count alone does not establish interchangeability. Shared
measurement code records each grammar's CST separately and checks fresh versus
unchanged-tree parity within each. Java timing is grammar-change cost, not an
optimization claim or pipeline/storage qualification.

The ABI-14 candidate adds 279,254 generated C bytes, 209 parser states and five
symbols. Five fresh-parser and five unchanged-tree samples over four pinned files
show non-overlapping ranges for Guava (median costs +3.1% and +6.5% respectively).
The other six comparisons overlap, with wide variability on the large RxJava
file. These observations must not be presented as a speedup or broad regression
gate; the candidate performs different CST work. The original Python/Go
unchanged-CST measurement controls still pass the shared driver.

Windows Rust, Python, Node and CMake tests pass for both grammars; CGo fails
because GCC is unavailable. Linux Rust, Go, Python, Node and CMake tests pass for
both, and Orchard's required WebAssembly build passes. Orchard lint fails on both
platforms with the same five source errors; original lint passes. Swift and macOS
remain unrun. Separate WebAssembly corpus attempts are unavailable because both
the official Windows CLI and the local Linux CLI omit the optional `wasm` runtime
feature; a successful WebAssembly build is not reported as a runtime test.

```text
python tools/tree-sitter-upstream/java/materialize.py --variant baseline --grammar <fresh-scratch> --cli <0.24.4> --receipt <fresh-json>
python tools/tree-sitter-upstream/java/materialize.py --variant candidate --grammar <fresh-scratch> --cli <0.26.12> --receipt <fresh-json>
python tools/tree-sitter-upstream/java/prepare_bindings.py --variant candidate --grammar <prepared-scratch> --receipt <fresh-json>
python tools/tree-sitter-upstream/java/qualify_features.py --baseline <original-library> --candidate <orchard-library> --jdk <jdk-27> --out <fresh-directory>
python tools/tree-sitter-upstream/java/qualify_examples.py --baseline <original-library> --candidate <orchard-library> --jdk <jdk-27> --out <fresh-directory>
python tools/tree-sitter-upstream/measure_parser.py --language java --baseline <original-library> --candidate <orchard-library> --out <fresh-json>
```

Use the shared platform drivers with the exact CLI/runtime peer. Linux requires
`--abi 14`; `--rust-bin` and `--wasm` request their separate lanes. Windows derives
the ABI from the prepared parser and passes it to CMake. Corpus tests always force
a rebuild, and each binding run has a fresh CMake directory. Retained
[dependency locks](build-locks/manifest.json) are applied only after verifying the
scratch grammar against its pinned Git object. On Windows, run `npm ci
--ignore-scripts`, then `node <node-gyp> rebuild` in the local `node_modules/tree-sitter`
directory with Node 22.23.3 before the shared driver. The Linux driver performs
those steps itself. No test golden is updated from observed parser output.

The [upstream owner](../README.md) records contribution and generated-file rules.
Orchard has collective maintenance and actual downstream users; neither fact
establishes ecosystem-wide migration. Any adoption is temporary and qualified,
reviewed on dependency updates and the repository move. Remove a temporary fork
when a qualified upstream release supplies equivalent behavior.

This is a local human-review packet. No external issue/PR was posted, and no
maintainer acceptance is claimed. Tree-sitter core's maintainer-only process is
recorded separately in the parent owner; the inspected grammar-specific rules
do not automatically inherit that core policy.
