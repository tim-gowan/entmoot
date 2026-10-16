"""Five fresh-parser and five unchanged-tree samples over fixed Git source blobs.

This isolates parser work; it is not an OS cold-cache or Entmoot pipeline/storage
benchmark. Source loading, DLL loading and CST verification are outside timing.
"""
import argparse
import json
from pathlib import Path
import statistics
import subprocess
import sys
import time

import tree_sitter

from grammar_runtime import digest, load_language, runtime_digest

REPO = Path(__file__).resolve().parents[2]
SOURCES = {
    "python": [("cpython", "Lib/asyncio/base_events.py"), ("django", "django/db/models/base.py"),
               ("numpy", "numpy/_core/_methods.py"), ("flask", "src/flask/app.py")],
    "go": [("go", "src/net/http/server.go"), ("go", "src/go/parser/parser.go"),
           ("moby", "daemon/daemon.go"), ("moby", "client/client.go")],
    "java": [("guava", "guava/src/com/google/common/collect/ImmutableList.java"),
             ("RxJava", "src/main/java/io/reactivex/rxjava4/core/Observable.java"),
             ("flink", "flink-core/src/main/java/org/apache/flink/api/common/ExecutionConfig.java"),
             ("log4j2", "log4j-api/src/main/java/org/apache/logging/log4j/Logger.java")],
    "csharp": [("Newtonsoft.Json", "Src/Newtonsoft.Json/JsonSerializerSettings.cs"),
               ("nunit", "src/NUnitFramework/framework/Assert.cs"),
               ("orchard", "src/Orchard/ContentManagement/DefaultContentManager.cs"),
               ("PowerToys", "src/settings-ui/Settings.UI/ViewModels/GeneralViewModel.cs")],
    "rust": [("bitflags", "src/parser.rs"), ("regex", "regex-automata/src/meta/regex.rs"),
             ("serde", "serde_core/src/de/mod.rs"), ("tokio", "tokio/src/runtime/runtime.rs")],
    "javascript": [("npm-cli", "lib/npm.js"), ("npm-cli", "lib/commands/config.js"),
                   ("npm-cli", "lib/commands/audit.js"), ("npm-cli", "lib/utils/display.js")],
    "typescript": [("redux", "src/createStore.ts"), ("desktop", "app/src/lib/stores/app-store.ts"),
                   ("vscode", "src/vs/base/common/async.ts"), ("vscode", "src/vs/base/common/event.ts")],
    "tsx": [("desktop", "app/src/ui/account-picker.tsx"),
            ("desktop", "app/src/ui/worktrees/worktree-list.tsx"),
            ("desktop", "app/src/ui/worktrees/worktree-list-item.tsx"),
            ("desktop", "app/src/ui/worktrees/rename-worktree-dialog.tsx")],
    "c": [("tree-sitter-c", "examples/cluster.c"), ("tree-sitter-c", "examples/malloc.c"),
          ("tree-sitter-c", "examples/parser.c")],
    "cpp": [("tree-sitter-cpp", "examples/marker-index.h"), ("tree-sitter-cpp", "examples/rule.cc")],
}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--language", choices=tuple(SOURCES), required=True)
    args = ap.parse_args()
    output = args.out.resolve()
    if output.exists():
        raise ValueError("select a fresh receipt path")
    index = json.loads((REPO / "tools/tree-sitter-upstream/checkouts.json").read_text(encoding="utf-8"))
    libraries = [path.resolve(strict=True) for path in (args.baseline, args.candidate)]
    name = "c_sharp" if args.language == "csharp" else args.language
    exports = ("java", "java_orchard") if args.language == "java" else (name, name)
    handles = [load_language(path, name) for path, name in zip(libraries, exports)]
    rows = []
    for corpus, path in SOURCES[args.language]:
        checkout = next(row for row in index["checkouts"] if row["id"] == corpus)
        source = subprocess.check_output(["git", "-C", checkout["path"], "show", checkout["commit_sha"] + ":" + path])
        trees = [tree_sitter.Parser(language).parse(source) for _, language in handles]
        equal = str(trees[0].root_node) == str(trees[1].root_node)
        if any(tree.root_node.has_error for tree in trees) or (not equal and args.language != "java"):
            raise ValueError("benchmark requires error-free qualified CSTs: " + corpus + "/" + path)
        row = {"corpus": corpus, "commit": checkout["commit_sha"], "path": path,
               "bytes": len(source), "source_digest": digest(source), "named_cst_equal": equal,
               "named_cst_digests": [digest(str(tree.root_node).encode()) for tree in trees], "measurements": []}
        for mode in ("fresh_parser", "unchanged_tree"):
            samples = [[], []]
            for repetition in range(5):
                # Alternate order to reduce consistent baseline/candidate ordering bias.
                for variant in ([0, 1] if repetition % 2 == 0 else [1, 0]):
                    language = handles[variant][1]
                    parser = tree_sitter.Parser(language)
                    prior = parser.parse(source)
                    start = time.perf_counter_ns()
                    for _ in range(25):
                        if mode == "fresh_parser":
                            parser = tree_sitter.Parser(language)
                            result = parser.parse(source)
                        else:
                            result = parser.parse(source, prior)
                            prior = result
                    elapsed = time.perf_counter_ns() - start
                    if result.root_node.has_error or str(result.root_node) != str(trees[variant].root_node):
                        raise ValueError("fresh/incremental CST mismatch")
                    samples[variant].append(elapsed / 25)
            baseline, candidate = (statistics.median(values) for values in samples)
            row["measurements"].append({"mode": mode, "samples_ns_per_parse": dict(zip(("baseline", "candidate"), samples)),
                                        "candidate_to_baseline_median": candidate / baseline,
                                        "sample_ranges_overlap": max(samples[0]) >= min(samples[1]) and max(samples[1]) >= min(samples[0])})
        rows.append(row)
    receipt = {"format": "entmoot.parser_measurement.v1", "driver_digest": digest(Path(__file__).read_bytes()),
               "runtime_driver_digest": runtime_digest(), "language": args.language,
               "libraries": [{"path": str(path), "digest": digest(path.read_bytes())} for path in libraries],
               "samples_per_mode": 5, "parses_per_sample": 25, "sources": rows,
               "limits": ["Fresh parser does not mean OS cold cache; unchanged-tree samples reuse an identical immutable source",
                          "Single-process Windows microbenchmark; no pipeline, storage, process-tree memory or capacity claim",
                          ("Java compares the qualified original and Orchard grammars, including intentional named modifier/visibility CST changes; this is grammar-change cost, not an optimization claim"
                           if args.language == "java" else "Error-free unchanged CST parity is required; syntax correction costs are excluded from this comparison"),
                          "Fresh and unchanged-tree named CST parity is required independently within each grammar",
                          "Sample range overlap is descriptive, not a statistical confidence bound or broad performance gate"]}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(receipt, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"receipt": str(output), "sources": len(rows)}))


if __name__ == "__main__":
    main()
