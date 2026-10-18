//! Inventory extension selection for normalized provider language families.
pub fn language_matches(language: &str, path: &str) -> bool {
    match language {
        "rust" => path.ends_with(".rs"),
        "python" => path.ends_with(".py") || path.ends_with(".pyi"),
        "go" => path.ends_with(".go"),
        "java" => path.ends_with(".java"),
        "csharp" => path.ends_with(".cs"),
        "typescript" | "tsx" => {
            path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".mts") || path.ends_with(".cts")
        }
        // Oxc pipeline language is `javascript` but also analyzes TS/TSX inventory.
        "javascript" => {
            path.ends_with(".js")
                || path.ends_with(".jsx")
                || path.ends_with(".mjs")
                || path.ends_with(".cjs")
                || path.ends_with(".ts")
                || path.ends_with(".tsx")
                || path.ends_with(".mts")
                || path.ends_with(".cts")
        }
        "cxx" | "c" | "cpp" => {
            path.ends_with(".c")
                || path.ends_with(".cc")
                || path.ends_with(".cpp")
                || path.ends_with(".cxx")
                || path.ends_with(".h")
                || path.ends_with(".hh")
                || path.ends_with(".hpp")
        }
        _ => false,
    }
}
