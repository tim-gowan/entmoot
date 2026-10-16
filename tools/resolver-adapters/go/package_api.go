package main

import (
	"encoding/json"
	"go/types"
	"golang.org/x/tools/go/packages"
	"sort"
	"strings"
)

const apiPolicy = "go.types.declaration_signature.v2"

type packageAPI struct {
	surfaces      map[string][]map[string]any
	boundaries    map[types.Object]map[string]any
	references    []map[string]any
	referenceKeys map[string]bool
	complete      bool
}

func newPackageAPI() *packageAPI {
	return &packageAPI{
		surfaces: map[string][]map[string]any{}, boundaries: map[types.Object]map[string]any{},
		references: []map[string]any{}, referenceKeys: map[string]bool{}, complete: true,
	}
}

func (api *packageAPI) declaration(pkg *packages.Package, object types.Object, target Target) {
	if len(pkg.Errors) != 0 {
		api.complete = false
		return
	}
	if pkg.Module == nil {
		return
	}
	if target.Path != "" && api.surfaces[pkg.Module.Path] == nil {
		api.surfaces[pkg.Module.Path] = []map[string]any{}
	}
	if !object.Exported() {
		return
	}
	qualifier := func(p *types.Package) string { return p.Path() }
	contract := types.TypeString(object.Type(), qualifier)
	switch declaration := object.(type) {
	case *types.Func:
		if receiver := declaration.Type().(*types.Signature).Recv(); receiver != nil {
			contract += "\nreceiver:" + types.TypeString(receiver.Type(), qualifier)
		}
	case *types.TypeName:
		contract += "\nalias:" + types.ObjectString(declaration, qualifier)
		contract += "\nunderlying:" + types.TypeString(declaration.Type().Underlying(), qualifier)
		if named, ok := declaration.Type().(*types.Named); ok {
			for i := 0; i < named.TypeParams().Len(); i++ {
				parameter := named.TypeParams().At(i)
				contract += "\nconstraint:" + types.TypeString(parameter.Constraint(), qualifier)
			}
			var methods []string
			for i := 0; i < named.NumMethods(); i++ {
				method := named.Method(i)
				if method.Exported() {
					methods = append(methods, types.ObjectString(method, qualifier))
				}
			}
			sort.Strings(methods)
			contract += "\nmethods:" + strings.Join(methods, "\n")
		}
	default:
		return
	}
	symbol := packageQualifiedName(pkg, object)
	coordinate := map[string]any{"ecosystem": "go", "name": pkg.Module.Path}
	api.boundaries[object] = map[string]any{
		"target":          map[string]any{"package": coordinate, "symbol": map[string]any{"scheme": "go.compiler.symbol", "spelling": symbol}},
		"boundary_policy": apiPolicy, "boundary_text": contract,
	}
	if target.Path != "" {
		api.surfaces[pkg.Module.Path] = append(api.surfaces[pkg.Module.Path], map[string]any{
			"path": target.Path, "start_byte": target.Start, "end_byte": target.End,
			"symbol": symbol, "boundary_policy": apiPolicy, "boundary_text": contract,
		})
	}
}

func (api *packageAPI) reference(call Call, object types.Object) error {
	if call.Intrinsic {
		return nil
	}
	if len(call.Targets) != 1 || call.Gap != "" {
		api.complete = false
		return nil
	}
	if call.Targets[0].Path != "" || call.Targets[0].PackageRole == "stdlib" {
		return nil
	}
	boundary := api.boundaries[object]
	if boundary == nil {
		api.complete = false
		return nil
	}
	reference := map[string]any{
		"path": call.Path, "start_byte": call.Start, "end_byte": call.End,
		"target": boundary["target"], "boundary_policy": apiPolicy, "boundary_text": boundary["boundary_text"],
	}
	// Test variants repeat production references as well as call facts. Use the
	// whole fact: a changed boundary at the same site must remain a conflict.
	encoded, err := json.Marshal(reference)
	if err != nil {
		api.complete = false
		return err
	}
	key := string(encoded)
	if !api.referenceKeys[key] {
		api.references = append(api.references, reference)
		api.referenceKeys[key] = true
	}
	return nil
}

func (api *packageAPI) result(invalid bool) map[string]any {
	names := make([]string, 0, len(api.surfaces))
	for name := range api.surfaces {
		names = append(names, name)
	}
	sort.Strings(names)
	surfaces := []map[string]any{}
	for _, name := range names {
		exports := api.surfaces[name]
		sort.Slice(exports, func(i, j int) bool { return exports[i]["symbol"].(string) < exports[j]["symbol"].(string) })
		// The sidecar may load selected patterns and nested modules. Positive
		// free-function boundaries are usable; package-wide absence is unqualified.
		surfaces = append(surfaces, map[string]any{
			"package":          map[string]any{"ecosystem": "go", "name": name},
			"export_universes": []map[string]any{{"symbol_scheme": "go.compiler.symbol", "boundary_policy": apiPolicy}},
			"completeness":     "partial", "exports": exports,
			"limitations": []string{"selected_build_declarations_only", "runtime_dispatch_and_dependency_closure_unassessed"},
		})
	}
	return map[string]any{"surfaces": surfaces, "references": api.references,
		"references_complete": api.complete && !invalid,
		"limitations":         []string{"signature_equality_excludes_named_type_definition_and_runtime_behavior"}}
}
