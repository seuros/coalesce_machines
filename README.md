# CoalesceMachines

> Four hundred requests for the same thing is one request and three hundred ninety-nine people waiting. 🛰️

An in-flight registry for Ruby. Before you compute something expensive, you join its key. The first caller is told to lead: go compute it. Everyone who joins while that's still running is told to follow: wait, you'll be woken. When the leader finishes, the registry hands you the list of followers to wake. It runs nothing and stores no results. It only keeps track of who's computing, who's waiting, and whom to notify.

This is an initial, unstable API. It's a policy core, not a runtime: there are no threads, promises, or timeouts in here.

## Quick Start

```bash
gem 'coalesce_machines'
```

```ruby
flights = CoalesceMachines::Coalescer.new
results = {}   # flight_id => value, yours to keep
parked  = {}   # waiter_id => whatever is waiting, yours to wake

case (role = flights.join(key))
when CoalesceMachines::Leader
  results[role.flight_id] = render_dashboard(key)
  flights.complete(role.flight_id)
when CoalesceMachines::Follower
  parked[role.waiter_id] = queue
else
  raise "coalescer refused #{key}: #{role}"   # :flights_full, :waiters_full
end

flights.drain(32).each do |note|
  parked.delete(note.waiter_id).push(results[note.flight_id])
end
```

One coalescer, five verbs: `join`, `complete`, `abandon`, `cancel`, `drain`. Keys and ids are Integers.

## A Message to the Resistance

So the cache entry for the homepage expired. It takes two seconds to rebuild. In those two seconds, four hundred requests arrived, each looked in the cache, each found nothing, and each started the same two-second query. Your database is now doing the same work four hundred times, which takes rather longer than two seconds, during which more requests arrive.

You know the fix. "Just put a mutex around it." So you do, and now four hundred threads queue on the mutex and run the query one after another, because each of them still thinks it has to. So you add a check-again-after-locking. Then the leader raises, and three hundred ninety-nine waiters hang, or all get the exception, or all retry at once. Then a client disconnects while waiting and its slot is never given back.

The piece you keep rebuilding is a small table: this key has a computation in the air, these callers are waiting on it, and when it lands, here's who to tell. That table is this gem.

## Chapter 1: Stardate 2026.316, the AMS Fitzpatrick

> **AMS Fitzpatrick**, NCC-4229, *Fitzpatrick*-class rendezvous tender.
> One shuttle goes down to the surface. Everyone else holds formation.

The tender doesn't fly the mission. It keeps the rendezvous board. The first ship to ask for a survey of a given moon is sent to do it. Every ship that asks for the same moon while that one is still out is assigned a holding position and a callsign. When the survey ship reports back, the tender reads out the callsigns. Then it wipes that moon off the board, because a survey from an hour ago is somebody else's department.

On a bridge somewhere, a cadet volunteers:

```ruby
def dashboard(key)
  Rails.cache.fetch(key) { render_dashboard(key) }
end
```

"That," says the tender's flight controller, "sends the whole fleet to the same moon."

## The Flight Plan

Each key the coalescer knows about is in flight or it isn't:

```mermaid
stateDiagram-v2
    [*] --> idle
    idle --> in_flight: join, returns a Leader
    in_flight --> in_flight: join, returns a Follower
    in_flight --> in_flight: cancel(waiter_id)
    in_flight --> idle: complete(flight_id), followers notified :completed
    in_flight --> idle: abandon(flight_id), followers notified :abandoned
```

And the afternoon three requests wanted dashboard 42 at once:

```mermaid
sequenceDiagram
    participant A as Request A
    participant B as Request B
    participant C as Coalescer
    participant D as Request D

    A->>C: join(42)
    C-->>A: Leader(flight 1)
    A->>A: render dashboard

    B->>C: join(42)
    C-->>B: Follower(waiter 2, flight 1)
    D->>C: join(42)
    C-->>D: Follower(waiter 3, flight 1)
    Note over B,D: parked, one render running

    A->>C: complete(1)
    C-->>A: 2
    A->>C: drain(32)
    C-->>A: [waiter 2, waiter 3]
    Note over A: wakes B and D with its result
```

## How the Coalescer Works

### Join

```ruby
flights = CoalesceMachines::Coalescer.new

leader = flights.join(42)
leader.class       # => CoalesceMachines::Leader
leader.flight_id   # => 1
leader.key         # => 42
```

A `Leader` means nobody was computing this key and now you are. Join again while it's still in flight and you get a `Follower`:

```ruby
follower = flights.join(42)
follower.class       # => CoalesceMachines::Follower
follower.waiter_id   # => 2
follower.flight_id   # => 1, the same flight
```

`waiter_id` is the notification id. Park whatever is waiting under it.

### Complete

```ruby
flights.complete(leader.flight_id)   # => 1, one follower to notify
flights.complete(leader.flight_id)   # => :stale, already settled
flights.join(42).class               # => CoalesceMachines::Leader
```

`complete` returns how many followers were queued for notification and takes the key off the board. The coalescer doesn't remember that 42 was ever computed. The next `join(42)` leads a new flight with a new id, and the old id stays `:stale` forever.

### Drain

```ruby
notes = flights.drain(32)
notes.first
# => #<data CoalesceMachines::Notification waiter_id=2, flight_id=1, key=42, outcome=:completed>
flights.drain(32)   # => []
```

