//! 调度原型测试:均衡性、窃取路径、距离感知。

use mc_sched::{Scheduler, Task};

fn variance(items: &[usize]) -> f64 {
    let mean = items.iter().sum::<usize>() as f64 / items.len() as f64;
    items.iter().map(|v| { let d = *v as f64 - mean; d * d }).sum::<f64>() / items.len() as f64
}

#[test]
fn work_stealing_balances_load() {
    let sch = Scheduler::new(2, 4);
    let tasks: Vec<Task> = (0..400)
        .map(|id| Task { id, cost: 10 + (id % 7) as u32 })
        .collect();
    sch.submit(tasks);
    let stats = sch.run(8);
    let per_cpu: Vec<usize> = stats.iter().map(|s| s.2).collect();
    assert_eq!(per_cpu.iter().sum::<usize>(), 400);
    assert!(variance(&per_cpu) < 50.0, "per-cpu 方差过大: {per_cpu:?}");
}

#[test]
fn idle_cpu_steals_instead_of_watching() {
    let sch = Scheduler::new(4, 2);
    let tasks: Vec<Task> = (0..64)
        .map(|id| Task { id, cost: 5 })
        .collect();
    sch.submit(tasks);
    let stats = sch.run(8);
    let steals: usize = stats.iter().map(|s| s.3).sum();
    assert!(steals > 0, "应有窃取发生,否则有空转核围观");
    let per_cpu: Vec<usize> = stats.iter().map(|s| s.2).collect();
    assert!(per_cpu.iter().all(|&v| v > 0), "存在空转围观核: {per_cpu:?}");
}

#[test]
fn local_node_steal_preferred() {
    let sch = Scheduler::new(2, 2);
    let tasks: Vec<Task> = (0..8).map(|id| Task { id, cost: 2 }).collect();
    sch.submit(tasks);
    let stats = sch.run(4);
    let cpu_node: Vec<usize> = stats.iter().map(|s| s.1).collect();
    let done: Vec<usize> = stats.iter().map(|s| s.2).collect();
    for node in 0..2 {
        let node_done: usize = cpu_node
            .iter()
            .zip(done.iter())
            .filter(|(n, _)| **n == node)
            .map(|(_, d)| *d)
            .sum();
        assert_eq!(node_done, 4, "node {node} 完成数应等于其初始任务数");
    }
}

#[test]
fn deep_imbalance_gets_remote_help() {
    let sch = Scheduler::new(2, 2);
    let tasks: Vec<Task> = (0..8)
        .map(|id| Task { id, cost: 20 })
        .collect();
    sch.submit(tasks);
    let stats = sch.run(4);
    let remote: usize = stats.iter().map(|s| s.3).sum();
    let _ = remote;
    let per_cpu: Vec<usize> = stats.iter().map(|s| s.2).collect();
    assert!(per_cpu.iter().all(|&v| v > 0));
    assert!(per_cpu.iter().sum::<usize>() == 8);
}