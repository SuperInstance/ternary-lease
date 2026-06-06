//! # ternary-lease
//!
//! Distributed lease management for GPU resources with ternary states.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState { Held = 1, Expired = 0, Revoked = -1 }

#[derive(Debug, Clone)]
pub struct Lease {
    pub id: u64,
    pub resource: String,
    pub holder: String,
    pub ttl_ticks: u32,
    pub remaining: u32,
}

pub struct LeaseManager {
    leases: HashMap<u64, Lease>,
    next_id: u64,
    tick: u64,
    revocations: u64,
}

impl LeaseManager {
    pub fn new() -> Self {
        Self { leases: HashMap::new(), next_id: 1, tick: 0, revocations: 0 }
    }

    pub fn acquire(&mut self, resource: &str, holder: &str, ttl: u32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.leases.insert(id, Lease { id, resource: resource.into(), holder: holder.into(), ttl_ticks: ttl, remaining: ttl });
        id
    }

    pub fn renew(&mut self, lease_id: u64) -> bool {
        if let Some(lease) = self.leases.get_mut(&lease_id) {
            if lease.remaining > 0 {
                lease.remaining = lease.ttl_ticks;
                return true;
            }
        }
        false
    }

    pub fn revoke(&mut self, lease_id: u64) -> bool {
        if let Some(lease) = self.leases.get_mut(&lease_id) {
            lease.remaining = 0;
            self.revocations += 1;
            return true;
        }
        false
    }

    pub fn state(&self, lease_id: u64) -> LeaseState {
        match self.leases.get(&lease_id) {
            None => LeaseState::Revoked,
            Some(l) if l.remaining == 0 => LeaseState::Expired,
            Some(_) => LeaseState::Held,
        }
    }

    pub fn tick(&mut self) {
        self.tick += 1;
        for lease in self.leases.values_mut() {
            if lease.remaining > 0 { lease.remaining -= 1; }
        }
    }

    pub fn find_deadlocks(&self) -> Vec<(u64, u64)> {
        // Simple cycle detection: A holds what B wants, B holds what A wants
        let mut deadlocks = Vec::new();
        let leases: Vec<&Lease> = self.leases.values().filter(|l| l.remaining > 0).collect();
        for i in 0..leases.len() {
            for j in (i+1)..leases.len() {
                if leases[i].holder == leases[j].resource && leases[j].holder == leases[i].resource {
                    deadlocks.push((leases[i].id, leases[j].id));
                }
            }
        }
        deadlocks
    }

    pub fn held_by(&self, holder: &str) -> Vec<&Lease> {
        self.leases.values().filter(|l| l.remaining > 0 && l.holder == holder).collect()
    }

    pub fn active_count(&self) -> usize { self.leases.values().filter(|l| l.remaining > 0).count() }
    pub fn revocations(&self) -> u64 { self.revocations }
    pub fn tick_count(&self) -> u64 { self.tick }
}

impl Default for LeaseManager { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquire_held() {
        let mut lm = LeaseManager::new();
        let id = lm.acquire("gpu0", "worker1", 10);
        assert_eq!(lm.state(id), LeaseState::Held);
    }

    #[test]
    fn test_expiry() {
        let mut lm = LeaseManager::new();
        let id = lm.acquire("gpu0", "worker1", 2);
        lm.tick(); lm.tick();
        assert_eq!(lm.state(id), LeaseState::Expired);
    }

    #[test]
    fn test_renew() {
        let mut lm = LeaseManager::new();
        let id = lm.acquire("gpu0", "w", 3);
        lm.tick(); lm.tick();
        assert!(lm.renew(id));
        lm.tick(); lm.tick();
        assert_eq!(lm.state(id), LeaseState::Held);
    }

    #[test]
    fn test_revoke() {
        let mut lm = LeaseManager::new();
        let id = lm.acquire("gpu0", "w", 10);
        lm.revoke(id);
        assert_eq!(lm.state(id), LeaseState::Expired);
        assert_eq!(lm.revocations(), 1);
    }

    #[test]
    fn test_deadlock_detection() {
        let mut lm = LeaseManager::new();
        // A holds "B", B holds "A" — circular wait
        lm.acquire("B", "A", 10);
        lm.acquire("A", "B", 10);
        let deadlocks = lm.find_deadlocks();
        assert_eq!(deadlocks.len(), 1);
    }

    #[test]
    fn test_held_by() {
        let mut lm = LeaseManager::new();
        lm.acquire("gpu0", "w1", 10);
        lm.acquire("gpu1", "w1", 10);
        lm.acquire("gpu2", "w2", 10);
        assert_eq!(lm.held_by("w1").len(), 2);
    }

    #[test]
    fn test_active_count() {
        let mut lm = LeaseManager::new();
        lm.acquire("g0", "w", 1);
        lm.acquire("g1", "w", 10);
        lm.tick(); // g0 expires
        assert_eq!(lm.active_count(), 1);
    }

    #[test]
    fn test_renew_expired_fails() {
        let mut lm = LeaseManager::new();
        let id = lm.acquire("g0", "w", 1);
        lm.tick(); // expired
        assert!(!lm.renew(id));
    }
}
