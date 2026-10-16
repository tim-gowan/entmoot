//! Normalized source coordinates (reject invalid UTF offsets).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Grain of a source location: whole file or a concrete region within a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "grain", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocationExtent {
    File,
    Region { region: SourceRegion },
}

impl LocationExtent {
    pub fn region(&self) -> Option<&SourceRegion> {
        match self {
            Self::Region { region } => Some(region),
            Self::File => None,
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Self::File)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub path: String,
    pub extent: LocationExtent,
}

/// Asserted coordinates, not proof that the source bytes match the snapshot.
/// Provider-native omissions remain in the retained artifact at its pointer;
/// adapters apply only defaults established by that provider's format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "coordinates", rename_all = "snake_case", deny_unknown_fields)]
#[serde(try_from = "SourceRegionInput")]
#[schemars(!try_from)]
pub enum SourceRegion {
    Bytes {
        span: crate::SiteSpan,
    },
    Text {
        range: SourceTextRange,
        encoding: ColumnEncoding,
    },
}

#[derive(Deserialize)]
#[serde(tag = "coordinates", rename_all = "snake_case", deny_unknown_fields)]
enum SourceRegionInput {
    Bytes {
        span: crate::SiteSpan,
    },
    Text {
        range: SourceTextRange,
        encoding: ColumnEncoding,
    },
}

impl TryFrom<SourceRegionInput> for SourceRegion {
    type Error = CoordinateError;

    fn try_from(value: SourceRegionInput) -> Result<Self, Self::Error> {
        let region = match value {
            SourceRegionInput::Bytes { span } => Self::Bytes { span },
            SourceRegionInput::Text { range, encoding } => Self::Text { range, encoding },
        };
        region.validate()?;
        Ok(region)
    }
}

/// A byte range checked against the exact snapshot file bytes. This is derived
/// attachment evidence; it never replaces the provider's asserted coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifiedSourceSpan {
    pub source_digest: crate::ContentDigest,
    pub span: crate::SiteSpan,
}

/// One-based line and column. Columns use the enclosing region's encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourcePosition {
    #[schemars(range(min = 1))]
    pub line: u32,
    #[schemars(range(min = 1))]
    pub column: u32,
}

/// Explicit exclusive end or the end of a line, excluding its newline sequence.
/// A line end is source-dependent; it is never approximated as one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceTextEnd {
    Position {
        position: SourcePosition,
    },
    LineEnd {
        #[schemars(range(min = 1))]
        line: u32,
    },
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceTextRange {
    pub start: SourcePosition,
    pub end: SourceTextEnd,
}

impl SourceRegion {
    pub(crate) fn identity_parts(&self) -> Vec<String> {
        match self {
            Self::Bytes { span } => vec![
                "bytes".into(),
                span.start_byte.to_string(),
                span.end_byte.to_string(),
            ],
            Self::Text { range, encoding } => {
                let mut parts = vec![
                    "text".into(),
                    encoding.as_str().into(),
                    range.start.line.to_string(),
                    range.start.column.to_string(),
                ];
                match range.end {
                    SourceTextEnd::Position { position } => parts.extend([
                        "position".into(),
                        position.line.to_string(),
                        position.column.to_string(),
                    ]),
                    SourceTextEnd::LineEnd { line } => {
                        parts.extend(["line_end".into(), line.to_string()])
                    }
                    SourceTextEnd::Unspecified => parts.push("unspecified".into()),
                }
                parts
            }
        }
    }

