//! mc-sched 演示:模拟双路/四路/八路拓扑,展示负载均衡与窃取行为。

use mc_sched::{Scheduler, Task};
use std::time::Instant;

fn variance(items: &[usize]) -> f64 {
    let mean = items.iter().sum::<usize>() as f64 / items.len() as f64;
    items
        .iter()
        .map(|v| {
            let d = *v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / items.len() as f64
}

fn main() {
    for (nodes, cpus_per_node) in [(2, 4), (2, 8), (4, 4), (8, 2)] {
        let sch = Scheduler::new(nodes, cpus_per_node);
        let n_cpu = nodes * cpus_per_node;
        let tasks: Vec<Task> = (0..1024)
            .map(|id| Task { id, cost: 8 + (id % 13) as u32 })
            .collect();
        sch.submit(tasks);
        let t0 = Instant::now();
        let stats = sch.run(n_cpu);
        let elapsed = t0.elapsed().as_micros();
        let done: Vec<usize> = stats.iter().map(|s| s.2).collect();
        let steals: Vec<usize> = stats.iter().map(|s| s.3).collect();
        let per_node: Vec<usize> = (0..nodes)
            .map(|n| stats.iter().filter(|s| s.1 == n).map(|s| s.2).sum())
            .collect();
        println!(
            "topology: {} nodes x {} cpus = {} cores | tasks=1024 | {}us | per-core variance={:.1} | steals={} | per-node: {:?}",
            nodes,
            cpus_per_node,
            n_cpu,
            elapsed,
            variance(&done),
            steals.iter().sum::<usize>(),
            per_node,
        );
    }
}
