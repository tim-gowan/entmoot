"""Authored Rust language/edition controls, independent of grammar output."""


def site(text, kind, **fields):
    return {"text": text, "kind": kind, "fields": fields}


def cases():
    result = []

    def add(identity, body="", *, declarations="", edition="2024", compiler="1.99.0",
            accepts=True, diagnostic_codes=(), diagnostic_text=(), sites=(), parse=True, crlf=False):
        source = declarations + '\nfn main() {\n' + body + '\nassert_eq!(after(), 7); println!("oracle:ok");\n}\nfn after() -> i32 { 7 }\n'
        if crlf:
            source = source.replace("\n", "\r\n")
        result.append({"id": identity, "source": source, "edition": edition, "compiler": compiler,
                       "compiler_accepts": accepts, "diagnostic_codes": list(diagnostic_codes), "diagnostic_text": list(diagnostic_text),
                       "parse_accepts": parse, "sites": list(sites), "sibling": "after"})

    for abi in ("C", "C-unwind"):
        declaration = 'unsafe extern "' + abi + '" fn add(mut values: ...) -> i32 { unsafe { values.next_arg::<i32>() + values.next_arg::<i32>() } }\n'
        for edition in ("2015", "2018", "2021", "2024"):
            add("variadic-" + abi.lower() + "-" + edition, "assert_eq!(unsafe { add(3i32, 4i32) }, 7);", declarations=declaration, edition=edition,
                sites=[site("mut values: ...", "variadic_parameter", pattern="values")])
    add("variadic-historical-compiler", declarations='unsafe extern "C" fn old(mut values: ...) -> i32 { 7 }\n',
        compiler="1.95.0", accepts=False, diagnostic_codes=["E0658"], sites=[site("mut values: ...", "variadic_parameter", pattern="values")])
    add("variadic-fixed-parameter", "assert_eq!(unsafe { add(3i32, 4i32) }, 7);",
        declarations='unsafe extern "C" fn add(first: i32, mut values: ...) -> i32 { first + unsafe { values.next_arg::<i32>() } }\n',
        sites=[site("first: i32", "parameter", pattern="first", type="i32"), site("mut values: ...", "variadic_parameter", pattern="values")])
    add("variadic-missing-pattern", declarations='unsafe extern "C" fn value(...) -> i32 { 7 }\n',
        accepts=False, diagnostic_text=["missing pattern"], sites=[site("...", "variadic_parameter")])
    add("variadic-ignored-pattern", "assert_eq!(unsafe { value(1i32, 2i32) }, 7);",
        declarations='unsafe extern "C" fn value(_: ...) -> i32 { 7 }\n', sites=[site("_: ...", "variadic_parameter", pattern="_")])
    add("variadic-wrong-abi", declarations='unsafe extern "Rust" fn value(_: ...) -> i32 { 7 }\n',
        accepts=False, diagnostic_text=['not supported for `extern "Rust"`'], sites=[site("_: ...", "variadic_parameter", pattern="_")])
    add("variadic-safe-function", declarations='extern "C" fn value(_: ...) -> i32 { 7 }\n', accepts=False,
        diagnostic_text=["unsafe"], sites=[site("_: ...", "variadic_parameter", pattern="_")])
    add("variadic-not-last", declarations='unsafe extern "C" fn value(_: ..., last: i32) -> i32 { last }\n',
        accepts=False, diagnostic_text=["last argument"], sites=[site("_: ...", "variadic_parameter", pattern="_")])
    add("variadic-read-unpromoted-float", declarations='unsafe extern "C" fn value(mut values: ...) -> f32 { unsafe { values.next_arg::<f32>() } }\n',
        accepts=False, diagnostic_codes=["E0277"], sites=[site("mut values: ...", "variadic_parameter", pattern="values")])
    add("variadic-pass-unpromoted-float", "unsafe { value(1f32); }",
        declarations='unsafe extern "C" fn value(_: ...) {}\n', accepts=False, diagnostic_codes=["E0617"])
    add("historical-extern-declaration", declarations='unsafe extern "C" { fn unlinked(first: i32, ...) -> i32; }\n',
        compiler="1.95.0", edition="2015", sites=[site("...", "variadic_parameter")])
    add("historical-2015-method", "assert_eq!(Item { value: 7 }.read(), 7);", edition="2015", compiler="1.95.0",
        declarations="struct Item { value: i32 }\nimpl Item { fn read(&self) -> i32 { self.value } }\n",
        sites=[site("fn read(&self) -> i32 { self.value }", "function_item", name="read")])
    add("async-2018", "let future = async move { 7i32 }; std::mem::drop(future);", edition="2018", compiler="1.95.0",
        sites=[site("async move { 7i32 }", "async_block")])
    add("const-generic-2021", "let value = Item::<2> { bytes: [3, 4] }; assert_eq!(value.bytes[0] + value.bytes[1], 7);", edition="2021", compiler="1.95.0",
        declarations="struct Item<const N: usize> { bytes: [u8; N] }\n")
    add("let-else-2021", "let Some(value) = Some(7) else { panic!(); }; assert_eq!(value, 7);", edition="2021", compiler="1.95.0")
    add("let-chain-2024", "if let Some(value) = Some(7) && value == 7 { } else { panic!(); }",
        sites=[site("let Some(value) = Some(7) && value == 7", "let_chain")])
    add("let-chain-old-edition", "if let Some(value) = Some(7) && value == 7 { } else { panic!(); }", edition="2021", accepts=False,
        diagnostic_text=["2024"], sites=[site("let Some(value) = Some(7) && value == 7", "let_chain")])
    add("inline-const", "let value = const { 3 + 4 }; assert_eq!(value, 7);", edition="2021", compiler="1.95.0",
        sites=[site("const { 3 + 4 }", "const_block")])
    add("c-string-literal", 'let value = c"abc"; assert_eq!(value.to_bytes(), b"abc");', edition="2021", compiler="1.95.0",
        sites=[site('c"abc"', "string_literal")])
    add("raw-pointer-borrow", "let value = 7; let pointer = &raw const value; assert_eq!(unsafe { *pointer }, 7);", edition="2021", compiler="1.95.0",
        sites=[site("&raw const value", "reference_expression", value="value")])
    add("async-closure-2024", "let closure = async |x: i32| x + 1; let future = closure(6); std::mem::drop(future);",
        sites=[site("async |x: i32| x + 1", "closure_expression")])
    add("raw-gen-identifier", "let r#gen = 7; assert_eq!(r#gen, 7);",
        sites=[site("let r#gen = 7;", "let_declaration", pattern="r#gen", value="7")])
    add("historical-gen-identifier", "let gen = 7; assert_eq!(gen, 7);", edition="2021", compiler="1.95.0",
        sites=[site("let gen = 7;", "let_declaration", pattern="gen", value="7")])
    add("reserved-gen-2024", "let gen = 7;", accepts=False, diagnostic_text=["reserved keyword"],
        sites=[site("let gen = 7;", "let_declaration", pattern="gen", value="7")])
    add("macro-syntax", "let value = sum!(3, 4); assert_eq!(value, 7);", declarations="macro_rules! sum { ($x:expr, $y:expr) => { $x + $y }; }\n",
        sites=[site("sum!(3, 4)", "macro_invocation", macro="sum")])
    add("unicode-crlf", "assert_eq!(λ(), 7);", declarations="fn λ() -> i32 { 7 }\n", edition="2015", compiler="1.95.0", crlf=True,
        sites=[site("fn λ() -> i32 { 7 }", "function_item", name="λ")])
    add("malformed-sibling", declarations="fn broken( }\n", accepts=False, diagnostic_text=["mismatched closing delimiter"], parse=False)
    return result
