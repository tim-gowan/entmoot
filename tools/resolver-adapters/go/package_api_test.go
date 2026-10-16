package main

import (
	"go/token"
	"go/types"
	"testing"
)

func TestReferencesCoalesceOnlyIdenticalVariantFacts(t *testing.T) {
	api := newPackageAPI()
	object := types.NewFunc(token.NoPos, nil, "Leaf", nil)
	boundary := map[string]any{
		"target": map[string]any{"package": map[string]any{"ecosystem": "go", "name": "example.test/dep"},
			"symbol": map[string]any{"scheme": "go.compiler.symbol", "spelling": "example.test/dep.Leaf"}},
		"boundary_text": "func() string",
	}
	api.boundaries[object] = boundary
	call := Call{Path: "api.go", Start: 10, End: 20, Targets: []Target{{PackageRole: "module"}}}
	for i := 0; i < 2; i++ {
		if err := api.reference(call, object); err != nil {
			t.Fatal(err)
		}
	}
	if len(api.references) != 1 {
		t.Fatalf("identical variants: %v", api.references)
	}
	boundary["boundary_text"] = "func() int"
	if err := api.reference(call, object); err != nil {
		t.Fatal(err)
	}
	if len(api.references) != 2 {
		t.Fatalf("conflicting boundaries were lost: %v", api.references)
	}
	if api.references[0]["boundary_text"] != "func() string" {
		t.Fatal("original evidence mutated")
	}
	call.Start, call.End = 30, 40
	if err := api.reference(call, object); err != nil {
		t.Fatal(err)
	}
	if len(api.references) != 3 {
		t.Fatal("distinct occurrence was lost")
	}
}
