"""Authored JS/TS/TSX controls; compiler results never generate CST expectations."""
AFTER = "function after(){return 7;}"


def site(text, kind, **fields):
    return {"text": text, "kind": kind, "fields": fields}


def cases():
    rows = []

    def add(name, source, *, languages=("javascript", "typescript", "tsx"), sites=(),
            node=True, compiler=True, execute=True, parse=True, files=None, options=None, note=""):
        rows.append(dict(id=name, source=source + "\n" + AFTER + "\n" + ("console.log('oracle:ok');\n" if execute else ""),
                         languages=list(languages), sites=list(sites), node_accepts=node, compiler_accepts=compiler,
                         execute=execute, parse_accepts=parse, files=files or {}, options=options or {}, note=note))

    add("optional-nullish", "const value = {x:7}; if ((value?.x ?? 0) !== 7) throw Error('value');",
        sites=[site("value?.x", "member_expression", object="value", property="x")], note="ES2020")
    add("bigint-separators-logical-assignment", "let value = 0; value ||= 1_007; value &&= 7; value ??= 9; if (value !== Number(7n)) throw Error('value');", note="ES2020/2021")
    add("private-static-block", "class Value { #x=7; static count=0; static { this.count=7; } read(){return this.#x;} has(other){return #x in other;} } const v=new Value; if(v.read()!==7 || !v.has(v) || Value.count!==7) throw Error('value');", note="ES2022")
    add("top-level-await", "const value = await Promise.resolve(7); if(value!==7) throw Error('value');", note="ES2022 module")
    add("dynamic-import", "const dep = await import('./dep.mjs'); if(dep.value!==7) throw Error('value');",
        files={"dep.mjs": "export const value=7;", "dep.d.mts": "export const value: number;"})
    add("hashbang", "#!/usr/bin/env node\nconst value=7; if(value!==7) throw Error('value');", note="ES2023 hashbang")
    add("import-attributes", "import data from './data.json' with {type:'json'}; if(data.value!==7) throw Error('value');",
        files={"data.json": '{"value":7}'}, sites=[site("with {type:'json'}", "import_attribute")])
    add("regexp-unicode-sets", r"const r=/[\p{ASCII}&&\p{Letter}]/v; if(!r.test('a') || r.test('1')) throw Error('regex');", note="ES2024 regexp sublanguage")
    add("regexp-modifiers", "const r=/(?i:a)b/; if(!r.test('Ab') || r.test('AB')) throw Error('regex');", note="RegExp body is opaque to the outer grammar")
    add("arbitrary-export-name", 'const value=7; export {value as "value-name"};', sites=[site('value as "value-name"', "export_specifier", name="value", alias='"value-name"')])
    add("reserved-export-name", "export {default as export} from './dep.mjs'; import value from './dep.mjs'; if(value!==7) throw Error('export');",
        files={"dep.mjs":"export default 7;", "dep.d.mts":"declare const value:number; export default value;"},
        sites=[site("default as export", "export_specifier", name="default", alias="export")])
    add("unicode-crlf", "// λ😀\r\nfunction café(){return 7;}\r\nif(café()!==7) throw Error('unicode');",
        sites=[site("function café(){return 7;}", "function_declaration", name="café", parameters="()", body="{return 7;}")])
    add("contextual-methods", "const value={get(){return 7;},set(x){return x;},async(){return 7;}}; if(value.get()!==7 || value.set(7)!==7 || value.async()!==7) throw Error('names');")
    add("optional-reserved-property", "const value={delete(){return 7;},class:7,return:7}; if(value?.delete()!==7 || value?.class!==7 || value?.return!==7) throw Error('property');",
        sites=[site("value?.delete", "member_expression", object="value", property="delete")])
    add("using-declaration", "let count=0; { using resource = { [Symbol.dispose](){count++;} }; } if(count!==1) throw Error('dispose');",
        sites=[site("using resource = { [Symbol.dispose](){count++;} };", "using_declaration"), site("resource = { [Symbol.dispose](){count++;} }", "variable_declarator", name="resource", value="{ [Symbol.dispose](){count++;} }")], note="Resource-management syntax; publication-year status is recorded separately")
    add("await-using", "let count=0; { await using resource = { async [Symbol.asyncDispose](){count++;} }; } if(count!==1) throw Error('dispose');",
        sites=[site("await using resource = { async [Symbol.asyncDispose](){count++;} };", "using_declaration")])
    add("using-for-of", "let count=0; for (using item of [{[Symbol.dispose](){count++;}}]) {} if(count!==1) throw Error('dispose');",
        sites=[site("for (using item of [{[Symbol.dispose](){count++;}}]) {}", "for_in_statement", left="item", right="[{[Symbol.dispose](){count++;}}]")])
    add("using-for-await", "let count=0; for await (using item of [{[Symbol.dispose](){count++;}}]) {} if(count!==1) throw Error('dispose');")
    add("using-ordinary-identifier", "const using=7; const value={using}; if(value.using!==7) throw Error('identifier');",
        sites=[site("using=7", "variable_declarator", name="using", value="7")])
    add("using-line-break", "let resource=7; const using=1; using\nresource = null; if(resource!==null) throw Error('asi');",
        sites=[site("using", "identifier")], note="Selector uses explicit occurrence below; no declaration across a line terminator")
    rows[-1]["sites"] = [dict(site("using\nresource = null;", "expression_statement"), absent=True),
                          site("resource = null", "assignment_expression", left="resource", right="null")]
    for name, gap in (("cr", "\r"), ("ls", "\u2028"), ("ps", "\u2029"), ("comment", "/*\n*/")):
        add("using-line-break-" + name, "let resource=7; const using=1; using" + gap + "resource = null; if(resource!==null) throw Error('asi');",
            sites=[site("resource = null", "assignment_expression", left="resource", right="null")])
    add("using-inline-comment", "let count=0; { using /* inline */ resource = {[Symbol.dispose](){count++;}}; } if(count!==1) throw Error('dispose');",
        sites=[site("using /* inline */ resource = {[Symbol.dispose](){count++;}};", "using_declaration")])
    add("using-without-semicolons", "let count=0\n{\nusing resource = {[Symbol.dispose](){count++;}}\nconst value=7\nif(value!==7) throw Error('value')\n}\nif(count!==1) throw Error('dispose')")
    add("using-keyword-prefix", "const usingLong=7; const awaitUsing=7; if(usingLong!==awaitUsing) throw Error('prefix');",
        sites=[site("usingLong=7", "variable_declarator", name="usingLong", value="7")])
    add("await-using-for-await", "let count=0; for await (await using item of [{async [Symbol.asyncDispose](){count++;}}]) {} if(count!==1) throw Error('dispose');")
    add("await-line-break-mode-boundary", "await\nusing resource = null;", node=False, compiler=True, execute=False,
        note="Node module check rejects; both pinned TypeScript compilers accept. Grammar treats await as an ordinary identifier here. This is an explicit compiler/runtime disagreement, not qualified module syntax.")
    add("using-for-line-break-negative", "for(using\nitem of [null]) {}", node=False, compiler=False, execute=False, parse=False)
    add("asi-call-continuation", "let count=0; function value(){count++;return 7;} const result=value /*\n*/ (); if(result!==7 || count!==1) throw Error('continuation');",
        sites=[site("value /*\n*/ ()", "call_expression", function="value", arguments="()")])
    add("asi-operator-continuation", "const value=1 /*\n*/ + 6; const half=14 /*\n*/ / 2; if(value!==7 || half!==7) throw Error('continuation');")
    add("asi-member-continuation", "const value={x:7}; const result=value /*\n*/ .x; if(result!==7) throw Error('continuation');")
    add("asi-return-cr", "function value(){return\r7;} if(value()!==undefined) throw Error('return');",
        sites=[site("return", "return_statement", argument=None)])
    rows[-1]["sites"][0]["occurrence"] = 0
    add("asi-return-comment", "function value(){return/*\n*/7;} if(value()!==undefined) throw Error('return');",
        sites=[dict(site("return", "return_statement", argument=None), occurrence=0)])
    add("using-for-in-negative", "for (using item in {}) {}", node=False, compiler=False, execute=False,
        note="Early-error legality belongs to compilers; permissive grammar acceptance alone proves nothing")
    add("using-missing-value-negative", "using resource;", node=False, compiler=False, execute=False)
    add("malformed-sibling", "const broken = ;", node=False, compiler=False, execute=False, parse=False)
    # New builtins are ordinary member/call expressions. These controls make no binding claim.
    for name, body in (
        ("map-upsert", "const map=new Map(); if(map.getOrInsert('x',7)!==7 || map.getOrInsertComputed('x',()=>9)!==7) throw Error('map');"),
        ("base64-hex", "if(Uint8Array.fromBase64('Bw==')[0]!==7 || new Uint8Array([7]).toHex()!=='07') throw Error('bytes');"),
        ("sum-precise", "if(Math.sumPrecise([1e16,7,-1e16])!==7) throw Error('sum');"),
        ("error-is-error", "if(!Error.isError(new Error()) || Error.isError({})) throw Error('error');"),
        ("array-from-async", "if((await Array.fromAsync([7]))[0]!==7) throw Error('array');"),
        ("iterator-concat", "if([...Iterator.concat([7],[8])].join()!=='7,8') throw Error('iterator');"),
        ("json-source", "let source=''; JSON.parse('7',(_key,value,context)=>{if(context) source=context.source; return value;}); if(source!=='7') throw Error('json');"),
    ):
        add("es2026-library-" + name, body, languages=("javascript",), note="Independent runtime/library qualification; unavailable APIs stay failed. No binding inference from names.")
    add("shadowed-builtins", "const Math={sumPrecise(values){return values[0];}}; const value=Math.sumPrecise([7]); if(value!==7) throw Error('shadow');",
        sites=[site("Math.sumPrecise([7])", "call_expression", function="Math.sumPrecise", arguments="([7])")])
    add("source-phase-wasm", "import source mod from './dep.wasm'; if(new WebAssembly.Instance(mod).exports.f()!==7) throw Error('wasm');", languages=("javascript",),
        files={"dep.wasm": {"hex": "0061736d010000000105016000017f03020100070501016600000a0601040041070b"}},
        sites=[site("import source mod from './dep.wasm';", "import_statement", source="'./dep.wasm'", phase="source")], note="Node-supported Wasm source-phase import; standard/proposal status separate")
    add("source-ordinary-import-name", "import source from './dep.mjs'; if(source!==7) throw Error('name');", languages=("javascript",),
        files={"dep.mjs":"export default 7;"}, sites=[site("import source from './dep.mjs';", "import_statement", source="'./dep.mjs'", phase=None)])
    add("node-deferred-import-unavailable", "import defer * as dep from './dep.mjs';", languages=("javascript",), node=False, execute=False, parse=False,
        files={"dep.mjs":"export const value=7;"}, note="Node 26.11.1 rejects this syntax; TypeScript compiler qualification remains separate")
    typed = ("typescript", "tsx")
    add("reserved-import-type-query", "const value: typeof import('./dep.mjs').default = 7; if(value!==7) throw Error('type-query');", languages=typed,
        files={"dep.mjs":"export default 7;", "dep.d.mts":"declare const value:number; export default value;"},
        sites=[site("import('./dep.mjs').default", "member_expression", property="default")])
    add("satisfies", "const value={x:7} satisfies {x:number}; if(value.x!==7) throw Error('value');", languages=typed,
        sites=[site("{x:7} satisfies {x:number}", "satisfies_expression")], note="TypeScript 4.9")
    add("const-type-parameters", "function identity<const T>(value:T){return value;} const value=identity(7); if(value!==7) throw Error('value');", languages=typed, note="TypeScript 5.0")
    add("template-mapped-types", "type Input={x:number}; type Renamed<T>={[K in keyof T as `get_${string & K}`]:T[K]}; const value:Renamed<Input>={get_x:7}; if(value.get_x!==7) throw Error('value');", languages=typed, note="TypeScript 4.1")
    add("variadic-labelled-tuple", "type Pair<T extends unknown[]>=[first:number,...rest:T]; const value:Pair<[string]>=[7,'x']; if(value[0]!==7) throw Error('value');", languages=typed, note="TypeScript 4.0")
    add("instantiation-expression", "function identity<T>(value:T){return value;} const numberIdentity=identity<number>; if(numberIdentity(7)!==7) throw Error('value');", languages=typed, note="TypeScript 4.7")
    add("variance-annotations", "interface Read<out T>{get():T;} interface Write<in T>{set:(value:T)=>void;} interface Both<in out T>{get():T;set:(value:T)=>void;} const value:Read<number>={get(){return 7;}}; if(value.get()!==7) throw Error('value');", languages=typed,
        sites=[dict(site("out T", "type_parameter", name="T"), occurrence=0), site("in T", "type_parameter", name="T"), site("in out T", "type_parameter", name="T")], note="TypeScript 4.7")
    add("variance-invalid-function", "function identity<out T>(value:T){return value;}", languages=typed, compiler=False, execute=False, note="Grammar may parse variance; compiler rejects illegal function placement")
    add("variance-constraint-default", "type Read<out T extends number=number>={get:()=>T}; const value:Read={get:()=>7}; if(value.get()!==7) throw Error('value');", languages=typed)
    add("variance-contradiction", "type Bad<out T>={set:(value:T)=>void};", languages=typed, compiler=False, execute=False, options={"strict":True}, note="Compiler validates annotation variance; grammar does not")
    add("parameter-property-enum", "enum Kind{Seven=7} class Value{constructor(public value:Kind){}} if(new Value(Kind.Seven).value!==7) throw Error('value');", languages=typed, note="Retain older TypeScript syntax")
    add("overload-selection", "function select(value:number):number; function select(value:string):string; function select(value:number|string){return value;} const value:number=select(7); if(value!==7) throw Error('value');", languages=typed, note="Actual type checking; no grammar overload-resolution claim")
    add("overload-rejection", "function select(value:number):number; function select(value:string):string; function select(value:number|string){return value;} select(true);", languages=typed, compiler=False, execute=False)
    add("accessor-and-decorator", "function keep(value:any,_context:any){return value;} @keep class Value{accessor x=7;} if(new Value().x!==7) throw Error('value');", languages=typed,
        options={"target":"es2022"}, note="TypeScript 5.0 standard decorator transform; ESNext preservation alone cannot execute in Node 26")
    add("type-star-export", "export type * as Types from './dep.mjs';", languages=typed, files={"dep.mjs":"export const value=7;", "dep.d.mts":"export interface Value{x:number;}"}, note="TypeScript 5.0 type-only namespace export")
    add("deferred-import", "import defer * as dep from './dep.mjs';", languages=typed, execute=False,
        files={"dep.mjs":"export const value=7;", "dep.d.mts":"export const value:number;"},
        sites=[site("import defer * as dep from './dep.mjs';", "import_statement", source="'./dep.mjs'", phase="defer")], note="TypeScript 5.9 accepts; Node runtime separately unqualified, no downlevel transformation")
    add("defer-ordinary-import-name", "import defer from './dep.mjs'; if(defer!==7) throw Error('name');", languages=typed,
        files={"dep.mjs":"export default 7;", "dep.d.mts":"declare const value:number; export default value;"})
    add("defer-named-negative", "import defer {value} from './dep.mjs';", languages=typed, compiler=False, execute=False,
        files={"dep.mjs":"export const value=7;", "dep.d.mts":"export const value:number;"}, parse=False)
    add("jsx-generic-component", "function h(component:any,props:any){return component(props);} function Value<T>(props:{value:T}){return props.value;} const value=<Value<number> value={7}/>; if(value!==7) throw Error('jsx');", languages=("tsx",), sites=[site("<Value<number> value={7}/>", "jsx_self_closing_element", name="Value")], options={"jsx":"react", "jsxFactory":"h"})
    add("tsx-reserved-properties", "function h(component:any,props:any){return component(props);} const Foo={class:(props:any)=>props.value}; const value=<Foo.class value={7}/>; const map=new Map([['x',7]]); if(value!==7 || map?.get('x')!==7 || !map?.delete('x')) throw Error('jsx');", languages=("tsx",), options={"jsx":"react", "jsxFactory":"h"})
    add("tsx-generic-arrow", "const identity=<T,>(value:T)=>value; if(identity(7)!==7) throw Error('value');", languages=("tsx",))
    add("tsx-const-generic-arrow", "const identity=<const T,>(value:T)=>value; if(identity(7)!==7) throw Error('value');", languages=("tsx",))
    add("tsx-keyword-tag-negative", "declare global {namespace JSX {interface IntrinsicElements {const:{in:number};}}} function h(_tag:any,props:any){return props.in;} const value=<const in={7}/>;", languages=("tsx",), compiler=False, execute=False, options={"jsx":"react", "jsxFactory":"h"}, note="Both TypeScript compilers reject the const-tag ambiguity; grammar acceptance is not language validity")
    add("tsx-keyword-attribute", "declare global {namespace JSX {interface IntrinsicElements {tag:{in:number};}}} function h(_tag:any,props:any){return props.in;} const value=<tag in={7}/>; if(value!==7) throw Error('jsx');", languages=("tsx",), options={"jsx":"react", "jsxFactory":"h"})
    add("ts-angle-assertion", "const value=<number>7; if(value!==7) throw Error('value');", languages=("typescript",))
    rows[-1]["extension"] = ".ts"
    return rows
