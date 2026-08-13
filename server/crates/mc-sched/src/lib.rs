//! 多路 CPU 调度原型:虚拟 NUMA 拓扑 + per-CPU 队列 + 距离感知工作窃取。
//! 验证目标:均衡性、无空转围观、同 node 窃取优先。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct Topology {
    pub nodes: Vec<usize>,
    pub cpu_node: Vec<usize>,
    pub distance: Vec<Vec<u32>>,
}

impl Topology {
    pub fn new(node_count: usize, cpus_per_node: usize) -> Self {
        let cpu_node: Vec<usize> = (0..node_count)
            .flat_map(|n| std::iter::repeat(n).take(cpus_per_node))
            .collect();
        let distance = (0..node_count)
            .map(|a| {
                (0..node_count)
                    .map(|b| if a == b { 0 } else { 10 * (a.abs_diff(b) as u32) })
                    .collect()
            })
            .collect();
        Topology { nodes: vec![cpus_per_node; node_count], cpu_node, distance }
    }

    pub fn cpus(&self) -> usize {
        self.cpu_node.len()
    }

    pub fn node_of(&self, cpu: usize) -> usize {
        self.cpu_node[cpu]
    }
}

pub struct Task {
    pub id: usize,
    pub cost: u32,
}

pub struct CpuState {
    queue: Mutex<VecDeque<Task>>,
    done: AtomicUsize,
    local_steals: AtomicUsize,
    remote_steals: AtomicUsize,
}

impl CpuState {
    fn new() -> Self {
        CpuState {
            queue: Mutex::new(VecDeque::new()),
            done: AtomicUsize::new(0),
            local_steals: AtomicUsize::new(0),
            remote_steals: AtomicUsize::new(0),
        }
    }
}

pub struct Scheduler {
    topology: Topology,
    cpus: Vec<CpuState>,
    running: AtomicBool,
    total: AtomicUsize,
}

impl Scheduler {
    pub fn new(node_count: usize, cpus_per_node: usize) -> Self {
        Scheduler {
            topology: Topology::new(node_count, cpus_per_node),
            cpus: (0..node_count * cpus_per_node).map(|_| CpuState::new()).collect(),
            running: AtomicBool::new(false),
            total: AtomicUsize::new(0),
        }
    }

    pub fn submit(&self, tasks: Vec<Task>) {
        let cpus = self.cpus.len();
        for t in tasks {
            let cpu = t.id % cpus;
            self.submit_to(cpu, t);
        }
    }

    pub fn submit_to(&self, cpu: usize, task: Task) {
        self.cpus[cpu].queue.lock().unwrap().push_back(task);
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn run(&self, workers: usize) -> Vec<(usize, usize, usize, usize, usize)> {
        let done_total = AtomicUsize::new(0);
        self.running.store(true, Ordering::SeqCst);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..workers)
                .map(|cpu| {
                    let sch = self;
                    let done_total = &done_total;
                    s.spawn(move || sch.worker(cpu, done_total))
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
        });
        (0..self.cpus.len())
            .map(|c| {
                (
                    c,
                    self.topology.node_of(c),
                    self.cpus[c].done.load(Ordering::Relaxed),
                    self.cpus[c].local_steals.load(Ordering::Relaxed),
                    self.cpus[c].remote_steals.load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    fn worker(&self, cpu: usize, done_total: &AtomicUsize) {
        loop {
            let task = self
                .pop(cpu)
                .or_else(|| self.steal(cpu));
            match task {
                Some(t) => {
                    std::thread::sleep(std::time::Duration::from_micros(t.cost as u64));
                    self.cpus[cpu].done.fetch_add(1, Ordering::Relaxed);
                    done_total.fetch_add(1, Ordering::Relaxed);
                }
                None => {
                    if done_total.load(Ordering::Relaxed) >= self.total.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::yield_now();
                }
            }
        }
    }

    fn pop(&self, cpu: usize) -> Option<Task> {
        self.cpus[cpu].queue.lock().unwrap().pop_front()
    }

    fn steal(&self, cpu: usize) -> Option<Task> {
        let node = self.topology.node_of(cpu);
        let mut best: Option<(usize, u32)> = None;
        for other in 0..self.cpus.len() {
            if other == cpu {
                continue;
            }
            let has = self.cpus[other].queue.lock().unwrap().front().is_some();
            if !has {
                continue;
            }
            let d = self.topology.distance[node][self.topology.node_of(other)];
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((other, d));
            }
        }
        let (from, _) = best?;
        let task = self.cpus[from].queue.lock().unwrap().pop_front()?;
        if self.topology.node_of(from) == node {
            self.cpus[cpu].local_steals.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cpus[cpu].remote_steals.fetch_add(1, Ordering::Relaxed);
        }
        Some(task)
    }
}
