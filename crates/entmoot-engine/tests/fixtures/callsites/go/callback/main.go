package callback

func Caller(cb func() int) int { return cb() }
