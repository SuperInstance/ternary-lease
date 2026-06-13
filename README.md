# ternary-lease

Distributed lease management for GPU resources with **ternary lease states**: **{+1 = held, 0 = expired, −1 = revoked}**. Provides time-to-live (TTL) based expiration, explicit revocation, and deadlock detection for multi-worker GPU clusters.

## Why It Matters

When multiple workers compete for GPU resources (tensor memory, compute streams, model slots), they need a coordination mechanism to prevent simultaneous access. Distributed leases are the standard solution (Gray & Reuter, 1993), but classical binary lease systems (granted/not-granted) cannot distinguish between:

- **Expired** (TTL elapsed, lease lapsed naturally) — safe to reclaim
- **Revoked** (explicitly taken away, holder may be non-cooperative) — requires repair

The ternary state {+1, 0, −1} makes this distinction explicit:
- **+1 (Held)**: Active lease. The holder has exclusive access for `remaining` ticks.
- **0 (Expired)**: TTL elapsed. The resource is eligible for re-acquisition.
- **−1 (Revoked)**: Forcefully terminated. The holder may need cleanup or reconciliation.

This three-state model enables more nuanced recovery policies than binary granted/revoked systems.

## How It Works

### Lease Lifecycle

```
   acquire()
      │
      ▼
  ┌────────┐   tick() decrements   ┌─────────┐
  │ Held   │ ──── remaining=0 ──► │ Expired │
  │ (+1)   │                       │ (0)     │
  └────────┘                       └─────────┘
      │                                 │
      │ revoke()                        │ acquire() reclaims
      ▼                                 ▼
  ┌─────────┐                      new lease
  │ Revoked │
  │ (−1)    │
  └─────────┘
```

### TTL Mechanism

Each lease has a `ttl_ticks` budget, decremented each `tick()` call:

```
remaining(t+1) = max(0, remaining(t) − 1)
```

When `remaining = 0`, the lease transitions to Expired. The holder can reset the timer via `renew()`:

```
remaining ← ttl_ticks   (if currently > 0)
```

Renewal of an expired lease fails — the resource must be re-acquired.

### Deadlock Detection

Two workers A and B are deadlocked if:

```
A holds resource X, wants resource Y
B holds resource Y, wants resource X
```

Formally: lease(A).resource = B.holder ∧ lease(B).resource = A.holder.

The `find_deadlocks()` function checks all pairs of active leases for this circular wait condition. This is a special case of the **wait-for graph** cycle detection (Coffman, Elphick & Shoshani, 1971):

```
For each pair (Lᵢ, Lⱼ) where both are Held:
    If Lᵢ.holder = Lⱼ.resource ∧ Lⱼ.holder = Lᵢ.resource:
        Deadlock detected → return (Lᵢ.id, Lⱼ.id)
```

### Complexity

| Operation | Time | Space |
|-----------|------|-------|
| `acquire(resource, holder, ttl)` | O(1) amortized | O(1) |
| `renew(lease_id)` | O(1) | O(1) |
| `revoke(lease_id)` | O(1) | O(1) |
| `state(lease_id)` | O(1) | O(1) |
| `tick()` | O(L) | O(1) |
| `find_deadlocks()` | O(L²) | O(L²) |
| `held_by(holder)` | O(L) | O(k) |

Where L = number of leases, k = leases held by queried holder.

The O(L²) deadlock check is acceptable for moderate cluster sizes (~1000 workers). For larger deployments, the wait-for graph should be maintained incrementally for O(L) detection.

### Tick-Based Model

The lease system uses a discrete tick counter rather than wall-clock time. This is deliberate:
- **Deterministic testing** — replay scenarios exactly
- **No clock skew** — all workers see the same tick count
- **Fairness** — no worker can game wall-clock precision
- **Decoupled from real time** — ticks can represent any granularity (1 ms, 10 ms, 1 s)

## Quick Start

```rust
use ternary_lease::{LeaseManager, LeaseState};

let mut lm = LeaseManager::new();

// Worker 1 acquires GPU 0 for 10 ticks
let gpu0 = lm.acquire("gpu0", "worker1", 10);
assert_eq!(lm.state(gpu0), LeaseState::Held);

// Worker 2 acquires GPU 1 for 5 ticks
let gpu1 = lm.acquire("gpu1", "worker2", 5);

// Simulate time passing
for _ in 0..6 {
    lm.tick();
}

// GPU 1 lease has expired (TTL was 5)
assert_eq!(lm.state(gpu1), LeaseState::Expired);
// GPU 0 still held
assert_eq!(lm.state(gpu0), LeaseState::Held);

// Check for deadlocks
let deadlocks = lm.find_deadlocks();
if !deadlocks.is_empty() {
    println!("WARNING: {} deadlock(s) detected", deadlocks.len());
}
```

## API

### `LeaseManager`

| Method | Description |
|--------|-------------|
| `new()` | Create empty lease manager |
| `acquire(resource, holder, ttl) -> u64` | Obtain a lease with TTL ticks |
| `renew(lease_id) -> bool` | Reset remaining to TTL (fails if expired) |
| `revoke(lease_id) -> bool` | Forcefully terminate a lease |
| `state(lease_id) -> LeaseState` | Query lease state |
| `tick()` | Advance one time unit (decrement all TTLs) |
| `find_deadlocks() -> Vec<(u64, u64)>` | Detect circular waits |
| `held_by(holder) -> Vec<&Lease>` | Active leases for a holder |
| `active_count() / revocations() / tick_count()` | Statistics |

### `LeaseState`

| Variant | Value | Meaning |
|---------|-------|---------|
| `Held` | +1 | Active, exclusive access |
| `Expired` | 0 | TTL elapsed, reclaimable |
| `Revoked` | −1 | Forcefully terminated |

## Architecture Notes

This crate implements the **γ (gamma) coordination layer** of the γ + η = C framework:

- **γ (gamma)**: Resource arbitration — deciding who can access what, for how long. This crate provides γ-level lease management for GPU resources.
- **η (eta)**: The compute layer — actual GPU work (tensor operations, model inference) performed while holding a lease.
- **C**: The complete distributed GPU management system. γ ensures η-layer workers don't corrupt shared resources.

The ternary lease states {+1, 0, −1} map to the same domain used for ternary weights and ternary consistency states across the ecosystem, making lease decisions expressible in the ternary algebra.

## References

- **Distributed Leases**: Gray, J. & Reuter, A., "Transaction Processing: Concepts and Techniques," Morgan Kaufmann, 1993. Chapter 7 on distributed commitment.
- **Deadlock Detection**: Coffman, E.G., Elphick, M. & Shoshani, A., "System Deadlocks," ACM Computing Surveys, 3(2), 67-78, 1971.
- **Wait-For Graphs**: Holt, R.C., "Some Deadlock Properties of Systems," ACM Transactions on Programming Languages and Systems, 1(2), 270-282, 1979.
- **Lease Algorithms**: Gray, C. & Cheriton, D., "Leases: An Efficient Fault-Tolerant Mechanism for Distributed File Cache Consistency," SOSP 1989.
- **Distributed Mutual Exclusion**: Ricart, G. & Agrawala, A.K., "An Optimal Algorithm for Mutual Exclusion in Computer Networks," CACM, 24(1), 9-17, 1981.

## License

MIT
