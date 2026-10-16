package concrete

type Box struct{}
func (Box) Get() int { return 1 }
func Caller(b Box) int { return b.Get() }
