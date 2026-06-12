//! Helpers around `std::sync` primitives — currently just `lock_recovered`.
//!
//! `Mutex::lock().unwrap()` panics the calling task when the mutex is
//! "poisoned" — i.e. when a previous holder panicked while owning the lock.
//! For the konfig server, those callers are gRPC handlers and watchers
//! whose locked state is bounded and self-recovering (a per-namespace
//! replay buffer, the freshness `LastEventAt`, the ArcSwap write
//! serialiser). Crashing the binary because some unrelated panic ago
//! left a poisoned marker is strictly worse than keeping serving.
//!
//! `lock_recovered` takes the inner data on poison and continues. Callers
//! that need to know the lock was poisoned can still inspect `is_poisoned`
//! before calling.

use std::sync::{Mutex, MutexGuard};

/// Acquire `m`'s lock, recovering by extracting the inner data when the
/// mutex is poisoned. Never panics.
pub fn lock_recovered<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn lock_recovered_returns_data_after_poison() {
        let m = Arc::new(Mutex::new(7u32));
        let m_clone = Arc::clone(&m);
        // Poison the mutex from a panicking thread.
        let _ = thread::spawn(move || {
            let _g = m_clone.lock().unwrap();
            panic!("intentional");
        })
        .join();
        assert!(m.is_poisoned());

        let g = lock_recovered(&m);
        assert_eq!(*g, 7);
    }

    #[test]
    fn lock_recovered_works_when_not_poisoned() {
        let m = Mutex::new(1u32);
        *lock_recovered(&m) = 2;
        assert_eq!(*lock_recovered(&m), 2);
    }
}
