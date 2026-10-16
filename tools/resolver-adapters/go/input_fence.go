package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"sync"

	"github.com/zeebo/blake3"
	"golang.org/x/tools/go/packages"
)

// A selected build's native package plan is frozen before type checking.
// This includes external source, ignored build selections, embed inputs and
// package directory entries. It does not claim cgo or arbitrary build closure.
type inputFence struct {
	mu          sync.Mutex
	Digests     map[string]string
	Directories map[string][]string
	packages    map[string]bool
}

func newInputFence() *inputFence {
	return &inputFence{Digests: map[string]string{}, Directories: map[string][]string{}, packages: map[string]bool{}}
}

func digestBytes(data []byte) string { return fmt.Sprintf("blake3:%x", blake3.Sum256(data)) }

func entries(directory string) ([]string, error) {
	rows, err := os.ReadDir(directory)
	if err != nil {
		return nil, err
	}
	result := make([]string, 0, len(rows))
	for _, row := range rows {
		result = append(result, row.Name()+":"+row.Type().String())
	}
	sort.Strings(result)
	return result, nil
}

func (f *inputFence) remember(file string) error {
	absolute, err := filepath.Abs(file)
	if err != nil {
		return err
	}
	if _, ok := f.Digests[absolute]; ok {
		return nil
	}
	data, err := os.ReadFile(absolute)
	if err != nil {
		return err
	}
	f.Digests[absolute] = digestBytes(data)
	directory := filepath.Dir(absolute)
	if _, ok := f.Directories[directory]; !ok {
		rows, err := entries(directory)
		if err != nil {
			return err
		}
		f.Directories[directory] = rows
	}
	return nil
}

func (f *inputFence) plan(config *packages.Config, patterns []string) error {
	preflight := *config
	preflight.Mode = packages.NeedName | packages.NeedFiles | packages.NeedCompiledGoFiles |
		packages.NeedImports | packages.NeedDeps | packages.NeedModule | packages.NeedEmbedFiles
	native, err := packages.Load(&preflight, patterns...)
	if err != nil {
		return err
	}
	var failure error
	packages.Visit(native, nil, func(pkg *packages.Package) {
		f.packages[pkg.ID] = true
		for _, nativeError := range pkg.Errors {
			failure = fmt.Errorf("native package plan: %s", nativeError)
		}
		files := append(append(append([]string{}, pkg.GoFiles...), pkg.CompiledGoFiles...), pkg.EmbedFiles...)
		files = append(files, pkg.IgnoredFiles...)
		files = append(files, pkg.OtherFiles...)
		if pkg.Module != nil {
			if pkg.Module.GoMod != "" {
				files = append(files, pkg.Module.GoMod)
				if sum := filepath.Join(filepath.Dir(pkg.Module.GoMod), "go.sum"); fileExists(sum) {
					files = append(files, sum)
				}
			}
			if pkg.Module.Replace != nil && pkg.Module.Replace.GoMod != "" {
				files = append(files, pkg.Module.Replace.GoMod)
			}
		}
		for _, file := range files {
			if err := f.remember(file); err != nil {
				failure = err
			}
		}
	})
	return failure
}

func fileExists(file string) bool {
	info, err := os.Stat(file)
	return err == nil && info.Mode().IsRegular()
}

func (f *inputFence) parse(set *token.FileSet, filename string, source []byte) (*ast.File, error) {
	f.mu.Lock()
	expected, ok := f.Digests[filename]
	f.mu.Unlock()
	if !ok || digestBytes(source) != expected {
		return nil, fmt.Errorf("input outside or changed since native plan: %s", filename)
	}
	return parser.ParseFile(set, filename, source, parser.ParseComments|parser.AllErrors)
}

func (f *inputFence) verify() error {
	for file, expected := range f.Digests {
		bytes, err := os.ReadFile(file)
		if err != nil {
			return err
		}
		if digestBytes(bytes) != expected {
			return fmt.Errorf("input changed during native bind: %s", file)
		}
	}
	for directory, expected := range f.Directories {
		actual, err := entries(directory)
		if err != nil {
			return err
		}
		a, _ := json.Marshal(actual)
		b, _ := json.Marshal(expected)
		if string(a) != string(b) {
			return fmt.Errorf("package directory changed during native bind: %s", directory)
		}
	}
	return nil
}
