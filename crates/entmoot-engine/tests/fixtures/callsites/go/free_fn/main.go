package free_fn

func Leaf() int { return 1 }
func Caller() int { return Leaf() }
