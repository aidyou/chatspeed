//! Shared accounting and circuit-breaking independent of conversation ownership.

use super::platform;
use super::types::{ProcessSample, ResourceLimits, ResourceViolation, TaskGuard, CONFIG_KEY};
use crate::db::MainStore;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

static CONTROLLER: OnceLock<Arc<Controller>> = OnceLock::new();

#[derive(Default)]
struct State {
    tasks: HashMap<u32, Task>,
    blocked: bool,
}

struct Task {
    root: ProcessSample,
    members: HashMap<u32, u64>,
    failure: Arc<Mutex<Option<ResourceViolation>>>,
    terminate: Arc<dyn Fn() + Send + Sync>,
    detached: bool,
}

struct Termination {
    members: Vec<ProcessSample>,
    root: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Default)]
pub(super) struct Controller {
    store: Mutex<Weak<MainStore>>,
    state: Mutex<State>,
    monitor_started: OnceLock<bool>,
}

impl TaskGuard {
    pub fn failure(&self) -> Option<ResourceViolation> {
        *self.failure.lock()
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some(task) = self.controller.state.lock().tasks.get_mut(&self.pid) {
            task.detached = true;
        }
    }
}

pub fn install(store: &Arc<MainStore>) {
    let controller = CONTROLLER.get_or_init(|| Arc::new(Controller::default()));
    *controller.store.lock() = Arc::downgrade(store);
}

pub fn reset() {
    if let Some(controller) = CONTROLLER.get() {
        controller.state.lock().blocked = false;
    }
}

pub fn check_admission() -> Result<(), ResourceViolation> {
    let Some(controller) = CONTROLLER.get() else {
        return Ok(());
    };
    let Some(limits) = controller.limits()? else {
        return Ok(());
    };
    if !limits.enabled {
        return Ok(());
    }
    {
        let state = controller.state.lock();
        if state.blocked || state.tasks.len() < limits.max_concurrent_tasks as usize {
            return admit(&state, &limits);
        }
    }
    let samples = platform::snapshot().map_err(|_| ResourceViolation::MonitoringUnavailable)?;
    let mut state = controller.state.lock();
    state
        .tasks
        .retain(|_, task| !task.detached || !task_members(task, &samples).is_empty());
    admit(&state, &limits)
}

pub fn register(
    pid: u32,
    terminate: impl Fn() + Send + Sync + 'static,
) -> Result<Option<TaskGuard>, ResourceViolation> {
    let Some(controller) = CONTROLLER.get() else {
        return Ok(None);
    };
    let Some(limits) = controller.limits()? else {
        return Ok(None);
    };
    if !limits.enabled {
        return Ok(None);
    }
    let samples = platform::snapshot().map_err(|_| ResourceViolation::MonitoringUnavailable)?;
    let root = samples
        .iter()
        .find(|process| process.pid == pid)
        .cloned()
        .or_else(|| {
            samples
                .iter()
                .find(|process| process.parent == pid || (cfg!(unix) && process.group == pid))
                .map(|_| ProcessSample {
                    pid,
                    parent: 0,
                    group: if cfg!(unix) { pid } else { 0 },
                    started: 0,
                    memory_bytes: 0,
                })
        });
    let Some(root) = root else { return Ok(None) };
    let started = controller.monitor_started.get_or_init(|| {
        let weak = Arc::downgrade(controller);
        std::thread::Builder::new()
            .name("ai-resource-monitor".to_string())
            .spawn(move || {
                while let Some(controller) = weak.upgrade() {
                    controller.tick();
                    drop(controller);
                    std::thread::sleep(Duration::from_millis(250));
                }
            })
            .is_ok()
    });
    if !started {
        return Err(ResourceViolation::MonitoringUnavailable);
    }
    let failure = Arc::new(Mutex::new(None));
    let mut state = controller.state.lock();
    state
        .tasks
        .retain(|_, task| !task.detached || !task_members(task, &samples).is_empty());
    admit(&state, &limits)?;
    state.tasks.insert(
        pid,
        Task {
            members: HashMap::from([(pid, root.started)]),
            root,
            failure: failure.clone(),
            terminate: Arc::new(terminate),
            detached: false,
        },
    );
    Ok(Some(TaskGuard {
        controller: controller.clone(),
        pid,
        failure,
    }))
}

fn admit(state: &State, limits: &ResourceLimits) -> Result<(), ResourceViolation> {
    if state.blocked {
        return Err(ResourceViolation::CircuitOpen);
    }
    if state.tasks.len() >= limits.max_concurrent_tasks as usize {
        return Err(ResourceViolation::ConcurrentTasks);
    }
    Ok(())
}

