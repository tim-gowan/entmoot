pub trait Read { fn value(&self) -> i32; }
pub fn caller(r: &dyn Read) -> i32 { r.value() }
