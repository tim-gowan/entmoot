// Install and parse the generated ABI; a non-nil language pointer is insufficient.
package main

import (
	"fmt"
	"os"

	sitter "github.com/tree-sitter/go-tree-sitter"
	grammar "github.com/tree-sitter/tree-sitter-javascript/bindings/go"
)

func main() {
	parser := sitter.NewParser()
	defer parser.Close()
	if err := parser.SetLanguage(sitter.NewLanguage(grammar.Language())); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	tree := parser.Parse([]byte("const value = 7; function after(){ return value; }"), nil)
	if tree == nil || tree.RootNode().HasError() {
		fmt.Fprintln(os.Stderr, "parse failed")
		os.Exit(1)
	}
	defer tree.Close()
	fmt.Println("oracle:ok")
}
