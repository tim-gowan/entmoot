# Entmoot Clang adapter

Python sidecar emitting lossless libclang call facts for the engine's validated
native join. C++ remains experimental. See [adapter setup](../../../docs/runbooks/resolver-adapters.md)
for builds and [qualification](../../../docs/runbooks/compatibility.md)
for the separate native test lane.

## Runtime and configuration

Use Python 3.10+ with the CI-pinned `libclang==18.1.1` distribution (Python
bindings and shared library). Install it into the selected tool environment:

```text
python -m pip install libclang==18.1.1
python scripts/build_resolver_adapters.py --out .tmp/adapter-build/resolver-adapters
```

Set `ENTMOOT_CLANG_ADAPTER` to the generated
`.tmp/adapter-build/resolver-adapters/clang/entmoot-clang-resolver.py`. An explicit override
is exclusive; a missing path fails. Otherwise discovery uses `ENTMOOT_ADAPTERS_ROOT`
or `.tmp/adapter-build/resolver-adapters/`. The engine launches `.py` adapters with
`python` on PATH; its environment must supply the pinned bindings. Set
`LIBCLANG_PATH` to the shared-library file when an explicit library is required.
An import/load failure emits `invalid_environment` and diagnostics; process exit
success does not establish a successful native bind.

## Build context and qualification

The engine requires `compile_commands.json` at the source root or a supported
build directory. Missing build context fails before launching a compiler. The
worker's direct empty-argument fallback is partial and does not qualify this
engine profile. One artifact represents one build configuration.

With the adapter and runtime configured, run:

```text
cargo test -p entmoot-engine --target-dir target/commit-check --test callsites_c_family -- --ignored
```

The [native control](../../../crates/entmoot-engine/tests/callsites_c_family.rs)
checks C and experimental C++ binding against structural inventory, plus a fatal
missing-header case that must not establish bindings. Missing adapters fail this
explicit invocation; default test runs report the control as ignored. Indirect
calls, unsupported constructs and invalid environments remain explicit gaps.

## Classification discipline (OOTB)

The shipped adapter is an ephemeral CompDB walk over pinned libclang, not a
whole-program index. Comparative study of Trail of Bits
[Multiplier](https://github.com/trailofbits/multiplier) informs **how** facts are
classified before they hit the sidecar envelope; Entmoot still owns structural
IDs and joins with `entmoot.native_compiler_exact_spans.v1`. Plan and evaluation
boundaries: [PLANNING](../../../PLANNING.md#multiplier--ootb-clang-adapter-classification-not-store).

| Discipline | OOTB expectation |
|------------|------------------|
| Call vs address-of | True call expressions → call facts; pointer/`&fn` uses → `indirect_call` (optional mechanism detail) |
| Site / owner / verb | Join on exact UTF-8 callee span; owner = enclosing callable; verb ≠ mere DeclRef |
| Macros | Spelling/expansion mismatch → `unsupported_construct`, not a wrong file span |
| Targets | Definition USR/mangled name + in-repo redecl spans for join; externals stay attributable spellings |
| Dispatch | Open; virtual → `dynamic_dispatch`; no closed completeness from definition lookup |
| C++ edges | Overrides/bases/specializations are not call facts; emit only when a later wire assessment exists |
| Configuration | One `compile_commands.json` per artifact; CompDB/env failure → `invalid_environment` |

Do not ingest Multiplier Cap’n Proto/SQLite entity databases into
KnowledgeRepository or treat packed entity IDs as LogicIds.
