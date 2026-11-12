//! Trusted-host actor task lifetime ledger (not a source Task compiler/runtime).
//! Actor execution and carrier authenticity MUST be checked by the native
//! scheduler before calling this ledger. No guest closure is executed here.
//! The ledger deliberately never invents a physical quiescence receipt.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorIncarnation {
    pub id: u64,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStage {
    Ready,
    Running,
    Suspended,
    Done,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskError {
    StaleActor,
    Stopped,
    Capacity,
    ExhaustedIds,
    MissingTask,
    InvalidTransition,
    ParentUnavailable,
    HasLiveChildren,
    NotQuiescent,
    Cancelled,
    ActorBusy,
}
#[derive(Clone, Copy, Debug)]
struct Entry {
    parent: Option<TaskId>,
    stage: TaskStage,
    cancel_requested: bool,
}

/// A bounded per-actor logical task ledger. `Done` records still count toward
/// the actor's completion barrier until the runtime explicitly calls `retire`.
/// An awaited Future producer does not hold the actor's physical carrier lease.
pub struct ActorTaskLedger {
    incarnation: ActorIncarnation,
    capacity: usize,
    next_id: u64,
    stopped: bool,
    tasks: BTreeMap<TaskId, Entry>,
}
impl ActorTaskLedger {
    pub fn new(incarnation: ActorIncarnation, capacity: usize) -> Result<Self, TaskError> {
        if incarnation.id == 0 || incarnation.generation == 0 || capacity == 0 || capacity > 1024 {
            return Err(TaskError::InvalidTransition);
        }
        Ok(Self { incarnation, capacity, next_id: 0, stopped: false, tasks: BTreeMap::new() })
    }
    fn check_owner(&self, owner: ActorIncarnation) -> Result<(), TaskError> {
        if owner != self.incarnation { Err(TaskError::StaleActor) } else { Ok(()) }
    }
    pub fn admit_in_authenticated_turn(
        &mut self, owner: ActorIncarnation, parent: Option<TaskId>
    ) -> Result<TaskId, TaskError> {
        self.check_owner(owner)?;
        if self.stopped { return Err(TaskError::Stopped); }
        if self.tasks.len() >= self.capacity { return Err(TaskError::Capacity); }
        if let Some(p) = parent {
            let entry = self.tasks.get(&p).ok_or(TaskError::ParentUnavailable)?;
            if entry.cancel_requested || entry.stage == TaskStage::Done {
                return Err(TaskError::ParentUnavailable);
            }
        }
        self.next_id = self.next_id.checked_add(1).ok_or(TaskError::ExhaustedIds)?;
        let id = TaskId(self.next_id);
        self.tasks.insert(id, Entry { parent, stage: TaskStage::Ready, cancel_requested: false });
        Ok(id)
    }
    pub fn start_or_resume(
        &mut self, owner: ActorIncarnation, id: TaskId
    ) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        if self.stopped { return Err(TaskError::Stopped); }
        // Suspended tasks are logical concurrency, not simultaneous
        // mutable execution on two physical carriers for one actor.
        if self.tasks.iter().any(|(other, task)|
            *other != id && task.stage == TaskStage::Running) {
            return Err(TaskError::ActorBusy);
        }
        let task = self.tasks.get_mut(&id).ok_or(TaskError::MissingTask)?;
        if task.cancel_requested { return Err(TaskError::Cancelled); }
        if task.stage != TaskStage::Ready && task.stage != TaskStage::Suspended {
            return Err(TaskError::InvalidTransition);
        }
        task.stage = TaskStage::Running;
        Ok(())
    }
    pub fn suspend(&mut self, owner: ActorIncarnation, id: TaskId) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        let task = self.tasks.get_mut(&id).ok_or(TaskError::MissingTask)?;
        if task.stage != TaskStage::Running { return Err(TaskError::InvalidTransition); }
        task.stage = TaskStage::Suspended;
        Ok(())
    }
    /// Called only after the task's physical turn has returned.
    pub fn complete_turn(&mut self, owner: ActorIncarnation, id: TaskId) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        let task = self.tasks.get_mut(&id).ok_or(TaskError::MissingTask)?;
        if task.stage != TaskStage::Running { return Err(TaskError::InvalidTransition); }
        task.stage = TaskStage::Done;
        Ok(())
    }
    fn is_descendant_or_self(&self, mut id: TaskId, ancestor: TaskId) -> bool {
        loop {
            if id == ancestor { return true; }
            let Some(parent) = self.tasks.get(&id).and_then(|task| task.parent) else {
                return false;
            };
            id = parent;
        }
    }
    /// Cancellation is a request. It does NOT imply physical completion.
    /// A completed parent's result remains settled, but live children are
    /// still marked for cancellation.
    pub fn request_cancel(
        &mut self, owner: ActorIncarnation, id: TaskId
    ) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        if !self.tasks.contains_key(&id) { return Err(TaskError::MissingTask); }
        let descendants: Vec<_> = self.tasks.keys().copied()
            .filter(|child| self.is_descendant_or_self(*child, id)).collect();
        for child in descendants {
            self.tasks.get_mut(&child).expect("live descendant").cancel_requested = true;
        }
        Ok(())
    }
    pub fn stop(&mut self, owner: ActorIncarnation) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        self.stopped = true;
        for task in self.tasks.values_mut() { task.cancel_requested = true; }
        Ok(())
    }
    /// Scheduler-owned cancellation cleanup callback for never-started or
    /// suspended tasks. An actively running guest turn MUST return first.
    pub fn cleanup_cancelled(
        &mut self, owner: ActorIncarnation, id: TaskId
    ) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        let task = self.tasks.get_mut(&id).ok_or(TaskError::MissingTask)?;
        if !task.cancel_requested ||
            (task.stage != TaskStage::Ready && task.stage != TaskStage::Suspended) {
            return Err(TaskError::NotQuiescent);
        }
        task.stage = TaskStage::Done;
        Ok(())
    }
    /// Must follow proof that this exact task's physical callback/cleanup has
    /// returned. Never infer this from a Promise result or Future settlement.
    pub fn retire(&mut self, owner: ActorIncarnation, id: TaskId) -> Result<(), TaskError> {
        self.check_owner(owner)?;
        let task = self.tasks.get(&id).ok_or(TaskError::MissingTask)?;
        if task.stage != TaskStage::Done { return Err(TaskError::NotQuiescent); }
        if self.tasks.values().any(|entry| entry.parent == Some(id)) {
            return Err(TaskError::HasLiveChildren);
        }
        self.tasks.remove(&id);
        Ok(())
    }
    pub fn is_cancel_requested(&self, id: TaskId) -> Option<bool> {
        self.tasks.get(&id).map(|task| task.cancel_requested)
    }
    pub fn stage(&self, id: TaskId) -> Option<TaskStage> {
        self.tasks.get(&id).map(|task| task.stage)
    }
    pub fn outstanding(&self) -> usize { self.tasks.len() }
    pub fn is_physically_quiescent(&self) -> bool { self.tasks.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: ActorIncarnation = ActorIncarnation { id: 7, generation: 11 };
    fn ledger(n: usize) -> ActorTaskLedger { ActorTaskLedger::new(OWNER, n).unwrap() }
    #[test]
    fn completed_parent_still_owns_and_cancels_nested_child() {
        let mut l = ledger(8);
        let p = l.admit_in_authenticated_turn(OWNER, None).unwrap();
        l.start_or_resume(OWNER, p).unwrap();
        let c = l.admit_in_authenticated_turn(OWNER, Some(p)).unwrap();
        l.complete_turn(OWNER, p).unwrap();
        assert_eq!(l.retire(OWNER, p), Err(TaskError::HasLiveChildren));
        l.request_cancel(OWNER, p).unwrap();
        assert_eq!(l.stage(p), Some(TaskStage::Done));
        assert_eq!(l.start_or_resume(OWNER, c), Err(TaskError::Cancelled));
        l.cleanup_cancelled(OWNER, c).unwrap();
        l.retire(OWNER, c).unwrap();
        l.retire(OWNER, p).unwrap();
        assert!(l.is_physically_quiescent());
    }
    #[test]
    fn stop_during_await_keeps_done_barrier_closed_until_cleanup() {
        let mut l = ledger(8);
        let t = l.admit_in_authenticated_turn(OWNER, None).unwrap();
        l.start_or_resume(OWNER, t).unwrap();
        l.suspend(OWNER, t).unwrap();
        l.stop(OWNER).unwrap();
        assert_eq!(l.start_or_resume(OWNER, t), Err(TaskError::Stopped));
        assert_eq!(l.admit_in_authenticated_turn(OWNER, None), Err(TaskError::Stopped));
        assert!(!l.is_physically_quiescent());
        l.cleanup_cancelled(OWNER, t).unwrap();
        assert!(!l.is_physically_quiescent());
        l.retire(OWNER, t).unwrap();
        assert!(l.is_physically_quiescent());
    }
    #[test]
    fn running_task_cannot_be_cleanup_retired_by_stop() {
        let mut l = ledger(8);
        let t = l.admit_in_authenticated_turn(OWNER, None).unwrap();
        l.start_or_resume(OWNER, t).unwrap();
        l.stop(OWNER).unwrap();
        assert_eq!(l.cleanup_cancelled(OWNER, t), Err(TaskError::NotQuiescent));
        assert_eq!(l.retire(OWNER, t), Err(TaskError::NotQuiescent));
        l.complete_turn(OWNER, t).unwrap();
        l.retire(OWNER, t).unwrap();
        assert!(l.is_physically_quiescent());
    }
    #[test]
    fn quota_and_generation_fence() {
        let mut l = ledger(1);
        let t = l.admit_in_authenticated_turn(OWNER, None).unwrap();
        assert_eq!(l.admit_in_authenticated_turn(OWNER, None), Err(TaskError::Capacity));
        let stale = ActorIncarnation { id: 7, generation: 12 };
        assert_eq!(l.stop(stale), Err(TaskError::StaleActor));
        assert_eq!(l.start_or_resume(stale, t), Err(TaskError::StaleActor));
        l.start_or_resume(OWNER, t).unwrap();
        l.complete_turn(OWNER, t).unwrap();
        l.retire(OWNER, t).unwrap();
        assert!(l.admit_in_authenticated_turn(OWNER, None).is_ok());
    }
    #[test]
    fn concurrent_logical_tasks_never_run_mutable_actor_turns_together() {
        let mut l = ledger(2);
        let a = l.admit_in_authenticated_turn(OWNER, None).unwrap();
        let b = l.admit_in_authenticated_turn(OWNER, None).unwrap();
        l.start_or_resume(OWNER, a).unwrap();
        assert_eq!(l.start_or_resume(OWNER, b), Err(TaskError::ActorBusy));
        l.suspend(OWNER, a).unwrap();
        l.start_or_resume(OWNER, b).unwrap();
        assert_eq!(l.start_or_resume(OWNER, a), Err(TaskError::ActorBusy));
        l.complete_turn(OWNER, b).unwrap();
        l.retire(OWNER, b).unwrap();
        l.start_or_resume(OWNER, a).unwrap();
        l.complete_turn(OWNER, a).unwrap();
        l.retire(OWNER, a).unwrap();
        assert!(l.is_physically_quiescent());
    }
}
