# coalesce-machines

> **AMS Fitzpatrick**, NCC-4229, *Fitzpatrick*-class rendezvous tender.

Engine-room flight manual: when many callers want the same key at once, one
flies the computation and the rest hold formation until it lands.

Allocation-free `no_std` in-flight registry, the bookkeeping half of
`singleflight`. The first `join` of a key returns `Leader(FlightId)`; every
`join` after that, until the flight settles, returns a `Follower` with its own
`WaiterId`. The leader settles the flight with `complete` or `abandon`, and each
follower still registered gets one `Notification`, read with `waiter()`,
`flight()`, `key()` and `outcome()`.

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
assert_eq!((note.waiter(), note.flight(), note.key()), (waiter, flight, 42));
assert_eq!(note.outcome(), Outcome::Completed);

// Settled keys are not remembered: the next caller leads again.
assert!(matches!(flights.join(42), Ok(Joined::Leader(_))));

// Two `new()` coalescers are both scope zero and issue the same ids; give
// each one that could meet another its own scope.
let (mut a, mut b) = (Coalescer::<1, 1>::with_scope(1), Coalescer::<1, 1>::with_scope(2));
assert_ne!(a.join(7), b.join(7));
```

## Contract

- **Capacities.** `Coalescer<FLIGHTS, WAITERS>`: at most `FLIGHTS` keys in
  flight; at most `WAITERS` waiting followers plus undrained notifications,
  across all keys. A full table answers `Refusal::FlightsFull` or
  `Refusal::WaitersFull`; nothing is evicted. Drain to free waiter slots.
- **Ids.** `FlightId` and `WaiterId` come from one `periscope_machines::Issuer`
  and are never reissued, so a settled, cancelled or drained id is
  `Refusal::Stale` forever, and a slot reused by a later registration cannot be
  settled by an old id. After `periscope_machines::SEQUENCE_MAX` (2^48 - 1)
  registrations `join` answers `Refusal::Exhausted`. A join refused for any
  reason consumes no id. `raw` / `from_raw` carry ids across FFI or queues;
  `from_raw` answers `None` for a sequence number of zero, which is no id. Ids
  are not ordered (no `PartialOrd`/`Ord`): they carry a scope, and an order
  across scopes would rank ids no coalescer ever compared. A
  `FlightId` is a capability: whoever holds it can settle the flight, so hand
  it only to the leader.
- **Rebuilt values.** `Joined::follower(waiter, flight)` and
  `Notification::new(waiter, flight, key, outcome)` answer `None` unless the
  two ids are ones a coalescer could have emitted together: one scope, the
  waiter issued after the flight. Nothing is looked up; liveness is the
  coalescer's answer. The placeholder `Notification::default()` is not one
  they build. `Notification`'s fields are private: a value is one the
  coalescer emitted, one `new` accepted, or the placeholder.
- **Scopes.** Ids are only meaningful to the coalescer that issued them. Two
  coalescers in the same scope issue the same ids, and an id handed to the
  wrong one can name a live registration there, so give each coalescer its own
  scope: `Coalescer::with_scope(scope)` puts a caller-chosen `u16` in the top
  16 bits of every id, and an id from another scope is always `Refusal::Stale`,
  refused before any table is read. `new()` is scope zero and issues 1, 2, 3,
  so two coalescers built with `new()` alias each other's ids.
  The coalescer cannot choose a scope itself: keeping them distinct is the
  caller's job. The layout belongs to `periscope-machines`; use its `scoped`,
  `scope_of` and `sequence_of` to build or read raw ids. `FlightId::scope` and
  `WaiterId::scope` read the scope of an id.
- **One sequence.** Every id a coalescer hands out comes from that one
  sequence, so no two of them are ever equal, whatever their kind, and an id
  rebuilt as the wrong kind names nothing. `reserve()` is the seam for a core
  built on top that issues handles of its own: it takes the next id of the
  sequence, which is never a flight or a waiter (`Refusal::Stale` or `None`
  from every method, forever) and is never issued again. `owns(raw)` proves an
  id was issued here, live or not; `remaining()` says how many are left.
- **Restarts.** A new coalescer counts from one again, so an id that outlived
  a restart could name a new registration. Either save `issued()` and build
  the replacement with `Coalescer::resumed(scope, issued)` (nothing in flight,
  the sequence continuing; `None` when `issued` is beyond `SEQUENCE_MAX`), or
  give each incarnation a new scope.
- **Repeats are not honoured.** The coalescer keeps nothing about a flight
  once it settles or a waiter once it leaves, so a second `complete`, `abandon`
  or `cancel` is `Refusal::Stale` even though the id was provably issued here.
- **Refusals change nothing.** Every `Err` leaves the coalescer bit-for-bit as
  it was. `complete`/`abandon` check the follower chains they will splice
  before taking the flight, so even a broken internal invariant (a
  `debug_assert!` in debug builds) answers `Refusal::Stale` in release with
  the flight still in the air.
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
- **No clock.** The coalescer takes no time, so no answer records one.
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
