"""Freeze qualified parser bytes for Cargo; never run a grammar generator at build time.

The existing packets fence generated parsers and scanner changes. This packaging
step additionally commits the complete local C include closure and licenses,
requiring byte parity with the separately generated Linux candidate. Auxiliary
files absent from earlier packets are newly committed here, not retroactively
described as covered by those packets.
"""
import argparse
import gzip
import io
import json
from pathlib import Path
import re

import blake3


REPO = Path(__file__).resolve().parents[2]
PACKETS = REPO / "tools/tree-sitter-upstream"
SCRATCH = REPO / ".tmp/grammar-qualification"
OUTPUT = REPO / "crates/entmoot-core/generated/grammars"

# Each tuple names a reviewed packet subtree, never an ambient installed grammar.
CANDIDATES = (
    ("python", "python/positive-patterns", ("previous_feature_controls",),
     ("windows_bindings", 1), "python/signed-combined", "python/linux-signed-combined"),
    ("go", "go/new-expressions", ("builds", "combined"), None,
     "go/qualified-combined", "go/linux-qualified-combined"),
    ("java", "java/modern-syntax", ("builds", "windows_candidate"), None,
     "java/orchard", "java/linux-orchard-host"),
    ("csharp", "csharp/file-based-directives", ("builds", "windows_file-directives"),
     ("platforms", "windows_directives_final"), "csharp/directives-qualified", "csharp/linux-isolated-file-directives"),
    ("rust", "rust", ("builds", "windows"), ("platforms", "windows"),
     "rust/windows-qualified", "rust/linux-qualified"),
    ("javascript", "javascript", ("builds", "javascript", "windows"),
     ("platforms", "javascript", "windows"), "ecmascript/windows-javascript-qualified-final", "ecmascript/linux-javascript-qualified-final"),
    ("typescript", "javascript", ("builds", "typescript", "windows"),
     ("platforms", "typescript", "windows"), "ecmascript/windows-typescript-modules", "ecmascript/linux-typescript-qualified-final"),
    ("c", "c", ("builds", "c", "windows"), None,
     "c-family/windows-c-final", "c-family/linux-c-final-qualified"),
    ("cpp", "c", ("builds", "cpp", "windows"), ("platforms", "cpp", "windows"),
     "c-family/windows-cpp-final", "c-family/linux-cpp-final"),
)


def digest(data):
    return "blake3:" + blake3.blake3(data).hexdigest()


def select(value, keys):
    for key in keys:
        value = value[key]
    return value


def safe_file(root, relative):
    """Resolve includes within a package, rejecting links at every component."""
    root = root.resolve(strict=True)
    candidate = root / relative
    # Include directives may use ../; normalize only after checking containment.
    resolved = candidate.resolve(strict=True)
    if not resolved.is_relative_to(root) or not resolved.is_file():
        raise ValueError(f"non-file or escaped grammar input: {candidate}")
    for part in (resolved, *resolved.parents):
        if part == root:
            break
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError(f"linked grammar input: {part}")
    # resolve() erases symlink components; inspect the original path too.
    for part in (candidate, *candidate.parents):
        if part == root:
            break
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError(f"linked grammar input: {part}")
    return resolved


def closure(root, parsers):
    includes = [Path(parser).parent for parser in parsers]
    pending = [safe_file(root, parser) for parser in parsers]
    for directory in includes:
        scanner = root / directory / "scanner.c"
        if scanner.exists():
            pending.append(safe_file(root, scanner.relative_to(root)))
    found = {}
    while pending:
        path = pending.pop()
        relative = path.relative_to(root).as_posix()
        if relative in found:
            continue
        data = path.read_bytes()
        found[relative] = data
        for local in re.findall(rb'^\s*#\s*include\s*"([^"\r\n]+)"', data, re.M):
            name = local.decode("ascii")
            options = [path.parent / name, *(root / directory / name for directory in includes)]
            target = next((item for item in options if item.is_file()), None)
            if target is None:
                raise ValueError(f"unresolved quoted include {name} in {relative}")
            pending.append(safe_file(root, target.relative_to(root)))
        for system in re.findall(rb'^\s*#\s*include\s*<([^>\r\n]+)>', data, re.M):
            if system.startswith(b"tree_sitter/"):
                raise ValueError(f"unaccounted package include: {relative}: {system!r}")
    return found


def compressed(data):
    stream = io.BytesIO()
    with gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0, compresslevel=9) as output:
        output.write(data)
    return stream.getvalue()


