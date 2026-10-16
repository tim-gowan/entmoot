def tag(fn):
    return fn

@tag
def leaf() -> int:
    return 1

def caller() -> int:
    return leaf()
