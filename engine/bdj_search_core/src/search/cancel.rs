use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct CancellationToken {
    generation: Arc<AtomicU64>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn bump(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn is_cancelled(&self, expected_gen: u64) -> bool {
        self.generation.load(Ordering::Relaxed) != expected_gen
    }
}
