import asyncio

chunks = [[1, 2], [3]]
chunk = "outer"
assert [*chunk for chunk in chunks] == [1, 2, 3]
assert {*chunk for chunk in chunks} == {1, 2, 3}
assert {**chunk for chunk in [{"a": 1}, {"a": 2, "b": 3}]} == {"a": 2, "b": 3}
assert list((*chunk for chunk in chunks)) == [1, 2, 3]
assert [*[*chunk for chunk in group] for group in [chunks, [[4]]]] == [1, 2, 3, 4]
assert [*chunk for chunk in chunks if len(chunk) > 1] == [1, 2]


async def stream(values):
    for value in values:
        yield value


async def check():
    assert [*chunk async for chunk in stream(chunks)] == [1, 2, 3]
    assert {*chunk async for chunk in stream(chunks)} == {1, 2, 3}
    assert {**chunk async for chunk in stream([{"a": 1}, {"b": 2}])} == {"a": 1, "b": 2}
    assert [value async for value in (*chunk async for chunk in stream(chunks))] == [1, 2, 3]


asyncio.run(check())
# Historical comprehension scopes must still avoid leaking the induction variable.
assert [value for value in range(3)] == [0, 1, 2]
assert chunk == "outer"
