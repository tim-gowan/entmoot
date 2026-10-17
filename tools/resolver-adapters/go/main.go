// External compiler producer: module-aware go/packages and go/types.
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/types"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

type Request struct {
	Canon    bool     `json:"canon"`
	Root     string   `json:"root"`
	Patterns []string `json:"patterns"`
	Flags    []string `json:"flags"`
	Plan     bool     `json:"plan"`
}
type Target struct {
	OriginFile     string `json:"origin_file,omitempty"`
	Path           string `json:"path,omitempty"`
	Start          int    `json:"start_byte"`
	End            int    `json:"end_byte"`
	Symbol         string `json:"symbol"`
	Kind           string `json:"kind"`
	PackageRole    string `json:"package_role,omitempty"`
	Ecosystem      string `json:"ecosystem,omitempty"`
	ModulePath     string `json:"module_path,omitempty"`
	PackageVersion string `json:"package_version,omitempty"`
}

func packageQualifiedName(pkg *packages.Package, obj types.Object) string {
	if function, ok := obj.(*types.Func); ok {
		return function.FullName()
	}
	name := obj.Name()
	if pkg != nil && pkg.PkgPath != "" && name != "" {
		return pkg.PkgPath + "." + name
	}
	if name != "" {
		return name
	}
	return obj.String()
}

// enrichTarget attaches package role / module coordinates. PURL canonicalization
// happens in Rust via the packageurl crate — do not hand-build pkg: strings here.
func enrichTarget(pkg *packages.Package, obj types.Object, target Target, rootModules map[string]bool) Target {
	target.Ecosystem = "go"
	if obj != nil {
		target.Symbol = packageQualifiedName(pkg, obj)
	}
	if pkg != nil && pkg.Module != nil {
		target.ModulePath = pkg.Module.Path
		if pkg.Module.Version != "" && pkg.Module.Version != "devel" {
			target.PackageVersion = pkg.Module.Version
		}
		if rootModules[pkg.Module.Path] {
			target.PackageRole = "application"
		} else {
			target.PackageRole = "module"
		}
		return target
	}
	if pkg != nil && pkg.Module == nil {
		target.PackageRole = "stdlib"
		return target
	}
	if obj != nil && obj.Pkg() != nil {
		pkgPath := obj.Pkg().Path()
		target.Symbol = packageQualifiedName(nil, obj)
		if isStdImportPath(pkgPath) {
			target.PackageRole = "stdlib"
		} else {
			target.PackageRole = "module"
		}
		return target
	}
	target.PackageRole = "unknown"
	return target
}

func isStdImportPath(path string) bool {
	if path == "C" || path == "unsafe" {
		return true
	}
	first := path
	if idx := strings.IndexByte(path, '/'); idx >= 0 {
		first = path[:idx]
	}
	return !strings.Contains(first, ".")
}

func packageForObject(allPkgs []*packages.Package, object types.Object) *packages.Package {
	pkg := object.Pkg()
	if pkg == nil {
		return nil
	}
	for _, candidate := range allPkgs {
		if candidate.Types != nil && candidate.Types == pkg {
			return candidate
		}
	}
	return nil
}

type Call struct {
	Dispatch  map[string]any `json:"dispatch,omitempty"`
	Owner     *Target        `json:"owner,omitempty"`
	Path      string         `json:"path"`
	Start     int            `json:"start_byte"`
	End       int            `json:"end_byte"`
	Targets   []Target       `json:"targets"`
	Gap       string         `json:"gap,omitempty"`
	Intrinsic bool           `json:"intrinsic,omitempty"`
}
type Result struct {
	BuildContext    map[string]any `json:"build_context"`
	CompilerVersion string         `json:"compiler_version"`
	Calls           []Call         `json:"calls"`
	Inputs          []string       `json:"inputs"`
	Diagnostics     []string       `json:"diagnostics"`
	Invalid         bool           `json:"invalid_environment"`
}

func solverEnv() []string {
	// Safe default: offline + pinned local toolchain. Docker/compose prewarms the
	// module cache, then the same offline load succeeds. Overrides are intentional
	// escape hatches for Trusted dogfood (ENTMOOT_GO_GOPROXY / ENTMOOT_GO_GOTOOLCHAIN).
	proxy := envOr("ENTMOOT_GO_GOPROXY", "off")
	toolchain := envOr("ENTMOOT_GO_GOTOOLCHAIN", "local")
	return append(os.Environ(),
		"GOPROXY="+proxy,
		"GOTOOLCHAIN="+toolchain,
		"CGO_ENABLED=0",
		"GOSUMDB=off",
		"GOPACKAGESDRIVER=off",
		// Docker bind-mounts of git worktrees often lack a usable .git; VCS
		// stamping must stay off or packages.Load fails with exit 128.
		"GOFLAGS=-buildvcs=false",
		"GOENV=off",
	)
}

