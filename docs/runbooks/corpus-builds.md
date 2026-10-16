# Native build context

Entmoot init/bind never installs tools. Configure adapters using the
[resolver setup](resolver-adapters.md), then use the [repository campaign](corpora.md)
for isolated upstream builds. After `entmoot init`, readiness is the home-scoped
adapter profile via `entmoot status`.

Keep virtual environments, generated code and logs under `.tmp/<task>/`. For C/C++
native (clang), supply an existing `compile_commands.json` at the analyzed root
or a supported build directory; Entmoot does not generate CompDBs. C++ remains
experimental. Missing headers, unsupported flags and unresolved dependencies stay
explicit in native results. Historical build reports remain in Git.
