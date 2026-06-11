/// Undo stack for partially-completed `bring_up` operations.
///
/// Every provisioning step registers its inverse right after it succeeds. If
/// `bring_up` returns early (any `?`), the guard runs the registered steps in
/// reverse order on drop, so whatever was created — containers, overlay files,
/// worktree, spawned processes — is torn down again. Call `disarm()` once the
/// session is fully up to keep everything.
///
/// This replaces hand-rolled cleanup blocks at each failure site, which had
/// drifted apart between modes and skipped several failure paths entirely.
pub struct Rollback {
    steps: Vec<Box<dyn FnOnce()>>,
    armed: bool,
}

impl Rollback {
    pub fn new() -> Self {
        Self {
            steps: vec![],
            armed: true,
        }
    }

    /// Register the inverse of a step that just succeeded.
    pub fn push<F: FnOnce() + 'static>(&mut self, undo: F) {
        self.steps.push(Box::new(undo));
    }

    /// The operation completed — keep everything that was created.
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Default for Rollback {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Rollback {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        while let Some(undo) = self.steps.pop() {
            undo();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn runs_steps_in_reverse_order_on_drop() {
        let order: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(vec![]));
        {
            let mut rb = Rollback::new();
            let o1 = Rc::clone(&order);
            rb.push(move || o1.borrow_mut().push(1));
            let o2 = Rc::clone(&order);
            rb.push(move || o2.borrow_mut().push(2));
            let o3 = Rc::clone(&order);
            rb.push(move || o3.borrow_mut().push(3));
        }
        assert_eq!(*order.borrow(), vec![3, 2, 1], "undo must run LIFO");
    }

    #[test]
    fn disarm_skips_all_steps() {
        let ran = Rc::new(RefCell::new(false));
        {
            let mut rb = Rollback::new();
            let r = Rc::clone(&ran);
            rb.push(move || *r.borrow_mut() = true);
            rb.disarm();
        }
        assert!(!*ran.borrow(), "disarmed guard must not run undo steps");
    }

    #[test]
    fn empty_guard_drops_cleanly() {
        let _ = Rollback::new();
    }

    #[test]
    fn steps_pushed_after_disarm_do_not_run() {
        let ran = Rc::new(RefCell::new(false));
        {
            let mut rb = Rollback::new();
            rb.disarm();
            let r = Rc::clone(&ran);
            rb.push(move || *r.borrow_mut() = true);
        }
        assert!(!*ran.borrow());
    }
}
