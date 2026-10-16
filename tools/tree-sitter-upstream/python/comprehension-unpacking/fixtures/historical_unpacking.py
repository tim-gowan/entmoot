"""Compiler AST and runtime oracles for the three affected historical CSTs."""
import ast


class Chunk:
    values = [1, 2]

    def __getitem__(self, key):
        assert key == 0
        return self

    def __call__(self):
        return self.values


chunk = Chunk()
assert [*chunk.values] == [1, 2]
assert [*chunk[0].values] == [1, 2]
assert [*chunk()] == [1, 2]

for source, kind in [
    ("[*chunk.values]", ast.Attribute),
    ("[*chunk[0].values]", ast.Attribute),
    ("[*chunk()]", ast.Call),
]:
    expression = ast.parse(source, mode="eval", feature_version=(3, 8)).body
    assert isinstance(expression, ast.List)
    assert len(expression.elts) == 1
    assert isinstance(expression.elts[0], ast.Starred)
    assert isinstance(expression.elts[0].value, kind)

# A successful historical AST mode is distinct from a Python 3.8 installation.
