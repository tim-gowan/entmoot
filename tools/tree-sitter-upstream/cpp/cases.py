"""Authored experimental C++ controls; no ABI, template closure or CompDB claim."""


def case(name, body, *, mode="c++23", prefix="", sites=(), accepted=True, parse=True,
         execute=True, profile="experimental", diagnostic=None, entrypoint=True):
    probe = "int probe() { " + body + " }"
    after = "int after() { return 7; }"
    source = prefix + "\n" + probe + "\n" + after + "\n"
    if entrypoint:
        source += 'extern "C" int puts(const char *);\nint main() { if (probe() != 7) return 1; puts("oracle:ok"); return 0; }\n'
    return {"id": name, "language": "cpp", "mode": mode, "profile": profile,
            "source": source, "accepted": accepted, "parse": parse,
            "execute": execute and accepted and entrypoint, "diagnostic": diagnostic,
            "after": after, "files": {}, "sites": [{"text": probe, "kind": "function_definition",
                "fields": {"declarator": "probe()"}}, *sites]}


CASES = [
    case("cpp98-template", "return choose(7);", mode="c++98", prefix="template<class T> T choose(T value) { return value; }"),
    case("cpp98-throw-specifier", "return choose();", mode="c++98", prefix="int choose() throw(int) { return 7; }"),
    case("cpp17-removed-throw-specifier", "return choose();", mode="c++17", prefix="int choose() throw(int) { return 7; }", accepted=False, diagnostic="dynamic exception specifications"),
    case("cpp11-lambda", "auto choose = [](int value) -> int { return value; }; return choose(7);", mode="c++11"),
    case("cpp11-alias-enum", "using Number = int; enum class Value : unsigned char { Seven = 7 }; Number value = static_cast<int>(Value::Seven); return value;", mode="c++11"),
    case("cpp11-delegating-constructor", "Item value; return value.value;", mode="c++11", prefix="struct Item { int value; Item(int v): value(v) {} Item(): Item(7) {} };"),
    case("cpp11-variadic-template", "return choose(3, 4);", mode="c++11", prefix="int choose() { return 0; } template<class T, class... Rest> int choose(T value, Rest... rest) { return value + choose(rest...); }"),
    case("cpp14-generic-lambda", "auto choose = [](auto value) { return value; }; return choose(7);", mode="c++14"),
    case("cpp14-binary-separators", "return 0b0'111;", mode="c++14"),
    case("cpp17-structured-bindings", "auto [left, right] = Pair{6, 1}; return left + right;", mode="c++17", prefix="struct Pair { int left; int right; };"),
    case("cpp17-if-initializer", "if (int value = 7; value > 0) return value; return 0;", mode="c++17"),
    case("cpp17-fold", "return sum(2, 5);", mode="c++17", prefix="template<class... T> int sum(T... value) { return (value + ...); }"),
    case("cpp17-inline-variable", "return value;", mode="c++17", prefix="inline int value = 7;"),
    case("cpp20-concepts", "return choose(Item{7});", mode="c++20", prefix="template<class T> concept HasValue = requires(T value) { value.value; }; struct Item { int value; }; template<HasValue T> int choose(T value) { return value.value; }"),
    case("cpp20-concept-negative", "return choose(7);", mode="c++20", prefix="template<class T> concept HasValue = requires(T value) { value.value; }; template<HasValue T> int choose(T value) { return value.value; }", accepted=False, diagnostic="constraints not satisfied"),
    case("cpp20-templated-lambda", "auto choose = []<class T>(T value) { return value; }; return choose(7);", mode="c++20"),
    case("cpp20-designated-init", "Item value = {.value = 7}; return value.value;", mode="c++20", prefix="struct Item { int value; };"),
    case("cpp20-char8", "const char8_t *value = u8\"a\"; return value[0] == u8'a' ? 7 : 0;", mode="c++20"),
    case("cpp20-consteval", "return answer();", mode="c++20", prefix="consteval int answer() { return 7; }"),
    case("cpp20-module", "return 7;", mode="c++20", prefix="export module fixture;", entrypoint=False),
    case("cpp20-coroutine", "set_value(); return value;", mode="c++20", prefix="#include <coroutine>\nint value = 0; struct Task { struct promise_type { Task get_return_object() { return {}; } std::suspend_never initial_suspend() { return {}; } std::suspend_never final_suspend() noexcept { return {}; } void return_void() {} void unhandled_exception() {} }; }; Task set_value() { value = 7; co_return; }"),
    case("cpp23-explicit-object", "Item value{7}; return value.get();", prefix="struct Item { int value; int get(this const Item& self) { return self.value; } };"),
    case("cpp23-multidimensional-subscript", "Item value; return value[3, 4];", prefix="struct Item { int operator[](int left, int right) const { return left + right; } };"),
    case("cpp23-static-call", "Item value; return value(7);", prefix="struct Item { static int operator()(int value) { return value; } };"),
    case("cpp23-if-consteval", "return answer();", prefix="constexpr int answer() { if consteval { return 1; } else { return 7; } }",
         sites=[{"text": "if consteval { return 1; } else { return 7; }", "kind": "if_statement", "fields": {"condition": None, "consequence": "{ return 1; }", "alternative": "else { return 7; }"}}]),
    case("cpp23-if-not-consteval", "return answer();", prefix="constexpr int answer() { if not consteval { return 7; } else { return 1; } }"),
    case("cpp23-if-bang-consteval", "return answer();", prefix="constexpr int answer() { if !consteval { return 7; } return 1; }"),
    case("cpp20-if-consteval-negative", "return 7;", mode="c++20", prefix="constexpr int answer() { if consteval { return 1; } else { return 7; } }", accepted=False, diagnostic="C++23 extension"),
    case("cpp23-if-consteval-body-negative", "return 7;", prefix="int answer() { if consteval return 1; else { return 7; } }", accepted=False, parse=False, diagnostic="expected"),
    case("cpp23-if-consteval-else-negative", "return 7;", prefix="int answer() { if consteval { return 1; } else return 7; }", accepted=False, parse=False, diagnostic="expected"),
    case("cpp17-if-constexpr-historical", "if constexpr (true) return 7; else return 0;", mode="c++17"),
    case("cpp23-auto-cast", "int value = 7; return auto(value);"),
    case("cpp23-using-enum", "using enum Value; return Seven;", prefix="enum Value { Seven = 7 };"),
    case("cpp23-unicode", "int \u03bb = 7; return \u03bb;", prefix="/* non-ASCII \u2603 */\r\n"),
    case("cpp23-shadowing", "int value = 7; { int value = 2; (void)value; } return value;"),
    case("cpp23-malformed-sibling", "return 7;", prefix="int broken = ;", accepted=False, parse=False, diagnostic="expected expression"),
    case("cpp26-pack-index", "return first(7, 9);", mode="c++26", profile="draft-experimental", prefix="template<class... T> int first(T... value) { return value...[0]; }",
         sites=[{"text": "value...[0]", "kind": "pack_index_expression", "fields": {"pack": "value", "index": "0"}}]),
    case("cpp26-pack-index-comment", "return first(7, 9);", mode="c++26", profile="draft-experimental", prefix="template<class... T> int first(T... value) { return value.../* gap */[0]; }",
         sites=[{"text": "value.../* gap */[0]", "kind": "pack_index_expression", "fields": {"pack": "value", "index": "0"}}]),
    case("cpp26-pack-type-index", "return first<int, long>(7);", mode="c++26", profile="draft-experimental", prefix="template<class... T> T...[0] first(T...[0] value) { return value; }",
         sites=[{"text": "T...[0]", "kind": "pack_index_type", "ordinal": 0, "fields": {"pack": "T", "index": "0"}},
                {"text": "T...[0]", "kind": "pack_index_type", "ordinal": 1, "fields": {"pack": "T", "index": "0"}}]),
]
