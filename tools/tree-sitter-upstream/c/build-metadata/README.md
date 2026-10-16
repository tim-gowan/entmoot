# C binding and build metadata corrections

The original C lock declares node-addon-api `^8.3.1`, while its manifest requires
`^8.5.0`; `npm ci` fails. The tool-generated
[lock](../build-locks/package-lock.json) changes only that requirement and the
installed node-addon-api pin to 8.5.0. The materializer verifies the exact old
lock digest before copying the qualified lock into owned scratch.

`bindings.patch` repairs the Python smoke test's missing `Language`/`Parser`
imports and replaces two CMake generation steps with one portable invocation.
The original CMake recipe uses `--no-parser`, unavailable in its locked CLI
0.25.4. A clean Linux build then removes generated files and fails. The shared
runner now retains a failed receipt with explicit missing-parser diagnostics;
it does not lose the failure while trying to fingerprint nonexistent files.

The original Go metadata lacks a checksum and references runtime 0.24.1, which
the module service rejects as an unknown revision. Its ABI is therefore
unqualified. The separate C++ 0.24.0 runtime accepts its pointer-only smoke test
but rejects actual ABI-15 parser installation. [Tool-generated metadata](../build-locks/go.mod)
pins runtime 0.25.0 and its Go 1.23 floor. [The oracle](../go_binding_oracle.go)
actually installs the language and parses a source file.

[prepare_bindings.py](../prepare_bindings.py) validates the profile, generated
parsers, original source and metadata before applying corrections. It does not
change foreign checkouts or parser bytes. [Qualification](../qualification_2026-10-10.json)
retains original failures and corrected lanes. Python/Go/CMake checks supplement
the Rust-only C upstream CI configuration; Node is upstream-disabled here.
These fixes do not qualify Entmoot native resolution. Remove them when upstream
metadata passes the same checks; reassess on dependency updates.