func envOr(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

// discoverModuleRoots returns directories that contain go.mod under root.
// Nested modules are included; when root itself is a module it is first.
func discoverModuleRoots(root string) ([]string, error) {
	var modules []string
	err := filepath.WalkDir(root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.IsDir() {
			name := entry.Name()
			// Local data/tool caches and scratch trees are not module roots.
			// An explicitly selected root may itself have a dot-prefixed name.
			if path != root && strings.HasPrefix(name, ".") {
				return filepath.SkipDir
			}
			// Nested example modules often pull unpublished deps under GOPROXY=off;
			// inventory the primary module tree, not demo apps.
			if name == "vendor" || name == "node_modules" || name == ".git" || name == "testdata" ||
				name == "_examples" || name == "examples" || name == "example" {
				return filepath.SkipDir
			}
			return nil
		}
		if entry.Name() == "go.mod" {
			modules = append(modules, filepath.Dir(path))
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	sort.Strings(modules)
	return modules, nil
}

func main() {
	var req Request
	if err := json.NewDecoder(os.Stdin).Decode(&req); err != nil {
		panic(err)
	}
	if len(req.Patterns) == 0 {
		req.Patterns = []string{"./..."}
	}
	if tags := os.Getenv("ENTMOOT_GO_TAGS"); tags != "" {
		req.Flags = append(req.Flags, "-tags="+tags)
	}
	for _, flag := range req.Flags {
		if !strings.HasPrefix(flag, "-tags=") {
			fmt.Fprintln(os.Stderr, "only explicit -tags= build flags are supported")
			os.Exit(2)
		}
	}
	// Inventory includes test files. Select their native build variants unless
	// the operator explicitly requests production-only scope.
	testScope := envOr("ENTMOOT_GO_TESTS", "include")
	if testScope != "include" && testScope != "exclude" {
		fmt.Fprintln(os.Stderr, "ENTMOOT_GO_TESTS must be include or exclude")
		os.Exit(2)
	}

	env := solverEnv()
	result := Result{CompilerVersion: runtime.Version(), Calls: []Call{}, Inputs: []string{}, Diagnostics: []string{}}
	contextCommand := exec.Command("go", "env", "-json", "GOOS", "GOARCH", "GOROOT", "GOVERSION", "GOWORK", "GOAMD64", "GO386", "CGO_ENABLED", "GOTOOLCHAIN")
	contextCommand.Env, contextCommand.Dir = env, req.Root
	contextBytes, contextError := contextCommand.Output()
	if contextError != nil || json.Unmarshal(contextBytes, &result.BuildContext) != nil {
		result.Invalid = true
		result.Diagnostics = append(result.Diagnostics, "cannot establish Go build context")
	}

	moduleRoots, discoverErr := discoverModuleRoots(req.Root)
	if result.BuildContext == nil {
		result.BuildContext = map[string]any{}
	}
	if os.Getenv("ENTMOOT_GO_SINGLE_MODULE") == "1" {
		moduleRoots = []string{req.Root}
		result.BuildContext["module_scope"] = "single_module.v1"
	}
	if discoverErr != nil {
		result.Invalid = true
		result.Diagnostics = append(result.Diagnostics, discoverErr.Error())
	}
	if len(moduleRoots) == 0 {
		// Fall back to treating root as a single module (fixture / go.work layouts).
		moduleRoots = []string{req.Root}
	}

	inputs := map[string]bool{}
	fence := newInputFence()
	if workspace, ok := result.BuildContext["GOWORK"].(string); ok && workspace != "" && workspace != "off" {
		inputs[workspace] = true
		if _, err := os.Stat(workspace + ".sum"); err == nil {
			inputs[workspace+".sum"] = true
		}
	}
	if executable, err := exec.LookPath("go"); err == nil {
		inputs[executable] = true
	}
	if executable, err := os.Executable(); err == nil {
		inputs[executable] = true
	}
	for input := range inputs {
		if err := fence.remember(input); err != nil {
			result.Invalid = true
			result.Diagnostics = append(result.Diagnostics, err.Error())
		}
	}
	targets := map[types.Object]Target{}
	api := newPackageAPI()
	rootModules := map[string]bool{}
	relative := func(path string) string {
		p, e := filepath.Rel(req.Root, path)
		if e != nil || p == ".." || strings.HasPrefix(p, ".."+string(filepath.Separator)) {
			return ""
		}
		return filepath.ToSlash(p)
	}

	var allPkgs []*packages.Package
	modulesLoaded := 0
	for _, moduleRoot := range moduleRoots {
		cfg := &packages.Config{
			Tests:      testScope == "include",
			Dir:        moduleRoot,
			Env:        env,
			Mode:       packages.LoadAllSyntax | packages.NeedModule | packages.NeedEmbedFiles,
			BuildFlags: append([]string{"-mod=readonly"}, req.Flags...),
		}
		if err := fence.plan(cfg, req.Patterns); err != nil {
			result.Invalid = true
			result.Diagnostics = append(result.Diagnostics, err.Error())
			continue
		}
		cfg.ParseFile = fence.parse
		pkgs, err := packages.Load(cfg, req.Patterns...)
		if err != nil {
			result.Diagnostics = append(result.Diagnostics, fmt.Sprintf("%s: %v", moduleRoot, err))
			continue
		}
		moduleHadFatal := false
		packages.Visit(pkgs, nil, func(p *packages.Package) {
			if !fence.packages[p.ID] {
				result.Invalid = true
				result.Diagnostics = append(result.Diagnostics, "package outside native build plan: "+p.ID)
			}
			for _, e := range p.Errors {
				result.Invalid = true
				result.Diagnostics = append(result.Diagnostics, e.Error())
				// Incomplete types still yield defs/refs when Syntax is present.
				if p.TypesInfo == nil || len(p.Syntax) == 0 {
					moduleHadFatal = true
				}
			}
			for _, path := range append(append(p.GoFiles, p.CompiledGoFiles...), p.EmbedFiles...) {
				inputs[path] = true
			}
			if p.Module != nil && p.Module.GoMod != "" {
				inputs[p.Module.GoMod] = true
				sum := filepath.Join(filepath.Dir(p.Module.GoMod), "go.sum")
				if _, e := os.Stat(sum); e == nil {
					inputs[sum] = true
				}
				if relative(p.Module.GoMod) != "" {
					rootModules[p.Module.Path] = true
				}
			}
			for _, file := range p.Syntax {
				ast.Inspect(file, func(n ast.Node) bool {
					// Interface methods declare callable signatures, not values or
					// executable bodies. Preserve their exact declaration span.
					if field, ok := n.(*ast.Field); ok && p.TypesInfo != nil {
						for _, name := range field.Names {
							if object, ok := p.TypesInfo.Defs[name].(*types.Func); ok {
								pos, end := p.Fset.PositionFor(field.Pos(), false), p.Fset.PositionFor(field.End(), false)
								targets[object] = enrichTarget(p, object, Target{OriginFile: pos.Filename, Path: relative(pos.Filename), Start: pos.Offset, End: end.Offset, Kind: "callable_signature"}, rootModules)
							}
						}
					}
					if declaration, ok := n.(*ast.TypeSpec); ok && p.TypesInfo != nil {
						if object := p.TypesInfo.Defs[declaration.Name]; object != nil {
							pos, end := p.Fset.PositionFor(declaration.Pos(), false), p.Fset.PositionFor(declaration.End(), false)
							target := enrichTarget(p, object, Target{OriginFile: pos.Filename, Path: relative(pos.Filename), Start: pos.Offset, End: end.Offset, Kind: "type"}, rootModules)
							targets[object] = target
							if len(p.Errors) == 0 && !strings.Contains(p.ID, " [") {
								api.declaration(p, object, target)
							}
						}
					}
					f, ok := n.(*ast.FuncDecl)
					if !ok || p.TypesInfo == nil {
						return true
					}
					obj := p.TypesInfo.Defs[f.Name]
					if obj == nil {
						return true
					}
					pos, end := p.Fset.PositionFor(f.Pos(), false), p.Fset.PositionFor(f.End(), false)
					kind := "callable"
					if f.Body == nil {
						kind = "callable_signature"
					}
					targets[obj] = enrichTarget(p, obj, Target{
						OriginFile: pos.Filename,
						Path:       relative(pos.Filename),
						Start:      pos.Offset,
						End:        end.Offset,
						Kind:       kind,
					}, rootModules)
					if len(p.Errors) == 0 && !strings.Contains(p.ID, " [") {
						api.declaration(p, obj, targets[obj])
					}
					return true
				})
			}
		})
		if !moduleHadFatal {
			modulesLoaded++
			allPkgs = append(allPkgs, pkgs...)
		}
	}
	if modulesLoaded == 0 {
		result.Invalid = true
		if len(result.Diagnostics) == 0 {
			result.Diagnostics = append(result.Diagnostics, "no Go modules loaded under root")
		}
	}

	if !req.Plan && modulesLoaded > 0 {
		for _, p := range allPkgs {
			if p.TypesInfo == nil {
				continue
			}
			for _, file := range p.Syntax {
				var ancestors []ast.Node
				ast.Inspect(file, func(n ast.Node) bool {
					if n == nil {
						ancestors = ancestors[:len(ancestors)-1]
						return true
					}
					ancestors = append(ancestors, n)
					if req.Canon && len(p.Errors) == 0 {
						if expression, ok := n.(ast.Expr); ok {
							if typeOf := p.TypesInfo.TypeOf(expression); typeOf != nil {
								if basic, ok := typeOf.Underlying().(*types.Basic); ok && basic.Info()&types.IsUntyped == 0 {
									var scalar map[string]any
									if basic.Kind() == types.Bool {
										scalar = map[string]any{"kind": "bool"}
									}
									if basic.Info()&types.IsInteger != 0 {
										scalar = map[string]any{"kind": "integer", "bits": p.TypesSizes.Sizeof(basic) * 8, "signed": basic.Info()&types.IsUnsigned == 0}
									}
									if scalar != nil {
										pos, end := p.Fset.PositionFor(n.Pos(), false), p.Fset.PositionFor(n.End(), false)
										rows, _ := result.BuildContext["canon_expressions"].([]map[string]any)
										if len(rows) < 100000 {
											result.BuildContext["canon_expressions"] = append(rows, map[string]any{"path": relative(pos.Filename), "start_byte": pos.Offset, "end_byte": end.Offset, "sort": scalar, "overflow": "wrap"})
										}
									}
								}
							}
						}
					}
					call, ok := n.(*ast.CallExpr)
					if !ok {
						return true
					}
					pos, end := p.Fset.PositionFor(call.Pos(), false), p.Fset.PositionFor(call.End(), false)
					row := Call{Path: relative(pos.Filename), Start: pos.Offset, End: end.Offset, Targets: []Target{}}
					for i := len(ancestors) - 1; i >= 0; i-- {
						switch owner := ancestors[i].(type) {
						case *ast.FuncDecl:
							start, stop := p.Fset.PositionFor(owner.Pos(), false), p.Fset.PositionFor(owner.End(), false)
							row.Owner = &Target{Path: relative(start.Filename), Start: start.Offset, End: stop.Offset}
						case *ast.FuncLit:
							start, stop := p.Fset.PositionFor(owner.Pos(), false), p.Fset.PositionFor(owner.End(), false)
							row.Owner = &Target{Path: relative(start.Filename), Start: start.Offset, End: stop.Offset}
						}
						if row.Owner != nil {
							break
						}
					}
					var object types.Object
					// A top-level initializer has no enclosing callable. Do not
					// manufacture an invalid span that erases its compiler fact.
					if row.Path == "" {
						return true // synthetic test main / out-of-root source
					}
					expression := call.Fun
					switch indexed := expression.(type) {
					case *ast.IndexExpr:
						expression = indexed.X
					case *ast.IndexListExpr:
						expression = indexed.X
					}
					switch fun := expression.(type) {
					case *ast.Ident:
						object = p.TypesInfo.Uses[fun]
					case *ast.SelectorExpr:
						object = p.TypesInfo.Uses[fun.Sel]
					}
					if value, ok := p.TypesInfo.Types[call.Fun]; ok && value.IsType() {
						// Conversion syntax is intrinsic, not an indirect function
						// call. Its symbolic target never denotes an executable body.
						row.Intrinsic = true
						row.Targets = append(row.Targets, Target{
							Symbol: "conversion:" + types.TypeString(value.Type, func(pkg *types.Package) string { return pkg.Path() }),
							Kind:   "intrinsic", PackageRole: "stdlib", Ecosystem: "go",
						})
					} else {
						switch function := object.(type) {
						case *types.Func:
							if signature, ok := function.Type().(*types.Signature); ok && signature.Recv() != nil {
								if _, ok := signature.Recv().Type().Underlying().(*types.Interface); ok {
									row.Gap = "dynamic_dispatch"
								}
							}
							// Concrete Func binds stay unbound-dispatch-open only when
							// selection is incomplete; do not force dynamic_dispatch on
							// every resolved Func (that blocked repository_callable joins).
							if target, ok := targets[object]; ok {
								row.Targets = append(row.Targets, target)
								if signature, ok := function.Type().(*types.Signature); ok && target.Kind == "callable" && row.Owner != nil && row.Gap == "" && len(p.Errors) == 0 {
									if signature.Recv() == nil {
										row.Dispatch = map[string]any{
											"kind": "closed", "policy_id": "entmoot.go.direct-function.v1", "evidence_pointer": "/calls",
											"assumptions": []string{"selected Go package inputs, build tags, GOOS and GOARCH", "explicit source calls; excludes unsafe, foreign-code rewriting and implicit calls"},
										}
									} else if _, iface := signature.Recv().Type().Underlying().(*types.Interface); !iface {
										row.Dispatch = map[string]any{
											"kind": "closed", "policy_id": "entmoot.go.direct-method.v1", "evidence_pointer": "/calls",
											"assumptions": []string{"selected Go package inputs, build tags, GOOS and GOARCH", "source-level concrete method on a non-interface receiver; interface, unsafe, foreign and implicit calls are outside this qualification"},
										}
									}
								}
							} else {
								declaration := p.Fset.PositionFor(object.Pos(), false)
								row.Targets = append(row.Targets, enrichTarget(
									packageForObject(allPkgs, object),
									object,
									Target{
										OriginFile: declaration.Filename,
										Path:       relative(declaration.Filename),
										Start:      declaration.Offset,
										End:        declaration.Offset,
										Kind:       "callable_signature",
									},
									rootModules,
								))
							}
						case *types.Builtin:
							row.Intrinsic = true
							row.Targets = append(row.Targets, Target{
								Symbol:      object.String(),
								Kind:        "intrinsic",
								PackageRole: "stdlib",
								Ecosystem:   "go",
							})
						default:
							row.Gap = "indirect_call"
						}
					}
					if len(p.Errors) == 0 {
						if err := api.reference(row, object); err != nil {
							result.Invalid = true
							result.Diagnostics = append(result.Diagnostics, err.Error())
						}
					} else {
						api.complete = false
					}
					result.Calls = append(result.Calls, row)
					return true
				})
			}
		}
	}
	if result.BuildContext == nil {
		result.BuildContext = map[string]any{}
	}
	// Test-augmented packages repeat production files. Coalesce only byte-identical
	// observations; disagreements at a site remain separate native facts.
	seenCalls := map[string]bool{}
	uniqueCalls := make([]Call, 0, len(result.Calls))
	for _, call := range result.Calls {
		encoded, err := json.Marshal(call)
		if err != nil {
			result.Invalid = true
			result.Diagnostics = append(result.Diagnostics, err.Error())
			continue
		}
		key := string(encoded)
		if !seenCalls[key] {
			uniqueCalls = append(uniqueCalls, call)
			seenCalls[key] = true
		}
	}
	result.Calls = uniqueCalls
	result.BuildContext["test_scope"] = testScope
	result.BuildContext["test_scope_policy"] = "go.native_test_variants.v1"
	if err := fence.verify(); err != nil {
		result.Invalid = true
		result.Diagnostics = append(result.Diagnostics, err.Error())
	}
	contextAfter := exec.Command("go", contextCommand.Args[1:]...)
	contextAfter.Env, contextAfter.Dir = env, req.Root
	if after, err := contextAfter.Output(); err != nil || string(after) != string(contextBytes) {
		result.Invalid = true
		result.Diagnostics = append(result.Diagnostics, "Go build context changed during native bind")
	}
	result.BuildContext["package_api"] = api.result(result.Invalid || len(result.Diagnostics) != 0)
	result.BuildContext["source_input_digests"] = fence.Digests
	result.BuildContext["package_directory_entries"] = fence.Directories
	result.BuildContext["build_flags"] = req.Flags
	result.BuildContext["input_closure"] = "native_package_plan_sources_and_package_directories_revalidated;cgo_and_unobserved_driver_inputs_open"
	for file := range fence.Digests {
		inputs[file] = true
	}
	for path := range inputs {
		result.Inputs = append(result.Inputs, path)
	}
	sort.Strings(result.Inputs)
	sort.Strings(result.Diagnostics)
	sort.Slice(result.Calls, func(i, j int) bool {
		a, b := result.Calls[i], result.Calls[j]
		if a.Path != b.Path {
			return a.Path < b.Path
		}
		return a.Start < b.Start
	})
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
