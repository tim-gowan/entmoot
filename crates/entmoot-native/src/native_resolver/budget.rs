//! Wall-clock and cardinality budgets for native producer runs.

use std::time::{Duration, Instant};

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerBudget {
    pub wall: Duration,
    pub max_files: usize,
    pub max_call_sites: usize,
    pub max_targets: usize,
}

impl ProducerBudget {
    pub const DEFAULT_WALL_SECONDS: u64 = 3600;
}

impl Default for ProducerBudget {
    fn default() -> Self {
        Self {
            wall: Duration::from_secs(Self::DEFAULT_WALL_SECONDS),
            max_files: 50_000,
            max_call_sites: 500_000,
            max_targets: 1_000_000,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProducerBudgetError {
    #[error("native producer wall-clock budget exhausted")]
    WallClock,
    #[error("native producer file budget exceeded ({observed} > {limit})")]
    Files { observed: usize, limit: usize },
    #[error("native producer call-site budget exceeded ({observed} > {limit})")]
    CallSites { observed: usize, limit: usize },
    #[error("native producer target budget exceeded ({observed} > {limit})")]
    Targets { observed: usize, limit: usize },
}

#[derive(Debug, Clone)]
pub struct BudgetWatch {
    budget: ProducerBudget,
    started: Instant,
}

impl BudgetWatch {
    pub fn start(budget: ProducerBudget) -> Self {
        Self {
            budget,
            started: Instant::now(),
        }
    }

    pub fn check_wall(&self) -> Result<(), ProducerBudgetError> {
        if self.started.elapsed() > self.budget.wall {
            return Err(ProducerBudgetError::WallClock);
        }
        Ok(())
    }

    pub fn check_files(&self, observed: usize) -> Result<(), ProducerBudgetError> {
        if observed > self.budget.max_files {
            return Err(ProducerBudgetError::Files {
                observed,
                limit: self.budget.max_files,
            });
        }
        Ok(())
    }

    pub fn check_call_sites(&self, observed: usize) -> Result<(), ProducerBudgetError> {
        if observed > self.budget.max_call_sites {
            return Err(ProducerBudgetError::CallSites {
                observed,
                limit: self.budget.max_call_sites,
            });
        }
        Ok(())
    }

    pub fn check_targets(&self, observed: usize) -> Result<(), ProducerBudgetError> {
        if observed > self.budget.max_targets {
            return Err(ProducerBudgetError::Targets {
                observed,
                limit: self.budget.max_targets,
            });
        }
        Ok(())
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn budget(&self) -> ProducerBudget {
        self.budget
    }
}
