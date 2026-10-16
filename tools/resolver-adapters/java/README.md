# Entmoot Java native adapter

JVM sidecar for **javac Trees** materialization. Family id `java`, env
`ENTMOOT_JAVA_ADAPTER`, producer receipts `javac_trees`.

There is no Spoon / CTModel engine path in this tree. Retired CTModel sources
may exist locally under `.tmp/retired/spoon-ctmodel/` (gitignored).

## Build and pin

Use a full JDK first on PATH (17+ for the default release 17; JDK 25 for the
release-25 controls) and Maven 3.8+. From the repository root:

```text
python scripts/build_resolver_adapters.py --families java
cargo build -p entmoot --features producer
```

The builder copies the shaded jar to
`.tmp/adapter-build/resolver-adapters/java/entmoot-java-resolver.jar`. Pin that jar:

```powershell
$env:PATH = "$env:JAVA_HOME/bin;$env:PATH"
$env:ENTMOOT_JAVA_ADAPTER = (Resolve-Path .tmp/adapter-build/resolver-adapters/java/entmoot-java-resolver.jar).Path
```

`ENTMOOT_JAVA_ENGINE` is retired; setting it yields `invalid_environment`.
`init` and `bind` never build adapters or install tools.

## Materialization and package APIs

javac emits `build_context.package_api`. Public source exports require an
operator-supplied `ENTMOOT_PACKAGE_API` JSON mapping with `packages` entries:
`ecosystem: java`, a Maven `group/artifact` name, and the absolute source `root`.
javac accepts release 8/11/17/21/25 under a capable JDK. It consumes explicit
`ENTMOOT_JAVA_CLASSPATH` jars or `ENTMOOT_JAVA_ENVIRONMENT` as a path to a JSON
file (release, source roots, classpath).

## Qualification

The opt-in `adapter_controls` Cargo suite controls explicit source scope, dispatch,
invalid engine/release selection, releases 8–25 and binary authority.
`callsites_java` validates package API survival into the attachment reader.
`producer_cli` checks actual occurrence selection through the operator entry points.