`drain(max)` hands back at most `max` notifications, oldest first, and keeps the rest for the next call. Flights are drained in the order they settled, and followers of one flight in the order they joined. Each notification is delivered once.

### Abandon

A leader that failed and has nothing to share gives the key back:

```ruby
flights = CoalesceMachines::Coalescer.new
leader = flights.join(7)
follower = flights.join(7)

flights.abandon(leader.flight_id)   # => 1
flights.drain(8).first.outcome      # => :abandoned
flights.join(7).class               # => CoalesceMachines::Leader
```

Followers of an abandoned flight are notified with `:abandoned`. The ones that still want the value `join` again, and the first one back leads. If you'd rather every follower see the leader's exception, call `complete` and store the exception as the result. That's your policy, not the coalescer's.

### Cancel

A follower that stopped caring, say its client hung up, withdraws:

```ruby
flights = CoalesceMachines::Coalescer.new
leader = flights.join(7)
follower = flights.join(7)

flights.cancel(follower.waiter_id)     # => :cancelled
flights.cancel(follower.waiter_id)     # => :stale
flights.followers(leader.flight_id)    # => 0
flights.complete(leader.flight_id)     # => 0, nobody to notify
```

Cancel also works after the flight settled, as long as the notification hasn't been drained yet. Either way, once you get `:cancelled`, that `waiter_id` will never come out of `drain`.

The flight keeps going when its last follower leaves. If you want to stop a computation nobody is waiting for, `followers(flight_id)` tells you when that's true.

### Capacity

The coalescer holds 64 flights and 256 waiters. A waiter slot is in use while its follower waits, and stays in use until its notification is drained:

```ruby
flights = CoalesceMachines::Coalescer.new
CoalesceMachines::Coalescer::FLIGHT_CAPACITY   # => 64
CoalesceMachines::Coalescer::WAITER_CAPACITY   # => 256

leaders = 64.times.map { |key| flights.join(key) }
flights.join(64)                               # => :flights_full

256.times { flights.join(0) }
flights.join(0)                                # => :waiters_full

flights.complete(leaders.first.flight_id)      # => 256
flights.join(1)                                # => :waiters_full, nothing drained yet
flights.drain(1)
flights.join(1).class                          # => CoalesceMachines::Follower
```

Nothing is evicted to make room. On `:flights_full` or `:waiters_full`, compute it yourself uncoalesced, shed the request, or drain and try again.

### Looking around

```ruby
flights.flight(42)      # => flight_id, or nil when 42 isn't in flight
flights.followers(id)   # => followers still waiting, or nil when id isn't in flight
flights.in_flight       # => flights not yet settled
flights.waiting         # => followers waiting, across all keys
flights.pending         # => notifications queued and not yet drained
```

## The Rules of the Tender

- **Keys and ids are Integers** in `0..2**64-1`. Floats, nil, and strings raise `TypeError`. Out of range raises `RangeError`. If your keys are strings, hash them yourself, and know that two keys with the same hash share a flight.
- **Ids belong to one coalescer.** They're plain Integers counted from 1, so an id from one `Coalescer` means something else, or nothing, in another. A flight id is not a waiter id.
- **Ids are never reused.** A settled flight, a cancelled waiter, and a drained notification are `:stale` from then on.
- **Refusals don't raise and don't change anything.** `:flights_full`, `:waiters_full`, `:exhausted`, and `:stale` are answers. Only malformed input raises.
- **Whoever holds the flight id can settle it.** Give it to the leader and nobody else.
- **Frozen means frozen.** `freeze` the coalescer and `join`, `complete`, `abandon`, `cancel`, and `drain` all raise `FrozenError`.
- **Not thread-safe.** Wrap it in your own mutex if several threads share one. Hold the mutex for the call, not for the computation.

## What You Think You're Doing vs Reality

### You Think: "The cache handles concurrent misses."
### Reality: The cache handles concurrent hits.

A cache answers "do I have it?" A miss means no, for everyone who asks until someone writes it. Four hundred misses are four hundred computations. Coalescing covers the gap between the first miss and the write.

### You Think: "This is deduplication."
### Reality: Deduplication remembers. This forgets.

[dedup_machines](https://github.com/seuros/dedup_machines) answers "has this already been done?" and keeps saying yes so the effect isn't repeated. The coalescer answers "is this being done right now?" and the moment the flight settles, the answer is no. The second request after completion computes again. If that's not what you want, you want a cache or a dedup ledger behind it.

### You Think: "If the leader dies, the followers will find out."
### Reality: Nobody finds out until you say so.

There's no clock in here. A leader that crashes without calling `complete` or `abandon` holds its key, and its followers, until you call `abandon` for it. Put the leader's work in an `ensure`, or run your own deadline and abandon on expiry.

## What's Yours

The computation, and the thread or fiber it runs on. The result, and how long you keep it: store it under `flight_id` and drop it once the followers are woken. The parking: a queue, a promise, a condition variable, keyed by `waiter_id`. The wake-up: call `drain` after you settle, and keep calling until it comes back empty. Timeouts for leaders that never land. Locking, if more than one thread touches the coalescer. And the keys: what counts as "the same request" is decided by the Integer you pass in.

The coalescer lives in your process and coordinates nothing across processes. Two app servers will each fly their own.

## License

MIT License. See [LICENSE.txt](LICENSE.txt) for details.

**Remember: the stampede isn't four hundred requests. It's four hundred computations.**
