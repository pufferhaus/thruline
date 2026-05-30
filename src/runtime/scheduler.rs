// src/runtime/scheduler.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelJob {
    pub slot:   usize,
    pub input:  serde_json::Value,
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scheduler {
    pub stage:     String,
    pub limit:     Option<u32>,
    pub jobs:      Vec<ParallelJob>,
    pub next_slot: usize,
    pub completed: usize,
}

impl Scheduler {
    pub fn new(stage: String, items: Vec<serde_json::Value>, limit: Option<u32>) -> Self {
        let jobs = items.into_iter().enumerate().map(|(i, input)| ParallelJob {
            slot: i,
            input,
            output: None,
        }).collect();
        Self { stage, limit, jobs, next_slot: 0, completed: 0 }
    }

    pub fn total(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_done(&self) -> bool {
        self.completed == self.jobs.len()
    }

    /// Returns the slot index of the next job to dispatch, respecting the concurrency
    /// limit. In-flight count = next_slot - completed.
    pub fn next_to_dispatch(&self) -> Option<usize> {
        let in_flight = self.next_slot - self.completed;
        let limit = self.limit.map(|l| l as usize).unwrap_or(usize::MAX);
        if self.next_slot < self.jobs.len() && in_flight < limit {
            Some(self.next_slot)
        } else {
            None
        }
    }

    /// Advance next_slot and return a reference to the newly dispatched job.
    pub fn dispatch(&mut self) -> Option<&ParallelJob> {
        let idx = self.next_to_dispatch()?;
        self.next_slot += 1;
        Some(&self.jobs[idx])
    }

    /// Record the output for the given slot and increment completed count.
    pub fn complete_slot(&mut self, slot: usize, output: serde_json::Value) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.slot == slot) {
            job.output = Some(output);
            self.completed += 1;
        }
    }

    /// Collect all outputs in slot order. Only valid when is_done() is true.
    pub fn collect_results(&self) -> Vec<serde_json::Value> {
        self.jobs.iter().filter_map(|j| j.output.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<serde_json::Value> {
        (0..n).map(|i| serde_json::json!({"i": i})).collect()
    }

    #[test]
    fn test_unbounded_dispatches_immediately() {
        let mut s = Scheduler::new("impl".to_string(), items(4), None);
        assert_eq!(s.next_to_dispatch(), Some(0));
        s.dispatch();
        assert_eq!(s.next_to_dispatch(), Some(1));
        s.dispatch();
        assert_eq!(s.next_to_dispatch(), Some(2));
    }

    #[test]
    fn test_limit_blocks_at_capacity() {
        let mut s = Scheduler::new("impl".to_string(), items(5), Some(2));
        s.dispatch(); // slot 0
        s.dispatch(); // slot 1
        // 2 in-flight, limit reached
        assert_eq!(s.next_to_dispatch(), None);
        // complete slot 0, frees a slot
        s.complete_slot(0, serde_json::json!({"ok": true}));
        assert_eq!(s.next_to_dispatch(), Some(2));
    }

    #[test]
    fn test_limit_1_serializes() {
        let mut s = Scheduler::new("w".to_string(), items(3), Some(1));
        assert_eq!(s.next_to_dispatch(), Some(0));
        s.dispatch();
        assert_eq!(s.next_to_dispatch(), None);
        s.complete_slot(0, serde_json::json!(null));
        assert_eq!(s.next_to_dispatch(), Some(1));
    }

    #[test]
    fn test_is_done_after_all_complete() {
        let mut s = Scheduler::new("w".to_string(), items(2), None);
        s.dispatch(); s.dispatch();
        s.complete_slot(0, serde_json::json!(null));
        assert!(!s.is_done());
        s.complete_slot(1, serde_json::json!(null));
        assert!(s.is_done());
    }

    #[test]
    fn test_collect_results_preserves_slot_order() {
        let mut s = Scheduler::new("w".to_string(), items(3), None);
        s.dispatch(); s.dispatch(); s.dispatch();
        // Complete out of order
        s.complete_slot(2, serde_json::json!({"slot": 2}));
        s.complete_slot(0, serde_json::json!({"slot": 0}));
        s.complete_slot(1, serde_json::json!({"slot": 1}));
        let results = s.collect_results();
        assert_eq!(results.len(), 3);
        // Jobs stored in slot order; collect_results iterates jobs in insertion order
        assert_eq!(results[0]["slot"], 0);
        assert_eq!(results[1]["slot"], 1);
        assert_eq!(results[2]["slot"], 2);
    }

    #[test]
    fn test_empty_scheduler_is_done() {
        let s = Scheduler::new("w".to_string(), vec![], None);
        assert!(s.is_done());
        assert_eq!(s.total(), 0);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut s = Scheduler::new("stage".to_string(), items(2), Some(1));
        s.dispatch();
        s.complete_slot(0, serde_json::json!({"done": true}));
        let json = serde_json::to_string(&s).unwrap();
        let back: Scheduler = serde_json::from_str(&json).unwrap();
        assert_eq!(back.completed, 1);
        assert_eq!(back.next_slot, 1);
    }
}
