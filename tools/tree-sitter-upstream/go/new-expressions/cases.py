"""Authored syntax and runtime expectations; never generated from parser output."""


def cases():
    rows = [
        ("integer", "", "new(42)", '*p != 42', "go1.26"),
        ("string", "", 'new("answer")', '*p != "answer"', "go1.26"),
        ("binary", "", "new(20 + 22)", '*p != 42', "go1.26"),
        ("call-once", "var calls int\nfunc answer() int { calls++; return 42 }", "new(answer())", '*p != 42 || calls != 1', "go1.26"),
        ("boolean", "var x = 42", "new(x > 0)", '!*p', "go1.26"),
        ("conversion", "", "new(int64(42))", '*p != int64(42)', "go1.26"),
        ("slice-value", "", "new([]int{4, 2})", '(*p)[1] != 2', "go1.26"),
        ("struct-value", "", "new(struct{ N int }{42})", 'p.N != 42', "go1.26"),
        ("historical-type", "", "new(int)", '*p != 0', "go1.14"),
        ("historical-map-type", "", "new(map[string]int)", '*p != nil', "go1.14"),
        ("historical-make-slice", "", "make([]int, 2)", 'len(p) != 2', "go1.14"),
        ("historical-make-channel", "", "make(chan int, 2)", 'cap(p) != 2', "go1.14"),
        ("shadow-new", "func new(x int) int { return x + 1 }", "new(41)", 'p != 42', "go1.14"),
        ("shadow-make", "func make(x int) int { return x + 1 }", "make(41)", 'p != 42', "go1.14"),
        ("generic-new", "func new[T any](x T) T { return x }", "new[int](42)", 'p != 42', "go1.18"),
        ("generic-make", "func make[T any](x T) T { return x }", "make[int](42)", 'p != 42', "go1.18"),
        ("variadic-shadow", "func new(xs ...int) int {return xs[0] + xs[1]}", "new(20, 22)", 'p != 42', "go1.14"),
        ("selector-value", "var box = struct{ Value int }{42}", "new(box.Value)", '*p != 42', "go1.26"),
    ]
    for identifier, declarations, expression, wrong, mode in rows:
        source = ("// π\npackage main\n" + declarations + "\nfunc main() { p := " + expression +
                  '; if ' + wrong + ' {panic("wrong result")}; if After()!=7 {panic("sibling")}; println("oracle:ok") }\nfunc After() int {return 7}\n')
        names = (["answer"] if identifier == "call-once" else ["new"] if "func new" in declarations else
                 ["make"] if "func make" in declarations else []) + ["main", "After"]
        fields = {"function": expression.split("[", 1)[0].split("(", 1)[0]}
        if identifier.startswith("generic-"):
            fields["type_arguments"] = "[int]"
        yield {"id": identifier, "source": source, "mode": mode, "compiler_accepts": True, "parse_accepts": True,
               "declarations": names, "sites": [{"text": expression, "kind": "call_expression", "fields": fields}]}
    # These regressions were found in full examples. The names are ordinary
    # variables here, even though a competing parse can start a builtin call.
    for identifier, declarations, expression, wrong, kind, fields in [
        ("new-index", "var new = []int{42}", "new[0]", "p != 42", "index_expression", {"operand": "new", "index": "0"}),
        ("make-index", "var make = []int{42}", "make[0]", "p != 42", "index_expression", {"operand": "make", "index": "0"}),
        ("new-slice", "var new = []int{40,42}", "new[1:]", "len(p)!=1 || p[0]!=42", "slice_expression", {"operand": "new"}),
        ("make-full-slice", "var make = []int{42,40}", "make[:1:2]", "len(p)!=1 || p[0]!=42", "slice_expression", {"operand": "make"}),
        ("new-indexed-call", "var new = []func()int{func()int{return 42}}", "new[0]()", "p != 42", "call_expression", {"function": "new[0]"}),
    ]:
        yield {"id": identifier, "source": "package main\n" + declarations + "\nfunc main(){p:=" + expression +
               '; if ' + wrong + '{panic("wrong")}; println("oracle:ok")}\nfunc After()int{return 7}\n',
               "mode": "go1.14", "compiler_accepts": True, "parse_accepts": True, "declarations": ["main", "After"],
               "sites": [{"text": expression, "kind": kind, "fields": fields}]}
    yield {"id": "parameter-new-index", "source": 'package main\nfunc get(new []int)int{return new[0]}\nfunc main(){if get([]int{42})!=42{panic("wrong")}; println("oracle:ok")}\nfunc After()int{return 7}\n',
           "mode": "go1.14", "compiler_accepts": True, "parse_accepts": True, "declarations": ["get", "main", "After"],
           "sites": [{"text": "new[0]", "kind": "index_expression", "fields": {"operand": "new", "index": "0"}}]}
    for identifier, expression, mode, parses, diagnostics in [
        ("old-mode", "new(42)", "go1.25", True, ["go1.26"]),
        ("untyped-nil", "new(nil)", "go1.27", True, ["use of untyped nil"]),
        ("invalid-make", "make(42)", "go1.27", True, ["not a type"]),
        ("builtin-type-arguments", "new[int](42)", "go1.27", True, ["cannot index new"]),
        ("missing-operand", "new(1 +)", "go1.27", False, ["syntax error"]),
    ]:
        yield {"id": identifier, "source": "package main\nfunc main(){ _ = " + expression + " }\nfunc After() int {return 7}\n",
               "mode": mode, "compiler_accepts": False, "diagnostic_contains": diagnostics, "parse_accepts": parses,
               "declarations": ["main", "After"]}
