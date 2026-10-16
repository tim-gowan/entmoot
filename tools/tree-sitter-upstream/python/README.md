# tree-sitter-python

Upstream: <https://github.com/tree-sitter/tree-sitter-python>, MIT.
Entmoot baseline: crate **0.25.0**, commit
`26855eabccb19c6abf499fbc5b8dc7cc9ab8bc64`; Cargo.lock owns the installed pin.
The project is maintained. Local grammar qualification is recorded below;
neither Python 3.15 proposal is adopted by Entmoot. Review upstream status at
adoption, not merely this cutoff.

The upstream workflow uses tree-sitter-cli `^0.25.9`, generated parser sources,
and Windows/Linux/macOS corpus and Rust/Node/Python/Go/Swift binding checks.
Use the exact locked generator and parser ABI reported by `grammar_probe`;
do not regenerate with an arbitrary newer CLI. Review generated parser diffs
according to upstream policy. Example corpus includes CPython, NumPy, Django
and Flask; upstream excludes some CPython annotationlib/compile/type-parameter
tests. The exact exclusions and source pins are retained with each example run.

No grammar-specific CONTRIBUTING file was present in the inspected upstream
tree. Its workflow and the linked parser-test action are the executable test
authority. Recheck contribution instructions before a human submits anything.
Core contribution restrictions are described in the [parent README](../README.md).

The [qualification receipt](qualification_2026-10-10.json) separates the baseline,
PR #336, comprehension patch and combined candidate. All four pass their entire
upstream corpus (117, 118, 120 and 121 cases respectively), 34 highlight assertions
and five tag assertions on Windows. The combined candidate meets all 32 authored
CST controls and their independently checked Python 3.15 compile outcomes. Each
error-free control survives an incomplete edit and recovery with identical node
kinds, fields and coordinates. Four runtime fixture files pass separately.

The complete example scope is 5,898 regular Python files: twelve historical
examples from the grammar plus pinned CPython, NumPy, Django and Flask clones.
Three upstream CPython exclusions and two nonregular Git entries are explicit.
The combined candidate recovers 75 files without new errors on compiler-accepted
source or upstream historical examples. One remaining compiler-accepted CPython
file uses unsupported explicitly positive literal patterns (`case +1`); the other
two remaining parse errors are intentional invalid programs. This is a grammar
gap, not an Entmoot visitor defect. The subsequent
[positive-pattern packet](positive-patterns/README.md) qualifies its separate fix.
Python 2 examples retain identical CSTs;
their rejection by Python 3.15 does not establish historical runtime behavior.

Windows Rust, Python, Node and CMake smoke/build lanes pass for all four variants.
Windows Go failed because a CGo C compiler is unavailable; Swift is unrun. The
receipt records Linux results separately and leaves macOS unqualified. These
upstream smoke tests do not qualify Entmoot's native semantic adapter.
On Linux/WSL, baseline and combined candidate pass repeated generation, full
corpus/query checks, Go/Python/Node binding tests, lint and CMake builds; generated
C/JSON digests match the Windows runs. Linux Rust and Swift remain unrun.

The combined generated C is 3,338,631 bytes versus the baseline's 3,448,505;
Windows libraries are 452,096 and 466,944 bytes. Five alternating samples over
four unchanged-CST files have overlapping baseline/candidate timing ranges in
both fresh-parser and unchanged-tree modes. This descriptive microbenchmark
does not establish an optimization or replace cold/warm pipeline, storage or
process-tree memory measurements.

The [compiler/runtime receipt](oracle_2026-10-10_checked.json) passes twelve controls under
official Python 3.15.0 on Windows. It explicitly does not qualify Tree-sitter or
the native adapter. The [release notes](https://docs.python.org/3.15/whatsnew/3.15.html)
explain lazy imports, unpacking and encoding changes. The executable assertions
in this packet establish the checked behavior.

Reproduce with an explicit Python 3.15.0 executable and a fresh receipt path:

```text
python tools/tree-sitter-upstream/python/run_oracles.py --python <python-3.15.0> --out <new-receipt.json>
python tools/tree-sitter-upstream/python/qualify_cst.py --grammar <scratch-grammar> --variant <variant> --cli <tree-sitter-0.25.9> --python <python-3.15.0> --out <new-receipt.json>
python tools/tree-sitter-upstream/python/qualify_examples.py --baseline <baseline-library> --candidate <candidate-library> --python <python-3.15.0> --out <new-directory>
python tools/tree-sitter-upstream/measure_parser.py --language python --baseline <baseline-library> --candidate <candidate-library> --out <new-receipt.json>
```

Materialize the pinned baseline or proposal with `git archive` into a fresh
directory under `.tmp/tools/tree-sitter-qualification/`. Apply the comprehension
patch only there. Never generate in the sibling source clone. `qualify_cst.py`
requires the pinned Python packages in [requirements.txt](requirements.txt); it checks CLI 0.25.9,
ABI 15, interpreter 3.15.0 and repeated generation. Expected CSTs are authored,
not regenerated from parser output. Source examples are streamed from indexed
Git objects without executing foreign code or materializing another corpus.

`../qualify_windows.py --help` and `../qualify_linux.py --help` describe the executable
platform lanes. Windows builds require the prepared optional Node peer
`tree-sitter@0.25.0`; Node 25's peer requires a C++20 build override, which must
not leak into grammar C builds. The proposal lock omits that optional peer, so
its prepared runtime is supplied through `NODE_PATH`. The upstream Cargo package
is isolated from this workspace with a scratch-only empty `[workspace]` section.
Force rebuilds prevent stale binaries from masquerading as successful tests.
Corpus commands also use `--rebuild`: the CLI's grammar-name cache is shared by
scratch variants. [Forced-corpus validation](parser_cache_2026-10-10.json)
rechecks the original packet and the positive-pattern candidates independently.
The original packet identifies its historical driver bytes; it does not claim
that subsequently edited source paths still have those bytes.
Linux uses isolated Go 1.27.2 and Node 25.1.0 archives with official checksum
verification, GCC 11.4 and Python 3.10.12. Its scratch Python build dependencies
are setuptools 80.9.0 and wheel 0.45.1, alongside the parser requirements.
Tool pins, exact commands, generated-file digests and failed setup attempts are
recorded in the receipt. Local detailed logs are named as local retained bytes;
the compact receipt does not claim to embed those bytes.

Adoption requires compiler evidence plus improved error-free CSTs, stable fields,
visitor controls and the upstream test matrix. Remove any temporary patch when
an upstream release supplies the qualified behavior; otherwise reassess at every
dependency update and the repository move. No patch/fork is adopted by this packet.
