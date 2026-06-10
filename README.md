# ternary-lease

Distributed lease management for GPU resources with ternary lifecycle states.

## Why This Exists

In a multi-GPU cluster, you need coordination: which worker owns which GPU, and for how long? Distributed leases solve this. But binary lease states (held/expired) aren't enough. A lease can be actively `Held`, naturally `Expired` (TTL ran out), or administratively `Revoked` (explicitly taken away). These three states have different recovery semantics: expired leases can be re-acquired, revoked leases signal a policy decision that shouldn't be silently overridden.

This crate implements a lease manager with TTL-based expiry, renewal, revocation, and deadlock detection — all using ternary state to distinguish the three lifecycle endpoints.

## Architecture

### Core Types

- **`LeaseState`** — Ternary enum: `Held (+1)`, `Expired (0)`, `Revoked (-1)`.
- **`Lease`** — A lease record: `id`, `resource`, `holder`, `ttl_ticks`, `remaining` ticks.
- **`LeaseManager`** — Central coordinator tracking all leases with tick-based time progression.

### Key Behaviors

- **Tick-based time**: Call `tick()` to advance the clock; all leases decrement their remaining TTL.
- **Renewal**: `renew(id)` resets remaining to TTL — but only if the lease hasn't expired yet.
- **Revocation**: `revoke(id)` immediately expires a lease and increments the revocation counter.
- **Deadlock detection**: `find_deadlocks()` finds pairs where holder A's resource is holder B and vice versa.

## Usage

```rust
use ternary_lease::{LeaseManager, LeaseState};

let mut lm = LeaseManager::new();

// Worker 1 claims GPU 0 for 10 ticks
let lease1 = lm.acquire("gpu0", "worker1", 10);
assert_eq!(lm.state(lease1), LeaseState::Held);

// Time passes
for _ in 0..10 { lm.tick(); }
assert_eq!(lm.state(lease1), LeaseState::Expired);

// Worker 2 gets a fresh lease
let lease2 = lm.acquire("gpu0", "worker2", 5);
let active = lm.held_by("worker2");
assert_eq!(active.len(), 1);
```

### Deadlock Detection

```rust
// A holds resource "B", B holds resource "A" — circular wait
lm.acquire("B", "A", 10);
lm.acquire("A", "B", 10);
let deadlocks = lm.find_deadlocks();
assert_eq!(deadlocks.len(), 1); // (A_lease, B_lease) detected
```

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `new()` | `LeaseManager` | Create a new manager |
| `acquire(resource, holder, ttl)` | `u64` | Acquire a lease, returns lease ID |
| `renew(lease_id)` | `bool` | Reset TTL if still held |
| `revoke(lease_id)` | `bool` | Force-expire a lease |
| `state(lease_id)` | `LeaseState` | Current ternary state |
| `tick()` | `()` | Advance clock by one tick |
| `find_deadlocks()` | `Vec<(u64,u64)>` | Detect circular wait pairs |
| `held_by(holder)` | `Vec<&Lease>` | All active leases for a holder |
| `active_count()` | `usize` | Currently held leases |
| `revocations()` | `u64` | Total revocations performed |
| `tick_count()` | `u64` | Total ticks elapsed |

## The Deeper Idea

The distinction between Expired and Revoked matters for **idempotency guarantees**. If a worker's lease expires, it should be able to re-acquire — the system forgives. If a lease is revoked (e.g., by a higher-priority preemption), the worker should back off — the system decided. Collapsing both into "not held" loses this signal and leads to either thundering herds (everyone retries on expiry) or starvation (no one retries on revocation).

## Related Crates

- **ternary-semaphore** — resource permits with ternary capacity states
- **ternary-rate-limiter** — rate limiting with ternary feedback signals
- **ternary-backpressure** — pipeline backpressure with ternary pressure
