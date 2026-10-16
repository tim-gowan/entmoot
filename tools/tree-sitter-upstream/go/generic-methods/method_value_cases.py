"""Method instantiation reproducers minimized independently from Go's examples."""


def cases():
    ordinary = "type Box struct{}\nfunc (Box) Map[A, B any](x A, y B) B { return y }"
    for identifier, declarations, expression, invoke, type_text in [
        ("composite-method-value", ordinary, 'Box{}.Map[int, string]', 'f(17,"ok")', 'Box{}.Map'),
        ("pointer-method-value", ordinary, '(&Box{}).Map[int, string]', 'f(17,"ok")', '(&Box{}).Map'),
        ("call-receiver", ordinary + "\nfunc box() Box { return Box{} }", 'box().Map[int, string]', 'f(17,"ok")', 'box().Map'),
        ("parenthesized-method-value", ordinary, '(Box{}.Map)[int, string]', 'f(17,"ok")', '(Box{}.Map)'),
        ("method-expression", ordinary, 'Box.Map[int, string]', 'f(Box{},17,"ok")', 'Box.Map'),
        ("trailing-comma", ordinary, 'Box{}.Map[int, string,]', 'f(17,"ok")', 'Box{}.Map'),
        ("generic-receiver-value", "type Box[T any] struct{}\nfunc (Box[T]) Map[A,B any](x A,y B) B {return y}",
         'Box[bool]{}.Map[int, string]', 'f(17,"ok")', 'Box[bool]{}.Map'),
        ("historical-generic-function", "func Map[A,B any](x A,y B) B {return y}", 'Map[int, string]', 'f(17,"ok")', 'Map'),
    ]:
        source = ("// π\r\npackage main\r\n" + declarations.replace("\n", "\r\n") + '\r\nfunc main() { f := ' + expression +
                  '; if ' + invoke + ' != "ok" {panic("wrong")}; if After()!=7 {panic("sibling")}; println("oracle:ok") }\r\nfunc After() int {return 7}\r\n')
        yield {"id": identifier, "source": source, "mode": "go1.18" if identifier.startswith("historical") else "go1.27",
               "compiler_accepts": identifier != "parenthesized-method-value", "parse_accepts": True,
               "diagnostic_contains": ["cannot use generic function", "without instantiation"] if identifier == "parenthesized-method-value" else [],
               "declarations": ["Map"] + (["box"] if identifier == "call-receiver" else []) + ["main", "After"],
               "sites": [{"text": expression, "kind": "type_instantiation_expression", "fields": {"type": type_text}}]}
    yield {"id": "historical-nested-index", "source": 'package main\nfunc main(){ values:=[]string{"ok"}; indices:=[]int{0}; f:=values[indices[0]]; if f!="ok"{panic("wrong")}; println("oracle:ok") }\nfunc After() int{return 7}\n',
           "mode": "go1.14", "compiler_accepts": True, "parse_accepts": True, "declarations": ["main", "After"],
           "sites": [{"text": "values[indices[0]]", "kind": "index_expression", "fields": {"operand": "values", "index": "indices[0]"}}]}
    yield {"id": "numeric-multiple-index", "source": "package main\nfunc main(){ xs:=[]int{1,2}; _ = xs[0,1] }\nfunc After() int{return 7}\n",
           "mode": "go1.27", "compiler_accepts": False, "diagnostic_contains": ["syntax error"],
           "parse_accepts": False, "declarations": ["main", "After"]}
