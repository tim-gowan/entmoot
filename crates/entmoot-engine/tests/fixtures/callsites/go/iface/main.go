package iface

type Worker interface { Help() int }
func Caller(w Worker) int { return w.Help() }
