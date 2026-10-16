//! Repository-wide admission, separate from host CPU/memory enforcement.
use super::{ContentDigest, OperationKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

impl OperationKind {
    pub const ALL: [Self; 7] = [
        Self::Inventory,
        Self::Binding,
        Self::Canon,
        Self::Revalidation,
        Self::Conflicts,
        Self::Integration,
        Self::Query,
    ];
}

/// Logical work credits bound simultaneous leased operations. They are not
/// measured CPU/memory or an operating-system resource limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerPolicy {
    #[schemars(range(min = 1, max = 64))]
    pub max_running: u16,
    /// Zero disables new binding admissions without revoking existing workers.
    #[schemars(range(min = 0, max = 64))]
    pub max_binding: u16,
    #[schemars(range(min = 1, max = 65_536))]
    pub work_units: u32,
    /// Per-kind overrides; binding costs two credits, other kinds one by default.
    pub costs: BTreeMap<OperationKind, u32>,
    #[schemars(range(min = 1, max = 10_000))]
    pub max_waiting: u32,
    /// Explicit run retries preserve FIFO position while this interest is live.
    /// An abandoned request expires; queued jobs do not execute automatically.
    #[schemars(range(min = 1_000, max = 3_600_000))]
    pub wait_lease_ms: u64,
}

impl Default for SchedulerPolicy {
    fn default() -> Self {
        Self {
            max_running: 2,
            max_binding: 1,
            work_units: 3,
            costs: BTreeMap::new(),
            max_waiting: 1024,
            wait_lease_ms: 60_000,
        }
    }
}

impl SchedulerPolicy {
    pub fn cost(&self, kind: OperationKind) -> u32 {
        self.costs
            .get(&kind)
            .copied()
            .unwrap_or(if kind == OperationKind::Binding { 2 } else { 1 })
    }
    pub fn enabled(&self, kind: OperationKind) -> bool {
        kind != OperationKind::Binding || self.max_binding > 0
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.max_running)
            || self.max_binding > self.max_running
            || !(1..=65_536).contains(&self.work_units)
            || !(1..=10_000).contains(&self.max_waiting)
            || !(1_000..=3_600_000).contains(&self.wait_lease_ms)
            || OperationKind::ALL
                .into_iter()
                .any(|kind| !(1..=self.work_units).contains(&self.cost(kind)))
        {
            return Err("scheduler requires 1..64 running slots, 0..running binding slots, 1..65536 credits, positive per-kind costs within credits, 1..10000 waiting tickets and a 1000..3600000 ms waiting lease".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchedulerConfiguration {
    pub generation: u64,
    pub policy: SchedulerPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduledOperation {
    pub id: ContentDigest,
    pub kind: OperationKind,
    pub work_units: u32,
    pub expires_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchedulerStatus {
    pub repository_id: String,
    pub configuration: SchedulerConfiguration,
    pub observed_at_ms: u64,
    pub active: Vec<ScheduledOperation>,
    /// At most 100 live tickets in FIFO order. Paused jobs have no tickets.
    pub waiting: Vec<ScheduledOperation>,
    pub waiting_total: usize,
    pub waiting_truncated: bool,
    pub used_work_units: u64,
    /// Oldest enabled waiter; this observation does not reserve admission.
    pub next: Option<ContentDigest>,
    pub next_can_run: bool,
}
