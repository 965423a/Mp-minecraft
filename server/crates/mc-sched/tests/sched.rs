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
    for id in 0..128 {
        sch.submit_to(0, Task { id, cost: 50 });
    }
    let stats = sch.run(8);
    let steals: usize = stats.iter().map(|s| s.3 + s.4).sum();
    assert!(steals > 0, "应有窃取发生,否则有空转核围观");
    let per_cpu: Vec<usize> = stats.iter().map(|s| s.2).collect();
    assert!(per_cpu.iter().all(|&v| v > 0), "存在空转围观核: {per_cpu:?}");
}

#[test]
fn local_node_steal_preferred() {
    let sch = Scheduler::new(2, 2);
    for id in 0..64 {
        sch.submit_to(0, Task { id, cost: 100 });
    }
    let stats = sch.run(2);
    let local: usize = stats.iter().map(|s| s.3).sum();
    let remote: usize = stats.iter().map(|s| s.4).sum();
    assert!(local > 0, "同 node 邻居应通过窃取获得任务");
    assert_eq!(remote, 0, "仅 node0 核运行时不应跨 node 窃取");
    assert!(stats[1].2 >= 1, "cpu1 应通过窃取获得至少一个任务");
    assert_eq!(stats.iter().map(|s| s.2).sum::<usize>(), 64);
}

#[test]
fn deep_imbalance_gets_remote_help() {
    let sch = Scheduler::new(2, 2);
    for id in 0..64 {
        sch.submit_to(0, Task { id, cost: 200 });
    }
    let stats = sch.run(4);
    let remote: usize = stats.iter().map(|s| s.4).sum();
    assert!(remote > 0, "node1 核应跨 node 窃取帮忙");
    let per_cpu: Vec<usize> = stats.iter().map(|s| s.2).collect();
    assert!(per_cpu.iter().all(|&v| v > 0), "跨 node 核应通过窃取帮忙: {per_cpu:?}");
    assert_eq!(per_cpu.iter().sum::<usize>(), 64);
}
