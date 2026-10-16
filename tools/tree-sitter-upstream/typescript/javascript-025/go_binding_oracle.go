// Actual parser installation and parsing; the upstream smoke test only wraps pointers.
package main

import (
	"fmt"
	"os"

	sitter "github.com/tree-sitter/go-tree-sitter"
	grammar "github.com/tree-sitter/tree-sitter-typescript/bindings/go"
)

func main() {
	for _, item := range []struct {
		name     string
		language *sitter.Language
	}{
		{"typescript", sitter.NewLanguage(grammar.LanguageTypescript())},
		{"tsx", sitter.NewLanguage(grammar.LanguageTSX())},
	} {
		parser := sitter.NewParser()
		if err := parser.SetLanguage(item.language); err != nil {
			fmt.Fprintln(os.Stderr, item.name, err)
			parser.Close()
			os.Exit(1)
		}
		tree := parser.Parse([]byte("const value: number = 7; function after(){ return value; }"), nil)
		if tree == nil || tree.RootNode().HasError() {
			fmt.Fprintln(os.Stderr, item.name, "parse failed")
			parser.Close()
			os.Exit(1)
		}
		tree.Close()
		parser.Close()
	}
	fmt.Println("oracle:ok")
}