    pub fn validate(&self) -> Result<(), CoordinateError> {
        let valid = match self {
            Self::Bytes { span } => span.start_byte <= span.end_byte,
            Self::Text { range, .. } => {
                range.start.line > 0
                    && range.start.column > 0
                    && match range.end {
                        SourceTextEnd::Position { position } => {
                            position.column > 0
                                && (position.line, position.column)
                                    >= (range.start.line, range.start.column)
                        }
                        SourceTextEnd::LineEnd { line } => line >= range.start.line,
                        SourceTextEnd::Unspecified => true,
                    }
            }
        };
        if valid {
            Ok(())
        } else {
            Err(CoordinateError::InvalidRange)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColumnEncoding {
    Utf8CodeUnits,
    Utf16CodeUnits,
    Utf32CodePoints,
    Unknown,
}

impl ColumnEncoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Utf8CodeUnits => "utf8_code_units",
            Self::Utf16CodeUnits => "utf16_code_units",
            Self::Utf32CodePoints => "utf32_code_points",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoordinateError {
    #[error("invalid source coordinate range")]
    InvalidRange,
    #[error("invalid column offset {column} for encoding on line")]
    InvalidOffset { column: u32 },
}

/// Convert UTF-16 code-unit column (1-based) to Unicode code-point column (1-based).
/// Rejects offsets that land past end-of-line or inside a multi-unit scalar.
pub fn utf16_column_to_codepoint(line: &str, utf16_column: u32) -> Result<u32, CoordinateError> {
    if utf16_column == 0 {
        return Err(CoordinateError::InvalidOffset {
            column: utf16_column,
        });
    }
    let target = (utf16_column - 1) as usize;
    let mut utf16_seen = 0usize;
    let mut codepoints = 0u32;
    for ch in line.chars() {
        if utf16_seen == target {
            return Ok(codepoints + 1);
        }
        let units = ch.len_utf16();
        if utf16_seen + units > target {
            return Err(CoordinateError::InvalidOffset {
                column: utf16_column,
            });
        }
        utf16_seen += units;
        codepoints += 1;
    }
    if utf16_seen == target {
        Ok(codepoints + 1)
    } else {
        Err(CoordinateError::InvalidOffset {
            column: utf16_column,
        })
    }
}

pub fn utf8_column_to_codepoint(line: &str, utf8_column: u32) -> Result<u32, CoordinateError> {
    if utf8_column == 0 {
        return Err(CoordinateError::InvalidOffset {
            column: utf8_column,
        });
    }
    let target = (utf8_column - 1) as usize;
    let mut bytes_seen = 0usize;
    let mut codepoints = 0u32;
    for ch in line.chars() {
        if bytes_seen == target {
            return Ok(codepoints + 1);
        }
        let n = ch.len_utf8();
        if bytes_seen + n > target {
            // Offset landed inside a multi-byte scalar — reject (do not round).
            return Err(CoordinateError::InvalidOffset {
                column: utf8_column,
            });
        }
        bytes_seen += n;
        codepoints += 1;
    }
    if bytes_seen == target {
        Ok(codepoints + 1)
    } else {
        Err(CoordinateError::InvalidOffset {
            column: utf8_column,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_regions_reject_legacy_partial_contradictory_and_reversed_payloads() {
        for value in [
            serde_json::json!({"start_line":1,"start_byte":0}),
            serde_json::json!({"coordinates":"bytes","span":{"start_byte":0}}),
            serde_json::json!({"coordinates":"bytes","span":{"start_byte":2,"end_byte":1}}),
            serde_json::json!({"coordinates":"bytes","span":{"start_byte":0,"end_byte":1},"range":{"start":{"line":1,"column":1},"end":{"kind":"unspecified"}}}),
            serde_json::json!({"coordinates":"text","encoding":"utf16_code_units","range":{"start":{"line":1,"column":0},"end":{"kind":"line_end","line":1}}}),
            serde_json::json!({"coordinates":"text","encoding":"utf16_code_units","range":{"start":{"line":2,"column":1},"end":{"kind":"line_end","line":1}}}),
            serde_json::json!({"coordinates":"text","encoding":"utf16_code_units","range":{"start":{"line":1,"column":3},"end":{"kind":"position","position":{"line":1,"column":2}}}}),
        ] {
            assert!(
                serde_json::from_value::<SourceRegion>(value.clone()).is_err(),
                "{value}"
            );
        }
        for value in [
            serde_json::json!({"coordinates":"bytes","span":{"start_byte":0,"end_byte":0}}),
            serde_json::json!({"coordinates":"text","encoding":"unknown","range":{"start":{"line":1,"column":1},"end":{"kind":"unspecified"}}}),
        ] {
            let region: SourceRegion = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(region).unwrap(), value);
        }
    }

    #[test]
    fn utf8_rejects_mid_multibyte() {
        let line = "a😀b";
        // '😀' occupies bytes 1..5 (0-based); column 3 (1-based) is mid-emoji
        assert!(utf8_column_to_codepoint(line, 3).is_err());
    }

    #[test]
    fn utf8_accepts_codepoint_boundary() {
        let line = "a😀b";
        assert_eq!(utf8_column_to_codepoint(line, 6).unwrap(), 3);
    }

    #[test]
    fn utf16_past_end_rejected() {
        assert!(utf16_column_to_codepoint("hi", 99).is_err());
    }

    #[test]
    fn utf16_rejects_mid_surrogate_pair() {
        assert!(utf16_column_to_codepoint("a😀b", 3).is_err());
    }

    #[test]
    fn source_location_file_grain_round_trips() {
        let loc = SourceLocation {
            path: "src/main.rs".into(),
            extent: LocationExtent::File,
        };
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["extent"]["grain"], "file");
        let back: SourceLocation = serde_json::from_value(json).unwrap();
        assert_eq!(back, loc);
        assert!(back.extent.is_file());
        assert!(back.extent.region().is_none());
    }

    #[test]
    fn source_location_region_grain_round_trips() {
        let loc = SourceLocation {
            path: "src/main.rs".into(),
            extent: LocationExtent::Region {
                region: SourceRegion::Bytes {
                    span: crate::SiteSpan {
                        start_byte: 100,
                        end_byte: 150,
                    },
                },
            },
        };
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["extent"]["grain"], "region");
        let back: SourceLocation = serde_json::from_value(json).unwrap();
        assert_eq!(back, loc);
        assert!(!back.extent.is_file());
        assert_eq!(back.extent.region(), loc.extent.region());
    }
}
