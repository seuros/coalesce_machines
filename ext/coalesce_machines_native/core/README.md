# coalesce-machines

> **AMS Fitzpatrick**, NCC-4229, *Fitzpatrick*-class rendezvous tender.

Engine-room flight manual: when many callers want the same key at once, one
flies the computation and the rest hold formation until it lands.

Allocation-free `no_std` in-flight registry, the bookkeeping half of
`singleflight`. The first `join` of a key returns `Leader(FlightId)`; every
`join` after that, until the flight settles, returns a `Follower` with its own
`WaiterId`. The leader settles the flight with `complete` or `abandon`, and each
follower still registered gets one `Notification { waiter, flight, key, outcome }`.

The coalescer runs nothing and stores no results. The caller executes the
computation, keeps the result (keyed by `FlightId` if followers need it), parks
followers under their `WaiterId`, and wakes them from drained notifications.
Once a flight settles its key is forgotten: the next `join` leads a new flight.
That is the difference from `dedup-machines`, which remembers completed keys to
suppress repeated effects.

```rust
use coalesce_machines::{Coalescer, Joined, Notification, Outcome};

let mut flights = Coalescer::<8, 32>::new();
let Ok(Joined::Leader(flight)) = flights.join(42) else { panic!("first caller leads") };
let Ok(Joined::Follower { waiter, flight: shared }) = flights.join(42)
    else { panic!("second caller follows") };
assert_eq!(shared, flight);

// ... the leader computes, the caller stores the result under `flight` ...
assert_eq!(flights.complete(flight), Ok(1));

let mut out = [Notification::default(); 4];
assert_eq!(flights.drain(&mut out), 1);
let note = out[0];
assert_eq!((note.waiter, note.flight, note.key), (waiter, flight, 42));
assert_eq!(note.outcome, Outcome::Completed);

// Settled keys are not remembered: the next caller leads again.
assert!(matches!(flights.join(42), Ok(Joined::Leader(_))));
```

## Contract

- **Capacities.** `Coalescer<FLIGHTS, WAITERS>`: at most `FLIGHTS` keys in
  flight; at most `WAITERS` waiting followers plus undrained notifications,
  across all keys. A full table answers `Refusal::FlightsFull` or
  `Refusal::WaitersFull`; nothing is evicted. Drain to free waiter slots.
- **Ids.** `FlightId` and `WaiterId` come from one `u64` counter and are never
  reissued, so a settled, cancelled or drained id is `Refusal::Stale` forever,
  and a slot reused by a later registration cannot be settled by an old id.
  After `SEQUENCE_MAX` (2^48 - 1) registrations `join` answers
  `Refusal::Exhausted`. `raw` / `from_raw` carry ids across FFI or queues. A `FlightId` is a capability:
  whoever holds it can settle the flight, so hand it only to the leader.
- **Scopes.** Ids are plain integers, so two coalescers in the same scope issue
  the same ones and an id handed to the wrong coalescer can name a live
  registration there. `Coalescer::with_scope(scope)` puts a caller-chosen `u16`
  in the top 16 bits of every id; an id from another scope is always
  `Refusal::Stale`. `new()` is scope zero. The coalescer cannot choose a scope
  itself: keeping them distinct is the caller's job. `scoped` and `scope_of`
  expose the layout to cores that issue ids alongside a coalescer.
- **Refusals change nothing.** Every `Err` leaves the coalescer bit-for-bit as
  it was.
- **Cancellation.** `cancel(waiter)` withdraws a follower while it waits, and
  also while its notification is queued but undrained. After `Ok(())` no
  notification for that id is ever drained. The flight keeps flying even with
  zero followers; `followers(flight)` tells the caller when nobody is left, and
  stopping the leader is the caller's call.
- **Notifications.** Queued in settlement order, then join order within a
  flight. `poll` takes one; `drain(out: &mut [Notification])` writes the first
  `n` entries of a caller-sized buffer, returns `n`, leaves the rest of the
  buffer untouched and the remainder queued, in order, for the next call.
- **Placeholders.** `Notification`, `FlightId`, `WaiterId` and `Outcome`
  implement `Default` so a buffer is `[Notification::default(); N]`. The
  default is never emitted: its ids are raw zero, which is never issued and is
  `Refusal::Stale` everywhere, and its outcome (`Abandoned`) describes no
  flight. Read only the `n` entries a `drain` reported.
- **Naming.** The refusal enum is `Refusal`. `Refused` remains as a deprecated
  alias for one release.
- **Abandon.** `abandon(flight)` settles without a result. Followers are
  notified with `Outcome::Abandoned` and join again; the first becomes leader.

## Not Included

No clock and no timeouts: a leader that dies without settling holds its key
until the caller calls `abandon`. No results, payloads, hashing, wakers,
threads or locks. Keys are caller-supplied `u64`s; two different requests
mapped to the same key share one flight. State is in memory and scoped to one
coalescer; there is no persistence or cross-process arbitration. Lookups are
linear scans over the fixed tables, which suits small capacities.

Initial, unstable API.