impl Controller {
    fn limits(&self) -> Result<Option<ResourceLimits>, ResourceViolation> {
        let Some(store) = self.store.lock().upgrade() else {
            return Ok(None);
        };
        let limits = match store.config.get_setting(CONFIG_KEY) {
            Some(value) => {
                serde_json::from_value(value).map_err(|_| ResourceViolation::InvalidConfig)?
            }
            None => ResourceLimits::default(),
        };
        limits.validate()?;
        Ok(Some(limits))
    }

    fn tick(&self) {
        if self.state.lock().tasks.is_empty() {
            return;
        }
        let limits = self.limits();
        let samples = platform::snapshot();
        for termination in self.evaluate(limits, samples) {
            if let Some(terminate) = termination.root {
                terminate();
            }
            platform::terminate(&termination.members);
        }
    }

    fn evaluate(
        &self,
        limits: Result<Option<ResourceLimits>, ResourceViolation>,
        samples: std::io::Result<Vec<ProcessSample>>,
    ) -> Vec<Termination> {
        let mut state = self.state.lock();
        let mut terminations = Vec::new();
        match (limits, samples) {
            (Ok(limits), Ok(samples)) => {
                let mut usages = Vec::new();
                state.tasks.retain(|pid, task| {
                    let members = task_members(task, &samples);
                    if members.is_empty() && task.detached {
                        return false;
                    }
                    let memory = members
                        .iter()
                        .map(|process| process.memory_bytes)
                        .sum::<u64>();
                    if let Some(limits) = limits.as_ref().filter(|limits| limits.enabled) {
                        if let Some(reason) = task_violation(limits, memory, members.len()) {
                            task.failure.lock().get_or_insert(reason);
                        }
                    }
                    usages.push((*pid, memory, members));
                    true
                });
                if let Some(limits) = limits.as_ref().filter(|limits| limits.enabled) {
                    let mut unique = HashSet::new();
                    let mut total = usages
                        .iter()
                        .flat_map(|(_, _, members)| members)
                        .filter(|process| unique.insert(process.pid))
                        .map(|process| process.memory_bytes)
                        .sum::<u64>();
                    usages.sort_by_key(|(_, memory, _)| std::cmp::Reverse(*memory));
                    for (pid, memory, _) in &usages {
                        if total <= u64::from(limits.total_memory_mb) * 1024 * 1024 {
                            break;
                        }
                        if let Some(task) = state.tasks.get(pid) {
                            task.failure
                                .lock()
                                .get_or_insert(ResourceViolation::TotalMemory);
                            total = total.saturating_sub(*memory);
                        }
                    }
                }
                for (pid, _, members) in usages {
                    if let Some(task) = state.tasks.get(&pid) {
                        if task.failure.lock().is_some() {
                            let root = members
                                .iter()
                                .any(|process| {
                                    process.pid == task.root.pid
                                        && process.started == task.root.started
                                })
                                .then(|| task.terminate.clone());
                            terminations.push(Termination { members, root });
                        }
                    }
                }
            }
            _ => {
                for task in state.tasks.values() {
                    task.failure
                        .lock()
                        .get_or_insert(ResourceViolation::MonitoringUnavailable);
                    terminations.push(Termination {
                        members: Vec::new(),
                        root: (!task.detached).then(|| task.terminate.clone()),
                    });
                }
            }
        }
        if !terminations.is_empty() {
            if !state.blocked {
                log::warn!("AI execution resource protection tripped; refusing new shell tasks until settings are saved");
            }
            state.blocked = true;
        }
        terminations
    }
}

fn task_violation(
    limits: &ResourceLimits,
    memory: u64,
    processes: usize,
) -> Option<ResourceViolation> {
    if memory > u64::from(limits.task_memory_mb) * 1024 * 1024 {
        Some(ResourceViolation::TaskMemory)
    } else if processes > limits.task_max_processes as usize {
        Some(ResourceViolation::ProcessCount)
    } else {
        None
    }
}

