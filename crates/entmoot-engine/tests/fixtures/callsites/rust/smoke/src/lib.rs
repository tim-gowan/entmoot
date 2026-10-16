mod sibling;

// Unicode offset sentinel: 😀
pub fn caller(value: &str) -> &str { sibling::help(value) }
pub fn indirect(callback: fn(&str) -> &str, value: &str) -> &str { callback(value) }
pub fn leaf() {}
