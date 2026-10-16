def outer():
    nested = lambda x: x + 1
    return nested(1)