fn task_members(task: &mut Task, samples: &[ProcessSample]) -> Vec<ProcessSample> {
    let root_reused = samples
        .iter()
        .any(|process| process.pid == task.root.pid && process.started != task.root.started);
    let mut members: HashSet<u32> = samples
        .iter()
        .filter(|process| {
            task.members.get(&process.pid) == Some(&process.started)
                || (!root_reused && process.parent == task.root.pid)
                || (!root_reused
                    && task.root.group == task.root.pid
                    && process.group == task.root.group)
        })
        .map(|process| process.pid)
        .collect();
    loop {
        let count = members.len();
        for process in samples {
            if members.contains(&process.parent) {
                members.insert(process.pid);
            }
        }
        if count == members.len() {
            break;
        }
    }
    let found: Vec<_> = samples
        .iter()
        .filter(|process| members.contains(&process.pid))
        .cloned()
        .collect();
    task.members = found
        .iter()
        .map(|process| (process.pid, process.started))
        .collect();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, parent: u32, group: u32, started: u64) -> ProcessSample {
        ProcessSample {
            pid,
            parent,
            group,
            started,
            memory_bytes: 1024,
        }
    }

    fn task() -> Task {
        Task {
            root: sample(100, 1, 100, 1),
            members: HashMap::from([(100, 1)]),
            failure: Arc::new(Mutex::new(None)),
            terminate: Arc::new(|| {}),
            detached: false,
        }
    }

    #[test]
    fn resource_accounting_follows_children_groups_and_reparented_members() {
        let mut task = task();
        let members = task_members(
            &mut task,
            &[
                sample(100, 1, 100, 1),
                sample(101, 100, 100, 2),
                sample(102, 101, 102, 3),
                sample(900, 1, 900, 4),
            ],
        );
        assert_eq!(members.len(), 3);
        let members = task_members(
            &mut task,
            &[sample(102, 1, 102, 3), sample(101, 1, 900, 99)],
        );
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].pid, 102);
    }

    #[test]
    fn resource_admission_is_global_and_circuit_stays_open() {
        let mut state = State::default();
        let limits = ResourceLimits {
            max_concurrent_tasks: 1,
            ..Default::default()
        };
        state.tasks.insert(100, task());
        assert_eq!(
            admit(&state, &limits),
            Err(ResourceViolation::ConcurrentTasks)
        );
        state.tasks.clear();
        state.blocked = true;
        assert_eq!(admit(&state, &limits), Err(ResourceViolation::CircuitOpen));
    }

    #[test]
    fn resource_task_memory_and_process_budgets_are_independent() {
        let limits = ResourceLimits::default();
        assert_eq!(
            task_violation(&limits, 4096 * 1024 * 1024 + 1, 1),
            Some(ResourceViolation::TaskMemory)
        );
        assert_eq!(
            task_violation(&limits, 1, 129),
            Some(ResourceViolation::ProcessCount)
        );
        assert_eq!(task_violation(&limits, 4096 * 1024 * 1024, 128), None);
    }

    #[test]
    fn resource_total_budget_terminates_only_enough_tasks_and_opens_circuit() {
        let controller = Controller::default();
        let first = task();
        let mut second = task();
        second.root = sample(200, 1, 200, 2);
        second.members = HashMap::from([(200, 2)]);
        controller.state.lock().tasks = HashMap::from([(100, first), (200, second)]);
        let limits = ResourceLimits {
            total_memory_mb: 512,
            task_memory_mb: 512,
            ..Default::default()
        };
        let samples = vec![
            ProcessSample {
                memory_bytes: 300 * 1024 * 1024,
                ..sample(100, 1, 100, 1)
            },
            ProcessSample {
                memory_bytes: 300 * 1024 * 1024,
                ..sample(200, 1, 200, 2)
            },
        ];
        let terminations = controller.evaluate(Ok(Some(limits)), Ok(samples));
        assert_eq!(terminations.len(), 1);
        let state = controller.state.lock();
        assert!(state.blocked);
        assert_eq!(
            state
                .tasks
                .values()
                .filter(|task| *task.failure.lock() == Some(ResourceViolation::TotalMemory))
                .count(),
            1
        );
    }

    #[test]
    fn resource_task_budget_counts_the_whole_process_tree() {
        let controller = Controller::default();
        controller.state.lock().tasks.insert(100, task());
        let limits = ResourceLimits {
            task_memory_mb: 128,
            ..Default::default()
        };
        let samples = vec![
            ProcessSample {
                memory_bytes: 64 * 1024 * 1024,
                ..sample(100, 1, 100, 1)
            },
            ProcessSample {
                memory_bytes: 80 * 1024 * 1024,
                ..sample(101, 100, 100, 2)
            },
        ];
        let terminations = controller.evaluate(Ok(Some(limits)), Ok(samples));
        assert_eq!(terminations[0].members.len(), 2);
        assert_eq!(
            *controller.state.lock().tasks[&100].failure.lock(),
            Some(ResourceViolation::TaskMemory)
        );
    }

    #[test]
    fn resource_accounting_does_not_adopt_a_reused_root_pid() {
        let mut task = task();
        let members = task_members(
            &mut task,
            &[sample(100, 1, 100, 99), sample(101, 100, 100, 100)],
        );
        assert!(members.is_empty());
    }

    #[test]
    fn resource_monitor_failure_blocks_execution_instead_of_silently_disabling_protection() {
        let controller = Controller::default();
        controller.state.lock().tasks.insert(100, task());
        let terminations = controller.evaluate(
            Ok(Some(ResourceLimits::default())),
            Err(std::io::Error::other("test")),
        );
        assert_eq!(terminations.len(), 1);
        assert_eq!(
            *controller.state.lock().tasks[&100].failure.lock(),
            Some(ResourceViolation::MonitoringUnavailable)
        );
        assert!(controller.state.lock().blocked);
    }
}
