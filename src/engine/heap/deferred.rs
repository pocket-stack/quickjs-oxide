//! Ordered work postponed while Runtime state is borrowed.
//!
//! The cached pending bit belongs to the queue, not its callers. Operations are
//! raw identities, so pushing/popping them cannot run a user destructor while
//! the queue is borrowed. Execute each operation only after releasing that borrow.

use super::runtime::DeferredRefOp;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct DeferredOperations {
    queue: RefCell<VecDeque<DeferredRefOp>>,
    pending: Cell<bool>,
    draining: Cell<bool>,
}

impl DeferredOperations {
    #[inline]
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.get()
    }

    /// Releases keep their existing FIFO order behind restoration operations.
    pub(crate) fn push_back(&self, operation: DeferredRefOp) {
        self.queue.borrow_mut().push_back(operation);
        self.pending.set(true);
    }

    /// Frame/backtrace restoration retains its existing unwind priority.
    pub(crate) fn push_front(&self, operation: DeferredRefOp) {
        self.queue.borrow_mut().push_front(operation);
        self.pending.set(true);
    }

    pub(crate) fn pop_front(&self) -> Option<DeferredRefOp> {
        let mut queue = self.queue.borrow_mut();
        let operation = queue.pop_front();
        self.pending.set(!queue.is_empty());
        operation
    }

    pub(crate) fn try_start_draining(&self) -> Option<DeferredDrain<'_>> {
        if self.draining.replace(true) {
            None
        } else {
            Some(DeferredDrain(&self.draining))
        }
    }

    #[cfg(test)]
    pub(crate) fn borrow(&self) -> std::cell::Ref<'_, VecDeque<DeferredRefOp>> {
        self.queue.borrow()
    }
}

pub(crate) struct DeferredDrain<'a>(&'a Cell<bool>);

impl Drop for DeferredDrain<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
