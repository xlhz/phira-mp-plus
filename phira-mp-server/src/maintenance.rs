use std::sync::atomic::{AtomicBool, Ordering};

pub struct Maintenance {
    closed: AtomicBool,
}

impl Maintenance {
    pub fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn set_closed(&self, closed: bool) {
        self.closed.store(closed, Ordering::SeqCst);
    }
}

impl Default for Maintenance {
    fn default() -> Self {
        Self::new()
    }
}
