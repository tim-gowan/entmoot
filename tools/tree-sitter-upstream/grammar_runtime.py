"""Shared local grammar qualification primitives; no compiler or semantic claims."""
from __future__ import annotations

import ctypes
import importlib.metadata
from pathlib import Path

import blake3
import tree_sitter


def scratch_grammar(path):
    """Only generated build materializations may be changed by a driver."""
    root = Path(__file__).resolve().parents[2] / ".tmp/tools/tree-sitter-qualification"
    candidate = path.absolute()
    for ancestor in (candidate, *candidate.parents):
        if ancestor.is_symlink() or (hasattr(ancestor, "is_junction") and ancestor.is_junction()):
            raise ValueError("grammar build path must not traverse links")
    grammar = candidate.resolve(strict=True)
    if not grammar.is_relative_to(root.resolve(strict=True)):
        raise ValueError("materialize the grammar under .tmp/tools/tree-sitter-qualification first")
    return grammar


def digest(data):
    return "blake3:" + blake3.blake3(data).hexdigest()


def runtime_digest():
    """Commit imported helper behavior as well as each caller's own source."""
    return digest(Path(__file__).read_bytes())


def nodes(node):
    yield node
    for child in node.children:
        yield from nodes(child)


def shape(tree):
    return [(node.type, node.start_byte, node.end_byte, tuple(node.start_point),
             tuple(node.end_point), node.is_missing, node.is_error,
             tuple(node.field_name_for_child(i) for i in range(node.child_count)))
            for node in nodes(tree.root_node)]


def endpoint(source):
    return (source.count(b"\n"), len(source.rsplit(b"\n", 1)[-1]))


def load_language(path, language_name):
    if importlib.metadata.version("tree-sitter") != "0.25.2":
        raise ValueError("CST probes require tree-sitter==0.25.2")
    if not language_name.isidentifier():
        raise ValueError("invalid grammar export name")
    library = ctypes.CDLL(str(path))
    function = getattr(library, "tree_sitter_" + language_name)
    function.restype = ctypes.c_void_p
    capsule = ctypes.pythonapi.PyCapsule_New
    capsule.restype = ctypes.py_object
    capsule.argtypes = (ctypes.c_void_p, ctypes.c_char_p, ctypes.c_void_p)
    # Retain the library handle for the entire Language lifetime.
    return library, tree_sitter.Language(capsule(function(), b"tree_sitter.Language", None))
