pub struct Api;
impl Api { pub fn make() -> Self { Self } }
pub fn caller() -> Api { Api::make() }
