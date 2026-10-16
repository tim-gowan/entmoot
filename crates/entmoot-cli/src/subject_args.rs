//! CLI parsing for publication subjects and correspondence policies.

use anyhow::{bail, Context, Result};
use entmoot_contracts::{CorrespondenceMode, CorrespondencePolicy, SourceSubject, SubjectAlias};

/// Parse `pkg:…` as a package subject, or `repo` / `repo:path` as repository subject.
pub fn parse_subject(raw: &str) -> Result<SourceSubject> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("subject must be non-empty");
    }
    if value.starts_with("pkg:") {
        return Ok(SourceSubject::package(value)?);
    }
    // Repository subjects use `#path` for monorepo scope (URLs may contain `:`).
    let (id, path) = match value.split_once('#') {
        Some((id, path)) if !path.is_empty() => (id.to_owned(), Some(path.to_owned())),
        _ => (value.to_owned(), None),
    };
    let subject = SourceSubject::Repository { id, path };
    subject.validate()?;
    Ok(subject)
}

/// Parse `from=to` alias pairs.
pub fn parse_alias(raw: &str) -> Result<SubjectAlias> {
    let (from, to) = raw
        .split_once('=')
        .context("alias must be from=to (subject forms)")?;
    Ok(SubjectAlias {
        from: parse_subject(from)?,
        to: parse_subject(to)?,
    })
}

pub fn parse_correspondence_mode(raw: &str) -> Result<CorrespondenceMode> {
    match raw.trim() {
        "subject_match" => Ok(CorrespondenceMode::SubjectMatch),
        "relative_occurrence" => Ok(CorrespondenceMode::RelativeOccurrence),
        "explicit_alias" => Ok(CorrespondenceMode::ExplicitAlias),
        other => bail!("unknown correspondence mode: {other}"),
    }
}

pub fn build_correspondence_policy(
    mode: Option<&str>,
    subject: Option<&str>,
    aliases: &[String],
) -> Result<Option<CorrespondencePolicy>> {
    if mode.is_none() && subject.is_none() && aliases.is_empty() {
        return Ok(None);
    }
    let mode = match mode {
        Some(raw) => parse_correspondence_mode(raw)?,
        None if !aliases.is_empty() => CorrespondenceMode::ExplicitAlias,
        None => CorrespondenceMode::RelativeOccurrence,
    };
    let policy = CorrespondencePolicy {
        mode,
        subject: subject.map(parse_subject).transpose()?,
        aliases: aliases
            .iter()
            .map(|row| parse_alias(row))
            .collect::<Result<Vec<_>>>()?,
    };
    policy.validate()?;
    Ok(Some(policy))
}
