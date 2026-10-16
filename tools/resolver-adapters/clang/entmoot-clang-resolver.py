#!/usr/bin/env python3
"""Entmoot Clang native solver sidecar (stdin JSON → stdout JSON).

Uses libclang via ``clang.cindex`` when available. Missing libclang or an
unusable compile database yields ``invalid_environment=true`` without crashing.
"""

from __future__ import annotations

from contextlib import contextmanager
import json
import os
import sys
from pathlib import Path
from typing import Any


GAP_DYNAMIC = "dynamic_dispatch"
GAP_INDIRECT = "indirect_call"
GAP_UNSUPPORTED = "unsupported_construct"
GAP_INVALID_ENV = "invalid_environment"


def emit(result: dict[str, Any]) -> None:
    result.setdefault("build_context", {})
    result.setdefault("compiler_version", "")
    result.setdefault("calls", [])
    result.setdefault("inputs", [])
    result.setdefault("diagnostics", [])
    result.setdefault("invalid_environment", False)
    result["calls"].sort(key=lambda c: (c.get("path", ""), c.get("start_byte", 0)))
    result["inputs"] = sorted(set(result["inputs"]))
    result["diagnostics"] = sorted(set(result["diagnostics"]))
    sys.stdout.write(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    sys.stdout.write("\n")


def relative_posix(root: Path, absolute: Path) -> str:
    try:
        rel = absolute.resolve().relative_to(root.resolve())
    except ValueError:
        return ""
    text = rel.as_posix()
    if text == ".." or text.startswith("../"):
        return ""
    return text


def load_libclang() -> tuple[Any | None, list[str], str]:
    """Return (clang_module, diagnostics, compiler_version)."""
    diagnostics: list[str] = []
    try:
        import clang.cindex as clang  # type: ignore
    except Exception as exc:  # noqa: BLE001 — environment probe
        return None, [f"clang.cindex unavailable: {exc}"], ""

    lib_path = os.environ.get("LIBCLANG_PATH", "").strip()
    if lib_path:
        try:
            clang.Config.set_library_file(lib_path)
        except Exception as exc:  # noqa: BLE001
            diagnostics.append(f"LIBCLANG_PATH rejected: {exc}")

    # Probe library load early so we can report invalid_environment cleanly.
    try:
        # Creating an index forces libclang.dll/.so load.
        clang.Index.create()
    except Exception as exc:  # noqa: BLE001
        hint = (
            "set LIBCLANG_PATH to libclang shared library "
            "(e.g. libclang.dll / libclang.so)"
        )
        diagnostics.append(f"libclang load failed: {exc}; {hint}")
        return None, diagnostics, ""

    version = "libclang"
    try:
        clang.conf.lib.clang_getClangVersion.restype = clang._CXString
        version = str(clang._CXString.from_result(clang.conf.lib.clang_getClangVersion()))
    except Exception:  # noqa: BLE001
        pass
    return clang, diagnostics, version


def find_compile_commands(root: Path) -> Path | None:
    explicit = os.environ.get("ENTMOOT_CLANG_COMPDB")
    if explicit is not None:
        chosen = Path(explicit)
        if not explicit or chosen.name != "compile_commands.json" or not chosen.is_file():
            raise ValueError("ENTMOOT_CLANG_COMPDB must identify an existing compilation database")
        return chosen.resolve()
    direct = root / "compile_commands.json"
    if direct.is_file():
        return direct
    # Common CMake build dirs
    for candidate in (
        root / "build" / "compile_commands.json",
        root / "out" / "compile_commands.json",
        root / "cmake-build-debug" / "compile_commands.json",
        root / "cmake-build-release" / "compile_commands.json",
    ):
        if candidate.is_file():
            return candidate
    return None


@contextmanager
def compilation_directory(directory: Path):
    """libclang may return relative file names even with -working-directory.

    This sidecar processes one request serially. Resolve every native location
    in its translation unit's working directory, then restore the caller's cwd.
    """
    previous = Path.cwd()
    os.chdir(directory)
    try:
        yield
    finally:
        os.chdir(previous)


def extent_bytes(cursor: Any) -> tuple[int, int] | None:
    extent = cursor.extent
    if extent is None:
        return None
    start = extent.start
    end = extent.end
    if not start.file or not end.file:
        return None
    # libclang offsets are byte offsets into the file buffer.
    return int(start.offset), int(end.offset)


def owner_for(cursor: Any, root: Path) -> dict[str, Any] | None:
    current = cursor.semantic_parent
    while current is not None and current.kind.name != "TRANSLATION_UNIT":
        kind = current.kind.name
        if kind in {
            "FUNCTION_DECL",
            "CXX_METHOD",
            "CONSTRUCTOR",
            "DESTRUCTOR",
            "FUNCTION_TEMPLATE",
            "CXX_METHOD",
        }:
            span = extent_bytes(current)
            if span is None or current.location.file is None:
                return None
            rel = relative_posix(root, Path(current.location.file.name))
            if not rel:
                return None
            return {"path": rel, "start_byte": span[0], "end_byte": span[1]}
        current = current.semantic_parent
    return None


def resolve_definition(cursor: Any) -> Any | None:
    referenced = cursor.referenced
    if referenced is None:
        return None
    # Prefer the definition when available.
    try:
        definition = referenced.get_definition()
        if definition is not None:
            return definition
    except Exception:  # noqa: BLE001
        pass
    return referenced


def target_from_def(defn: Any, root: Path) -> dict[str, Any] | None:
    if defn is None:
        return None
    symbol = defn.get_usr() or defn.spelling or defn.displayname or ""
    kind = "type" if defn.kind.name in {"STRUCT_DECL", "CLASS_DECL", "UNION_DECL", "ENUM_DECL",
                                       "TYPEDEF_DECL", "TYPE_ALIAS_DECL", "CLASS_TEMPLATE"} else "callable"
    span = extent_bytes(defn)
    path = None
    start_byte = 0
    end_byte = 0
    if defn.location.file is not None and span is not None:
        path = relative_posix(root, Path(defn.location.file.name))
        start_byte, end_byte = span
    return {
        "path": path,
        "origin_file": str(Path(defn.location.file.name).resolve()) if defn.location.file else None,
        "start_byte": start_byte,
        "end_byte": end_byte,
        "symbol": symbol,
        "kind": kind,
    }


def call_uses_macro(cursor: Any, clang: Any) -> bool:
    """True when a CALL_EXPR child is a macro instantiation or spelling/expansion diverge."""
    try:
        for child in cursor.get_children():
            if child.kind == clang.CursorKind.MACRO_INSTANTIATION:
                return True
            if child.location.file and child.extent.start.file:
                if child.location.file.name != child.extent.start.file.name:
                    return True
        # Token-level: expansion location differs from spelling for the callee.
        loc = cursor.location
        if loc is not None and loc.file is not None and cursor.extent is not None:
            expansion = cursor.extent.start
            if expansion.file is not None and loc.file.name == expansion.file.name:
                if int(loc.offset) != int(expansion.offset):
                    # Different offset in the same file can still be a macro body;
                    # only treat as macro when tokens include a macro name pattern.
                    for token in cursor.get_tokens():
                        cursor_for_token = token.cursor
                        if (
                            cursor_for_token is not None
                            and cursor_for_token.kind == clang.CursorKind.MACRO_INSTANTIATION
                        ):
                            return True
    except Exception:  # noqa: BLE001
        return False
    return False


def collect_calls_from_tu(
    clang: Any,
    tu: Any,
    root: Path,
    calls: list[dict[str, Any]],
    diagnostics: list[str],
    api: Any,
) -> None:
    # libclang often leaves CALL_EXPR.semantic_parent / lexical_parent unset; keep
    # an enclosing-callable stack while walking so Closed can attach an owner.
    enclosing: list[dict[str, Any]] = []

    def owner_span(node: Any) -> dict[str, Any] | None:
        span = extent_bytes(node)
        if span is None or node.location.file is None:
            return None
        rel = relative_posix(root, Path(node.location.file.name))
        if not rel:
            return None
        return {"path": rel, "start_byte": span[0], "end_byte": span[1]}

    def visit(node: Any) -> None:
        try:
            kind = node.kind
        except Exception:  # noqa: BLE001
            return

        if kind.name in {'FUNCTION_DECL', 'CXX_METHOD', 'FUNCTION_TEMPLATE', 'CLASS_TEMPLATE',
                         'STRUCT_DECL', 'CLASS_DECL', 'UNION_DECL'}:
            api.export(node)

        push_owner = kind.name in {
            "FUNCTION_DECL",
            "CXX_METHOD",
            "CONSTRUCTOR",
            "DESTRUCTOR",
            "FUNCTION_TEMPLATE",
        }
        pushed = False
        if push_owner:
            span_owner = owner_span(node)
            if span_owner is not None:
                enclosing.append(span_owner)
                pushed = True

        if kind == clang.CursorKind.CALL_EXPR:
            span = extent_bytes(node)
            if span is None or node.location.file is None:
                if pushed:
                    enclosing.pop()
                return
            rel = relative_posix(root, Path(node.location.file.name))
            if not rel:
                # Outside root (system headers etc.)
                for child in node.get_children():
                    visit(child)
                if pushed:
                    enclosing.pop()
                return

            fact: dict[str, Any] = {
                "path": rel,
                "start_byte": span[0],
                "end_byte": span[1],
                "targets": [],
                "intrinsic": False,
            }
            owner = owner_for(node, root)
            if owner is None and enclosing:
                owner = enclosing[-1]
            if owner is not None:
                fact["owner"] = owner

            if call_uses_macro(node, clang):
                fact["gap"] = GAP_UNSUPPORTED
                diagnostics.append(
                    f"macro spelling/expansion mismatch at {rel}:{span[0]}"
                )

            defn = resolve_definition(node)
            if defn is None:
                # No definition is a binding failure unless the callee expression
                # itself is already an indirect call form (handled below via kinds).
                fact.setdefault("gap", "unresolved")
            else:
                # Function pointers / indirect callees with native evidence.
                if defn.kind.name in {
                    "VAR_DECL",
                    "FIELD_DECL",
                    "PARM_DECL",
                    "TYPE_REF",
                }:
                    fact["gap"] = GAP_INDIRECT
                else:
                    target = target_from_def(defn, root)
                    if target is not None:
                        # Drop null path key for cleaner JSON (engine accepts optional path).
                        if target.get("path") is None:
                            target.pop("path", None)
                        fact["targets"].append(target)
                    else:
                        fact.setdefault("gap", "unresolved")

                    # Virtual CXX method without closed hierarchy → dynamic_dispatch.
                    try:
                        if defn.kind.name == "CXX_METHOD" and defn.is_virtual_method():
                            fact["gap"] = GAP_DYNAMIC
                    except Exception:  # noqa: BLE001
                        pass

                    if (
                        not fact.get("gap")
                        and fact.get("owner")
                        and len(fact.get("targets") or []) == 1
                        and (fact["targets"][0].get("kind") or "callable") == "callable"
                        and defn.kind.name in {"FUNCTION_DECL", "CXX_METHOD"}
                    ):
                        fact["dispatch"] = {
                            "kind": "closed",
                            "policy_id": "entmoot.clang.direct-function.v1",
                            "evidence_pointer": "/calls",
                            "assumptions": [
                                "selected compile_commands.json configuration",
                                "source-level non-virtual function or method; virtual CXX, macro, foreign and implicit calls are outside this qualification",
                            ],
                        }

            api.reference(defn, fact)
            calls.append(fact)

        for child in node.get_children():
            visit(child)

        if pushed:
            enclosing.pop()

    visit(tu.cursor)

    for diag in tu.diagnostics:
        severity = getattr(diag, "severity", None)
        # 3 == Error in clang.cindex
        if severity is not None and int(severity) >= 3:
            diagnostics.append(str(diag))


class PackageApi:
    policy = "libclang.external_callable_contract.v2"
    template_policy = "libclang.template_declaration_headers.v1"
    layout_policy = "libclang.record_layout_selected_target.v1"

    def __init__(self, root, result):
        self.root = root
        self.packages = []
        self.exports = {}
        self.references = []
        self.target_profile = {}
        self.frozen_sources = {}
        mapping = os.environ.get("ENTMOOT_PACKAGE_API")
        if mapping:
            import re
            document = json.loads(Path(mapping).read_text(encoding="utf-8"))
            for row in document["packages"]:
                if row["ecosystem"] != "native":
                    continue
                purl = row["purl"]
                if not re.fullmatch(r"pkg:(generic|conan)/[A-Za-z0-9._/-]+", purl):
                    raise ValueError("native package mapping requires unversioned generic/conan coordinate")
                kind, name = purl[4:].split("/", 1)
                self.packages.append((Path(row["root"]).resolve(strict=True), kind + ":" + name, purl))
            result["inputs"].append(str(Path(mapping).resolve()))
            result["build_context"]["package_mapping"] = document

    def boundary(self, node):
        if node is None or node.kind.name not in {"FUNCTION_DECL", "CXX_METHOD", "FUNCTION_TEMPLATE",
                "CLASS_TEMPLATE", "STRUCT_DECL", "CLASS_DECL", "UNION_DECL"} or node.location.file is None:
            return None
        if node.linkage.name != "EXTERNAL" or not node.get_usr():
            return None
        file = Path(node.location.file.name).resolve()
        candidates = [(root, name, purl) for root, name, purl in self.packages if file.is_relative_to(root)]
        if len(candidates) != 1:
            return None
        _, name, purl = candidates[0]
        contract = node.type.get_canonical().spelling
        policy = self.policy
        if node.kind.name in {"FUNCTION_TEMPLATE", "CLASS_TEMPLATE"}:
            span = extent_bytes(node)
            if not span:
                return None
            data = self.frozen_sources.get(str(file))
            if data is None:
                data = file.read_bytes()
            contract_bytes = data[span[0]:span[1]]
            # Keep template parameters, constraints, bases, fields and member
            # declarations; function bodies remain implementation evidence.
            cuts = []
            def bodies(cursor):
                for child in cursor.get_children():
                    if child.kind.name == "COMPOUND_STMT":
                        body_span = extent_bytes(child)
                        if body_span and span[0] <= body_span[0] <= body_span[1] <= span[1]:
                            cuts.append(body_span)
                    else:
                        bodies(child)
            bodies(node)
            for start, end in sorted(cuts, reverse=True):
                contract_bytes = contract_bytes[:start - span[0]] + b"{}" + contract_bytes[end - span[0]:]
            contract = contract_bytes.decode("utf-8")
            policy = self.template_policy
            if node.semantic_parent.kind.name in {"CLASS_DECL", "STRUCT_DECL", "CLASS_TEMPLATE"} and node.access_specifier.name != "PUBLIC":
                return None
        elif node.kind.name in {"STRUCT_DECL", "CLASS_DECL", "UNION_DECL"}:
            if not node.is_definition():
                return None
            fields = [dict(name=f.spelling, type=f.type.get_canonical().spelling,
                bit_offset=f.get_field_offsetof(), bit_width=f.get_bitfield_width() if f.is_bitfield() else None)
                for f in node.get_children() if f.kind.name == "FIELD_DECL"]
            size, alignment = node.type.get_size(), node.type.get_align()
            # Dependent/incomplete layouts and polymorphic/base-class ABI need
            # more than libclang's exposed field offsets; retain them raw only.
            if size < 0 or alignment < 0 or not self.target_profile or any(f["bit_offset"] < 0 for f in fields):
                return None
            if any(c.kind.name == "CXX_BASE_SPECIFIER" or (c.kind.name == "CXX_METHOD" and c.is_virtual_method())
                   for c in node.get_children()):
                return None
            contract = json.dumps(dict(target=self.target_profile, kind=node.kind.name,
                size=size, alignment=alignment, fields=fields), sort_keys=True)
            policy = self.layout_policy
        if node.kind.name == "CXX_METHOD":
            if node.access_specifier.name != "PUBLIC":
                return None
            contract += "\n" + json.dumps(dict(static=node.is_static_method(), const=node.is_const_method(),
                virtual=node.is_virtual_method(), pure_virtual=node.is_pure_virtual_method(),
                owner=node.semantic_parent.get_usr()), sort_keys=True)
        return name, purl, node.get_usr(), contract, policy

    def export(self, node):
        boundary = self.boundary(node)
        if not boundary:
            return
        target = target_from_def(node, self.root)
        if not target or not target.get("path"):
            return
        name, _, symbol, contract, policy = boundary
        exports = self.exports.setdefault(name, {})
        previous = exports.get(symbol)
        if previous and (previous["boundary_policy"], previous["boundary_text"]) != (policy, contract):
            raise ValueError("conflicting compilation contracts for " + symbol)
        if symbol not in exports or node.is_definition():
            exports[symbol] = dict(path=target["path"], start_byte=target["start_byte"],
                end_byte=target["end_byte"], symbol=symbol, boundary_policy=policy, boundary_text=contract)

    def reference(self, node, call):
        boundary = self.boundary(node)
        if not boundary or call.get("gap") or len(call["targets"]) != 1 or call["targets"][0].get("path"):
            return
        name, purl, symbol, contract, policy = boundary
        # Keep package coords for seeking; assessed ref still requires contract text.
        call["targets"][0].update(symbol=symbol, package_purl=purl, package_role="module")
        self.references.append(dict(path=call["path"], start_byte=call["start_byte"], end_byte=call["end_byte"],
            target=dict(package=dict(ecosystem="native", name=name),
                        symbol=dict(scheme="cxx.compiler.symbol", spelling=symbol)),
            boundary_policy=policy, boundary_text=contract))

    def result(self):
        return dict(surfaces=[dict(package=dict(ecosystem="native", name=name),
            export_universes=[dict(symbol_scheme="cxx.compiler.symbol", boundary_policy=policy)
                             for policy in [self.policy, self.template_policy, self.layout_policy]],
            completeness="partial", exports=[exports[symbol] for symbol in sorted(exports)],
            limitations=["source_root_package_mapping_operator_declared", "external_free_functions_selected_compilation_only"])
            for name, exports in sorted(self.exports.items())],
            references=self.references, references_complete=False,
            limitations=["template_instantiation_dispatch_linker_visibility_and_full_ABI_compatibility_unassessed"])


class ClosedVfs:
    """Explicit, non-fallthrough Clang filesystem over retained physical inputs."""
    def __init__(self, filename):
        from blake3 import blake3
        self.hash = lambda data: "blake3:" + blake3(data).hexdigest()
        self.path = Path(filename).resolve(strict=True)
        self.document_bytes = self.path.read_bytes()
        self.document = json.loads(self.document_bytes)
        if self.document.get("version") != 0 or self.document.get("fallthrough") is not False:
            raise ValueError("closed Clang VFS requires version 0 and fallthrough=false")
        if self.document.get("use-external-names") is not False:
            raise ValueError("closed Clang VFS requires use-external-names=false")
        if set(self.document) - {"version", "fallthrough", "use-external-names", "case-sensitive", "roots"}:
            raise ValueError("unsupported closed Clang VFS option")
        self.contents = {}
        self.digests = {}
        total_bytes = 0
        for entry in self.document["roots"]:
            if set(entry) != {"type", "name", "external-contents"} or entry["type"] != "file":
                raise ValueError("closed Clang VFS v1 accepts explicit file entries only")
            virtual = Path(entry["name"])
            physical = Path(entry["external-contents"])
            if not virtual.is_absolute() or not physical.is_absolute() or virtual.resolve(strict=True) != physical.resolve(strict=True):
                raise ValueError("VFS virtual source must name its exact retained physical file")
            if virtual.is_symlink() or str(virtual) in self.contents:
                raise ValueError("duplicate or linked VFS input")
            total_bytes += physical.stat().st_size
            if len(self.contents) >= 50000 or total_bytes > 512 * 1024 * 1024:
                raise ValueError("closed Clang VFS input budget exceeded")
            data = physical.read_bytes()
            self.contents[str(virtual)] = data
            self.digests[str(virtual)] = self.hash(data)
        if len(self.contents) > 50000 or sum(map(len, self.contents.values())) > 512 * 1024 * 1024:
            raise ValueError("closed Clang VFS input budget exceeded")

    def verify(self):
        if self.path.read_bytes() != self.document_bytes:
            raise ValueError("Clang VFS manifest changed during bind")
        for file, expected in self.digests.items():
            if self.hash(Path(file).read_bytes()) != expected:
                raise ValueError("Clang VFS input changed during bind: " + file)


def analyze(root: Path, plan: bool, canon: bool = False) -> dict[str, Any]:
    result: dict[str, Any] = {
        "build_context": {"project_system": "libclang"},
        "compiler_version": "",
        "calls": [],
        "inputs": [],
        "diagnostics": [],
        "invalid_environment": False,
    }

    api = PackageApi(root, result)
    clang, load_diags, version = load_libclang()
    result["diagnostics"].extend(load_diags)
    result["compiler_version"] = version or "libclang"
    if clang is None:
        result["invalid_environment"] = True
        result["build_context"]["project_system"] = "libclang_missing"
        return result

    compile_db_path = find_compile_commands(root)
    if compile_db_path is None:
        result["invalid_environment"] = True
        result["diagnostics"].append("compile_commands.json required; an empty-argument parse is not resolved coverage")
        return result
    closed_vfs = ClosedVfs(os.environ["ENTMOOT_CLANG_VFS"]) if os.environ.get("ENTMOOT_CLANG_VFS") else None
    compile_db_bytes = compile_db_path.read_bytes()
    if closed_vfs:
        api.frozen_sources = closed_vfs.contents
        result["inputs"].extend([str(closed_vfs.path), *closed_vfs.contents])
        result["build_context"]["closed_vfs"] = dict(manifest=closed_vfs.document,
            manifest_digest=closed_vfs.hash(closed_vfs.document_bytes), input_digests=closed_vfs.digests,
            coverage="complete_relative_to_declared_non_fallthrough_vfs",
            limitations=["selected_materialization_not_ambient_build_or_linker_closure"])
    index = clang.Index.create()
    tus: list[tuple[Any, Path]] = []

    if compile_db_path is not None:
        result["build_context"]["compile_commands"] = relative_posix(root, compile_db_path) or str(
            compile_db_path
        )
        result["inputs"].append(
            relative_posix(root, compile_db_path) or compile_db_path.name
        )
        try:
            compdb = clang.CompilationDatabase.fromDirectory(str(compile_db_path.parent))
        except Exception as exc:  # noqa: BLE001
            result["invalid_environment"] = True
            result["diagnostics"].append(f"failed to load compile_commands.json: {exc}")
            return result

        try:
            commands = list(compdb.getAllCompileCommands())
        except Exception as exc:  # noqa: BLE001
            result["invalid_environment"] = True
            result["diagnostics"].append(f"compile_commands enumeration failed: {exc}")
            return result

        if not commands:
            result["invalid_environment"] = True
            result["diagnostics"].append("compile_commands.json contained no entries")
            return result

        for command in commands:
            directory = Path(command.directory)
            if not directory.is_absolute():
                directory = (compile_db_path.parent / directory).resolve()
            src = Path(command.filename)
            if not src.is_absolute():
                src = (directory / src).resolve()
            rel = relative_posix(root, src)
            if rel:
                result["inputs"].append(rel)
            if plan:
                continue
            args = list(command.arguments)
            result["build_context"].setdefault("compile_invocations", []).append(dict(
                directory=str(directory), file=str(src), arguments=args))
            # First argument is usually the compiler driver; libclang wants the rest.
            if args:
                args = args[1:]
            # Drop the source path duplicate if present.
            filtered: list[str] = []
            for arg in args:
                if arg.startswith('@'):
                    result["invalid_environment"] = True
                    result["diagnostics"].append("response_file_closure_unassessed:" + arg)
                    return result
                if arg == command.filename or Path(arg).name == src.name and arg.endswith(src.suffix):
                    continue
                filtered.append(arg)
            filtered.extend(["-working-directory=" + str(directory)])
            if closed_vfs:
                if str(src) not in closed_vfs.contents:
                    raise ValueError("compilation source outside declared Clang VFS: " + str(src))
                if any(arg.startswith(("-ivfsoverlay", "-fplugin", "-Xclang", "-fmodules", "-fmodule-file", "-include-pch"))
                       for arg in filtered):
                    raise ValueError("plugins, stacked overlays and precompiled module inputs are unqualified in closed VFS mode")
                filtered.extend(["-ivfsoverlay", str(closed_vfs.path)])
            # Ensure we compile in the recorded working directory via -working-directory when needed.
            try:
                with compilation_directory(directory):
                    tu = index.parse(
                        str(src),
                        args=filtered,
                        unsaved_files=list(closed_vfs.contents.items()) if closed_vfs else None,
                        options=clang.TranslationUnit.PARSE_DETAILED_PROCESSING_RECORD,
                    )
                    tus.append((tu, directory))
                    for included in tu.get_includes():
                        result["inputs"].append(str(Path(included.include.name).resolve(strict=True)))
            except Exception as exc:  # noqa: BLE001
                result["invalid_environment"] = True
                result["diagnostics"].append(f"parse failed for {src}: {exc}")
    if closed_vfs:
        closed_vfs.verify()
    if plan:
        return result

    for tu, directory in tus:
        with compilation_directory(directory):
            errors = [str(d) for d in tu.diagnostics if d.severity >= clang.Diagnostic.Error]
            if errors:
                result["diagnostics"].extend(errors)
                result["build_context"]["partial"] = True
                result["invalid_environment"] = True
                # Recovered/error types cannot establish a package boundary.
                continue
            # Native target information scopes layouts; never substitute the host's
            # architecture for the compilation command's selected target.
            import ctypes
            lib = clang.conf.lib
            lib.clang_getTranslationUnitTargetInfo.argtypes = [clang.TranslationUnit]
            lib.clang_getTranslationUnitTargetInfo.restype = ctypes.c_void_p
            lib.clang_TargetInfo_getTriple.argtypes = [ctypes.c_void_p]
            lib.clang_TargetInfo_getTriple.restype = clang._CXString
            lib.clang_TargetInfo_getPointerWidth.argtypes = [ctypes.c_void_p]
            lib.clang_TargetInfo_getPointerWidth.restype = ctypes.c_int
            lib.clang_TargetInfo_dispose.argtypes = [ctypes.c_void_p]
            info = lib.clang_getTranslationUnitTargetInfo(tu)
            try:
                api.target_profile = dict(triple=clang._CXString.from_result(lib.clang_TargetInfo_getTriple(info)),
                    pointer_width=lib.clang_TargetInfo_getPointerWidth(info)) if info else {}
            finally:
                if info:
                    lib.clang_TargetInfo_dispose(info)
            result["build_context"].setdefault("translation_unit_targets", []).append(dict(
                file=tu.spelling, **api.target_profile))
            collect_calls_from_tu(clang, tu, root, result["calls"], result["diagnostics"], api)
            # These are compiler observations for this target and configuration,
            # independent of the package API's deliberately narrower contracts.
            layouts = result["build_context"].setdefault("record_layouts", [])
            macros = result["build_context"].setdefault("macro_definitions", [])
            capture_scalars = canon and not any(d.severity >= 3 for d in tu.diagnostics)
            for node in tu.cursor.walk_preorder():
                if node.location.file is None or not relative_posix(root, Path(node.location.file.name)):
                    continue
                if capture_scalars:
                    native_type = node.type.get_canonical()
                    sort = None
                    if native_type.kind.name == "BOOL":
                        sort = dict(kind="bool")
                    elif native_type.kind.name in {"CHAR_S", "SCHAR", "SHORT", "INT", "LONG", "LONGLONG", "CHAR_U", "UCHAR", "USHORT", "UINT", "ULONG", "ULONGLONG"}:
                        width = native_type.get_size() * 8
                        if 0 < width <= 128:
                            sort = dict(kind="integer", bits=width, signed=native_type.kind.name not in {"CHAR_U", "UCHAR", "USHORT", "UINT", "ULONG", "ULONGLONG"})
                    span = extent_bytes(node)
                    rows = result["build_context"].setdefault("canon_expressions", [])
                    if sort and span and span[0] < span[1] and len(rows) < 100000:
                        rows.append(dict(path=relative_posix(root, Path(node.location.file.name)), start_byte=span[0], end_byte=span[1], sort=sort,
                            overflow="undefined" if sort.get("signed") else "wrap"))
                if node.kind.name in {"STRUCT_DECL", "CLASS_DECL", "UNION_DECL"} and node.is_definition():
                    layouts.append(dict(usr=node.get_usr(), source=str(Path(node.location.file.name).resolve()),
                        size=node.type.get_size(), alignment=node.type.get_align(),
                        fields=[dict(name=f.spelling, type=f.type.get_canonical().spelling,
                            bit_offset=f.get_field_offsetof()) for f in node.get_children() if f.kind.name == "FIELD_DECL"]))
                elif node.kind.name == "MACRO_DEFINITION":
                    macros.append(dict(name=node.spelling, source=str(Path(node.location.file.name).resolve()),
                        span=extent_bytes(node), tokens=[token.spelling for token in node.get_tokens()]))

    if closed_vfs:
        closed_vfs.verify()
    if compile_db_path.read_bytes() != compile_db_bytes:
        raise ValueError("compilation database changed during bind")
    result["build_context"]["input_closure"] = ("declared_non_fallthrough_vfs_revalidated"
        if closed_vfs else "partial:negative_include_lookups_and_driver_environment")

    result['build_context']['package_api'] = api.result()

    return result


def main() -> int:
    try:
        # PowerShell pipelines may prepend a UTF-8 BOM.
        payload = sys.stdin.buffer.read().decode("utf-8-sig")
        request = json.loads(payload) if payload.strip() else {}
    except Exception as exc:  # noqa: BLE001
        emit(
            {
                "invalid_environment": True,
                "diagnostics": [f"invalid request JSON: {exc}"],
                "compiler_version": "",
                "build_context": {},
                "calls": [],
                "inputs": [],
            }
        )
        return 0

    root_raw = request.get("root") or ""
    if not str(root_raw).strip():
        emit(
            {
                "invalid_environment": True,
                "diagnostics": ["root is required"],
                "compiler_version": "",
                "build_context": {},
                "calls": [],
                "inputs": [],
            }
        )
        return 0

    try:
        root = Path(root_raw).resolve()
    except Exception as exc:  # noqa: BLE001
        emit(
            {
                "invalid_environment": True,
                "diagnostics": [f"invalid root path: {exc}"],
                "compiler_version": "",
                "build_context": {},
                "calls": [],
                "inputs": [],
            }
        )
        return 0

    if not root.is_dir():
        emit(
            {
                "invalid_environment": True,
                "diagnostics": [f"root directory does not exist: {root}"],
                "compiler_version": "",
                "build_context": {},
                "calls": [],
                "inputs": [],
            }
        )
        return 0

    plan = bool(request.get("plan", False))
    try:
        result = analyze(root, plan=plan, canon=bool(request.get("canon", False)))
    except Exception as exc:  # noqa: BLE001 — never crash the sidecar protocol
        result = {
            "build_context": {"project_system": "libclang"},
            "compiler_version": "libclang",
            "calls": [],
            "inputs": [],
            "diagnostics": [f"fatal: {exc}"],
            "invalid_environment": True,
        }

    emit(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
