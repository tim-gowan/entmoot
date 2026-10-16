package main

import "fmt"

type Box[T any] struct { Value T }

func (b Box[T]) Map[U any](f func(T) U) Box[U] {
    return Box[U]{Value: f(b.Value)}
}

func (b Box[T]) MapSlice[S ~[]U, U any](f func(T) S) S {
    return f(b.Value)
}

// A declaration after a generic method must remain a sibling in the CST.
func After() int { return 7 }

func main() {
    mapped := Box[int]{Value: 3}.Map(func(x int) string { return fmt.Sprint(x) })
    values := Box[int]{Value: 3}.MapSlice[[]string, string](func(x int) []string { return []string{fmt.Sprint(x)} })
    if len(values) != 1 || values[0] != "3" { panic("constraint binding changed") }
    if mapped.Value != "3" || After() != 7 { panic("generic method or sibling lost") }
    fmt.Println("generic-method-and-following-declaration:ok")
}
