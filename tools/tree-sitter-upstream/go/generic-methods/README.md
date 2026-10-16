# Generic methods: qualify existing PR #198

Existing proposal: <https://github.com/tree-sitter/tree-sitter-go/pull/198>.
Exact head and base are retained in [proposal.json](proposal.json). Status at the
cutoff: open, not merged. This packet qualifies the existing proposal rather than
creating a duplicate contribution.

[positive.go](fixtures/positive.go) covers method-local type parameters, a
dependent slice constraint, explicit method instantiation, and an unrelated
following function. The [oracle driver](run_oracles.py) runs those assertions,
an ordinary method in Go 1.14 source mode, an illegal generic interface method,
and an unsatisfied constraint. The actual compiler is Go 1.27.2; historical mode
is not a claim that a Go 1.14 runtime was executed.

Expected CST: `method_declaration` retains receiver, name, parameters, result and
body; add a distinct `type_parameters` field between name and parameters. Its
constraint grammar must disambiguate type parameters from simple types. `After`
must remain a following `function_declaration`, and editing/removing a type
parameter list must recover incrementally without consuming that sibling.
Semantic constraint failures remain the compiler's responsibility.

```text
python tools/tree-sitter-upstream/go/generic-methods/run_oracles.py --go <go1.27.2> --cache <scratch-cache> --out <fresh-receipt.json>
cargo run -p entmoot-core --example grammar_probe -- go tools/tree-sitter-upstream/go/generic-methods/fixtures/positive.go
```

[Compiler/runtime receipt](oracle_2026-10-10_checked.json): four initial controls
pass. The [qualification receipt](qualification_2026-10-10.json) extends this to
17 declaration, constraint, negative, Unicode/CRLF and historical-mode controls.
The baseline meets 4/17 CST expectations; #198 meets 17/17. Compiler/runtime
expectations pass for both. The receipt distinguishes process success from
feature qualification: a completed diagnostic baseline is not a feature pass.
Malformed edits and removal/restoration of type parameters preserve fresh versus
incremental tree parity. Every following `After` declaration has an authored
name and byte-position expectation.

Both variants pass all 67 upstream corpus groups with forced parser rebuilds.
Windows Rust/Python/Node/lint/C builds and Linux Go/Python/Node/lint/C builds pass.
Windows Go fails because GCC is absent. Other platform gaps remain explicit in
the [owner record](../README.md). The proposal's generated parser differs from
its checked-in parser only in the generator banner; Windows/Linux generation
agrees exactly. Its C parser grows by 9,210 bytes, compiled Windows library by
1,536 bytes, and parser states by 12. Five fresh-parser and five unchanged-tree
samples over four pinned files have overlapping ranges in every comparison.
These are parser microbenchmarks, not cold-cache, storage or pipeline claims.

The example lane streams 22,524 tracked regular Go files (185,706,010 bytes) from
the grammar, Go and Moby pins in the checkout index. It parses all files matching
the upstream invalid patterns rather than excluding them. Errors fall from 165
to 156, nine files recover, 14 named-node CST signatures change, and no previously
error-free file gains a grammar error. S-expression signatures omit anonymous
tokens; they are diagnostic correspondence, not full occurrence identity.
The standard-library `go/parser` oracle is syntax evidence only; it rejects 133
files independently. Compiler/runtime evidence comes from the authored controls.

The remaining 156 errors prevent blanket qualification. In particular,
`go/test/genmeth1.go` exposes instantiated method values with multiple type
arguments that #198 still rejects. Eleven unmarked errors are accepted by
`go/parser`; ten involve `new(expression)`, and one involves method values.
Other unmarked errors also fail `go/parser`. Follow-up changes require minimal
compiler/runtime reproducers, not just a parser's acceptance.

```text
python tools/tree-sitter-upstream/go/generic-methods/qualify_cst.py --variant proposal --grammar <scratch-proposal> --cli <tree-sitter-0.25.8> --go <go1.27.2> --cache <scratch-cache> --out <fresh-json>
python tools/tree-sitter-upstream/go/generic-methods/qualify_examples.py --baseline <baseline-library> --candidate <proposal-library> --go <go1.27.2> --cache <scratch-cache> --out <fresh-directory>
python tools/tree-sitter-upstream/measure_parser.py --language go --baseline <baseline-library> --candidate <proposal-library> --out <fresh-json>
```

Shared Windows/Linux binding drivers live one level above the language owners;
both require explicit grammar and tool paths. Build only scratch materializations
under `.tmp/tools/tree-sitter-qualification`, never the indexed source clones.
This packet is not an adoption or upstream submission approval.

The [method-value supplement](method-values.patch) now covers selector and
parenthesized operands in the existing `type_instantiation_expression` shape,
with an explicit ambiguity against ordinary expressions. Its historical `type`
field remains a grammar label, not a semantic assertion. Ten [authored controls](method_value_cases.py)
pass, including ordinary nested indexing, CRLF/Unicode coordinates, trailing
commas, receiver forms and a parenthesized generic method rejected by the compiler.
Two [corpus groups](fixtures/method_values.txt) pass alongside all 67 existing
groups. The [composed receipt](../new-expressions/qualification_2026-10-10.json)
records independent and combined validation with PR #193, including the latter's
historical-shadowing regression and correction. No production adoption follows.
