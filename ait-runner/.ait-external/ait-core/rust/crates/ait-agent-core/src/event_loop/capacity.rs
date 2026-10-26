use serde::{Deserialize, Serialize};

use super::AgentEventLoopBackend;

pub const DEFAULT_WORKERS_PER_EPOLL_SHARD: usize = 256;
pub const DEFAULT_WORKERS_PER_POLL_SHARD: usize = 32;
pub const MIN_HIGH_CONCURRENCY_WORKERS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEventLoopConfig {
    pub backend: AgentEventLoopBackend,
    pub workers_per_shard: usize,
    pub expected_workers: usize,
}

impl AgentEventLoopConfig {
    pub fn for_expected_workers(expected_workers: usize) -> Self {
        let backend = AgentEventLoopBackend::current_platform_default();
        let workers_per_shard = match backend {
            AgentEventLoopBackend::LinuxEpoll => DEFAULT_WORKERS_PER_EPOLL_SHARD,
            AgentEventLoopBackend::PortablePoll => DEFAULT_WORKERS_PER_POLL_SHARD,
        };
        Self {
            backend,
            workers_per_shard,
            expected_workers,
        }
    }

    pub fn shard_count(&self) -> usize {
        if self.expected_workers == 0 {
            return 0;
        }
        self.expected_workers
            .div_ceil(self.workers_per_shard.max(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeCapacity {
    pub backend: AgentEventLoopBackend,
    pub expected_workers: usize,
    pub workers_per_shard: usize,
    pub shard_count: usize,
    pub high_concurrency: bool,
    pub requires_epoll_for_target_scale: bool,
}

impl AgentRuntimeCapacity {
    pub fn from_config(config: AgentEventLoopConfig) -> Self {
        let shard_count = config.shard_count();
        let high_concurrency = config.expected_workers >= MIN_HIGH_CONCURRENCY_WORKERS;
        Self {
            backend: config.backend,
            expected_workers: config.expected_workers,
            workers_per_shard: config.workers_per_shard,
            shard_count,
            high_concurrency,
            requires_epoll_for_target_scale: high_concurrency && !config.backend.is_epoll(),
        }
    }
}

#[cfg(test)]
mod tests;
