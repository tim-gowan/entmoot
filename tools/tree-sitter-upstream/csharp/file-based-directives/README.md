# C# file-based directive qualification

This local packet addresses [upstream issue #431](https://github.com/tree-sitter/tree-sitter-c-sharp/issues/431)
against MIT-licensed `tree-sitter-c-sharp` at
`8c0abe0b84a3681d3e6852ce2e2a1eecb4f731c5`. It changes no production dependency
and makes no submission or maintainer-acceptance claim. The pinned generator is
0.26.5 and the generated parser ABI is 15. Upstream tracks generated sources.
[proposal.json](proposal.json) records ownership and the existing extension PR.
The [compact qualification receipt](qualification_2026-10-10.json) retains exact
input blobs, pins, compiler outcomes, scope counts and platform failures.

The [minimal patch](grammar.patch) adds a named `file_based_directive` token
containing the entire `#:` line. Its payload stays opaque: the compiler's file
mode accepts even an empty or unknown payload, while the SDK interprets directive
names and values. Splitting the token into mandatory name/value fields would
incorrectly discard compiler-accepted syntax. A space after `#:` is accepted;
a space between `#` and `:` is a compiler syntax error. The existing shebang rule
is unchanged. A directive after a declaration can have a valid CST while the
compiler rejects its placement; grammar acceptance never implies semantic validity.

[Authored controls](cases.py) use explicit node kinds, fields, exact byte spans,
Unicode/CRLF coordinates and the following `After` declaration. They cover
properties, SDK selection, package/project/include directives, shebangs, combined
directives, empty and unknown payloads, EOF, placement errors, ordinary compilation,
comments, strings, historical preprocessing, C# 7.3/8–14 forms and existing extension
members. Incremental insertion of malformed syntax and repair must match a fresh
parse across every node, field and span. Expected results are not parser-generated.
Two independently authored upstream corpus groups supplement the full corpus.

The [feature driver](../qualify_features.py) uses explicit reference assemblies
and real SDK/runtime installations: 9.0.318/9.0.20, 10.0.401/10.0.12 and preview
11.0.100-rc.1.26425.128. Historical language modes run on the declared toolchain;
they do not establish acceptance by every old compiler. All accepted compiler
controls execute assertions and an exact success marker. Negative controls require
their specific compiler diagnostics. Compiler configuration is disabled on the
command line: putting `-noconfig` in a response file is ignored by Roslyn.

The separate offline SDK lane clears external NuGet sources, pins `global.json`,
isolates caches and disables ambient build props/targets, shared compilation and
AOT publishing. Package, project and include cases use a tiny authored helper;
the package is built locally. Direct compiler controls explicitly supply that
helper source. A directive's spelling never manufactures a bind. SDK warnings are
retained separately from the executed program's final oracle marker. Unknown,
empty and punctuation directives compile in file mode but are rejected by the SDK.
A `LangVersion=13` directive separately demonstrates SDK override of the compiler
control's explicit mode 14.

All 35 stable compiler/runtime/SDK and candidate CST controls pass. The baseline
meets 20 of those CST expectations. One additional C# 15 preview extension-indexer
program compiles and executes with preview .NET 11 but fails both grammars. This
is an explicit preview gap, not stable C# 15 support. C# 14 extension methods and
properties already supplied by [merged PR #429](https://github.com/tree-sitter/tree-sitter-c-sharp/pull/429)
pass without a duplicate patch; an ordinary identifier named `extension` remains
valid in the historical control. The installed Entmoot visitor/native adapter has
not been qualified by these grammar results.

Language references: [file-based apps](https://learn.microsoft.com/en-us/dotnet/core/sdk/file-based-apps),
[preprocessor directives](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/preprocessor-directives),
[C# 14](https://learn.microsoft.com/en-us/dotnet/csharp/whats-new/csharp-14) and
[C# 15 preview](https://learn.microsoft.com/en-us/dotnet/csharp/whats-new/csharp-15).
The actual pinned compiler/SDK receipts establish the tested outcomes; prose
descriptions of ignored directives do not replace the file-mode distinction.

The example driver streams 10,284 regular C# Git objects, 46,509,918 bytes, from
the four original workflow repositories. All have identical named CSTs before
and after this patch. Both parsers retain 95 errors, with no introduced errors.
Of 130 exact upstream exclusions, 123 paths exist at these pins: 60 errors occur
there, and another 35 occur in the 10,161 workflow-included files. The seven
unmatched exclusions are retained explicitly. No error is hidden by exclusion.
The grammar itself has no selected `examples/**/*.cs` files at this pin.

[ParseOracle.cs](ParseOracle.cs) parses in C# 14 file mode with no preprocessor
symbols, without attribution, project evaluation, package restore or foreign
code execution. It accepts 10,283 files; one non-UTF-8 input remains unqualified.
Parse acceptance does not resolve the grammar's conditional-compilation limitations
or establish a build. The complete selected upstream example scope includes any
tracked administration scripts ending in `.cs`; it is not Entmoot source-coverage
accounting and does not change that pipeline's administration-file exclusion rule.

`materialize.py` verifies the exact source archive, patch destination and reverse
application before building. It compares two generations, forces corpus rebuilds
and uses a unique `TREE_SITTER_LIBDIR`. A concurrent Linux preflight exposed the
default same-name parser-cache collision; that failed run is diagnostic only.
Shared Windows/Linux runners also isolate the loader cache and fence parser bytes
through every binding lane. Corpus and platform results must name the isolated
run, not the earlier racing attempt.

Both isolated full corpora pass on Windows and Linux with identical generated
C/JSON bytes: 179 baseline and 181 candidate groups, plus 892 highlighting
assertions each. Linux Rust, Go, Python, Node, lint and CMake lanes pass for both
variants. Windows Rust, Python, Node, lint and CMake pass; CGo fails because GCC
is unavailable. Swift/macOS and WebAssembly execution are unrun. These results
are upstream grammar/binding checks, not Entmoot semantic binding qualification.

The candidate adds 370,619 generated C bytes, 94,208 Windows DLL bytes and one
symbol; the 8,495 parser states and ABI 15 remain unchanged. Five fresh-parser
and five unchanged-tree samples, each containing 25 parses of each of four pinned
files, preserve identical named CSTs. All eight baseline/candidate sample ranges
overlap. This supports neither a speedup claim nor whole-pipeline performance
qualification. Source loading and DLL loading are outside the measured interval;
fresh parser does not mean an OS cold cache.

Reproduce with a fresh grammar directory under
`.tmp/tools/tree-sitter-qualification/csharp/` and a fresh receipt namespace:

```text
python tools/tree-sitter-upstream/csharp/materialize.py --variant baseline --grammar <fresh-baseline> --cli <0.26.5-cli> --receipt <baseline-receipt.json>
python tools/tree-sitter-upstream/csharp/materialize.py --variant file-directives --grammar <fresh-candidate> --cli <0.26.5-cli> --receipt <candidate-receipt.json>
python tools/tree-sitter-upstream/csharp/prepare_bindings.py --materialization <receipt.json> --receipt <preparation.json>
python tools/tree-sitter-upstream/csharp/qualify_features.py --baseline <baseline-library> --candidate <candidate-library> --sdk9 <9.0.318-root> --sdk10 <10.0.401-root> --sdk11 <11.0.100-rc.1.26425.128-root> --out <fresh-feature-output>
python tools/tree-sitter-upstream/csharp/qualify_examples.py --baseline <baseline-library> --candidate <candidate-library> --sdk <10.0.401-root> --out <fresh-example-output>
python tools/tree-sitter-upstream/measure_parser.py --language csharp --baseline <baseline-library> --candidate <candidate-library> --out <fresh-measurement.json>
```

`prepare_bindings.py` only appends scratch workspace isolation; upstream Cargo,
Node and Go locks stay unchanged. Prepare the exact Node peer with `npm ci
--ignore-scripts` and its native build before the Windows driver. The Linux driver
performs these steps with explicitly prepared tools. Both use Node 22.23.3 and
tree-sitter 0.25.0. No dependency install or compiler build enters Entmoot's default
structural path.

Future production changes need focused visitor/revision-pair controls and honest
directive scope; native target modes are qualified separately.
Remove a temporary patch when upstream supplies qualified equivalent behavior;
reassess it at dependency updates and the repository move. Swift/macOS and any
failed platform lane remain historical limitations, alongside example errors and
the preview indexer gap. Parser microbenchmarks do not qualify pipeline/storage costs.
