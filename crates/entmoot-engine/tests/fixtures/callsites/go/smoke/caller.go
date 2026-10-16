package fixture

import "example.com/entmootfixture/sibling"

// Unicode offset sentinel: 😀
func Caller(value string) string { return sibling.Help(value) }
func Indirect(callback func(string) string, value string) string { return callback(value) }
func Leaf() {}
func Nested(value string) func() string { return func() string { return sibling.Help(value) } }
type Worker interface { Help(string) string }
func Virtual(worker Worker, value string) string { return worker.Help(value) }
