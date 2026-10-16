pub struct Api { pub value: i32 }
impl Api { pub fn direct(&self) -> i32 { self.value } }
pub fn caller(api: &Api) -> i32 { api.direct() }
