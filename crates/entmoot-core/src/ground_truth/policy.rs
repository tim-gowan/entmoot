//! Resolve the sole supported structural policy name.
use entmoot_contracts::StructuralHashPolicy;

pub fn policy_from_config_name(name: &str) -> Result<StructuralHashPolicy, String> {
    if name == "v1" {
        Ok(StructuralHashPolicy::v1())
    } else {
        Err(format!("unsupported normalization config: {name}; expected v1"))
    }
}

pub fn config_name_from_policy(_: &StructuralHashPolicy) -> &'static str {
    "v1"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_v1_is_a_policy_name() {
        assert_eq!(config_name_from_policy(&policy_from_config_name("v1").unwrap()), "v1");
        for name in ["minimal", "standard", "sound", "v2", "legacy", "aggressive", "nope"] {
            assert!(policy_from_config_name(name).is_err(), "{name}");
        }
    }
}
