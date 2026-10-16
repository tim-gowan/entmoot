//! Provider-neutral component, dependency, vulnerability, and VEX observations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    hash_parts, normalize_repo_path, purl_address, Address, ArtifactId, IdParseError,
    LocationExtent, SourceLocation,
};

pub const COMPONENT_OCCURRENCE_SCHEMA: &str = "logic.component_occurrence.v1";
pub const COMPONENT_DEPENDENCY_SCHEMA: &str = "logic.component_dependency.v1";
pub const COMPONENT_VULNERABILITY_SCHEMA: &str = "logic.component_vulnerability.v1";
pub const BOM_COMPOSITION_SCHEMA: &str = "logic.bom_composition.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentOccurrence {
    pub schema_version: String,
    pub occurrence_id: String,
    pub artifact_id: String,
    pub provider_pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bom_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purl: Option<Address>,
    pub component_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_in: Option<SourceLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComponentDependencyKind {
    DependsOn,
    Provides,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BomReferenceOutcome {
    Resolved,
    MissingSubject,
    MissingObject,
    MissingSubjectAndObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentDependency {
    pub schema_version: String,
    pub dependency_id: String,
    pub artifact_id: String,
    pub provider_pointer: String,
    pub kind: ComponentDependencyKind,
    pub subject_bom_ref: String,
    pub object_bom_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_occurrence_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_occurrence_id: Option<String>,
    pub outcome: BomReferenceOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VulnerabilityRating {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VexAnalysis {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_issued: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AffectedVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryRangeEvent {
    pub event: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryRange {
    pub range_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<AdvisoryRangeEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryAffectedPackage {
    pub ecosystem: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purl: Option<Address>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<AdvisoryRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixed_versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentAffect {
    pub affect_id: String,
    /// Provider-native component reference (`bom-ref`) or normalized occurrence id.
    pub component_reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_occurrence_id: Option<String>,
    pub outcome: BomReferenceOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<AffectedVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentVulnerability {
    pub schema_version: String,
    pub observation_id: String,
    pub artifact_id: String,
    pub provider_pointer: String,
    pub vulnerability_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bom_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advisory_schema_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ratings: Vec<VulnerabilityRating>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<VexAnalysis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<ComponentAffect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advisory_affected: Vec<AdvisoryAffectedPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BomComposition {
    pub schema_version: String,
    pub composition_id: String,
    pub artifact_id: String,
    pub provider_pointer: String,
    pub aggregate: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assemblies: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vulnerabilities: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ComponentValidationError {
    #[error("unsupported {record} schema_version: {version}")]
    UnsupportedVersion {
        record: &'static str,
        version: String,
    },
    #[error("component field `{0}` must be non-empty and canonical")]
    InvalidField(&'static str),
    #[error("invalid artifact id: {0}")]
    InvalidArtifactId(#[from] IdParseError),
    #[error("invalid component PURL address")]
    InvalidPurl,
    #[error("derived record identity does not match its fields")]
    IdentityMismatch,
    #[error("BOM reference outcome does not match resolved occurrence ids")]
    OutcomeMismatch,
}

impl ComponentOccurrence {
    pub fn build(
        artifact_id: String,
        provider_pointer: String,
        component_type: String,
        name: String,
    ) -> Result<Self, ComponentValidationError> {
        let occurrence_id = hash_parts(&["component_occurrence", &artifact_id, &provider_pointer]);
        let record = Self {
            schema_version: COMPONENT_OCCURRENCE_SCHEMA.into(),
            occurrence_id,
            artifact_id,
            provider_pointer,
            bom_ref: None,
            purl: None,
            component_type,
            ecosystem: None,
            group: None,
            name,
            version: None,
            source_type: None,
            declared_in: None,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn with_bom_ref(
        mut self,
        bom_ref: Option<String>,
    ) -> Result<Self, ComponentValidationError> {
        self.bom_ref = bom_ref;
        self.validate()?;
        Ok(self)
    }

    pub fn with_purl(mut self, purl: Option<String>) -> Result<Self, ComponentValidationError> {
        self.purl = purl
            .map(|purl| purl_address(&purl))
            .transpose()
            .map_err(|_| ComponentValidationError::InvalidPurl)?;
        self.validate()?;
        Ok(self)
    }

    pub fn with_coordinates(
        mut self,
        group: Option<String>,
        version: Option<String>,
    ) -> Result<Self, ComponentValidationError> {
        self.group = group;
        self.version = version;
        self.validate()?;
        Ok(self)
    }

    pub fn with_source(
        mut self,
        ecosystem: Option<String>,
        source_type: Option<String>,
        declared_in: Option<SourceLocation>,
    ) -> Result<Self, ComponentValidationError> {
        self.ecosystem = ecosystem;
        self.source_type = source_type;
        self.declared_in = declared_in;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ComponentValidationError> {
        schema(
            &self.schema_version,
            COMPONENT_OCCURRENCE_SCHEMA,
            "component occurrence",
        )?;
        ArtifactId::parse(&self.artifact_id)?;
        required(&self.provider_pointer, "provider_pointer")?;
        required(&self.component_type, "component_type")?;
        required(&self.name, "name")?;
        optional(&self.bom_ref, "bom_ref")?;
        optional(&self.group, "group")?;
        optional(&self.ecosystem, "ecosystem")?;
        optional(&self.version, "version")?;
        optional(&self.source_type, "source_type")?;
        if let Some(location) = &self.declared_in {
            let path = normalize_repo_path(&location.path)
                .map_err(|_| ComponentValidationError::InvalidField("declared_in.path"))?;
            if path != location.path || !matches!(location.extent, LocationExtent::File) {
                return Err(ComponentValidationError::InvalidField("declared_in"));
            }
        }
        if let Some(purl) = &self.purl {
            if !matches!(purl, Address::Purl { .. }) || purl.validate().is_err() {
                return Err(ComponentValidationError::InvalidPurl);
            }
        }
        if self.occurrence_id
            != hash_parts(&[
                "component_occurrence",
                &self.artifact_id,
                &self.provider_pointer,
            ])
        {
            return Err(ComponentValidationError::IdentityMismatch);
        }
        Ok(())
    }
}

impl ComponentDependency {
    pub fn build(
        artifact_id: String,
        provider_pointer: String,
        kind: ComponentDependencyKind,
        subject_bom_ref: String,
        object_bom_ref: String,
        subject_occurrence_id: Option<String>,
        object_occurrence_id: Option<String>,
    ) -> Result<Self, ComponentValidationError> {
        let dependency_id = hash_parts(&[
            "component_dependency",
            &artifact_id,
            &provider_pointer,
            &subject_bom_ref,
            &object_bom_ref,
            match kind {
                ComponentDependencyKind::DependsOn => "depends_on",
                ComponentDependencyKind::Provides => "provides",
            },
        ]);
        let outcome = reference_outcome(&subject_occurrence_id, &object_occurrence_id);
        let record = Self {
            schema_version: COMPONENT_DEPENDENCY_SCHEMA.into(),
            dependency_id,
            artifact_id,
            provider_pointer,
            kind,
            subject_bom_ref,
            object_bom_ref,
            subject_occurrence_id,
            object_occurrence_id,
            outcome,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), ComponentValidationError> {
        schema(
            &self.schema_version,
            COMPONENT_DEPENDENCY_SCHEMA,
            "component dependency",
        )?;
        ArtifactId::parse(&self.artifact_id)?;
        for (value, field) in [
            (&self.provider_pointer, "provider_pointer"),
            (&self.subject_bom_ref, "subject_bom_ref"),
            (&self.object_bom_ref, "object_bom_ref"),
        ] {
            required(value, field)?;
        }
        if self.outcome
            != reference_outcome(&self.subject_occurrence_id, &self.object_occurrence_id)
        {
            return Err(ComponentValidationError::OutcomeMismatch);
        }
        let expected = hash_parts(&[
            "component_dependency",
            &self.artifact_id,
            &self.provider_pointer,
            &self.subject_bom_ref,
            &self.object_bom_ref,
            match self.kind {
                ComponentDependencyKind::DependsOn => "depends_on",
                ComponentDependencyKind::Provides => "provides",
            },
        ]);
        if self.dependency_id != expected {
            return Err(ComponentValidationError::IdentityMismatch);
        }
        Ok(())
    }
}

impl ComponentVulnerability {
    pub fn validate(&self) -> Result<(), ComponentValidationError> {
        schema(
            &self.schema_version,
            COMPONENT_VULNERABILITY_SCHEMA,
            "component vulnerability",
        )?;
        ArtifactId::parse(&self.artifact_id)?;
        required(&self.provider_pointer, "provider_pointer")?;
        required(&self.vulnerability_id, "vulnerability_id")?;
        optional(&self.bom_ref, "bom_ref")?;
        optional(&self.advisory_schema_version, "advisory_schema_version")?;
        optional(&self.modified, "modified")?;
        optional(&self.withdrawn, "withdrawn")?;
        if self.observation_id
            != hash_parts(&[
                "component_vulnerability",
                &self.artifact_id,
                &self.provider_pointer,
            ])
        {
            return Err(ComponentValidationError::IdentityMismatch);
        }
        for (affect_index, affect) in self.affects.iter().enumerate() {
            required(&affect.component_reference, "component_reference")?;
            if affect.affect_id
                != hash_parts(&[
                    "component_affect",
                    &self.observation_id,
                    &affect_index.to_string(),
                    &affect.component_reference,
                ])
            {
                return Err(ComponentValidationError::IdentityMismatch);
            }
            if affect.outcome
                != reference_outcome(
                    &Some(self.observation_id.clone()),
                    &affect.component_occurrence_id,
                )
            {
                return Err(ComponentValidationError::OutcomeMismatch);
            }
            for version in &affect.versions {
                optional(&version.version, "affected_version.version")?;
                optional(&version.range, "affected_version.range")?;
                optional(&version.status, "affected_version.status")?;
            }
        }
        for alias in &self.aliases {
            required(alias, "alias")?;
        }
        for related in &self.related {
            required(related, "related")?;
        }
        optional(&self.source_name, "source_name")?;
        optional(&self.source_url, "source_url")?;
        for rating in &self.ratings {
            optional(&rating.source, "rating.source")?;
            optional(&rating.score, "rating.score")?;
            optional(&rating.severity, "rating.severity")?;
            optional(&rating.method, "rating.method")?;
            optional(&rating.vector, "rating.vector")?;
        }
        if let Some(analysis) = &self.analysis {
            required(&analysis.state, "analysis.state")?;
            optional(&analysis.justification, "analysis.justification")?;
            optional(&analysis.detail, "analysis.detail")?;
            optional(&analysis.first_issued, "analysis.first_issued")?;
            optional(&analysis.last_updated, "analysis.last_updated")?;
            for response in &analysis.responses {
                required(response, "analysis.response")?;
            }
        }
        for affected in &self.advisory_affected {
            required(&affected.ecosystem, "advisory_affected.ecosystem")?;
            required(&affected.name, "advisory_affected.name")?;
            if let Some(purl) = &affected.purl {
                if !matches!(purl, Address::Purl { .. }) || purl.validate().is_err() {
                    return Err(ComponentValidationError::InvalidPurl);
                }
            }
            for range in &affected.ranges {
                required(&range.range_type, "advisory_range.range_type")?;
                optional(&range.repository, "advisory_range.repository")?;
                for event in &range.events {
                    required(&event.event, "advisory_range.event")?;
                    required(&event.value, "advisory_range.value")?;
                }
            }
            for version in &affected.versions {
                required(version, "advisory_affected.version")?;
            }
            for fixed_version in &affected.fixed_versions {
                required(fixed_version, "advisory_affected.fixed_version")?;
            }
        }
        Ok(())
    }
}

impl BomComposition {
    pub fn validate(&self) -> Result<(), ComponentValidationError> {
        schema(
            &self.schema_version,
            BOM_COMPOSITION_SCHEMA,
            "BOM composition",
        )?;
        ArtifactId::parse(&self.artifact_id)?;
        required(&self.provider_pointer, "provider_pointer")?;
        required(&self.aggregate, "aggregate")?;
        if self.composition_id
            != hash_parts(&["bom_composition", &self.artifact_id, &self.provider_pointer])
        {
            return Err(ComponentValidationError::IdentityMismatch);
        }
        Ok(())
    }
}

pub fn reference_outcome(subject: &Option<String>, object: &Option<String>) -> BomReferenceOutcome {
    match (subject.is_some(), object.is_some()) {
        (true, true) => BomReferenceOutcome::Resolved,
        (false, true) => BomReferenceOutcome::MissingSubject,
        (true, false) => BomReferenceOutcome::MissingObject,
        (false, false) => BomReferenceOutcome::MissingSubjectAndObject,
    }
}

fn schema(
    actual: &str,
    expected: &str,
    record: &'static str,
) -> Result<(), ComponentValidationError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ComponentValidationError::UnsupportedVersion {
            record,
            version: actual.into(),
        })
    }
}
fn required(value: &str, field: &'static str) -> Result<(), ComponentValidationError> {
    if value.trim().is_empty() || value.trim() != value || value.contains('\0') {
        Err(ComponentValidationError::InvalidField(field))
    } else {
        Ok(())
    }
}
fn optional(value: &Option<String>, field: &'static str) -> Result<(), ComponentValidationError> {
    if let Some(value) = value {
        required(value, field)?;
    }
    Ok(())
}