def generate():
    outputs = {}
    packages = []
    newly_committed = []
    for name, packet_dir, build_keys, platform_keys, windows, linux in CANDIDATES:
        packet_path = PACKETS / packet_dir / "qualification_2026-10-10.json"
        packet_bytes = packet_path.read_bytes()
        packet = json.loads(packet_bytes)
        build = select(packet, build_keys)
        root = (SCRATCH / windows).resolve(strict=True)
        linux_root = (SCRATCH / linux).resolve(strict=True)
        expected = dict(build["generated_files"])
        if platform_keys:
            expected.update(select(packet, platform_keys)["inputs"])
        for patch in build.get("patches", []):
            expected.update({key.replace("\\", "/"): value for key, value in patch["after"].items()})
        parsers = sorted(key for key in build["generated_files"] if key.endswith("/parser.c"))
        files = closure(root, parsers)
        linux_files = closure(linux_root, parsers)
        if files != linux_files:
            changed = sorted(key for key in files.keys() | linux_files.keys() if files.get(key) != linux_files.get(key))
            raise ValueError(f"{name}: Windows/Linux source closure differs: {changed}")
        for relative, data in files.items():
            if relative in expected:
                if digest(data) != expected[relative]:
                    raise ValueError(f"{name}/{relative}: differs from qualification evidence")
            else:
                newly_committed.append(f"{name}/{relative}")
        languages = []
        for parser in parsers:
            data = files[parser]
            abi = re.search(rb"^#define LANGUAGE_VERSION (\d+)$", data, re.M)
            export = re.findall(rb"TS_PUBLIC const TSLanguage \*(tree_sitter_\w+)\(void\)", data)
            if abi is None or len(export) != 1:
                raise ValueError(f"{name}/{parser}: ambiguous ABI/export")
            language = Path(parser).parts[0] if name == "typescript" else name
            languages.append((language.upper(), export[0].decode(), int(abi[1]), parser))
        sources = []
        for relative, data in sorted(files.items()):
            packed = compressed(data)
            outputs[f"{name}/{relative}.gz"] = packed
            sources.append((relative, digest(data), len(data), digest(packed), len(packed)))
        # Preserve license notices, including grammars from which generated code derives.
        notices = {"LICENSE": safe_file(root, "LICENSE").read_bytes()}
        dependency = {"typescript": "javascript", "cpp": "c"}.get(name)
        if dependency:
            notices[f"LICENSE.tree-sitter-{dependency}"] = safe_file(root, f"node_modules/tree-sitter-{dependency}/LICENSE").read_bytes()
        for relative, data in sorted(notices.items()):
            outputs[f"{name}/{relative}"] = data
        packages.append((name, packet_path.relative_to(REPO).as_posix(), digest(packet_bytes), languages, sources, notices))

    lines = ["// Generated by tools/tree-sitter-upstream/bundle_grammars.py; do not edit.",
             "#[rustfmt::skip]", "const PACKAGES: &[Package] = &["]
    quote = json.dumps
    for name, packet, commitment, languages, sources, notices in packages:
        lines.extend(["    Package {", f"        name: {quote(name)},", f"        qualification: {quote(packet)},",
                      f"        qualification_digest: {quote(commitment)},", "        languages: &["])
        for constant, symbol, abi, parser in languages:
            lines.append(f"            Language {{ constant: {quote(constant)}, symbol: {quote(symbol)}, abi: {abi}, parser: {quote(parser)} }},")
        lines.extend(["        ],", "        sources: &["])
        for relative, plain_digest, plain_size, packed_digest, packed_size in sources:
            lines.append(f"            Source {{ path: {quote(relative)}, digest: {quote(plain_digest)}, bytes: {plain_size}, compressed_digest: {quote(packed_digest)}, compressed_bytes: {packed_size} }},")
        lines.extend(["        ],", "        notices: &["])
        for relative, data in sorted(notices.items()):
            lines.append(f"            Notice {{ path: {quote(relative)}, digest: {quote(digest(data))}, bytes: {len(data)} }},")
        lines.extend(["        ],", "    },"])
    lines.append("];")
    outputs["manifest.rs"] = ("\n".join(lines) + "\n").encode()
    return outputs, newly_committed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify tracked outputs without writing")
    args = parser.parse_args()
    outputs, newly_committed = generate()
    existing = {path.relative_to(OUTPUT).as_posix() for path in OUTPUT.rglob("*") if path.is_file()}
    obsolete = existing - outputs.keys()
    if obsolete:
        raise ValueError(f"unexpected bundle files require explicit review/removal: {sorted(obsolete)}")
    for relative, data in outputs.items():
        path = OUTPUT / relative
        if args.check:
            if not path.is_file() or path.read_bytes() != data:
                raise ValueError(f"generated output differs: {relative}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    print(json.dumps({"files": len(outputs), "stored_bytes": sum(map(len, outputs.values())),
                      "new_auxiliary_commitments": newly_committed, "checked": args.check}))


if __name__ == "__main__":
    main()
