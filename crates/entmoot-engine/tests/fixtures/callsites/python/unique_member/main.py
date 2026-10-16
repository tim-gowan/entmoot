class Api:
    def leaf(self) -> int:
        return 1

def caller(api: Api) -> int:
    return api.leaf()
