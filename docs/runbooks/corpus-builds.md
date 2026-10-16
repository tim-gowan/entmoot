# Native build context

Entmoot init/bind never installs tools. Configure adapters using the
[resolver setup](resolver-adapters.md), then use the [repository campaign](corpora.md)
for isolated upstream builds. After `entmoot init`, readiness is the home-scoped
adapter profile via `entmoot status`. Optional host probe:
`python scripts/check_native_env.py`.

Keep virtual environments, CompDB files, generated code and logs under
`.tmp/<task>/` (CompDB checkouts under `.tmp/evals/checkouts/`). The scripts
`prepare_curl_compdb.py`, `prepare_openssl_compdb.py` and
`prepare_postgres_compdb.py` prepare selected C/C++ compilation contexts; they
do not establish coverage of every build configuration. C++ remains experimental.
Missing headers, unsupported flags and unresolved dependencies stay explicit in
native results. Historical build reports remain in Git.
