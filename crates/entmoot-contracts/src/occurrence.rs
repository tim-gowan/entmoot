//! Complete structural occurrence references; provider symbols are a separate domain.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AddressId, SiteSpan};

/// The readable occurrence envelope and its committed compact identity.
/// This does not establish a definition hash, a provider symbol, or a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceRef {
    pub address_id: AddressId,
    pub logic_address: String,
}

impl OccurrenceRef {
    pub fn new(logic_address: String) -> Result<Self, String> {
        let value = Self {
            address_id: AddressId::from_trusted(
                blake3::hash(logic_address.as_bytes()).to_hex().to_string(),
            ),
            logic_address,
        };
        value.validate()?;
        Ok(value)
    }

    /// Verify the commitment at a trust boundary. Envelope interpretation stays
    /// with the source inventory owner; provider text cannot become an occurrence.
    pub fn validate(&self) -> Result<(), String> {
        AddressId::parse(self.address_id.as_str()).map_err(|error| error.to_string())?;
        if !self.logic_address.starts_with("logic|")
            || blake3::hash(self.logic_address.as_bytes())
                .to_hex()
                .as_str()
                != self.address_id.as_str()
        {
            return Err("occurrence address commitment mismatch".into());
        }
        Ok(())
    }
}

/// An inventoried occurrence at a UTF-8 byte span in the containing file.
/// Repository, snapshot and path come from the enclosing source record; the
/// source inventory owner verifies byte boundaries against that file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceOccurrenceRef {
    pub occurrence: OccurrenceRef,
    pub span: SiteSpan,
}

impl SourceOccurrenceRef {
    pub fn validate(&self) -> Result<(), String> {
        self.occurrence.validate()?;
        if self.span.start_byte >= self.span.end_byte {
            return Err("source occurrence requires a nonempty span".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_occurrence_rejects_partial_legacy_and_forged_references() {
        let reference =
            OccurrenceRef::new("logic|repo|python|a.py|-|f|function|-|-".into()).unwrap();
        reference.validate().unwrap();
        let wire = serde_json::to_value(&reference).unwrap();
        assert_eq!(
            serde_json::from_value::<OccurrenceRef>(wire.clone()).unwrap(),
            reference
        );
        for field in ["address_id", "logic_address"] {
            let mut partial = wire.clone();
            partial.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<OccurrenceRef>(partial).is_err());
        }
        let mut forged = reference.clone();
        forged.logic_address.push('x');
        assert!(forged.validate().is_err());
        forged = reference;
        forged.address_id = AddressId::from_trusted("bad");
        assert!(forged.validate().is_err());
        assert!(OccurrenceRef::new("provider_symbol".into()).is_err());
        assert!(OccurrenceRef::new(String::new()).is_err());
        let mut extra = wire;
        extra["callable_start_byte"] = serde_json::json!(1);
        assert!(serde_json::from_value::<OccurrenceRef>(extra).is_err());
    }

    #[test]
    fn source_occurrence_requires_a_complete_nonempty_span() {
        let mut reference = SourceOccurrenceRef {
            occurrence: OccurrenceRef::new("logic|repo|python|a.py|-|f|function|-|-".into())
                .unwrap(),
            span: SiteSpan {
                start_byte: 0,
                end_byte: 10,
            },
        };
        reference.validate().unwrap();
        for span in [
            SiteSpan {
                start_byte: 0,
                end_byte: 0,
            },
            SiteSpan {
                start_byte: 10,
                end_byte: 9,
            },
        ] {
            reference.span = span;
            assert!(reference.validate().is_err());
        }
        assert!(
            serde_json::from_value::<SourceOccurrenceRef>(serde_json::json!({
                "occurrence": reference.occurrence, "span": {"start_byte":0}
            }))
            .is_err()
        );
    }
}
