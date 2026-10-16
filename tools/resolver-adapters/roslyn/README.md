# Entmoot Roslyn native adapter

Isolated .NET 9 console worker that emits Entmoot `NativeRawObservation`-shaped
call facts from Roslyn semantic models. JSON request on stdin, JSON result on
stdout, consumed by the Rust-owned native resolver pipeline. The canonical Go
adapter is under `tools/resolver-adapters/go/`; the old evaluation helpers are
retired.

## Build

```bash
dotnet build -c Release
```

Release output (SDK-dependent RID folder may vary):

```text
bin/Release/net9.0/Entmoot.RoslynSolver.dll
bin/Release/net9.0/Entmoot.RoslynSolver.exe   # Windows apphost
```

Point the Entmoot invoker at the built binary:

```bash
# Windows
set ENTMOOT_ROSLYN_ADAPTER=G:\path\to\Entmoot.RoslynSolver.exe

# Unix
export ENTMOOT_ROSLYN_ADAPTER=/path/to/Entmoot.RoslynSolver.dll
# then: dotnet "$ENTMOOT_ROSLYN_ADAPTER"
```

## Request

```json
{
  "root": "<absolute source root>",
  "patterns": [],
  "flags": [],
  "plan": false
}
```

When a `.sln` or `.csproj` exists under `root`, the worker prefers
`MSBuildWorkspace`. Otherwise it parses every `*.cs` file (excluding `bin` /
`obj`) with an `AdhocWorkspace` / `CSharpCompilation`.

**Cut (perf) / honesty (playbook §6):** prefer inventory-span site filters (match
Rust) over full-tree walks after compile. After MSBuild failure, prefer loud
`invalid_environment` / `partial` over Adhoc parse-all as a “success” path.
See [`docs/architecture/ARCHITECTURE.md`](../../../docs/architecture/ARCHITECTURE.md).

`plan: true` discovers inputs only and skips call extraction.

## Fixture

Source interface, abstract and extern signatures remain declaration evidence;
they never select an implementation occurrence. Default interface bodies and
expression-bodied methods remain callable. A partial method binds to its
compiler-selected implementation. The
[production CLI tests](../../../crates/entmoot-cli/tests/producer_cli.rs)
checks these cases with actual Roslyn output and authored source selectors.

`fixtures/mini/` has no project file so the adhoc path is exercised: `Caller.Run`
calls `Sibling.Help`, `Caller.Indirect` invokes a delegate (`indirect_call`), and
`Caller.Make` constructs `Sibling`. `BindingGaps` retains missing-symbol and
overload-ambiguity sites as unresolved/ambiguous bindings, not
`indirect_call` / `dynamic_dispatch`.
