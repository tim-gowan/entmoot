handler = lambda req: (
    __import__("builtins").print(req.q) or req.q
)
