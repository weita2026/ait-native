use super::*;

#[test]
fn high_concurrency_capacity_requires_epoll_when_not_linux() {
    let capacity = AgentRuntimeCapacity::from_config(AgentEventLoopConfig {
        backend: AgentEventLoopBackend::PortablePoll,
        workers_per_shard: 32,
        expected_workers: 128,
    });

    assert_eq!(capacity.shard_count, 4);
    assert!(capacity.high_concurrency);
    assert!(capacity.requires_epoll_for_target_scale);
}

#[test]
fn epoll_capacity_keeps_large_worker_sets_on_few_shards() {
    let capacity = AgentRuntimeCapacity::from_config(AgentEventLoopConfig {
        backend: AgentEventLoopBackend::LinuxEpoll,
        workers_per_shard: 256,
        expected_workers: 513,
    });

    assert_eq!(capacity.shard_count, 3);
    assert!(capacity.high_concurrency);
    assert!(!capacity.requires_epoll_for_target_scale);
}
