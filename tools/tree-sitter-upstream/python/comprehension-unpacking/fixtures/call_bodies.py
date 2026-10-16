"""Unpacking applies to the whole call result, including member calls."""
import ast


class Chunk:
    def values(self):
        return [1, 2]

    def mapping(self):
        return {"a": 1}


chunks = [Chunk(), Chunk()]
assert [*chunk.values() for chunk in chunks] == [1, 2, 1, 2]
assert {*chunk.values() for chunk in chunks} == {1, 2}
assert {**chunk.mapping() for chunk in chunks} == {"a": 1}
assert list((*chunk.values() for chunk in chunks)) == [1, 2, 1, 2]

for source in [
    "[*chunk.values() for chunk in chunks]",
    "{*chunk.values() for chunk in chunks}",
    "(*chunk.values() for chunk in chunks)",
]:
    expression = ast.parse(source, mode="eval").body
    assert isinstance(expression.elt, ast.Starred)
    assert isinstance(expression.elt.value, ast.Call)
    assert isinstance(expression.elt.value.func, ast.Attribute)

mapping = ast.parse("{**chunk.mapping() for chunk in chunks}", mode="eval").body
assert isinstance(mapping, ast.DictComp)
assert isinstance(mapping.key, ast.Call)
assert mapping.value is None


def following_declaration():
    return 17


assert following_declaration() == 17
