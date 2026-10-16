"""Independent C# expectations: grammar, compiler mode, runtime and SDK differ."""
from __future__ import annotations


def site(text, kind, **fields):
    return {"text": text, "kind": kind, "fields": fields}


def cases():
    result = []

    def add(identity, body="", *, prefix="", suffix="", declarations="", mode="14", sdk="10",
            file_mode=False, accepts=True, diagnostics=(), sites=(), parse=True,
            sdk_accepts=None, sdk_diagnostic=None, helper=None, crlf=False):
        source = prefix + declarations + "\nclass Program { static void Main() {\n" + body
        source += '\nSystem.Console.WriteLine("oracle:ok");\n} }\nclass After {}\n' + suffix
        if crlf:
            source = source.replace("\n", "\r\n")
        result.append({"id": identity, "source": source, "language_mode": mode, "sdk": sdk,
                       "file_mode": file_mode, "compiler_accepts": accepts, "diagnostic_codes": list(diagnostics),
                       "parse_accepts": parse, "sites": list(sites), "sdk_accepts": sdk_accepts,
                       "sdk_diagnostic": sdk_diagnostic, "helper": helper, "sibling": "After"})

    directives = [
        ("property", "#:property LangVersion=14", True, None),
        ("spaced-name", "#: property LangVersion=14", True, None),
        ("sdk", "#:sdk Microsoft.NET.Sdk", True, None),
        ("unknown", "#:future value", False, "Unrecognized directive"),
        ("empty", "#:", False, "Unrecognized directive"),
        ("punctuation", "#:<> text", False, "Unrecognized directive"),
    ]
    for name, directive, sdk_accepts, diagnostic in directives:
        add("directive-" + name, prefix=directive + "\n", file_mode=True,
            sites=[site(directive, "file_based_directive")], sdk_accepts=sdk_accepts, sdk_diagnostic=diagnostic)
    for helper in ("package", "project", "include"):
        payload = {"package": "Entmoot.Oracle@1.0.0", "project": "helper/Helper.csproj", "include": "helper/Helper.cs"}[helper]
        directive = "#:" + helper + " " + payload
        add("directive-" + helper, "if (Helper.Value != 7) throw new System.Exception();",
            prefix=directive + "\n", file_mode=True, helper=helper, sdk_accepts=True,
            sites=[site(directive, "file_based_directive")])
    add("directives-combined", prefix="#!/usr/bin/env dotnet\n#:sdk Microsoft.NET.Sdk\n#:property LangVersion=14\n",
        file_mode=True, sdk_accepts=True, sites=[site("#!/usr/bin/env dotnet", "shebang_directive"),
        site("#:sdk Microsoft.NET.Sdk", "file_based_directive"), site("#:property LangVersion=14", "file_based_directive")])
    add("directive-ordinary-compilation", prefix="#:property LangVersion=14\n", accepts=False, diagnostics=["CS9298"],
        sites=[site("#:property LangVersion=14", "file_based_directive")])
    add("shebang-file-mode", prefix="#!/usr/bin/env dotnet\n", file_mode=True, sdk_accepts=True,
        sites=[site("#!/usr/bin/env dotnet", "shebang_directive")])
    add("shebang-ordinary-compilation", prefix="#!/usr/bin/env dotnet\n", accepts=False, diagnostics=["CS9314"],
        sites=[site("#!/usr/bin/env dotnet", "shebang_directive")])
    add("directive-after-declaration", suffix="#:property LangVersion=14\n", file_mode=True,
        accepts=False, diagnostics=["CS9297"], sites=[site("#:property LangVersion=14", "file_based_directive")])
    add("shebang-after-declaration", suffix="#!later\n", file_mode=True, accepts=False, diagnostics=["CS9378"], parse=False)
    add("directive-spaced-colon", prefix="# :property LangVersion=14\n", file_mode=True,
        accepts=False, diagnostics=["CS1024"], parse=False)
    add("directive-in-comment", prefix="// #:property LangVersion=14\n", mode="8", sdk="9",
        sites=[site("// #:property LangVersion=14", "comment")])
    add("directive-in-string", 'string text = "#:property LangVersion=14"; if (text.Length != 25) throw new System.Exception();',
        mode="8", sdk="9", sites=[site('"#:property LangVersion=14"', "string_literal")])
    add("directive-unicode-crlf", prefix="// λ 漢\n#:property LangVersion=14\n", file_mode=True, sdk_accepts=True, crlf=True,
        sites=[site("#:property LangVersion=14", "file_based_directive")])
    add("directive-eof", suffix="#:", file_mode=True, accepts=False, diagnostics=["CS9297"], sites=[site("#:", "file_based_directive")])
    add("preprocessor-historical", prefix="#define ENABLED\n#if ENABLED\nclass Enabled {}\n#endif\n", sdk="9", mode="8")

    extension = "public static class Extensions { extension(string text) { public int TwiceLength() => text.Length * 2; public int Size => text.Length; } }\n"
    ext_site = site("extension(string text) { public int TwiceLength() => text.Length * 2; public int Size => text.Length; }", "extension_declaration")
    add("extension-members-14", 'if ("abc".TwiceLength() != 6 || "abc".Size != 3) throw new System.Exception();',
        declarations=extension, sites=[ext_site])
    add("extension-old-mode", declarations=extension, mode="13", accepts=False, diagnostics=["CS1513", "CS1022"], sites=[ext_site])
    add("directive-language-mode-override", declarations=extension, prefix="#:property LangVersion=13\n", file_mode=True,
        sites=[site("#:property LangVersion=13", "file_based_directive"), ext_site], sdk_accepts=False, sdk_diagnostic="CS1513")
    add("extension-ordinary-identifier", "int extension = 7; if (extension != 7) throw new System.Exception();", sdk="9", mode="8",
        sites=[site("extension = 7", "variable_declarator", name="extension")])
    add("historical-extension-method", 'if ("abc".TwiceLength() != 6) throw new System.Exception();', sdk="9", mode="7.3",
        declarations="public static class Extensions { public static int TwiceLength(this string value) => value.Length * 2; }\n",
        sites=[site("public static int TwiceLength(this string value) => value.Length * 2;", "method_declaration", name="TwiceLength")])
    add("nullable-switch-8", "int? value = 7; int answer = value switch { null => 0, int x => x }; if (answer != 7) throw new System.Exception();", sdk="9", mode="8")
    add("record-9", "if (new Item(7).Value != 7) throw new System.Exception();", declarations="record Item(int Value);\n", sdk="9", mode="9")
    add("record-struct-10", "if (new Item(7).Value != 7) throw new System.Exception();", declarations="record struct Item(int Value);\n", sdk="9", mode="10")
    add("raw-string-list-pattern-11", 'string text = """hello"""; int[] xs = {1, 2}; if (text != "hello" || xs is not [1, 2]) throw new System.Exception();', sdk="9", mode="11")
    add("primary-constructor-collection-12", "int[] xs = [1, 2]; if (new Item(7).Value != 7 || xs.Length != 2) throw new System.Exception();",
        declarations="class Item(int value) { public int Value => value; }\n", sdk="9", mode="12")
    add("params-collection-13", "if (Sum(3, 4) != 7) throw new System.Exception();", sdk="9", mode="13",
        declarations="", sites=[site("Sum(3, 4)", "invocation_expression", function="Sum")])
    result[-1]["source"] = result[-1]["source"].replace("static void Main()", "static int Sum(params System.ReadOnlySpan<int> values) { int sum = 0; foreach (int value in values) sum += value; return sum; } static void Main()")
    add("field-backed-property-14", "var item = new Item(); item.Value = 7; if (item.Value != 7) throw new System.Exception();",
        declarations="class Item { public int Value { get; set { field = value; } } }\n")
    add("null-conditional-assignment-14", "Item item = null; item?.Value = 7; item = new Item(); item?.Value = 7; if (item.Value != 7) throw new System.Exception();",
        declarations="class Item { public int Value; }\n")
    add("malformed-preserve-sibling", declarations="class Broken { void Missing( }\n", accepts=False, diagnostics=["CS1026"], parse=False)
    # A preview qualification probe, not a stable-language support claim.
    add("preview-extension-indexer-15", "var item = new Item(); if (item[2] != 9) throw new System.Exception();", mode="preview", sdk="11",
        declarations="class Item { public int Value = 7; }\nstatic class Extensions { extension(Item item) { public int this[int index] => item.Value + index; } }\n",
        sites=[site("public int this[int index] => item.Value + index;", "indexer_declaration")])
    return result
