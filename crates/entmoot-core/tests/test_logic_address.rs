//! Logic address (definition site envelope) tests.

use entmoot_core::logic_address::LogicKind;
use entmoot_core::StructuralHasher;

#[test]
fn python_top_level_function_address_includes_lang() {
    let hasher = StructuralHasher::new();
    let code = "def verify(x):\n    return x + 1\n";
    let result = hasher
        .compute_granular_hash_at(code, "python", Some("auth/service.py"), Some("acme/app"), None)
        .unwrap();
    let c = &result.callables["verify"];
    assert_eq!(c.kind, LogicKind::Function);
    assert_eq!(
        c.logic_address,
        "logic|acme/app|python|auth/service.py|-|verify|function|-|-"
    );
    assert_eq!(c.address_id.len(), 64);
}

#[test]
fn python_method_gets_class_scope_and_method_kind() {
    let hasher = StructuralHasher::new();
    let code = r#"
class AuthService:
    def verify(self, token):
        return token
"#;
    let result = hasher
        .compute_granular_hash_at(code, "python", Some("auth/service.py"), None, None)
        .unwrap();
    let c = &result.callables["verify"];
    assert_eq!(c.kind, LogicKind::Method);
    assert_eq!(c.scope, vec!["AuthService".to_string()]);
    assert_eq!(
        c.logic_address,
        "logic|-|python|auth/service.py|AuthService|verify|method|-|-"
    );
}

#[test]
fn python_nested_def_inside_method_is_function() {
    let hasher = StructuralHasher::new();
    let code = r#"
class AuthService:
    def verify(self, token):
        def helper(x):
            return x
        return helper(token)
"#;
    let result = hasher
        .compute_granular_hash_at(code, "python", Some("auth/service.py"), None, None)
        .unwrap();
    assert_eq!(result.callables["verify"].kind, LogicKind::Method);
    assert_eq!(result.callables["helper"].kind, LogicKind::Function);
}

#[test]
fn rust_mod_function_is_not_method() {
    let hasher = StructuralHasher::new();
    let code = r#"
mod inner {
    pub fn helper() -> i32 {
        1
    }
}
"#;
    let result = hasher
        .compute_granular_hash_at(code, "rust", Some("lib.rs"), None, None)
        .unwrap();
    assert_eq!(result.callables["helper"].kind, LogicKind::Function);
    assert_eq!(result.callables["helper"].scope, vec!["inner".to_string()]);
}

#[test]
fn rust_impl_method_is_method() {
    let hasher = StructuralHasher::new();
    let code = r#"
struct S;
impl S {
    fn run(&self) {}
}
"#;
    let result = hasher
        .compute_granular_hash_at(code, "rust", Some("lib.rs"), None, None)
        .unwrap();
    assert_eq!(result.callables["run"].kind, LogicKind::Method);
}

#[test]
fn structural_hash_stable_when_logic_address_path_changes() {
    let hasher = StructuralHasher::new();
    let code = "def foo(x):\n    return x\n";
    let a = hasher
        .compute_granular_hash_at(code, "python", Some("a.py"), None, None)
        .unwrap();
    let b = hasher
        .compute_granular_hash_at(code, "python", Some("b.py"), None, None)
        .unwrap();
    assert_eq!(a.callables["foo"].structural_hash, b.callables["foo"].structural_hash);
    assert_ne!(a.callables["foo"].address_id, b.callables["foo"].address_id);
    assert_ne!(a.callables["foo"].logic_address, b.callables["foo"].logic_address);
}

#[test]
fn java_method_address_uses_java_lang_field() {
    let hasher = StructuralHasher::new();
    let code = r#"
public class Auth {
    public boolean verify(String t) {
        return true;
    }
}
"#;
    let result = hasher
        .compute_granular_hash_at(code, "java", Some("com/Auth.java"), Some("corp/repo"), None)
        .unwrap();
    let c = &result.callables["verify"];
    assert_eq!(c.kind, LogicKind::Method);
    assert!(c.scope.iter().any(|s| s == "Auth"));
    assert!(c.logic_address.starts_with("logic|corp/repo|java|com/Auth.java|"));
    assert!(c.logic_address.contains("|verify|method|"));
}

#[test]
fn duplicate_symbols_disambiguate_with_span() {
    let hasher = StructuralHasher::new();
    let code = r#"
class A:
    def run(self):
        return 1
class B:
    def run(self):
        return 2
"#;
    let result = hasher
        .compute_granular_hash_at(code, "python", Some("m.py"), None, None)
        .unwrap();
    let runs: Vec<_> = result.callables.values().filter(|c| c.name == "run").collect();
    assert_eq!(runs.len(), 2);
    assert_ne!(runs[0].logic_address, runs[1].logic_address);
    assert_ne!(runs[0].address_id, runs[1].address_id);
    assert!(runs.iter().all(|c| c.logic_address.ends_with("|-|-")));
}

#[test]
fn language_is_part_of_occurrence_identity() {
    let hasher = StructuralHasher::new();
    let py = hasher
        .compute_granular_hash("def f():\n    return 1\n", "python", None)
        .unwrap();
    let js = hasher
        .compute_granular_hash("function f() { return 1; }", "javascript", None)
        .unwrap();
    assert_eq!(py.policy_id, js.policy_id);
    let py = &py.callables["f"];
    let js = &js.callables["f"];
    assert_ne!(py.logic_address, js.logic_address);
    assert_ne!(py.address_id, js.address_id);
}
