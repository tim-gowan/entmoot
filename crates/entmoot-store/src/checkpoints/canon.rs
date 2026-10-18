//! Persisted canon progress; no parsing or production.
use entmoot_contracts::*;
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonProduction {
    pub parsed_files: usize,
    pub reused_files: usize,
}

/// Durable counters are private operational state, never evidence on their own.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonCursor {
    pub completed: usize,
    pub patterns_left: usize,
    pub total_bytes: usize,
    pub limits: Vec<CanonLimit>,
    pub stopped: bool,
    pub production: CanonProduction,
}

impl CanonCursor {
    pub fn follows(&self, prior: &Self) -> bool {
        self.completed >= prior.completed
            && self.patterns_left <= prior.patterns_left
            && self.total_bytes >= prior.total_bytes
            && self.production.parsed_files >= prior.production.parsed_files
            && self.production.reused_files >= prior.production.reused_files
            && self
                .production
                .parsed_files
                .saturating_add(self.production.reused_files)
                <= self.completed
            && (!prior.stopped || self.stopped)
            && prior.limits.iter().all(|limit| self.limits.contains(limit))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonFileOutcome {
    pub input: entmoot_core::scan::ScanInput,
    pub claim: Option<RelationClaim>,
    pub edges: Vec<RelationEdge>,
}

impl CanonFileOutcome {
    pub fn valid_for(&self, request: &ProducerRequest) -> bool {
        if !matches!(request.task, ProducerTask::Canon { .. }) {
            return false;
        }
        let Some(claim) = &self.claim else {
            return self.edges.is_empty();
        };
        let RelationClaimValue::Canon {
            observation,
            digest,
        } = &claim.value
        else {
            return false;
        };
        let CanonObservation::File { file } = observation.as_ref() else {
            return false;
        };
        file.path == self.input.path
            && file.source_digest.as_str() == self.input.content_digest
            && observation.validate().is_ok()
            && observation.digest().is_ok_and(|value| &value == digest)
            && claim.producer.id == "entmoot.canon"
            && claim.scope.domain == request.source.snapshot.repository_id
            && claim.scope.snapshot_revision.as_ref() == Some(&request.source.snapshot.revision)
            && claim.scope.snapshot_digest == request.source.snapshot.snapshot_digest
    }
}
