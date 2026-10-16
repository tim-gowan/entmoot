"""Authored language behavior and CST selectors, never inferred from parser output."""
from __future__ import annotations


def site(text, kind, *, named_children=None, **fields):
    result = {"text": text, "kind": kind, "fields": fields}
    if named_children is not None:
        result["named_children"] = named_children
    return result


def cases():
    result = []

    def add(identity, body, *, release=27, prefix="", members="", preview=False,
            accepts=True, diagnostic=(), sites=(), parse_accepts=True, compact=False, crlf=False):
        main = "void main()" if compact else "public static void main(String[] args)"
        content = members + "\n" + main + " {\n" + body + '\nSystem.out.println("oracle:ok");\n}\n'
        source = prefix + "\n" + (content if compact else "class Fixture {\n" + content + "}\n")
        source += "class After { static int sibling() { return 7; } }\n"
        if crlf:
            source = source.replace("\n", "\r\n")
        result.append({"id": identity, "source": source, "release": release, "preview": preview,
                       "compiler_accepts": accepts, "diagnostic_codes": list(diagnostic),
                       "parse_accepts": parse_accepts, "sites": list(sites), "sibling": "After"})

    module_sites = [site("import module java.base;", "import_declaration")]
    for release in (25, 27):
        add(f"module-import-{release}", 'if (List.of(1, 2).size() != 2) throw new AssertionError();',
            release=release, prefix="import module java.base;", sites=module_sites)
    add("module-import-old-mode", "", release=21, prefix="import module java.base;",
        accepts=False, diagnostic=["compiler.err.feature.not.supported.in.source"], sites=module_sites)
    add("module-import-ambiguity", "List<?> value = null;", prefix="import module java.base;\nimport module java.desktop;",
        accepts=False, diagnostic=["compiler.err.ref.ambiguous"], sites=module_sites)
    add("module-ordinary-identifier", "int module = 7; if (module != 7) throw new AssertionError();", release=8,
        sites=[site("module = 7", "variable_declarator", name="module", value="7")])
    add("static-import-historical", "if (max(2, 3) != 3) throw new AssertionError();", release=8,
        prefix="import static java.lang.Math.max;", sites=[site("import static java.lang.Math.max;", "import_declaration")])
    add("named-modifiers-historical", "", release=8,
        sites=[site("public static", "modifiers", named_children=["visibility", "modifier"])])
    add("varargs-field-contract", "if (count(1, 2) != 2) throw new AssertionError();", release=8,
        members="static int count(final int... values) { return values.length; }",
        sites=[site("final int... values", "spread_parameter", modifiers="final", type="int")])

    parent = "class Parent { final int value; Parent(int value) { this.value = value; } }\n"
    child = "static class Child extends Parent { Child(int value) { int checked = Math.abs(value); super(checked); } }"
    ctor = site("super(checked);", "explicit_constructor_invocation", constructor="super", arguments="(checked)")
    add("constructor-prologue", "if (new Child(-7).value != 7) throw new AssertionError();", release=25,
        prefix=parent, members=child, sites=[ctor])
    add("constructor-prologue-old-mode", "", release=21, prefix=parent, members=child, sites=[ctor],
        accepts=False, diagnostic=["compiler.err.feature.not.supported.in.source"])
    delegated = "static class Child { final int value; Child(int v) { value = v; } Child() { int v = 7; this(v); } }"
    add("constructor-this-prologue", "if (new Child().value != 7) throw new AssertionError();", members=delegated,
        sites=[site("this(v);", "explicit_constructor_invocation", constructor="this", arguments="(v)")])
    add("constructor-early-field-write", "if (new Child().value != 7) throw new AssertionError();",
        members="static class Child { int value; Child() { value = 7; super(); } }",
        sites=[site("super();", "explicit_constructor_invocation", constructor="super", arguments="()")])
    add("constructor-early-field-read", "", members="static class Child { int value; Child() { int read = value; super(); } }",
        accepts=False, diagnostic=["compiler.err.cant.ref.before.ctor.called"],
        sites=[site("super();", "explicit_constructor_invocation", constructor="super", arguments="()")])
    add("constructor-double-invocation", "", members="static class Child { Child() { super(); super(); } }",
        accepts=False, diagnostic=["compiler.err.redundant.superclass.init"])
    for release in (25, 27):
        add(f"compact-source-{release}", "if (answer() != 7) throw new AssertionError();", release=release,
            members="int answer() { return 7; }", compact=True,
            sites=[site("int answer() { return 7; }", "method_declaration", name="answer", type="int")])
    add("compact-source-old-mode", "", release=21, compact=True, accepts=False,
        diagnostic=["compiler.err.feature.not.supported.in.source"])

    add("lambda-java8", "java.util.function.IntUnaryOperator plus = x -> x + 1; if (plus.applyAsInt(6) != 7) throw new AssertionError();",
        release=8, sites=[site("x -> x + 1", "lambda_expression", parameters="x", body="x + 1")])
    add("var-java11", "var number = 7; if (number != 7) throw new AssertionError();", release=11,
        sites=[site("number = 7", "variable_declarator", name="number", value="7")])
    add("switch-java14", "int n = 2; int answer = switch (n) { case 2 -> 7; default -> 0; }; if (answer != 7) throw new AssertionError();",
        release=14, sites=[site("switch (n) { case 2 -> 7; default -> 0; }", "switch_expression", condition="(n)")])
    add("text-block-java15", 'String text = """\nhello\n"""; if (!text.equals("hello\\n")) throw new AssertionError();', release=15)
    add("record-java16", "if (new Pair(7).value() != 7) throw new AssertionError();", release=16,
        members="record Pair(int value) { Pair { if (value < 0) throw new IllegalArgumentException(); } }",
        sites=[site("Pair { if (value < 0) throw new IllegalArgumentException(); }", "compact_constructor_declaration", name="Pair")])
    add("sealed-java17", "Base b = new Child(); if (!(b instanceof Child)) throw new AssertionError();", release=17,
        members="sealed interface Base permits Child {} static final class Child implements Base {}")
    add("record-pattern-java21", "Object value = new Pair(7); if (!(value instanceof Pair(int number)) || number != 7) throw new AssertionError();",
        release=21, members="record Pair(int value) {}",
        sites=[site("value instanceof Pair(int number)", "instanceof_expression", left="value", pattern="Pair(int number)")])
    add("qualified-record-pattern", "Object value = new Box.Pair(7); if (!(value instanceof Box.Pair(int number)) || number != 7) throw new AssertionError();",
        release=21, members="static class Box { record Pair(int value) {} }",
        sites=[site("Box.Pair(int number)", "record_pattern")])
    add("final-record-pattern-component", "Object value = new Pair(7); if (!(value instanceof Pair(final int number)) || number != 7) throw new AssertionError();",
        release=21, members="record Pair(int value) {}",
        sites=[site("final int number", "record_pattern_component")])
    add("guarded-switch-pattern", 'Object value = "seven"; int length = switch (value) { case final String text when text.length() > 2 -> text.length(); default -> 0; }; if (length != 5) throw new AssertionError();',
        release=21, sites=[site("final String text", "type_pattern"), site("when text.length() > 2", "guard")])
    add("unnamed-record-pattern-java22", "Object value = new Pair(7, 8); if (!(value instanceof Pair(int number, _)) || number != 7) throw new AssertionError();",
        release=22, members="record Pair(int first, int second) {}", sites=[site("_", "underscore_pattern")])
    primitive_body = "Object value = 7; if (!(value instanceof int number) || number != 7) throw new AssertionError();"
    primitive_site = site("value instanceof int number", "instanceof_expression", left="value", right="int", name="number")
    add("primitive-pattern-preview27", primitive_body, preview=True, sites=[primitive_site])
    add("primitive-pattern-preview-disabled", primitive_body, accepts=False, sites=[primitive_site],
        diagnostic=["compiler.err.preview.feature.disabled.plural"])
    add("primitive-switch-preview27", "int value = 7; int answer = switch (value) { case int number when number > 0 -> number; default -> 0; }; if (answer != 7) throw new AssertionError();",
        preview=True, sites=[site("int number", "type_pattern")])
    add("unicode-crlf-historical", 'String café = "λ"; if (!café.equals("λ")) throw new AssertionError();', release=8, crlf=True,
        sites=[site('café = "λ"', "variable_declarator", name="café", value='"λ"')])
    add("malformed-preserves-following-class", "", members="static void broken( { }", accepts=False,
        parse_accepts=False, diagnostic=["compiler.err.illegal.start.of.type"])
    return result
