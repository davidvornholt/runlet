use crate::config::{Config, TrustClass};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Default)]
struct ConcurrencyState {
    total: u32,
    trusted: u32,
    untrusted: u32,
}

#[derive(Debug)]
pub struct ConcurrencyLimiter {
    state: Mutex<ConcurrencyState>,
    released: Condvar,
    max_total: u32,
    max_trusted: u32,
    max_untrusted: u32,
}

pub struct ConcurrencyPermit {
    limiter: Arc<ConcurrencyLimiter>,
    trust_class: TrustClass,
}

impl ConcurrencyLimiter {
    pub fn new(config: &Config) -> Self {
        Self {
            state: Mutex::new(ConcurrencyState::default()),
            released: Condvar::new(),
            max_total: config.runtime.max_concurrent_jobs,
            max_trusted: config.runtime.trusted.max_concurrent_jobs,
            max_untrusted: config.runtime.untrusted.max_concurrent_jobs,
        }
    }

    pub fn try_acquire(self: &Arc<Self>, trust_class: TrustClass) -> Option<ConcurrencyPermit> {
        let mut state = self
            .state
            .lock()
            .expect("concurrency lock should not be poisoned");
        self.take(&mut state, trust_class)
            .then(|| self.permit(trust_class))
    }

    /// Waits until a slot for `trust_class` is free and takes it.
    pub fn acquire(self: &Arc<Self>, trust_class: TrustClass) -> ConcurrencyPermit {
        let mut state = self
            .state
            .lock()
            .expect("concurrency lock should not be poisoned");
        while !self.take(&mut state, trust_class) {
            state = self
                .released
                .wait(state)
                .expect("concurrency lock should not be poisoned");
        }
        self.permit(trust_class)
    }

    fn take(&self, state: &mut ConcurrencyState, trust_class: TrustClass) -> bool {
        let class_allowed = match trust_class {
            TrustClass::Trusted => state.trusted < self.max_trusted,
            TrustClass::Untrusted => state.untrusted < self.max_untrusted,
        };
        if state.total >= self.max_total || !class_allowed {
            return false;
        }
        state.total += 1;
        match trust_class {
            TrustClass::Trusted => state.trusted += 1,
            TrustClass::Untrusted => state.untrusted += 1,
        }
        true
    }

    fn permit(self: &Arc<Self>, trust_class: TrustClass) -> ConcurrencyPermit {
        ConcurrencyPermit {
            limiter: Arc::clone(self),
            trust_class,
        }
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        let mut state = self
            .limiter
            .state
            .lock()
            .expect("concurrency lock should not be poisoned");
        state.total -= 1;
        match self.trust_class {
            TrustClass::Trusted => state.trusted -= 1,
            TrustClass::Untrusted => state.untrusted -= 1,
        }
        self.limiter.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_global_and_per_class_limits() {
        let mut config = Config::default();
        config.runtime.max_concurrent_jobs = 1;
        config.runtime.trusted.max_concurrent_jobs = 1;
        config.runtime.untrusted.max_concurrent_jobs = 1;
        let limiter = Arc::new(ConcurrencyLimiter::new(&config));

        let trusted = limiter.try_acquire(TrustClass::Trusted);
        assert!(trusted.is_some());
        assert!(limiter.try_acquire(TrustClass::Untrusted).is_none());
        drop(trusted);

        let untrusted = limiter.try_acquire(TrustClass::Untrusted);
        assert!(untrusted.is_some());
        assert!(limiter.try_acquire(TrustClass::Untrusted).is_none());
    }

    #[test]
    fn acquire_waits_until_a_slot_is_released() {
        let mut config = Config::default();
        config.runtime.max_concurrent_jobs = 1;
        let limiter = Arc::new(ConcurrencyLimiter::new(&config));
        let held = limiter.acquire(TrustClass::Trusted);

        let waiting = Arc::clone(&limiter);
        let waiter = std::thread::spawn(move || drop(waiting.acquire(TrustClass::Untrusted)));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!waiter.is_finished());

        drop(held);
        waiter
            .join()
            .expect("waiting job should get the released slot");
    }
}
