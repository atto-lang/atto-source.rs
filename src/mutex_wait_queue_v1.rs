//! Host-reference, single-arbiter, NON-GUEST mutex waiter state machine.
//!
//! This module deliberately does not implement the Atto Mutex<T> type,
//! a scheduler, tenant isolation or an async ABI. A privileged scheduler
//! would serialize calls and mint continuation identities. No blocking OS
//! primitive, thread spawn, spin-loop or guest pointer is used here.
//! Never treat these unit tests as compiled-guest / TenantVM security evidence.
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Domain {
    tenant: u64,
    actor: u64,
    incarnation: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Continuation {
    domain: Domain,
    logical_task: u64,
    segment: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket {
    generation: u64,
    sequence: u64,
    continuation: Continuation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Granted(Ticket),
    Waiting(Ticket),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Closed,
    ForeignDomain,
    Reentrant,
    QueueFull,
    Exhausted,
    StaleTicket,
    CancellationPending,
    AlreadyClaimed,
    UnknownTask,
    DuplicateTask,
    NotQuiescent,
    CleanupFailed,
    AdmissionBusy,
    UnclaimedGrant,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cancellation {
    Queued,
    GrantedAndRevoked,
    /// Active lease cancellation is deferred until cleanup releases it.
    ActiveDeferred,
}

/// This intentionally exposes no constructor for an untrusted guest.
/// The caller must be an already privileged, serialized host scheduler.
pub(crate) struct WaitQueue {
    domain: Domain,
    generation: u64,
    next_sequence: u64,
    max_waiters: usize,
    closed: bool,
    owner: Option<Ticket>,
    active: bool,
    cancel_requested: bool,
    waiting: VecDeque<Ticket>,
    // At most one outstanding direct handoff wakeup for the current owner.
    wakeup: Option<Ticket>,
}
impl WaitQueue {
    pub(crate) fn new(domain: Domain, generation: u64, max_waiters: usize) -> Result<Self, Error> {
        if generation == 0 || max_waiters == 0 || max_waiters > 8192 {
            return Err(Error::Exhausted);
        }
        Ok(Self {
            domain, generation, next_sequence: 1, max_waiters, closed: false,
            owner: None, active: false, cancel_requested: false,
            waiting: VecDeque::new(), wakeup: None,
        })
    }

    fn check_domain(&self, who: Continuation) -> Result<(), Error> {
        if who.domain != self.domain { Err(Error::ForeignDomain) } else { Ok(()) }
    }
    fn fresh(&mut self, who: Continuation) -> Result<Ticket, Error> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(Ticket { generation: self.generation, sequence, continuation: who })
    }

    pub(crate) fn acquire(&mut self, who: Continuation) -> Result<Admission, Error> {
        if self.closed { return Err(Error::Closed); }
        self.check_domain(who)?;
        if self.owner.is_some_and(|t| t.continuation == who)
            || self.waiting.iter().any(|t| t.continuation == who) {
            return Err(Error::Reentrant);
        }
        // Admission and the capacity check happen before minting a ticket.
        // A failed request never silently consumes owner or queue rights.
        if self.owner.is_some() && self.waiting.len() >= self.max_waiters {
            return Err(Error::QueueFull);
        }
        let token = self.fresh(who)?;
        if self.owner.is_none() {
            debug_assert!(self.waiting.is_empty());
            self.owner = Some(token);
            Ok(Admission::Granted(token))
        } else {
            self.waiting.push_back(token);
            Ok(Admission::Waiting(token))
        }
    }

    fn handoff(&mut self) {
        // Only a privileged reactor may drain this wakeup; any token it
        // delivers must still be checked with `claim` on resume.
        let next = self.waiting.pop_front();
        self.owner = next;
        self.active = false;
        self.cancel_requested = false;
        self.wakeup = next;
    }

    pub(crate) fn release(&mut self, owner: Ticket) -> Result<(), Error> {
        if self.owner != Some(owner) || owner.generation != self.generation {
            return Err(Error::StaleTicket);
        }
        // Only an active claimed guard may be *released*. An unclaimed grant
        // must be revoked through cancel(), which serializes the handoff.
        // Otherwise a stale grant could be "released" without ever acquiring
        // the exclusive lease, hiding a lifetime/cancellation bug.
        if !self.active { return Err(Error::UnclaimedGrant); }
        // An active task can still unwind and release after shutdown, but
        // shutdown must not hand this lock to another task.
        if self.closed {
            self.owner = None;
            self.active = false;
            self.cancel_requested = false;
            self.wakeup = None;
        } else {
            self.handoff();
        }
        Ok(())
    }

    /// A wakeup is a hint to the scheduler, not a capability that can bypass
    /// revalidation after cancellation/revocation. It is not exposed to guest.
    pub(crate) fn take_wakeup(&mut self) -> Option<Ticket> {
        self.wakeup.take()
    }

    pub(crate) fn claim(&mut self, token: Ticket) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        self.check_domain(token.continuation)?;
        if self.owner != Some(token) || token.generation != self.generation {
            return Err(Error::StaleTicket);
        }
        if self.cancel_requested { return Err(Error::CancellationPending); }
        if self.active { return Err(Error::AlreadyClaimed); }
        // Claim is the guarded-entry event. An active grant cannot be revoked.
        self.active = true;
        if self.wakeup == Some(token) { self.wakeup = None; }
        Ok(())
    }

    /// Metadata is not proof that a guest continuation physically retired.
    pub(crate) fn has_active_lease(&self) -> bool { self.owner.is_some() && self.active }
    pub(crate) fn cancellation_pending(&self) -> bool { self.cancel_requested }
    pub(crate) fn has_outstanding_grant(&self) -> bool { self.owner.is_some() }

    /// Same ticket cancels either a still-pending request or its already-
    /// granted handoff. The arbiter serializes cancellation versus release.
    pub(crate) fn cancel(&mut self, token: Ticket) -> Result<Cancellation, Error> {
        self.check_domain(token.continuation)?;
        if token.generation != self.generation { return Err(Error::StaleTicket); }
        if self.closed {
            // A shutdown may have closed new admission while an active holder
            // still needs a safe-point unwind; cancellation remains idempotent.
            if self.owner == Some(token) && self.active {
                self.cancel_requested = true;
                return Ok(Cancellation::ActiveDeferred);
            }
            return Err(Error::Closed);
        }
        if let Some(i) = self.waiting.iter().position(|t| *t == token) {
            self.waiting.remove(i);
            return Ok(Cancellation::Queued);
        }
        if self.owner == Some(token) {
            if self.active {
                // Request cancellation and wait for the holder to run its
                // lexical using/defer cleanup. No successor may be granted yet.
                self.cancel_requested = true;
                return Ok(Cancellation::ActiveDeferred);
            }
            // Pending handoff has not entered the critical section.
            self.handoff();
            return Ok(Cancellation::GrantedAndRevoked);
        }
        Err(Error::StaleTicket)
    }

    /// Shutdown cancels waiters and disallows new grants, but cannot revoke
    /// a task actively inside its critical section. Retain active owner until
    /// lexical cleanup calls release(). Task-scope quiescence is still needed.
    pub(crate) fn shutdown(&mut self) -> usize {
        self.closed = true;
        self.wakeup = None;
        let retired = self.waiting.len();
        self.waiting.clear();
        if self.active {
            // Inform active holder to unwind; do NOT publish a free lock.
            self.cancel_requested = true;
        } else {
            self.owner = None;
            self.cancel_requested = false;
        }
        retired
    }
}

/// Host-only, serialized task-local lexical guard stack. Caller MUST stop
/// executing protected guest code before invoking unwind_after_safe_point().
/// This is not an Atto scheduler and does not itself prove descendant quiescence.
pub(crate) struct TaskLeaseStack {
    continuation: Continuation,
    held: Vec<(u64, Ticket)>,
    cancellation_requested: bool,
}
impl TaskLeaseStack {
    pub(crate) fn new(continuation: Continuation) -> Self {
        Self { continuation, held: Vec::new(), cancellation_requested: false }
    }
    pub(crate) fn enter(
        &mut self, lock_id: u64, lock: &mut WaitQueue, ticket: Ticket,
    ) -> Result<(), Error> {
        if self.cancellation_requested { return Err(Error::CancellationPending); }
        if lock_id == 0 || ticket.continuation != self.continuation {
            return Err(Error::ForeignDomain);
        }
        if self.held.iter().any(|(id, _)| *id == lock_id) {
            return Err(Error::Reentrant);
        }
        if self.held.len() >= 256 { return Err(Error::Exhausted); }
        lock.claim(ticket)?;
        self.held.push((lock_id, ticket));
        Ok(())
    }
    /// Idempotent cancellation request. It never releases an active lock;
    /// each active holder must run lexical cleanup at a legal safe point.
    pub(crate) fn request_cancel(
        &mut self, locks: &mut BTreeMap<u64, WaitQueue>,
    ) -> Result<(), Error> {
        if self.cancellation_requested { return Ok(()); }
        // Validate the entire registered ledger before modifying any lock.
        for (id, ticket) in &self.held {
            let lock = locks.get(id).ok_or(Error::StaleTicket)?;
            if lock.owner != Some(*ticket) || !lock.active {
                return Err(Error::StaleTicket);
            }
        }
        for (id, ticket) in &self.held {
            let lock = locks.get_mut(id).ok_or(Error::StaleTicket)?;
            match lock.cancel(*ticket)? {
                Cancellation::ActiveDeferred => {}
                _ => return Err(Error::StaleTicket),
            }
        }
        self.cancellation_requested = true;
        Ok(())
    }
    /// Local cleanup of ALL successfully entered guards, in reverse
    /// acquisition order. A failed release is retained for quarantine/retry,
    /// while independent guards still get their own release attempt.
    /// This does NOT mark the TaskScope or its descendants physically retired.
    pub(crate) fn unwind_after_safe_point(
        &mut self, locks: &mut BTreeMap<u64, WaitQueue>,
    ) -> Result<Vec<u64>, Error> {
        let mut failed = Vec::new();
        let mut released = Vec::new();
        for &(id, ticket) in self.held.iter().rev() {
            let result = locks.get_mut(&id).ok_or(Error::StaleTicket)
                .and_then(|lock| lock.release(ticket));
            match result {
                Ok(()) => released.push(id),
                Err(_) => failed.push((id, ticket)),
            }
        }
        // Preserve acquisition order for future retry, not unwind order.
        failed.reverse();
        self.held = failed;
        if self.held.is_empty() { Ok(released) } else { Err(Error::StaleTicket) }
    }
    pub(crate) fn all_local_locks_released(&self) -> bool { self.held.is_empty() }
}


/// TaskScope-to-mutex integration reference. It owns the per-task waiter and
/// claimed-guard reverse indexes, not just a standalone mutex wait queue.
/// IMPORTANT: this file is #[cfg(test)] in lib.rs. It is NOT a guest scheduler,
/// a signed physical-quiescence witness, or a production Atto capability API.
/// The trusted scheduler must replace the modeled guest-exit/callback receipts
/// with authenticated, separately ordered runtime facts before guest admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskExitKind { Succeeded, Cancelled }

struct ScopeTask {
    continuation: Continuation,
    parent: Option<u64>,
    guards: TaskLeaseStack,
    waiting: Vec<(u64, Ticket)>,
    unclaimed: Vec<(u64, Ticket)>,
    cancel_requested: bool,
    /// Only cancellation of a *suspended lock waiter* needs a scheduler
    /// wake hint. An active critical holder must not be reentered.
    lock_wait_cancel_wake: bool,
    guest_exited: bool,
    notifications_drained: bool,
    physically_retired: bool,
    cleanup_failed: bool,
    exit_kind: Option<TaskExitKind>,
}

/// Serial arbiter: each mutation (grant, cancellation, claim, cleanup) is
/// linearized before a different logical task may observe the lock state.
/// No OS thread ID, ambient socket access, guest pointer, or blocking wait.
struct TaskScopeLockCoordinator {
    domain: Domain,
    /// Terminal TenantVM/TaskScope admission fence. Cleanup of previously
    /// registered guards still proceeds after closure, but neither new
    /// root tasks nor new mutexes may be admitted to the stopped domain.
    closed: bool,
    tasks: BTreeMap<u64, ScopeTask>,
    locks: BTreeMap<u64, WaitQueue>,
}
impl TaskScopeLockCoordinator {
    const MAX_TASKS: usize = 1024;
    const MAX_LOCKS: usize = 256;
    const MAX_TASK_PERMITS: usize = 256;

    fn new(domain: Domain) -> Self {
        Self { domain, closed: false, tasks: BTreeMap::new(), locks: BTreeMap::new() }
    }
    fn add_lock(&mut self, id: u64, queue: WaitQueue) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        if id == 0 || queue.domain != self.domain || self.locks.contains_key(&id) {
            return Err(Error::ForeignDomain);
        }
        // Tickets do not carry a separate lock id: generations MUST be unique
        // across all locks in this TaskScope or a matching sequence/task token
        // could be replayed against a different mutex.
        if self.locks.values().any(|other| other.generation == queue.generation) {
            return Err(Error::StaleTicket);
        }
        if self.locks.len() >= Self::MAX_LOCKS { return Err(Error::Exhausted); }
        self.locks.insert(id, queue);
        Ok(())
    }
    fn registered(&self, who: Continuation) -> Result<&ScopeTask, Error> {
        let task = self.tasks.get(&who.logical_task).ok_or(Error::UnknownTask)?;
        if who.domain != self.domain || task.continuation != who {
            return Err(Error::ForeignDomain);
        }
        Ok(task)
    }
    fn register(&mut self, who: Continuation, parent: Option<Continuation>) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        if who.domain != self.domain || who.logical_task == 0 || who.segment == 0 {
            return Err(Error::ForeignDomain);
        }
        if self.tasks.contains_key(&who.logical_task) {
            return Err(Error::DuplicateTask);
        }
        if self.tasks.len() >= Self::MAX_TASKS { return Err(Error::Exhausted); }
        let parent_id = if let Some(p) = parent {
            let owner = self.registered(p)?;
            if owner.cancel_requested || owner.guest_exited || owner.physically_retired {
                return Err(Error::CancellationPending);
            }
            Some(p.logical_task)
        } else { None };
        self.tasks.insert(who.logical_task, ScopeTask {
            continuation: who,
            parent: parent_id,
            guards: TaskLeaseStack::new(who),
            waiting: Vec::new(), unclaimed: Vec::new(),
            cancel_requested: false, lock_wait_cancel_wake: false,
            guest_exited: false,
            notifications_drained: false, physically_retired: false,
            cleanup_failed: false, exit_kind: None,
        });
        Ok(())
    }
    fn acquire(&mut self, who: Continuation, lock_id: u64) -> Result<Admission, Error> {
        let task = self.registered(who)?;
        if task.cancel_requested || task.guest_exited { return Err(Error::CancellationPending); }
        let permits = task.waiting.len() + task.unclaimed.len() + task.guards.held.len();
        if permits >= Self::MAX_TASK_PERMITS { return Err(Error::Exhausted); }
        let admission = self.locks.get_mut(&lock_id).ok_or(Error::StaleTicket)?.acquire(who)?;
        let task = self.tasks.get_mut(&who.logical_task).ok_or(Error::UnknownTask)?;
        match admission {
            Admission::Waiting(ticket) => task.waiting.push((lock_id, ticket)),
            Admission::Granted(ticket) => task.unclaimed.push((lock_id, ticket)),
        }
        Ok(admission)
    }
    /// A wakeup does not confer ownership: the exact token must be revalidated
    /// against the trusted queue and the owning task's unclaimed/wait ledger.
    fn enter(&mut self, who: Continuation, lock_id: u64, token: Ticket)
        -> Result<(), Error> {
        self.registered(who)?;
        let (tasks, locks) = (&mut self.tasks, &mut self.locks);
        let task = tasks.get_mut(&who.logical_task).ok_or(Error::UnknownTask)?;
        if task.cancel_requested || task.guest_exited { return Err(Error::CancellationPending); }
        let granted = task.unclaimed.iter().position(|(id, t)| *id == lock_id && *t == token);
        let queued = task.waiting.iter().position(|(id, t)| *id == lock_id && *t == token);
        if granted.is_none() && queued.is_none() { return Err(Error::StaleTicket); }
        let queue = locks.get_mut(&lock_id).ok_or(Error::StaleTicket)?;
        // Enter before consuming ledger entry: failure leaves it cancellable.
        task.guards.enter(lock_id, queue, token)?;
        if let Some(i) = granted { task.unclaimed.swap_remove(i); }
        if let Some(i) = queued { task.waiting.swap_remove(i); }
        Ok(())
    }
    fn cancel_one(&mut self, id: u64) -> Result<(), Error> {
        let (tasks, locks) = (&mut self.tasks, &mut self.locks);
        let task = tasks.get_mut(&id).ok_or(Error::UnknownTask)?;
        if task.physically_retired || task.guest_exited { return Ok(()); }
        task.cancel_requested = true; // prevents new claims before any release
        // Wake only a task parked on lock acquisition. Running critical
        // holders observe cancellation at safe points, never by duplicate
        // runnable admission. Channel/timer waits have separate registrars.
        if !task.waiting.is_empty() || !task.unclaimed.is_empty() {
            task.lock_wait_cancel_wake = true;
        }
        let mut failure = None;
        let pending = std::mem::take(&mut task.waiting);
        for (lock_id, ticket) in pending {
            let result = locks.get_mut(&lock_id).ok_or(Error::StaleTicket)
                .and_then(|queue| queue.cancel(ticket));
            if !matches!(result, Ok(Cancellation::Queued | Cancellation::GrantedAndRevoked)) {
                failure.get_or_insert(result.err().unwrap_or(Error::StaleTicket));
                task.waiting.push((lock_id, ticket));
            }
        }
        let unclaimed = std::mem::take(&mut task.unclaimed);
        for (lock_id, ticket) in unclaimed {
            let result = locks.get_mut(&lock_id).ok_or(Error::StaleTicket)
                .and_then(|queue| queue.cancel(ticket));
            if !matches!(result, Ok(Cancellation::GrantedAndRevoked)) {
                failure.get_or_insert(result.err().unwrap_or(Error::StaleTicket));
                task.unclaimed.push((lock_id, ticket));
            }
        }
        if let Err(err) = task.guards.request_cancel(locks) {
            failure.get_or_insert(err);
        }
        if failure.is_some() { task.cleanup_failed = true; }
        failure.map_or(Ok(()), Err)
    }
    /// Requests cancellation for target and all descendants, without joining
    /// or releasing an active guard. The bounded iteration avoids recursion
    /// from untrusted task nesting. Siblings outside the target remain alive.
    fn cancel_tree(&mut self, target: Continuation) -> Result<(), Error> {
        self.registered(target)?;
        let mut ids = vec![target.logical_task];
        let mut index = 0;
        while index < ids.len() {
            let parent = ids[index];
            for (id, task) in &self.tasks {
                if task.parent == Some(parent) && !task.physically_retired {
                    ids.push(*id);
                }
            }
            index += 1;
        }
        // First fence new claims for the entire subtree.
        for id in &ids {
            let task = self.tasks.get_mut(id).ok_or(Error::UnknownTask)?;
            if !task.guest_exited { task.cancel_requested = true; }
        }
        let mut first_failure = None;
        for id in ids {
            if let Err(err) = self.cancel_one(id) {
                first_failure.get_or_insert(err);
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    /// Trusted actor/TenantVM stop: fence all task admissions first, cancel
    /// every registered waiter/grant, then close each lock. Active leases
    /// remain occupied until the real holder completes safe-point unwind.
    /// A failure leaves the domain quarantined; shutdown is NOT retirement.
    fn shutdown_domain(&mut self) -> Result<(), Error> {
        // Fence admission BEFORE cancelling tasks or closing queues. This is
        // a one-way transition; repeated shutdown never reopens the domain.
        // A failed cleanup still keeps the fence in force for all new work.
        self.closed = true;
        for task in self.tasks.values_mut() {
            if !task.guest_exited && !task.physically_retired {
                task.cancel_requested = true;
            }
        }
        let tasks: Vec<u64> = self.tasks.keys().copied().collect();
        let mut first_error = None;
        for id in tasks {
            if let Err(err) = self.cancel_one(id) {
                first_error.get_or_insert(err);
            }
        }
        for lock in self.locks.values_mut() {
            lock.shutdown();
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Called ONLY after a trusted scheduler proves that this guest task has
    /// stopped executing protected code. Do not call this at cancel request.
    /// Normal return, exceptional unwind and cancellation share this path.
    fn guest_exited_at_safe_point(&mut self, who: Continuation) -> Result<Vec<u64>, Error> {
        self.registered(who)?;
        let (tasks, locks) = (&mut self.tasks, &mut self.locks);
        let task = tasks.get_mut(&who.logical_task).ok_or(Error::UnknownTask)?;
        if task.guest_exited { return Err(Error::AlreadyClaimed); }
        // This safe-point transition wins the modeled success/cancel race.
        // A real runtime must derive the winner from an atomic scheduler
        // terminal claim, not from a guest-writable or callback-owned flag.
        task.exit_kind = Some(if task.cancel_requested {
            TaskExitKind::Cancelled
        } else {
            TaskExitKind::Succeeded
        });
        task.guest_exited = true;
        // Lexical TaskScope exit MUST cancel unfinished descendants, even
        // when the parent itself returned successfully. Otherwise discarded
        // children continue running after their scope and may strand locks.
        // Claim the parent's terminal outcome first (so cancelling a child
        // does not turn a successful parent into TaskCancelled), fence future
        // child admission, then request transitive child cancellation.
        let child_ids: Vec<u64> = tasks.iter()
            .filter_map(|(id, child)| {
                (child.parent == Some(who.logical_task) && !child.physically_retired)
                    .then_some(*id)
            })
            .collect();
        // Release the local task's mutable borrow before recursively
        // cancelling children through the same trusted serialized arbiter.
        let _ = task;
        let mut failure = None;
        for id in child_ids {
            let child = self.tasks.get(&id).ok_or(Error::UnknownTask)?.continuation;
            if let Err(err) = self.cancel_tree(child) {
                failure.get_or_insert(err);
            }
        }
        let (tasks, locks) = (&mut self.tasks, &mut self.locks);
        let task = tasks.get_mut(&who.logical_task).ok_or(Error::UnknownTask)?;
        for (lock_id, ticket) in std::mem::take(&mut task.waiting) {
            let r = locks.get_mut(&lock_id).ok_or(Error::StaleTicket)
                .and_then(|queue| queue.cancel(ticket));
            if !matches!(r, Ok(Cancellation::Queued | Cancellation::GrantedAndRevoked)) {
                failure.get_or_insert(r.err().unwrap_or(Error::StaleTicket));
                task.waiting.push((lock_id, ticket));
            }
        }
        for (lock_id, ticket) in std::mem::take(&mut task.unclaimed) {
            let r = locks.get_mut(&lock_id).ok_or(Error::StaleTicket)
                .and_then(|queue| queue.cancel(ticket));
            if !matches!(r, Ok(Cancellation::GrantedAndRevoked)) {
                failure.get_or_insert(r.err().unwrap_or(Error::StaleTicket));
                task.unclaimed.push((lock_id, ticket));
            }
        }
        let released = match task.guards.unwind_after_safe_point(locks) {
            Ok(ids) => ids,
            Err(err) => { failure.get_or_insert(err); Vec::new() }
        };
        if failure.is_some() { task.cleanup_failed = true; }
        match failure { None => Ok(released), Some(err) => Err(err) }
    }
    fn exit_kind(&self, who: Continuation) -> Result<Option<TaskExitKind>, Error> {
        Ok(self.registered(who)?.exit_kind)
    }
    fn notification_drain_ack(&mut self, who: Continuation) -> Result<(), Error> {
        let task = self.registered(who)?;
        if !task.guest_exited { return Err(Error::NotQuiescent); }
        self.tasks.get_mut(&who.logical_task).ok_or(Error::UnknownTask)?
            .notifications_drained = true;
        Ok(())
    }
    /// Public Task DONE cannot be published from just a cancellation flag or
    /// guest-return bit. All active guards, waiters, descendants and callbacks
    /// must have settled; any cleanup failure retains the Task/tenant fence.
    fn certify_physical_retirement(&mut self, who: Continuation) -> Result<(), Error> {
        let task = self.registered(who)?;
        if task.physically_retired { return Ok(()); }
        if task.cleanup_failed { return Err(Error::CleanupFailed); }
        if !task.guest_exited || !task.notifications_drained
            || !task.guards.all_local_locks_released()
            || !task.waiting.is_empty() || !task.unclaimed.is_empty() {
            return Err(Error::NotQuiescent);
        }
        if self.tasks.values().any(|child|
            child.parent == Some(who.logical_task) && !child.physically_retired) {
            return Err(Error::NotQuiescent);
        }
        self.tasks.get_mut(&who.logical_task).ok_or(Error::UnknownTask)?
            .physically_retired = true;
        Ok(())
    }
}


/// Internal, host-side scheduling adapter. It serializes the reference kernel
/// without ever parking a scheduler carrier: a busy arbiter returns Busy and
/// its caller must reschedule. NOT an Atto guest API, scheduler implementation,
/// or a substitute for verified guest-exit and callback-drain evidence.
struct HostTaskLockKernel {
    core: std::sync::Mutex<TaskScopeLockCoordinator>,
    scope_id: u64,
}
static NEXT_KERNEL_SCOPE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);
impl HostTaskLockKernel {
    /// The trusted host must supply its authenticated TenantVM domain.
    fn new(domain: Domain) -> Result<Self, Error> {
        if domain.tenant == 0 || domain.actor == 0 || domain.incarnation == 0 {
            return Err(Error::ForeignDomain);
        }
        let scope_id = NEXT_KERNEL_SCOPE.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |n| n.checked_add(1),
        ).map_err(|_| Error::Exhausted)?;
        Ok(Self {
            core: std::sync::Mutex::new(TaskScopeLockCoordinator::new(domain)),
            scope_id,
        })
    }
    fn try_with_core<R>(
        &self, f: impl FnOnce(&mut TaskScopeLockCoordinator) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let mut guard = match self.core.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::WouldBlock) => return Err(Error::AdmissionBusy),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::CleanupFailed),
        };
        f(&mut guard)
    }
    fn issue(&self, parent: Option<Continuation>) -> Result<Continuation, Error> {
        self.try_with_core(|core| {
            // Never mint a raw task id from guest or actor source input. The
            // complete continuation is a host-issued, scope-bound capability.
            let next = u64::try_from(core.tasks.len())
                .map_err(|_| Error::Exhausted)?
                .checked_add(1).ok_or(Error::Exhausted)?;
            if next > TaskScopeLockCoordinator::MAX_TASKS as u64 {
                return Err(Error::Exhausted);
            }
            let segment = self.scope_id.checked_mul(1024)
                .and_then(|n| n.checked_add(next))
                .ok_or(Error::Exhausted)?;
            let who = Continuation {
                domain: core.domain,
                logical_task: next,
                segment,
            };
            core.register(who, parent)?;
            Ok(who)
        })
    }
    fn add_mutex(&self, max_waiters: usize) -> Result<u64, Error> {
        self.try_with_core(|core| {
            let lock_id = u64::try_from(core.locks.len())
                .map_err(|_| Error::Exhausted)?
                .checked_add(1).ok_or(Error::Exhausted)?;
            // Reserve 512 disjoint generations per scope (MAX_LOCKS=256).
            // Both the scope epoch and lock slot are host-minted, so grants
            // cannot alias across locks, tasks, or TaskScope incarnations.
            let generation = self.scope_id.checked_mul(512)
                .and_then(|epoch| epoch.checked_add(lock_id))
                .ok_or(Error::Exhausted)?;
            let queue = WaitQueue::new(core.domain, generation, max_waiters)?;
            core.add_lock(lock_id, queue)?;
            Ok(lock_id)
        })
    }
    fn acquire(&self, who: Continuation, lock_id: u64) -> Result<Admission, Error> {
        self.try_with_core(|core| core.acquire(who, lock_id))
    }
    fn claim(&self, who: Continuation, lock_id: u64, ticket: Ticket)
        -> Result<(), Error> {
        self.try_with_core(|core| core.enter(who, lock_id, ticket))
    }
    fn cancel(&self, who: Continuation) -> Result<(), Error> {
        self.try_with_core(|core| core.cancel_tree(who))
    }
    /// Host-only STOP; no guest can force retirement by requesting shutdown.
    fn shutdown(&self) -> Result<(), Error> {
        self.try_with_core(|core| core.shutdown_domain())
    }
    /// This must only be invoked by a scheduler after its actual guest code
    /// has stopped touching leased memory. It is NOT an untrusted guest call.
    fn trusted_exit(&self, who: Continuation) -> Result<Vec<u64>, Error> {
        self.try_with_core(|core| core.guest_exited_at_safe_point(who))
    }
    /// The future scheduler must prove all outstanding callbacks drained;
    /// a call to cancel() is not such evidence.
    fn trusted_notifications_drained(&self, who: Continuation) -> Result<(), Error> {
        self.try_with_core(|core| core.notification_drain_ack(who))
    }
    fn try_retire(&self, who: Continuation) -> Result<(), Error> {
        self.try_with_core(|core| core.certify_physical_retirement(who))
    }
    fn terminal(&self, who: Continuation) -> Result<Option<TaskExitKind>, Error> {
        self.try_with_core(|core| core.exit_kind(who))
    }
    /// Level-triggered cancellation wake hints. The caller may schedule any
    /// listed continuation again until its trusted guest-exit receipt arrives.
    /// Repeated polling is safe: a cancellation wake is not physical retirement.
    /// This does NOT run callbacks or unlock a claimed guard.
    fn cancelled_continuation_wake_hints(&self) -> Result<Vec<Continuation>, Error> {
        self.try_with_core(|core| Ok(core.tasks.values()
            .filter(|task| task.cancel_requested && task.lock_wait_cancel_wake && !task.guest_exited)
            .map(|task| task.continuation)
            .collect()))
    }
    /// Grant wake hints never confer ownership: the scheduler still calls
    /// claim() to revalidate the exact ticket. Level-triggered grant hints. Merely inspecting a wake hint must NOT
    /// consume it: failed scheduler enqueue or a busy carrier may retry.
    /// A trusted scheduler acknowledges only AFTER admission to its ready
    /// queue; guest code can never acknowledge a raw lock ticket.
    fn pending_grant_wake_hints(&self) -> Result<Vec<(u64, Ticket)>, Error> {
        self.try_with_core(|core| {
            let mut out = Vec::with_capacity(core.locks.len());
            for (&lock_id, lock) in &core.locks {
                if let Some(ticket) = lock.wakeup {
                    out.push((lock_id, ticket));
                }
            }
            Ok(out)
        })
    }
    /// Acknowledgment is independent from actually claiming the guard. A
    /// cancel/regrant race can invalidate the ticket between hint and ack;
    /// stale acks must not erase a successor's still-pending wakeup.
    fn acknowledge_grant_wake_hint(&self, lock_id: u64, ticket: Ticket)
        -> Result<(), Error> {
        self.try_with_core(|core| {
            let lock = core.locks.get_mut(&lock_id).ok_or(Error::StaleTicket)?;
            if lock.wakeup != Some(ticket) { return Err(Error::StaleTicket); }
            lock.wakeup = None;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d() -> Domain { Domain { tenant: 1, actor: 9, incarnation: 2 } }
    fn c(task: u64) -> Continuation {
        Continuation { domain: d(), logical_task: task, segment: task }
    }
    fn queue(max: usize) -> WaitQueue { WaitQueue::new(d(), 7, max).unwrap() }
    fn owned(a: Admission) -> Ticket {
        match a { Admission::Granted(t) => t, Admission::Waiting(_) => panic!("not granted") }
    }
    fn pending(a: Admission) -> Ticket {
        match a { Admission::Waiting(t) => t, Admission::Granted(_) => panic!("not pending") }
    }

    #[test]
    fn immediate_grant_and_double_release_fails_closed() {
        let mut q = queue(2);
        let t = owned(q.acquire(c(1)).unwrap());
        q.claim(t).unwrap();
        q.release(t).unwrap();
        assert_eq!(q.release(t), Err(Error::StaleTicket));
        assert_eq!(q.take_wakeup(), None);
    }

    #[test]
    fn fifo_handoff_and_stale_wake_are_fenced() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        let c = pending(q.acquire(c(3)).unwrap());
        q.release(a).unwrap();
        assert_eq!(q.take_wakeup(), Some(b));
        q.claim(b).unwrap();
        q.release(b).unwrap();
        assert_eq!(q.take_wakeup(), Some(c));
        q.claim(c).unwrap();
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        q.release(c).unwrap();
        assert_eq!(q.take_wakeup(), None);
    }

    #[test]
    fn cancellation_before_and_after_handoff_never_double_grants() {
        let mut q = queue(4);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        let queued_c = pending(q.acquire(c(3)).unwrap());
        assert_eq!(q.cancel(b), Ok(Cancellation::Queued));
        q.release(a).unwrap();
        assert_eq!(q.take_wakeup(), Some(queued_c));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        assert_eq!(q.cancel(queued_c), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.claim(queued_c), Err(Error::StaleTicket));
        assert_eq!(q.release(queued_c), Err(Error::StaleTicket));
        assert!(matches!(q.acquire(c(4)), Ok(Admission::Granted(_))));
    }

    #[test]
    fn canceled_unconsumed_wakeup_is_not_delivered() {
        let mut q = queue(4);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        let c = pending(q.acquire(c(3)).unwrap());
        q.release(a).unwrap();
        assert_eq!(q.cancel(b), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), Some(c));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        q.claim(c).unwrap();
    }

    #[test]
    fn foreign_tenant_actor_and_generation_fail_closed() {
        let mut q = queue(2);
        let foreign = Continuation {
            domain: Domain { tenant: 2, ..d() },
            logical_task: 2, segment: 2,
        };
        assert_eq!(q.acquire(foreign), Err(Error::ForeignDomain));
        let t = owned(q.acquire(c(1)).unwrap());
        let old = Ticket { generation: 6, ..t };
        assert_eq!(q.claim(old), Err(Error::StaleTicket));
        assert_eq!(q.release(old), Err(Error::StaleTicket));
        assert_eq!(q.cancel(old), Err(Error::StaleTicket));
    }

    #[test]
    fn recursion_and_quota_are_refused_without_parking() {
        let mut q = queue(1);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        assert_eq!(q.acquire(c(1)), Err(Error::Reentrant));
        let b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.acquire(c(2)), Err(Error::Reentrant));
        assert_eq!(q.acquire(c(3)), Err(Error::QueueFull));
        q.release(a).unwrap();
        q.claim(b).unwrap();
    }

    #[test]
    fn shutdown_revokes_active_and_queued_ledger() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        let _b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.shutdown(), 1);
        assert_eq!(q.claim(a), Err(Error::Closed));
        assert_eq!(q.acquire(c(3)), Err(Error::Closed));
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.shutdown(), 0);
    }

    #[test]
    fn exhausted_epoch_never_reuses_a_ticket() {
        let mut q = queue(2);
        q.next_sequence = u64::MAX;
        assert_eq!(q.acquire(c(1)), Err(Error::Exhausted));
        assert_eq!(q.owner, None);
        assert!(q.waiting.is_empty());
    }

    #[test]
    fn cancelling_active_lease_waits_for_guard_unwind_before_handoff() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.cancel(a), Ok(Cancellation::ActiveDeferred));
        assert_eq!(q.cancel(a), Ok(Cancellation::ActiveDeferred));
        assert!(q.cancellation_pending() && q.has_active_lease());
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.claim(a), Err(Error::CancellationPending));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        // Called by the task cleanup path after it stops using the lease.
        q.release(a).unwrap();
        assert!(!q.cancellation_pending());
        assert_eq!(q.take_wakeup(), Some(b));
        q.claim(b).unwrap();
        q.release(b).unwrap();
        assert!(!q.has_outstanding_grant());
    }
    #[test]
    fn unclaimed_handoff_may_be_revoked_at_cancellation() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        let c = pending(q.acquire(c(3)).unwrap());
        q.release(a).unwrap();
        assert_eq!(q.cancel(b), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), Some(c));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        q.claim(c).unwrap();
        q.release(c).unwrap();
    }
    #[test]
    fn shutdown_retains_active_owner_until_explicit_cleanup_acknowledgment() {
        let mut q = queue(2);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.shutdown(), 1);
        assert!(q.has_outstanding_grant() && q.has_active_lease());
        assert_eq!(q.claim(b), Err(Error::Closed));
        assert_eq!(q.acquire(c(3)), Err(Error::Closed));
        assert_eq!(q.release(b), Err(Error::StaleTicket));
        q.release(a).unwrap();
        assert!(!q.has_outstanding_grant());
        assert_eq!(q.release(a), Err(Error::StaleTicket));
    }
    #[test]
    fn claimed_lease_cannot_replay_entry_or_reenter() {
        let mut q = queue(2);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        assert_eq!(q.claim(a), Err(Error::AlreadyClaimed));
        assert_eq!(q.acquire(c(1)), Err(Error::Reentrant));
        q.release(a).unwrap();
    }

    #[test]
    fn cancelled_task_unwinds_multiple_locks_lifo_before_successor_grants() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        let first = owned(locks.get_mut(&1).unwrap().acquire(c(1)).unwrap());
        scope.enter(1, locks.get_mut(&1).unwrap(), first).unwrap();
        let second = owned(locks.get_mut(&2).unwrap().acquire(c(1)).unwrap());
        scope.enter(2, locks.get_mut(&2).unwrap(), second).unwrap();
        let waiting_first = pending(locks.get_mut(&1).unwrap().acquire(c(2)).unwrap());
        let waiting_second = pending(locks.get_mut(&2).unwrap().acquire(c(2)).unwrap());
        scope.request_cancel(&mut locks).unwrap();
        scope.request_cancel(&mut locks).unwrap();
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), None);
        assert_eq!(locks.get_mut(&2).unwrap().take_wakeup(), None);
        assert!(!scope.all_local_locks_released());
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), vec![2, 1]);
        assert!(scope.all_local_locks_released());
        assert_eq!(locks.get_mut(&2).unwrap().take_wakeup(), Some(waiting_second));
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), Some(waiting_first));
        locks.get_mut(&2).unwrap().claim(waiting_second).unwrap();
        locks.get_mut(&1).unwrap().claim(waiting_first).unwrap();
        locks.get_mut(&2).unwrap().release(waiting_second).unwrap();
        locks.get_mut(&1).unwrap().release(waiting_first).unwrap();
    }
    #[test]
    fn cleanup_attempts_remaining_locks_if_one_lease_is_lost() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        for id in [1_u64, 2_u64] {
            let q = locks.get_mut(&id).unwrap();
            let t = owned(q.acquire(c(1)).unwrap());
            scope.enter(id, q, t).unwrap();
        }
        // Inject a malformed/unavailable cleanup dependency.
        locks.remove(&2);
        assert_eq!(scope.unwind_after_safe_point(&mut locks), Err(Error::StaleTicket));
        assert!(!scope.all_local_locks_released());
        assert!(!locks.get(&1).unwrap().has_outstanding_grant());
        // Local task may not advertise physical retirement on this failure.
    }
    #[test]
    fn cancellation_forbids_new_enter_and_shutdown_still_allows_unwind() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        let a = owned(locks.get_mut(&1).unwrap().acquire(c(1)).unwrap());
        scope.enter(1, locks.get_mut(&1).unwrap(), a).unwrap();
        scope.request_cancel(&mut locks).unwrap();
        locks.get_mut(&1).unwrap().shutdown();
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), vec![1]);
        assert!(scope.all_local_locks_released());
        let new = owned(queue(2).acquire(c(1)).unwrap());
        assert_eq!(scope.enter(2, locks.get_mut(&1).unwrap(), new),
            Err(Error::CancellationPending));
    }

    #[test]
    fn late_cancel_after_shutdown_preserves_active_lock_until_cleanup() {
        let mut q = queue(2);
        let t = owned(q.acquire(c(1)).unwrap());
        q.claim(t).unwrap();
        assert_eq!(q.shutdown(), 0);
        assert!(q.cancellation_pending());
        assert_eq!(q.cancel(t), Ok(Cancellation::ActiveDeferred));
        assert!(q.has_outstanding_grant());
        q.release(t).unwrap();
        assert!(!q.has_outstanding_grant());
        assert_eq!(q.cancel(t), Err(Error::Closed));
    }
    #[test]
    fn pending_waiter_cancellation_never_releases_an_active_task_guard() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(3));
        let mut active_scope = TaskLeaseStack::new(c(1));
        let owner = owned(locks.get_mut(&1).unwrap().acquire(c(1)).unwrap());
        active_scope.enter(1, locks.get_mut(&1).unwrap(), owner).unwrap();
        let waiting = pending(locks.get_mut(&1).unwrap().acquire(c(2)).unwrap());
        assert_eq!(locks.get_mut(&1).unwrap().cancel(waiting), Ok(Cancellation::Queued));
        assert!(locks.get(&1).unwrap().has_active_lease());
        assert!(locks.get(&1).unwrap().has_outstanding_grant());
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), None);
        // A pending waiter's cancellation must not unlock another owner's guard.
        active_scope.request_cancel(&mut locks).unwrap();
        assert!(locks.get(&1).unwrap().has_active_lease());
        assert_eq!(locks.get_mut(&1).unwrap().claim(waiting), Err(Error::StaleTicket));
        assert_eq!(active_scope.unwind_after_safe_point(&mut locks).unwrap(), vec![1]);
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), None);
        assert!(!locks.get(&1).unwrap().has_outstanding_grant());
    }

    #[test]
    fn task_guard_cleanup_on_normal_exit_is_lifo_and_idempotent() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        for id in [1_u64, 2_u64] {
            let q = locks.get_mut(&id).unwrap();
            let grant = owned(q.acquire(c(1)).unwrap());
            scope.enter(id, q, grant).unwrap();
        }
        // Scoped cleanup applies to successful return as well as cancellation.
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), vec![2, 1]);
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), Vec::<u64>::new());
        assert!(scope.all_local_locks_released());
        for id in [1_u64, 2_u64] {
            assert!(!locks.get(&id).unwrap().has_outstanding_grant());
        }
    }

    #[test]
    fn cancellation_validation_failure_keeps_existing_guard_fenced() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        for id in [1_u64, 2_u64] {
            let q = locks.get_mut(&id).unwrap();
            let grant = owned(q.acquire(c(1)).unwrap());
            scope.enter(id, q, grant).unwrap();
        }
        locks.remove(&2); // inject loss of a required cleanup dependency
        assert_eq!(scope.request_cancel(&mut locks), Err(Error::StaleTicket));
        assert!(locks.get(&1).unwrap().has_active_lease());
        assert!(!locks.get(&1).unwrap().cancellation_pending());
        assert!(!scope.all_local_locks_released());
        // This mock is NOT permission to retire the Task or unlock a live guard.
        assert_eq!(scope.unwind_after_safe_point(&mut locks), Err(Error::StaleTicket));
        assert!(!locks.get(&1).unwrap().has_outstanding_grant());
        assert!(!scope.all_local_locks_released());
    }


    fn scoped() -> TaskScopeLockCoordinator {
        let mut scope = TaskScopeLockCoordinator::new(d());
        scope.add_lock(1, queue(8)).unwrap();
        scope.add_lock(2, WaitQueue::new(d(), 8, 8).unwrap()).unwrap();
        scope
    }

    #[test]
    fn task_scope_cancel_keeps_active_guard_until_guest_exit() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        scope.register(c(2), Some(c(1))).unwrap();
        scope.register(c(3), None).unwrap();

        let held = owned(scope.acquire(c(2), 1).unwrap());
        scope.enter(c(2), 1, held).unwrap();
        let next = pending(scope.acquire(c(3), 1).unwrap());
        scope.cancel_tree(c(1)).unwrap();
        assert!(scope.locks[&1].has_active_lease());
        assert_eq!(scope.locks.get_mut(&1).unwrap().take_wakeup(), None);

        // Parent exits first. Returning from its body does NOT certify DONE
        // while a child is executing or holding a critical lease.
        assert!(scope.guest_exited_at_safe_point(c(1)).unwrap().is_empty());
        scope.notification_drain_ack(c(1)).unwrap();
        assert_eq!(scope.certify_physical_retirement(c(1)), Err(Error::NotQuiescent));

        assert_eq!(scope.guest_exited_at_safe_point(c(2)).unwrap(), vec![1]);
        assert_eq!(scope.locks.get_mut(&1).unwrap().take_wakeup(), Some(next));
        // The successor may enter after release, independent of the parent's
        // and child's later callback-drain/physical-retire acknowledgments.
        scope.enter(c(3), 1, next).unwrap();
        assert_eq!(scope.certify_physical_retirement(c(2)), Err(Error::NotQuiescent));
        scope.notification_drain_ack(c(2)).unwrap();
        scope.certify_physical_retirement(c(2)).unwrap();
        scope.certify_physical_retirement(c(1)).unwrap();

        assert_eq!(scope.guest_exited_at_safe_point(c(3)).unwrap(), vec![1]);
        scope.notification_drain_ack(c(3)).unwrap();
        scope.certify_physical_retirement(c(3)).unwrap();
    }

    #[test]
    fn task_scope_cancel_pending_waiter_never_unlocks_owner() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        scope.register(c(2), None).unwrap();
        let first = owned(scope.acquire(c(1), 1).unwrap());
        scope.enter(c(1), 1, first).unwrap();
        let waiter = pending(scope.acquire(c(2), 1).unwrap());
        scope.cancel_tree(c(2)).unwrap();
        assert!(scope.locks[&1].has_active_lease());
        assert_eq!(scope.enter(c(2), 1, waiter), Err(Error::CancellationPending));
        assert!(scope.guest_exited_at_safe_point(c(2)).unwrap().is_empty());
        scope.notification_drain_ack(c(2)).unwrap();
        scope.certify_physical_retirement(c(2)).unwrap();
        assert_eq!(scope.guest_exited_at_safe_point(c(1)).unwrap(), vec![1]);
        assert_eq!(scope.locks.get_mut(&1).unwrap().take_wakeup(), None);
        scope.notification_drain_ack(c(1)).unwrap();
        scope.certify_physical_retirement(c(1)).unwrap();
    }

    #[test]
    fn task_scope_cancel_granted_unclaimed_handoff_fences_stale_wake() {
        let mut scope = scoped();
        for id in [1_u64, 2, 3] { scope.register(c(id), None).unwrap(); }
        let a = owned(scope.acquire(c(1), 1).unwrap());
        scope.enter(c(1), 1, a).unwrap();
        let b = pending(scope.acquire(c(2), 1).unwrap());
        let successor = pending(scope.acquire(c(3), 1).unwrap());
        assert_eq!(scope.guest_exited_at_safe_point(c(1)).unwrap(), vec![1]);
        // B owns a granted handoff but has not entered the critical section.
        scope.cancel_tree(c(2)).unwrap();
        assert_eq!(scope.enter(c(2), 1, b), Err(Error::CancellationPending));
        assert_eq!(scope.locks.get_mut(&1).unwrap().take_wakeup(), Some(successor));
        scope.enter(c(3), 1, successor).unwrap();
        assert!(scope.guest_exited_at_safe_point(c(2)).unwrap().is_empty());
        scope.notification_drain_ack(c(2)).unwrap();
        scope.certify_physical_retirement(c(2)).unwrap();
        assert_eq!(scope.guest_exited_at_safe_point(c(3)).unwrap(), vec![1]);
        scope.notification_drain_ack(c(1)).unwrap();
        scope.notification_drain_ack(c(3)).unwrap();
        scope.certify_physical_retirement(c(1)).unwrap();
        scope.certify_physical_retirement(c(3)).unwrap();
        assert!(!scope.locks[&1].has_outstanding_grant());
    }

    #[test]
    fn task_scope_cancel_is_idempotent_and_unwinds_nested_guards_lifo() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        let a = owned(scope.acquire(c(1), 1).unwrap());
        scope.enter(c(1), 1, a).unwrap();
        let b = owned(scope.acquire(c(1), 2).unwrap());
        scope.enter(c(1), 2, b).unwrap();
        scope.cancel_tree(c(1)).unwrap();
        scope.cancel_tree(c(1)).unwrap();
        assert!(scope.locks[&1].has_active_lease());
        assert!(scope.locks[&2].has_active_lease());
        assert_eq!(scope.guest_exited_at_safe_point(c(1)).unwrap(), vec![2, 1]);
        assert_eq!(scope.guest_exited_at_safe_point(c(1)), Err(Error::AlreadyClaimed));
        scope.notification_drain_ack(c(1)).unwrap();
        scope.certify_physical_retirement(c(1)).unwrap();
        scope.certify_physical_retirement(c(1)).unwrap();
        assert!(!scope.locks[&1].has_outstanding_grant());
        assert!(!scope.locks[&2].has_outstanding_grant());
    }

    #[test]
    fn task_scope_cleanup_failure_withholds_retirement_and_attempts_other_guards() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        for id in [1_u64, 2] {
            let token = owned(scope.acquire(c(1), id).unwrap());
            scope.enter(c(1), id, token).unwrap();
        }
        scope.locks.remove(&2); // fault inject an unavailable cleanup dependency
        assert_eq!(scope.cancel_tree(c(1)), Err(Error::StaleTicket));
        assert_eq!(scope.guest_exited_at_safe_point(c(1)), Err(Error::StaleTicket));
        scope.notification_drain_ack(c(1)).unwrap();
        assert_eq!(scope.certify_physical_retirement(c(1)), Err(Error::CleanupFailed));
        assert!(!scope.locks[&1].has_outstanding_grant());
        assert!(!scope.tasks[&1].guards.all_local_locks_released());
    }

    #[test]
    fn task_scope_parent_guard_unwinds_before_child_retirement_join() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        scope.register(c(2), Some(c(1))).unwrap();
        let parent_lease = owned(scope.acquire(c(1), 1).unwrap());
        scope.enter(c(1), 1, parent_lease).unwrap();
        let child_waiter = pending(scope.acquire(c(2), 1).unwrap());

        scope.cancel_tree(c(1)).unwrap();
        assert_eq!(scope.enter(c(2), 1, child_waiter), Err(Error::CancellationPending));
        assert_eq!(scope.certify_physical_retirement(c(1)), Err(Error::NotQuiescent));
        assert_eq!(scope.guest_exited_at_safe_point(c(1)).unwrap(), vec![1]);
        scope.notification_drain_ack(c(1)).unwrap();
        // Parent lock is already released, but scope must still await child.
        assert_eq!(scope.certify_physical_retirement(c(1)), Err(Error::NotQuiescent));
        assert!(scope.guest_exited_at_safe_point(c(2)).unwrap().is_empty());
        scope.notification_drain_ack(c(2)).unwrap();
        scope.certify_physical_retirement(c(2)).unwrap();
        scope.certify_physical_retirement(c(1)).unwrap();
        assert!(!scope.locks[&1].has_outstanding_grant());
    }

    #[test]
    fn task_scope_rejects_child_after_cancel_and_foreign_tenant_segments() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        scope.cancel_tree(c(1)).unwrap();
        assert_eq!(scope.register(c(2), Some(c(1))), Err(Error::CancellationPending));
        assert_eq!(scope.acquire(c(1), 1), Err(Error::CancellationPending));
        let forged = Continuation {
            domain: Domain { tenant: 99, ..d() },
            logical_task: 1, segment: 1,
        };
        assert_eq!(scope.cancel_tree(forged), Err(Error::ForeignDomain));
        assert_eq!(scope.notification_drain_ack(forged), Err(Error::ForeignDomain));
        assert_eq!(scope.enter(forged, 1, Ticket {
            generation: 7, sequence: 1, continuation: forged,
        }), Err(Error::ForeignDomain));
    }


    #[test]
    fn task_scope_terminal_cancellation_and_success_race_winners() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        scope.register(c(2), None).unwrap();

        // Cancellation won before the trusted guest safe-point terminal claim.
        scope.cancel_tree(c(1)).unwrap();
        assert_eq!(scope.exit_kind(c(1)).unwrap(), None);
        scope.guest_exited_at_safe_point(c(1)).unwrap();
        assert_eq!(scope.exit_kind(c(1)).unwrap(), Some(TaskExitKind::Cancelled));

        // Terminal success has already claimed its outcome. A later cancel
        // cannot replace it with cancellation or trigger duplicate cleanup.
        scope.guest_exited_at_safe_point(c(2)).unwrap();
        assert_eq!(scope.exit_kind(c(2)).unwrap(), Some(TaskExitKind::Succeeded));
        scope.cancel_tree(c(2)).unwrap();
        assert_eq!(scope.exit_kind(c(2)).unwrap(), Some(TaskExitKind::Succeeded));

        for id in [1_u64, 2] {
            scope.notification_drain_ack(c(id)).unwrap();
            scope.certify_physical_retirement(c(id)).unwrap();
        }
    }


    #[test]
    fn host_kernel_never_parks_when_arbiter_is_busy() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let held = kernel.core.lock().unwrap(); // host-only contention injection
        assert_eq!(kernel.issue(None), Err(Error::AdmissionBusy));
        assert_eq!(kernel.add_mutex(4), Err(Error::AdmissionBusy));
        drop(held);
        let owner = kernel.issue(None).unwrap();
        let mutex = kernel.add_mutex(4).unwrap();
        let grant = owned(kernel.acquire(owner, mutex).unwrap());
        kernel.claim(owner, mutex, grant).unwrap();
        assert_eq!(kernel.terminal(owner).unwrap(), None);
        assert_eq!(kernel.trusted_exit(owner).unwrap(), vec![mutex]);
        kernel.trusted_notifications_drained(owner).unwrap();
        kernel.try_retire(owner).unwrap();
        assert_eq!(kernel.terminal(owner).unwrap(), Some(TaskExitKind::Succeeded));
    }

    #[test]
    fn host_kernel_cancel_unwinds_child_guards_and_preserves_sibling() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let parent = kernel.issue(None).unwrap();
        let child = kernel.issue(Some(parent)).unwrap();
        let sibling = kernel.issue(None).unwrap();
        let lock = kernel.add_mutex(3).unwrap();
        let owner_grant = owned(kernel.acquire(child, lock).unwrap());
        kernel.claim(child, lock, owner_grant).unwrap();
        let sibling_wait = pending(kernel.acquire(sibling, lock).unwrap());
        kernel.cancel(parent).unwrap();
        assert_eq!(kernel.try_retire(parent), Err(Error::NotQuiescent));
        assert_eq!(kernel.try_retire(child), Err(Error::NotQuiescent));
        assert_eq!(kernel.trusted_exit(parent).unwrap(), Vec::<u64>::new());
        kernel.trusted_notifications_drained(parent).unwrap();
        assert_eq!(kernel.try_retire(parent), Err(Error::NotQuiescent));
        assert_eq!(kernel.trusted_exit(child).unwrap(), vec![lock]);
        assert_eq!(kernel.terminal(child).unwrap(), Some(TaskExitKind::Cancelled));
        // Exact handoff from the cancelled child is revalidated by the
        // un-cancelled sibling; no early unlock and no privilege escalation.
        kernel.claim(sibling, lock, sibling_wait).unwrap();
        kernel.trusted_notifications_drained(child).unwrap();
        kernel.try_retire(child).unwrap();
        kernel.try_retire(parent).unwrap();
        assert_eq!(kernel.trusted_exit(sibling).unwrap(), vec![lock]);
        kernel.trusted_notifications_drained(sibling).unwrap();
        kernel.try_retire(sibling).unwrap();
        assert_eq!(kernel.terminal(sibling).unwrap(), Some(TaskExitKind::Succeeded));
    }

    #[test]
    fn host_kernel_epoch_and_tenant_handles_fail_closed() {
        let one = HostTaskLockKernel::new(d()).unwrap();
        let two = HostTaskLockKernel::new(d()).unwrap();
        let task_one = one.issue(None).unwrap();
        let task_two = two.issue(None).unwrap();
        assert_ne!(task_one.segment, task_two.segment);
        assert_eq!(one.cancel(task_two), Err(Error::ForeignDomain));
        assert_eq!(two.cancel(task_one), Err(Error::ForeignDomain));
        let invalid = HostTaskLockKernel::new(Domain { tenant: 0, ..d() });
        assert!(matches!(invalid, Err(Error::ForeignDomain)));
    }

    #[test]
    fn host_kernel_rejected_cancel_during_claim_keeps_physical_retire_blocked() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let owner = kernel.issue(None).unwrap();
        let wait = kernel.issue(None).unwrap();
        let lock = kernel.add_mutex(3).unwrap();
        let held = owned(kernel.acquire(owner, lock).unwrap());
        kernel.claim(owner, lock, held).unwrap();
        let pending_ticket = pending(kernel.acquire(wait, lock).unwrap());
        kernel.cancel(wait).unwrap();
        assert_eq!(kernel.claim(wait, lock, pending_ticket), Err(Error::CancellationPending));
        assert_eq!(kernel.try_retire(wait), Err(Error::NotQuiescent));
        assert_eq!(kernel.trusted_exit(wait).unwrap(), Vec::<u64>::new());
        kernel.trusted_notifications_drained(wait).unwrap();
        kernel.try_retire(wait).unwrap();
        assert_eq!(kernel.trusted_exit(owner).unwrap(), vec![lock]);
        kernel.trusted_notifications_drained(owner).unwrap();
        kernel.try_retire(owner).unwrap();
    }


    #[test]
    fn host_kernel_shutdown_fences_active_lease_and_cancels_waiters() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let owner = kernel.issue(None).unwrap();
        let queued = kernel.issue(None).unwrap();
        let lock = kernel.add_mutex(3).unwrap();
        let grant = owned(kernel.acquire(owner, lock).unwrap());
        kernel.claim(owner, lock, grant).unwrap();
        let waiter = pending(kernel.acquire(queued, lock).unwrap());

        kernel.shutdown().unwrap();
        kernel.shutdown().unwrap(); // idempotent: no duplicate callbacks
        assert_eq!(kernel.acquire(queued, lock), Err(Error::CancellationPending));
        assert_eq!(kernel.claim(queued, lock, waiter), Err(Error::CancellationPending));
        assert_eq!(kernel.try_retire(owner), Err(Error::NotQuiescent));
        kernel.try_with_core(|core| {
            assert!(core.locks[&lock].has_active_lease());
            assert!(core.locks[&lock].cancellation_pending());
            assert_eq!(core.locks.get_mut(&lock).unwrap().take_wakeup(), None);
            Ok(())
        }).unwrap();

        assert_eq!(kernel.trusted_exit(queued).unwrap(), Vec::<u64>::new());
        kernel.trusted_notifications_drained(queued).unwrap();
        kernel.try_retire(queued).unwrap();
        assert_eq!(kernel.trusted_exit(owner).unwrap(), vec![lock]);
        kernel.try_with_core(|core| {
            assert!(!core.locks[&lock].has_outstanding_grant());
            assert_eq!(core.locks.get_mut(&lock).unwrap().take_wakeup(), None);
            Ok(())
        }).unwrap();
        kernel.trusted_notifications_drained(owner).unwrap();
        kernel.try_retire(owner).unwrap();
        assert_eq!(kernel.terminal(owner).unwrap(), Some(TaskExitKind::Cancelled));
        assert_eq!(kernel.terminal(queued).unwrap(), Some(TaskExitKind::Cancelled));
    }


    #[test]
    fn terminal_shutdown_rejects_new_roots_and_mutexes_but_allows_old_guard_cleanup() {
        // Verify the direct coordinator rather than inferring a stop fence
        // from individual queues alone.
        let mut empty = TaskScopeLockCoordinator::new(d());
        empty.shutdown_domain().unwrap();
        assert!(empty.closed);
        assert_eq!(empty.register(c(1), None), Err(Error::Closed));
        assert_eq!(empty.add_lock(1, queue(2)), Err(Error::Closed));
        empty.shutdown_domain().unwrap(); // terminal and idempotent

        // Host-facing methods must inherit that same permanent fence.
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let holder = kernel.issue(None).unwrap();
        let lock = kernel.add_mutex(2).unwrap();
        let permit = owned(kernel.acquire(holder, lock).unwrap());
        kernel.claim(holder, lock, permit).unwrap();
        kernel.shutdown().unwrap();
        assert_eq!(kernel.issue(None), Err(Error::Closed));
        assert_eq!(kernel.add_mutex(2), Err(Error::Closed));
        kernel.shutdown().unwrap();
        assert_eq!(kernel.issue(None), Err(Error::Closed));
        // Closed admission MUST NOT revoke an active holder before the
        // trusted guest safe-point event establishes safe cleanup.
        assert_eq!(kernel.try_retire(holder), Err(Error::NotQuiescent));
        assert_eq!(kernel.trusted_exit(holder).unwrap(), vec![lock]);
        kernel.trusted_notifications_drained(holder).unwrap();
        kernel.try_retire(holder).unwrap();
        assert_eq!(kernel.terminal(holder).unwrap(), Some(TaskExitKind::Cancelled));
        assert_eq!(kernel.issue(None), Err(Error::Closed));
    }

    #[test]
    fn host_kernel_cross_lock_ticket_replay_is_denied() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let task = kernel.issue(None).unwrap();
        let lock_a = kernel.add_mutex(2).unwrap();
        let lock_b = kernel.add_mutex(2).unwrap();
        let grant_a = owned(kernel.acquire(task, lock_a).unwrap());
        let grant_b = owned(kernel.acquire(task, lock_b).unwrap());
        assert_ne!(grant_a, grant_b, "each lock needs an independent grant epoch");

        // An attacker who obtains one valid ticket must not claim another lock
        // even when the logical task and per-lock sequence are identical.
        assert_eq!(kernel.claim(task, lock_b, grant_a), Err(Error::StaleTicket));
        assert_eq!(kernel.claim(task, lock_a, grant_b), Err(Error::StaleTicket));
        kernel.claim(task, lock_a, grant_a).unwrap();
        kernel.claim(task, lock_b, grant_b).unwrap();
        assert_eq!(kernel.trusted_exit(task).unwrap(), vec![lock_b, lock_a]);
        kernel.trusted_notifications_drained(task).unwrap();
        kernel.try_retire(task).unwrap();
    }

    #[test]
    fn task_scope_rejects_duplicate_generations_on_distinct_locks() {
        let mut scope = TaskScopeLockCoordinator::new(d());
        scope.add_lock(1, queue(2)).unwrap();
        assert_eq!(scope.add_lock(2, queue(2)), Err(Error::StaleTicket));
        assert_eq!(scope.locks.len(), 1);
        scope.add_lock(2, WaitQueue::new(d(), 8, 2).unwrap()).unwrap();
        assert_eq!(scope.locks.len(), 2);
    }


    #[test]
    fn unclaimed_grant_cannot_be_released_or_used_as_held_guard() {
        let mut q = queue(2);
        let never_entered = owned(q.acquire(c(1)).unwrap());
        let waiting = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.release(never_entered), Err(Error::UnclaimedGrant));
        assert!(q.has_outstanding_grant());
        assert!(!q.has_active_lease());
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.cancel(never_entered), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), Some(waiting));
        assert_eq!(q.claim(never_entered), Err(Error::StaleTicket));
        assert_eq!(q.release(never_entered), Err(Error::StaleTicket));
        q.claim(waiting).unwrap();
        q.release(waiting).unwrap();
        assert!(!q.has_outstanding_grant());
    }

    #[test]
    fn host_kernel_cancellation_wakes_queued_task_without_unlocking_owner() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let owner = kernel.issue(None).unwrap();
        let waiter = kernel.issue(None).unwrap();
        let mutex = kernel.add_mutex(4).unwrap();
        let claimed = owned(kernel.acquire(owner, mutex).unwrap());
        kernel.claim(owner, mutex, claimed).unwrap();
        let queued = pending(kernel.acquire(waiter, mutex).unwrap());
        assert!(kernel.cancelled_continuation_wake_hints().unwrap().is_empty());
        kernel.cancel(waiter).unwrap();

        // A suspended guest waiting on this mutex must be scheduled to see
        // TaskCancelled, without the cancellation unlocking another holder.
        assert_eq!(kernel.cancelled_continuation_wake_hints().unwrap(), vec![waiter]);
        assert_eq!(kernel.cancelled_continuation_wake_hints().unwrap(), vec![waiter]);
        assert!(kernel.pending_grant_wake_hints().unwrap().is_empty());
        assert_eq!(kernel.claim(waiter, mutex, queued), Err(Error::CancellationPending));

        assert_eq!(kernel.trusted_exit(waiter).unwrap(), Vec::<u64>::new());
        assert!(kernel.cancelled_continuation_wake_hints().unwrap().is_empty());
        kernel.trusted_notifications_drained(waiter).unwrap();
        kernel.try_retire(waiter).unwrap();
        assert_eq!(kernel.trusted_exit(owner).unwrap(), vec![mutex]);
        assert!(kernel.pending_grant_wake_hints().unwrap().is_empty());
        kernel.trusted_notifications_drained(owner).unwrap();
        kernel.try_retire(owner).unwrap();
    }

    #[test]
    fn host_kernel_grant_hints_remain_claim_checked_across_cancel_race() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let owner = kernel.issue(None).unwrap();
        let cancelled = kernel.issue(None).unwrap();
        let successor = kernel.issue(None).unwrap();
        let mutex = kernel.add_mutex(4).unwrap();
        let active = owned(kernel.acquire(owner, mutex).unwrap());
        kernel.claim(owner, mutex, active).unwrap();
        let stale = pending(kernel.acquire(cancelled, mutex).unwrap());
        let next = pending(kernel.acquire(successor, mutex).unwrap());

        assert_eq!(kernel.trusted_exit(owner).unwrap(), vec![mutex]);
        // Delivery is only a wake *hint*, not a grant claim or permission.
        assert_eq!(kernel.pending_grant_wake_hints().unwrap(), vec![(mutex, stale)]);
        assert_eq!(kernel.pending_grant_wake_hints().unwrap(), vec![(mutex, stale)]);
        kernel.cancel(cancelled).unwrap();
        assert_eq!(kernel.acknowledge_grant_wake_hint(mutex, stale), Err(Error::StaleTicket));
        assert_eq!(kernel.claim(cancelled, mutex, stale), Err(Error::CancellationPending));
        assert_eq!(kernel.pending_grant_wake_hints().unwrap(), vec![(mutex, next)]);
        kernel.acknowledge_grant_wake_hint(mutex, next).unwrap();
        assert!(kernel.pending_grant_wake_hints().unwrap().is_empty());
        kernel.claim(successor, mutex, next).unwrap();
        assert_eq!(kernel.trusted_exit(cancelled).unwrap(), Vec::<u64>::new());
        assert_eq!(kernel.trusted_exit(successor).unwrap(), vec![mutex]);
        for task in [owner, cancelled, successor] {
            kernel.trusted_notifications_drained(task).unwrap();
            kernel.try_retire(task).unwrap();
        }
    }

    #[test]
    fn host_kernel_wake_hint_queries_never_park_scheduler_carrier() {
        let kernel = HostTaskLockKernel::new(d()).unwrap();
        let guard = kernel.core.lock().unwrap(); // inject host arbiter contention
        assert_eq!(kernel.cancelled_continuation_wake_hints(), Err(Error::AdmissionBusy));
        assert_eq!(kernel.pending_grant_wake_hints(), Err(Error::AdmissionBusy));
        drop(guard);
        assert!(kernel.cancelled_continuation_wake_hints().unwrap().is_empty());
        assert!(kernel.pending_grant_wake_hints().unwrap().is_empty());
    }


    #[test]
    fn scope_exit_auto_cancels_unfinished_descendants_without_clobbering_success() {
        let mut scope = scoped();
        scope.register(c(1), None).unwrap();
        scope.register(c(2), Some(c(1))).unwrap();
        scope.register(c(3), Some(c(2))).unwrap();
        scope.register(c(4), None).unwrap();

        // Parent owns a guard; child waits for that mutex. Successful
        // parent return must cancel every unfinished descendant first.
        let owner = owned(scope.acquire(c(1), 1).unwrap());
        scope.enter(c(1), 1, owner).unwrap();
        let waiting = pending(scope.acquire(c(2), 1).unwrap());
        assert_eq!(scope.guest_exited_at_safe_point(c(1)).unwrap(), vec![1]);
        assert_eq!(scope.exit_kind(c(1)).unwrap(), Some(TaskExitKind::Succeeded));
        assert!(scope.tasks[&2].cancel_requested);
        assert!(scope.tasks[&3].cancel_requested);
        assert!(!scope.tasks[&4].cancel_requested);
        assert_eq!(scope.enter(c(2), 1, waiting), Err(Error::CancellationPending));
        assert!(!scope.locks[&1].has_outstanding_grant());
        scope.notification_drain_ack(c(1)).unwrap();
        assert_eq!(scope.certify_physical_retirement(c(1)), Err(Error::NotQuiescent));

        for child in [3_u64, 2] {
            assert!(scope.guest_exited_at_safe_point(c(child)).unwrap().is_empty());
            scope.notification_drain_ack(c(child)).unwrap();
            scope.certify_physical_retirement(c(child)).unwrap();
        }
        scope.certify_physical_retirement(c(1)).unwrap();
        assert!(scope.guest_exited_at_safe_point(c(4)).unwrap().is_empty());
        scope.notification_drain_ack(c(4)).unwrap();
        scope.certify_physical_retirement(c(4)).unwrap();
        assert_eq!(scope.exit_kind(c(2)).unwrap(), Some(TaskExitKind::Cancelled));
        assert_eq!(scope.exit_kind(c(3)).unwrap(), Some(TaskExitKind::Cancelled));
        assert_eq!(scope.exit_kind(c(4)).unwrap(), Some(TaskExitKind::Succeeded));
    }

}
