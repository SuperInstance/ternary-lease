# ternary-lease

Lease-based distributed coordination with ternary states {+1=Held, 0=Expired, −1=Revoked} for GPU resource management.

## Background

Distributed lease management is a coordination primitive where a resource (e.g., a GPU, a file, a network port) is temporarily granted to a client for a fixed time-to-live (TTL). The concept originates from Gray & Lorie's work on distributed locks and was refined by the Chubby and ZooKeeper systems at Google and Yahoo respectively. A lease differs from a lock in that it has an automatic expiration mechanism—if the leaseholder crashes or becomes partitioned, the lease expires after TTL ticks, freeing the resource without manual intervention.

The classic lease lifecycle has two states: **held** (active) and **expired** (inactive). The `ternary-lease` crate introduces a third state: **revoked**, representing a lease that was explicitly terminated by authority rather than passively expiring. This distinction matters in GPU cluster management. A lease that expires because a worker was slow is different from one revoked because the worker was misbehaving—expired leases suggest network issues; revoked leases suggest correctness problems.

Deadlock detection is a critical feature for lease-based systems. When worker A holds a lease on resource B while waiting for resource A (held by worker B), the system deadlocks. The ternary state model simplifies resolution: revoked (−1) leases can be force-released to break cycles, while expired (0) leases may simply need renewal.

## How It Works

### Architecture

`LeaseManager` maintains a `HashMap<u64, Lease>` indexed by lease ID. Each `Lease` tracks:
- `resource`: the resource identifier (e.g., `"gpu0"`)
- `holder`: the client identity (e.g., `"worker1"`)
- `ttl_ticks`: the original TTL
- `remaining`: countdown timer decremented each tick

### Ternary State Machine

```
                 acquire()
   [none] ──────────────────→ Held (+1)
                                  │
                    ┌─────────────┼──────────────┐
                    │ tick()      │ revoke()      │ tick()
                    │ (count=0)   │               │ (count=0)
                    ▼             ▼               ▼
              Expired (0)    Expired (0)    Expired (0)
                                  │
                      renew() ────┘ (only if remaining > 0)
```

The `state()` function returns one of three `LeaseState` values:
- **Held** (+1): `remaining > 0` — lease is active
- **Expired** (0): `remaining == 0` — lease ran out of time
- **Revoked** (−1): lease ID not found — was explicitly removed

### Key Operations

- **acquire**: Allocate a new lease with given TTL, returns unique ID
- **renew**: Reset remaining timer to original TTL (only works if lease hasn't expired)
- **revoke**: Force-set remaining to 0, increment revocation counter
- **tick**: Decrement all active leases' remaining timers by 1
- **find_deadlocks**: O(n²) pairwise cycle detection—checks if holder A wants resource B while holder B wants resource A
- **held_by**: Return all active leases for a given holder

### Design Decisions

The revocation counter is maintained separately from the lease states, allowing monitoring systems to track how often resources are force-reclaimed versus naturally expiring. The `renew()` function refuses to renew expired leases (returning `false`), enforcing the invariant that once a lease has expired, the holder must re-acquire from scratch.

## Experimental Results

All 8 unit tests pass:

| Test | Result | Key Observation |
|------|--------|-----------------|
| `test_acquire_held` | ✅ | Fresh lease immediately reports `Held` |
| `test_expiry` | ✅ | After 2 ticks with TTL=2, lease transitions to `Expired` |
| `test_renew` | ✅ | Mid-life renewal resets TTL; lease survives additional ticks |
| `test_revoke` | ✅ | Revoked lease reports `Expired` state; revocation counter = 1 |
| `test_deadlock_detection` | ✅ | A holds "B", B holds "A" → 1 deadlock detected |
| `test_held_by` | ✅ | Worker with 2 of 3 leases: `held_by("w1")` returns exactly 2 |
| `test_active_count` | ✅ | After TTL=1 lease expires, `active_count()` drops to 1 |
| `test_renew_expired_fails` | ✅ | Attempting to renew an expired lease returns `false` |

The deadlock detection test is particularly instructive: creating leases where holder `"A"` acquires resource `"B"` and holder `"B"` acquires resource `"A"` immediately identifies the circular wait. The algorithm detects this in O(n²) time, suitable for the typically small number of concurrent GPU leases in a cluster.

## Impact of Ternary {-1, 0, +1}

The three-state model enables **triage semantics** for resource coordination:

- **+1 (Held)**: The resource is in active use. No action needed.
- **0 (Expired)**: The lease timed out. Likely a performance or connectivity issue. Consider reassigning.
- **−1 (Revoked)**: The lease was forcibly terminated. Likely a correctness issue. Investigate before reassigning.

This distinction enables automated incident response: expired leases trigger retry logic, while revoked leases trigger alerts and diagnostic logging. A binary model (held/not-held) collapses these two failure modes, losing critical diagnostic signal.

## Use Cases

1. **GPU Cluster Scheduling**: Assign exclusive access to GPUs with TTL-based leases. Workers renew leases while computing; expired leases free GPUs for rescheduling. Revoked leases indicate worker misbehavior (e.g., using wrong CUDA version).

2. **Distributed Lock Service**: Implement a Chubby/ZooKeeper-style lock service with automatic expiration. The ternary state enables clients to distinguish "I lost the lock because I was slow" (expired) from "I lost the lock because I was voted out" (revoked).

3. **Cloud Resource Billing**: Tie lease TTL to billing periods. Expired leases represent usage that ended naturally; revoked leases represent early termination (potentially refundable). The state distinction drives different billing logic.

4. **Deadlock Recovery in DAG Schedulers**: When a DAG of GPU kernels has circular dependencies on shared buffers, `find_deadlocks()` identifies the cycle. Revoking one lease in the cycle breaks the deadlock with minimal disruption.

5. **Hot-Swap Detection**: Monitor `revocations()` counter over time. A rising revocation rate indicates systemic issues (bad drivers, firmware bugs) rather than individual worker failures.

## Open Questions

1. **Scalable Deadlock Detection**: The current O(n²) pairwise approach works for small clusters. For large-scale deployments with thousands of concurrent leases, what graph-based cycle detection algorithm maintains accuracy while reducing complexity?

2. **Lease Chaining and Priority Inversion**: When high-priority tasks wait on low-priority leaseholders, priority inversion occurs. How should the ternary state model handle priority-aware revocation? Should −1 be subdivided into "revoked for deadlock" vs. "revoked for priority"?

3. **Byzantine Revocation**: In an adversarial environment, a malicious coordinator could revoke leases selectively. What consensus protocol (Raft, PBFT) should underpin the revocation authority to ensure safety?

## Connection to Oxide Stack

Within the five-layer Oxide ternary architecture:

- **Layer 1 (Ternary Genome)**: The lease states {+1, 0, −1} map directly to genome bases, encoding resource availability as a genetic signal that drives adaptive behavior.
- **Layer 2 (Cellular Computation)**: Each lease acts as a computational cell managing a single resource. The tick() operation is the cell's heartbeat; state transitions are cell-level decisions.
- **Layer 3 (Organism Behavior)**: Deadlock detection and revocation are organism-level behaviors—an individual agent detects systemic pathology and takes corrective action.
- **Layer 4 (Population Dynamics)**: In a multi-organism system, lease distribution across the population determines load balance. Expired/revoked rates are population health metrics.
- **Layer 5 (Ecosystem)**: The lease manager sits at the ecosystem boundary, mediating access to shared physical resources (GPUs, memory, network) among all participants.
