"""Independently authored Go declaration, scope and historical-mode expectations."""

def declaration_shapes():
    ordinary = "type Box struct{}"
    for name, parameters, argument, main, imports in [
        ("inferred", "[T any]", "x T", 'if (Box{}).Map(17) != 17 { panic("wrong") }', ""),
        ("pointer-constraint", "[T *U, U any]", "x T", 'v:=17; if *(Box{}).Map[*int,int](&v) != 17 { panic("wrong") }', ""),
        ("slice-constraint", "[T ~[]U, U any]", "x T", 'if (Box{}).Map[[]int,int]([]int{17})[0] != 17 { panic("wrong") }', ""),
        ("map-constraint", "[T ~map[string]U, U any]", "x T", 'if (Box{}).Map[map[string]int,int](map[string]int{"v":17})["v"] != 17 { panic("wrong") }', ""),
        ("array-constraint", "[T ~[2]U, U any]", "x T", 'if (Box{}).Map[[2]int,int]([2]int{17,18})[0] != 17 { panic("wrong") }', ""),
        ("union-constraint", "[T interface{ ~int | ~int64 }]", "x T", 'if (Box{}).Map(int64(17)) != 17 { panic("wrong") }', ""),
        ("qualified-constraint", "[T cmp.Ordered]", "x T", 'if (Box{}).Map(17) != 17 { panic("wrong") }', 'import "cmp"'),
    ]:
        yield {"id": name, "types": ordinary, "imports": imports, "method": f"func (Box) Map{parameters}({argument}) T {{ return x }}",
               "type_parameters": parameters, "parameters": "(" + argument + ")", "main": main, "compiler": True, "parse": True, "mode": "go1.27"}
    yield {"id": "generic-receiver", "types": "type Box[T any] struct { Value T }", "method": "func (b Box[T]) Map[U any](f func(T) U) U { return f(b.Value) }",
           "type_parameters": "[U any]", "parameters": "(f func(T) U)", "main": 'if (Box[int]{17}).Map(func(x int) int {return x}) != 17 {panic("wrong")}', "compiler": True, "parse": True, "mode": "go1.27"}
    yield {"id": "historical-method", "types": ordinary, "method": "func (Box) Map(x int) int { return x }", "type_parameters": None,
           "parameters": "(x int)", "main": 'if (Box{}).Map(17) != 17 {panic("wrong")}', "compiler": True, "parse": True, "mode": "go1.14"}
    yield {"id": "historical-generic-function", "types": ordinary, "method": "func Map[T any](x T) T { return x }", "type_parameters": "[T any]",
           "parameters": "(x T)", "main": 'if Map(17) != 17 {panic("wrong")}', "compiler": True, "parse": True, "mode": "go1.18", "kind": "function_declaration"}
    for name, types, method, main, parses, mode in [
        ("historical-mode-rejects-generic-method", ordinary, "func (Box) Map[T any](x T) T {return x}", "", True, "go1.26"),
        ("duplicate-type-parameter", ordinary, "func (Box) Map[T any, T any](x T) T {return x}", "", True, "go1.27"),
        ("receiver-parameter-shadow", "type Box[T any] struct{}", "func (Box[T]) Map[T any](x T) T {return x}", "", True, "go1.27"),
        ("unsatisfied-constraint", ordinary, "func (Box) Map[T ~int](x T) T {return x}", 'Box{}.Map("wrong")', True, "go1.27"),
        ("generic-interface-method", "type Box interface { Map[T any](T) T }", "", "", False, "go1.27"),
        ("unclosed-parameters", ordinary, "func (Box) Map[T any(x T) T {return x}", "", False, "go1.27"),
    ]:
        yield {"id": name, "types": types, "method": method, "main": main, "compiler": False, "parse": parses, "mode": mode}
    yield {"id": "unicode-crlf", "types": ordinary, "method": "func (Box) Map[T any](δ T) T { return δ }", "type_parameters": "[T any]",
           "parameters": "(δ T)", "main": 'if (Box{}).Map(17) != 17 {panic("wrong")}', "compiler": True, "parse": True, "mode": "go1.27", "crlf": True}


def cases():
    diagnostics = {
        "historical-mode-rejects-generic-method": ["go1.27"],
        "duplicate-type-parameter": ["T redeclared"],
        "receiver-parameter-shadow": ["T redeclared"],
        "unsatisfied-constraint": ["string", "int"],
        "generic-interface-method": ["interface method", "type parameters"],
        "unclosed-parameters": ["syntax error"],
    }
    for case in declaration_shapes():
        main = case["main"] + "; " if case["main"] else ""
        source = ("// π\npackage main\n" + case.get("imports", "") + "\n" + case["types"] + "\n" + case["method"] +
                  '\nfunc After() int { return 7 }\nfunc main() { ' + main + 'if After()!=7 {panic("sibling lost")}; println("oracle:ok") }\n')
        if case.get("crlf"):
            source = source.replace("\n", "\r\n")
        fields = {key: case[key] for key in ("parameters", "type_parameters") if key in case}
        yield {"id": case["id"], "source": source, "mode": case["mode"],
               "compiler_accepts": case["compiler"], "diagnostic_contains": diagnostics.get(case["id"], []),
               "parse_accepts": case["parse"], "declarations": ["Map", "After", "main"],
               "sites": [{"text": case["method"], "kind": case.get("kind", "method_declaration"), "fields": fields}] if case["method"] else []}
