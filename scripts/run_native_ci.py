"""Run exactly one required native CI target, rejecting an empty selection."""
import json
import subprocess
import sys

SUITES = {
    "native_library": ("entmoot-native", [], []),
    "program_adapters": ("entmoot", [], ["--include-ignored"]),
    "adapter_controls": ("entmoot", [], []),
    "producer_cli": ("entmoot", [], ["--include-ignored"]),
    "campaign_harness": ("entmoot", ["revision_campaign_joins_actual_occurrences_and_preserves_unconsumed_frontiers"], ["--ignored", "--exact"]),
    "dispatch_expectations": ("entmoot-engine", [], ["--include-ignored"]),
    **{name: ("entmoot-engine", [], ["--include-ignored"]) for name in (
        "callsites_c_family", "callsites_csharp", "callsites_go", "callsites_java", "callsites_rust", "callsites_javascript")},
    "package_compatibility": ("entmoot-engine", ["native_python_five_controls"], ["--ignored", "--exact"]),
}


def command(suite):
    package, filters, flags = SUITES[suite]
    features = "producer,native-tests" if package == "entmoot" else "resolved-rust-analyzer,resolved-oxc,native-tests"
    target = ["--lib"] if suite == "native_library" else ["--test", suite]
    return ["cargo", "test", "--locked", "-p", package, "--features", features, *target, *filters, "--", *flags, "--test-threads=1"]


def main():
    suite = sys.argv[1]
    if suite in ("historical_rust", "packages"):
        return  # Dedicated workflow steps perform these controls.
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))
    package = SUITES[suite][0]
    assert any(p["name"] == package and any(
        "lib" in t["kind"] if suite == "native_library" else t["name"] == suite and "test" in t["kind"]
        for t in p["targets"]) for p in metadata["packages"]), (package, suite)
    argv = command(suite)
    listing = subprocess.check_output([*argv, "--list"], text=True)
    if not any(line.endswith(": test") for line in listing.splitlines()):
        raise SystemExit(f"Native CI suite selected zero tests: {suite}\n{listing}")
    subprocess.run(argv, check=True)


if __name__ == "__main__":
    main()
