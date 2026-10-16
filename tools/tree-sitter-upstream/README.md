# Grammar contribution evidence

These retained local packets explain the production grammar inputs and past
experiments. Upstream submission is deferred. A packet is not a language
support claim. Source syntax, native target-mode qualification, and actual
compiler/runtime behavior are independent results.

Keep upstream repositories outside this directory. Keep only minimal patches or
pinned existing proposals, independently authored fixtures, executable oracles,
and compact receipts here. Never generate expected CSTs or binding outcomes from
the implementation being tested. Preserve older upstream syntax, including syntax
before the originally surveyed 2020 window.

The [Tree-sitter corpus guidance](https://tree-sitter.github.io/tree-sitter/creating-parsers/5-writing-tests.html)
requires named nodes and fields, positive/negative cases and dialect coverage.
When changing a production grammar, use focused positive/adversarial visitor
fixtures and relevant upstream generation, corpus and binding checks. Broader
example, platform and performance runs are useful when needed for a specific
claim. Unrun checks remain limitations, not a universal release gate.

The [Tree-sitter core process](https://github.com/tree-sitter/tree-sitter/issues/5922)
requires design discussion before maintainer-opened PRs. Core's AI policy requires
human ownership and forbids direct agent interaction. This workspace prepares
local material only. Check each grammar's own rules; core's maintainer-only PR
rule must not be silently attributed to every grammar repository.

Current executable packets: [Python](python/README.md), [Go](go/README.md),
[Java](java/modern-syntax/README.md), [C#](csharp/file-based-directives/README.md)
and [Rust](rust/README.md), plus the shared
[JavaScript](javascript/README.md)/[TypeScript/TSX](typescript/README.md) packet,
and the [C](c/README.md)/[experimental C++](cpp/README.md) packet,
with separate compiler, CST, full-corpus, binding, example, recovery and
parser-measurement receipts. Shared local runtime, Windows/Linux binding and
parser-measurement drivers live here; language owners retain their fixtures and
feature expectations. Unrun platform checks are historical limitations, not
current completion gates. Production bundle integration and its focused visitor
controls are recorded separately below. Current ownership guidance is in [ARCHITECTURE](../../docs/architecture/ARCHITECTURE.md).
Each runner needs its own `TREE_SITTER_LIBDIR`: forcing a rebuild alone does not
prevent concurrent variants with the same export name from colliding in the
default cache. No packet implies native adapter qualification or broad pipeline
performance.
Multi-parser packages commit every generated parser, deduplicate aliases and
reject mixed ABIs. A pointer-only binding smoke test is insufficient: the
JavaScript/TypeScript and C/C++ packets also install the grammar into a real Go
parser. The C-family runner reports baseline expectations independently of the
selected candidate's qualification status; successful comparisons can retain
baseline failures. Binding runners preserve a failed receipt when a clean build
removes generated parser files, with missing-file diagnostics and invalidated
parser commitments.

## Production grammar bundle

`crates/entmoot-core/build_support.rs` owns the private Rust build manifest.
`bundle_grammars.py` generates its recipe, compressed parser/scanner/header
sources and license notices under `crates/entmoot-core/generated/grammars/`.
Only the C include closure is retained, not upstream repositories. Packaging
checks the candidate parser/scanner commitments in these packets and requires
byte parity with the independently generated Linux candidate. Headers and two
TypeScript scanner wrappers absent from earlier packet commitments are newly
committed by the bundle; the earlier evidence is not rewritten.

Ordinary Cargo builds verify both compressed and expanded BLAKE3 commitments,
reject linked/escaping paths, expand bounded sources into `OUT_DIR`, and compile
them with the same C toolchain already required by Tree-sitter. They need no
Node, grammar generator, Python or language-specific SDK. Regeneration is a
maintainer operation after recreating the exact qualified candidate directories:

```powershell
python tools/tree-sitter-upstream/bundle_grammars.py
python tools/tree-sitter-upstream/bundle_grammars.py --check
cargo test -p entmoot-core --test grammar_bundle --test revision_pairs
```

Never hand-edit the generated recipe or source blobs. Changes to any compiled
source, scanner, header or qualification commitment change the grammar bundle
fingerprint. The engine's derivation fingerprint includes the recipe and build
implementation, invalidating old derived publications without renaming v1.

The Java implementation is the temporary qualified Orchard candidate. Reassess
its adoption at dependency updates and the repository move; use its contribution
policy when refreshing generated files. Experimental C++ retains its original
shared C grammar dependency. Existing packet platform, recovery and cost limits
still apply; compiled bundle adoption does not turn them into passes.

`visitor_revision_cases.json` selects independently authored sources in the
shared core revision-pair corpus. Run `qualify_visitors.py --selection
tools/tree-sitter-upstream/visitor_revision_cases.json --out <fresh-directory>`
on the recorded Windows/WSL workstation after preparing the pinned tools. This
separate lane records exact compiler modes, tool/source digests and output;
runtime execution, native providers and C# SDK dependency resolution remain
unqualified. Compiler checks never run in the structural unit test lane.
