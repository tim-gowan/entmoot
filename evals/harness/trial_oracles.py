"""Independent, finite source oracles for local agent calibration.

Python ast observes syntax, not typed dispatch. An attribute spelling does not
prove the runtime receiver or a path through a dependency implementation.
"""
from __future__ import annotations

import ast
from pathlib import Path

from evals.harness.contracts import ContractError
from evals.harness.revision_campaign import canonical_json, material_digest


def python_calls(root: Path, files: list[str], selector: str, side: str) -> tuple[list[dict], dict]:
    results, materials = [], {}

    class Visitor(ast.NodeVisitor):
        def __init__(self, path):
            self.path, self.scope = path, []

        def visit_ClassDef(self, node):
            self.scope.append(node.name)
            self.generic_visit(node)
            self.scope.pop()

        def visit_FunctionDef(self, node):
            self.scope.append(node.name)
            self.generic_visit(node)
            self.scope.pop()

        visit_AsyncFunctionDef = visit_FunctionDef

        def visit_Call(self, node):
            name = node.func.attr if isinstance(node.func, ast.Attribute) else node.func.id if isinstance(node.func, ast.Name) else None
            if name is not None and name.startswith(selector):
                results.append({'side': side, 'path': self.path, 'line': node.lineno,
                                'enclosing_scope': '.'.join(self.scope), 'callee_spelling': name})
            self.generic_visit(node)

    for name in sorted(files):
        path = (root / name).resolve(strict=True)
        if not path.is_relative_to(root.resolve()) or not name.endswith('.py'):
            raise ContractError('Python oracle requires contained .py inputs')
        materials[name] = material_digest(path)
        try:
            tree = ast.parse(path.read_bytes(), filename=name)
        except (SyntaxError, ValueError) as error:
            raise ContractError(f'Python oracle could not parse {name}: {error}') from error
        Visitor(name).visit(tree)
    return sorted(results, key=canonical_json), materials


def grade_calls(expected: list[dict], answer: dict | None, issued: dict) -> dict:
    if answer is None:
        return {'passed': False, 'reason': 'no_final_answer'}
    expected_set = {canonical_json(row) for row in expected}
    actual_set = {canonical_json(row) for row in answer['call_sites']}
    references = [issued[ref] for ref in answer['evidence'] if ref in issued]
    unsupported = [row for row in answer['call_sites'] if not any(
        ref.get('operation') == 'read' and ref.get('side') == row['side'] and ref.get('path') == row['path']
        and ref['first_line'] <= row['line'] <= ref['last_line'] for ref in references)]
    result = {'expected': len(expected_set), 'returned': len(actual_set),
              'missing': len(expected_set - actual_set), 'unexpected': len(actual_set - expected_set),
              'unsupported_locations': unsupported, 'duplicate_answers': len(actual_set) != len(answer['call_sites']),
              'unknown_references': sorted(set(answer['evidence']) - set(issued)),
              'scope': 'direct-name/attribute call syntax in the pinned .py files, not typed dispatch'}
    result['passed'] = (answer['complete'] and bool(references) and expected_set == actual_set and not unsupported
                        and not result['duplicate_answers'] and not result['unknown_references']
                        and not any(answer['changes'].values()))
    return result
