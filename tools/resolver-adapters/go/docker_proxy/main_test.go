package main

import (
	"reflect"
	"testing"
)

func TestContainerReceivesSelectedScope(t *testing.T) {
	t.Setenv("ENTMOOT_GO_SINGLE_MODULE", "1")
	t.Setenv("ENTMOOT_GO_TAGS", "extra")
	for _, scope := range []string{"include", "exclude", "invalid"} {
		t.Setenv("ENTMOOT_GO_TESTS", scope)
		want := []string{"-e", "ENTMOOT_GO_SINGLE_MODULE=1", "-e", "ENTMOOT_GO_TAGS=extra", "-e", "ENTMOOT_GO_TESTS=" + scope}
		if got := sidecarEnvironment(); !reflect.DeepEqual(got, want) {
			t.Fatalf("scope %q: got %v, want %v", scope, got, want)
		}
	}
	t.Setenv("ENTMOOT_GO_TESTS", "")
	if got := sidecarEnvironment(); len(got) != 4 {
		t.Fatalf("unset scope was invented: %v", got)
	}
}
