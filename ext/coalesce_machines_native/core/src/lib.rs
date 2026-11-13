#![no_std]
#![doc = include_str!("../README.md")]

use core::fmt;

/// Bits of an id that count registrations; the bits above carry the scope.
pub const SEQUENCE_BITS: u32 = 48;

/// The last sequence number a coalescer issues before it is [`Refusal::Exhausted`].
pub const SEQUENCE_MAX: u64 = (1 << SEQUENCE_BITS) - 1;

/// Build a raw id from the issuer's scope and a sequence number at or below [`SEQUENCE_MAX`].
///
/// Shared by cores that issue ids alongside a coalescer and want the same layout.
#[must_use]
pub const fn scoped(scope: u16, sequence: u64) -> u64 {
    ((scope as u64) << SEQUENCE_BITS) | (sequence & SEQUENCE_MAX)
}

/// The scope a raw id was issued under.
#[must_use]
pub const fn scope_of(raw: u64) -> u16 {
    (raw >> SEQUENCE_BITS) as u16
}

/// Identity of one in-flight computation, and the token that settles it.
///
/// Issued to the leader by [`Coalescer::join`]. Unique for the lifetime of the
/// coalescer that issued it: an id is never reissued, so a settled flight's id
/// is refused as [`Refusal::Stale`] forever after. It carries its issuer's
/// scope, so coalescers built with different scopes never share an id.
///
/// `FlightId::default()` is raw zero, which is never issued: a placeholder for
/// filling an output buffer, refused as [`Refusal::Stale`] everywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlightId(u64);

/// Identity of one follower's registration on a flight.
///
/// This is the notification id: the caller parks whatever is waiting under it
/// and wakes it when a [`Notification`] carrying the same id is drained.
///
/// `WaiterId::default()` is raw zero, which is never issued: a placeholder for
/// filling an output buffer, refused as [`Refusal::Stale`] everywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WaiterId(u64);

macro_rules! raw_id {
    ($name:ident) => {
        impl $name {
            /// Rebuild an id from [`Self::raw`], e.g. after it crossed an FFI or queue boundary.
            ///
            /// An id the coalescer never issued, or already retired, is refused as stale.
            #[must_use]
            pub const fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            /// The id as a plain integer. Never zero for an issued id.
            #[must_use]
            pub const fn raw(self) -> u64 {
                self.0
            }

            /// The scope of the coalescer that issued this id, see [`Coalescer::with_scope`].
            #[must_use]
            pub const fn scope(self) -> u16 {
                scope_of(self.0)
            }
        }
    };
}
raw_id!(FlightId);
raw_id!(WaiterId);

/// Role handed to a caller that joined a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joined {
    /// No flight existed for the key. The caller runs the computation and must
    /// settle the flight with [`Coalescer::complete`] or [`Coalescer::abandon`].
    Leader(FlightId),
    /// A flight for the key is already in the air. The caller waits for a
    /// [`Notification`] carrying `waiter`, or gives up with [`Coalescer::cancel`].
    Follower {
        /// The follower's notification id.
        waiter: WaiterId,
        /// The flight being shared.
        flight: FlightId,
    },
}

/// Why a call changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// Every flight slot holds an unsettled flight; no new key can lead.
    FlightsFull,
    /// Every waiter slot holds a waiting follower or an undrained notification.
    WaitersFull,
    /// The sequence space is spent; this coalescer can register nothing more.
    Exhausted,
    /// The id names nothing live: never issued, already settled, already
    /// cancelled, or its notification was already drained.
    Stale,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::FlightsFull => "every flight slot is in use",
            Self::WaitersFull => "every waiter slot is in use",
            Self::Exhausted => "id space exhausted",
            Self::Stale => "id does not name a live registration",
        })
    }
}

impl core::error::Error for Refusal {}

/// The former name of [`Refusal`].
#[deprecated(note = "renamed to `Refusal`")]
pub type Refused = Refusal;

/// How a flight ended. The result itself is the caller's to keep.
///
/// `Outcome::default()` is [`Outcome::Abandoned`]. It exists only so a
/// placeholder [`Notification`] can be built; it says nothing about any flight.
/// `Abandoned` is the one that promises no result if a placeholder is ever
/// read by mistake.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Outcome {
    /// The leader finished; the caller holds a result to share with followers.
    Completed,
    /// The leader gave up without a result; followers should join again.
    #[default]
    Abandoned,
}

/// One follower to wake, emitted when its flight settles.
///
/// `Notification::default()` is a placeholder for sizing the buffer handed to
/// [`Coalescer::drain`], e.g. `[Notification::default(); 8]`. The coalescer
/// never emits it: its ids are raw zero, which is never issued.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Notification {
    /// The follower's id, as returned by [`Coalescer::join`].
    pub waiter: WaiterId,
    /// The flight that settled.
    pub flight: FlightId,
    /// The key the flight was computing.
    pub key: u64,
    /// How the flight ended.
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Chain {
    head: Option<usize>,
    tail: Option<usize>,
    len: usize,
}

impl Chain {
    const EMPTY: Self = Self {
        head: None,
        tail: None,
        len: 0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flight {
    id: u64,
    key: u64,
    followers: Chain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Waiter {
    id: u64,
    key: u64,
    flight: u64,
    /// Flight slot while waiting; meaningless once `outcome` is set.
    home: usize,
    outcome: Option<Outcome>,
    next: Option<usize>,
}

/// Bounded per-key in-flight registry: one leader per key, followers share it.
///
/// `FLIGHTS` bounds the keys in flight at once. `WAITERS` bounds followers
/// that are waiting plus notifications not yet drained, across all keys.
///
/// The coalescer runs nothing and stores no results. It answers who computes,
/// who waits, and whom to wake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coalescer<const FLIGHTS: usize, const WAITERS: usize> {
    flights: [Option<Flight>; FLIGHTS],
    waiters: [Option<Waiter>; WAITERS],
    ready: Chain,
    in_flight: usize,
    /// Sequence number of the last id issued; the scope is added on the way out.
    issued: u64,
    scope: u16,
}

impl<const FLIGHTS: usize, const WAITERS: usize> Default for Coalescer<FLIGHTS, WAITERS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const FLIGHTS: usize, const WAITERS: usize> Coalescer<FLIGHTS, WAITERS> {
    /// Start with nothing in flight and a fresh id space, in scope zero.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_scope(0)
    }

    /// Start with nothing in flight, issuing ids that carry `scope`.
    ///
    /// Ids are plain integers, so two coalescers in the same scope issue the
    /// same ones, and an id handed to the wrong coalescer can name a live
    /// registration there. Give each coalescer whose ids could meet its own
    /// scope: an id from another scope is then always [`Refusal::Stale`].
    /// The coalescer cannot pick a scope itself; keeping them distinct is the
    /// caller's job.
    #[must_use]
    pub const fn with_scope(scope: u16) -> Self {
        Self {
            flights: [None; FLIGHTS],
            waiters: [None; WAITERS],
            ready: Chain::EMPTY,
            in_flight: 0,
            issued: 0,
            scope,
        }
    }

    /// The scope carried by every id this coalescer issues.
    #[must_use]
    pub const fn scope(&self) -> u16 {
        self.scope
    }

    /// Register interest in `key`.
    ///
    /// The first caller for a key becomes its [`Joined::Leader`]; every caller
    /// after that, until the flight settles, becomes a [`Joined::Follower`].
    /// Once a flight settles the key is unknown again and the next caller leads.
    ///
    /// # Errors
    ///
    /// [`Refusal::FlightsFull`] when the key is new and no flight slot is free,
    /// [`Refusal::WaitersFull`] when the key is in flight and no waiter slot is
    /// free, [`Refusal::Exhausted`] when no id is left. Nothing changes.
    pub fn join(&mut self, key: u64) -> Result<Joined, Refusal> {
        if let Some(home) = self.slot_of_key(key) {
            let slot = free_slot(&self.waiters).ok_or(Refusal::WaitersFull)?;
            let (sequence, id) = self.next_id()?;
            let Some(flight) = self.flights[home].as_mut() else {
                return Err(Refusal::Stale);
            };
            self.waiters[slot] = Some(Waiter {
                id,
                key,
                flight: flight.id,
                home,
                outcome: None,
                next: None,
            });
            push_back(&mut self.waiters, &mut flight.followers, slot);
            self.issued = sequence;
            return Ok(Joined::Follower {
                waiter: WaiterId(id),
                flight: FlightId(flight.id),
            });
        }
        let slot = free_slot(&self.flights).ok_or(Refusal::FlightsFull)?;
        let (sequence, id) = self.next_id()?;
        self.flights[slot] = Some(Flight {
            id,
            key,
            followers: Chain::EMPTY,
        });
        self.in_flight += 1;
        self.issued = sequence;
        Ok(Joined::Leader(FlightId(id)))
    }

    /// Settle a flight whose leader finished, freeing its key.
    ///
    /// Every follower still waiting gets a [`Notification`] with
    /// [`Outcome::Completed`], queued in join order behind those of flights
    /// settled earlier. Returns how many were queued.
    ///
    /// # Errors
    ///
    /// [`Refusal::Stale`] when `flight` is not in flight. Nothing changes.
    pub fn complete(&mut self, flight: FlightId) -> Result<usize, Refusal> {
        self.settle(flight, Outcome::Completed)
    }

    /// Settle a flight whose leader gave up without a result, freeing its key.
    ///
    /// Followers are notified with [`Outcome::Abandoned`]; one that still wants
    /// the value joins again and the first to do so leads. Returns how many
    /// notifications were queued.
    ///
    /// # Errors
    ///
    /// [`Refusal::Stale`] when `flight` is not in flight. Nothing changes.
    pub fn abandon(&mut self, flight: FlightId) -> Result<usize, Refusal> {
        self.settle(flight, Outcome::Abandoned)
    }

    /// Withdraw a follower, freeing its slot.
    ///
    /// Works while the follower is waiting and while its notification is
    /// queued but undrained; either way no notification for `waiter` is ever
    /// drained afterwards. The flight itself is unaffected, even when its last
    /// follower leaves: stopping the leader is the caller's decision, see
    /// [`Coalescer::followers`].
    ///
    /// # Errors
    ///
    /// [`Refusal::Stale`] when `waiter` is not registered. Nothing changes.
    pub fn cancel(&mut self, waiter: WaiterId) -> Result<(), Refusal> {
        let (slot, found) = self
            .waiters
            .iter()
            .enumerate()
            .find_map(|(slot, entry)| Some((slot, entry.filter(|w| w.id == waiter.0)?)))
            .ok_or(Refusal::Stale)?;
        let unlinked = if found.outcome.is_some() {
            unlink(&mut self.waiters, &mut self.ready, slot)
        } else if let Some(flight) = self.flights.get_mut(found.home).and_then(Option::as_mut) {
            unlink(&mut self.waiters, &mut flight.followers, slot)
        } else {
            false
        };
        if !unlinked {
            return Err(Refusal::Stale);
        }
        self.waiters[slot] = None;
        Ok(())
    }

    /// Take the oldest queued notification, freeing its waiter slot.
    pub fn poll(&mut self) -> Option<Notification> {
        let slot = pop_front(&self.waiters, &mut self.ready)?;
        let waiter = self.waiters[slot].take()?;
        Some(Notification {
            waiter: WaiterId(waiter.id),
            flight: FlightId(waiter.flight),
            key: waiter.key,
            outcome: waiter.outcome?,
        })
    }

    /// Move queued notifications into `out`, oldest first, and return how many.
    ///
    /// Writes the first `n` entries and leaves the rest untouched; only those
    /// `n` are notifications. Resumable: whatever did not fit stays queued, in
    /// order, for the next call.
    pub fn drain(&mut self, out: &mut [Notification]) -> usize {
        let mut written = 0;
        for entry in out {
            let Some(notification) = self.poll() else {
                break;
            };
            *entry = notification;
            written += 1;
        }
        written
    }

    /// The flight currently computing `key`, if any.
    #[must_use]
    pub fn flight(&self, key: u64) -> Option<FlightId> {
        let flight = self.flights[self.slot_of_key(key)?].as_ref()?;
        Some(FlightId(flight.id))
    }

    /// Followers still waiting on `flight`, or `None` when it is not in flight.
    #[must_use]
    pub fn followers(&self, flight: FlightId) -> Option<usize> {
        let flight = self.flights[self.slot_of_flight(flight)?].as_ref()?;
        Some(flight.followers.len)
    }

    /// Flights not yet settled.
    #[must_use]
    pub const fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Followers waiting on unsettled flights, across all keys.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.waiters.iter().flatten().count() - self.ready.len
    }

    /// Notifications queued and not yet drained.
    #[must_use]
    pub const fn pending(&self) -> usize {
        self.ready.len
    }

    /// Maximum flights at once (`FLIGHTS`).
    #[must_use]
    pub const fn flight_capacity(&self) -> usize {
        FLIGHTS
    }

    /// Maximum waiting followers plus undrained notifications (`WAITERS`).
    #[must_use]
    pub const fn waiter_capacity(&self) -> usize {
        WAITERS
    }

    fn settle(&mut self, flight: FlightId, outcome: Outcome) -> Result<usize, Refusal> {
        let slot = self.slot_of_flight(flight).ok_or(Refusal::Stale)?;
        let settled = self.flights[slot].take().ok_or(Refusal::Stale)?;
        self.in_flight -= 1;
        let mut cursor = settled.followers.head;
        while let Some(index) = cursor {
            let Some(waiter) = self.waiters[index].as_mut() else {
                break;
            };
            waiter.outcome = Some(outcome);
            cursor = waiter.next;
        }
        append(&mut self.waiters, &mut self.ready, settled.followers);
        Ok(settled.followers.len)
    }

    /// The next sequence number and the scoped id that carries it; issues nothing yet.
    const fn next_id(&self) -> Result<(u64, u64), Refusal> {
        let sequence = self.issued + 1;
        if sequence > SEQUENCE_MAX {
            return Err(Refusal::Exhausted);
        }
        Ok((sequence, scoped(self.scope, sequence)))
    }

    fn slot_of_key(&self, key: u64) -> Option<usize> {
        self.flights
            .iter()
            .position(|entry| entry.is_some_and(|flight| flight.key == key))
    }

    fn slot_of_flight(&self, flight: FlightId) -> Option<usize> {
        self.flights
            .iter()
            .position(|entry| entry.is_some_and(|live| live.id == flight.0))
    }
}

fn free_slot<T>(slots: &[Option<T>]) -> Option<usize> {
    slots.iter().position(Option::is_none)
}

fn set_next(waiters: &mut [Option<Waiter>], slot: usize, next: Option<usize>) {
    if let Some(waiter) = waiters.get_mut(slot).and_then(Option::as_mut) {
        waiter.next = next;
    }
}

fn next_of(waiters: &[Option<Waiter>], slot: usize) -> Option<usize> {
    waiters.get(slot)?.as_ref()?.next
}

fn push_back(waiters: &mut [Option<Waiter>], chain: &mut Chain, slot: usize) {
    set_next(waiters, slot, None);
    match chain.tail {
        Some(tail) => set_next(waiters, tail, Some(slot)),
        None => chain.head = Some(slot),
    }
    chain.tail = Some(slot);
    chain.len += 1;
}

fn pop_front(waiters: &[Option<Waiter>], chain: &mut Chain) -> Option<usize> {
    let head = chain.head?;
    chain.head = next_of(waiters, head);
    if chain.head.is_none() {
        chain.tail = None;
    }
    chain.len -= 1;
    Some(head)
}

/// Splice all of `tail` behind `chain`, keeping both orders.
fn append(waiters: &mut [Option<Waiter>], chain: &mut Chain, tail: Chain) {
    if tail.head.is_none() {
        return;
    }
    match chain.tail {
        Some(last) => set_next(waiters, last, tail.head),
        None => chain.head = tail.head,
    }
    chain.tail = tail.tail;
    chain.len += tail.len;
}

/// Remove `slot` from `chain`; `false` when the chain does not hold it.
fn unlink(waiters: &mut [Option<Waiter>], chain: &mut Chain, slot: usize) -> bool {
    let mut previous = None;
    let mut cursor = chain.head;
    while let Some(index) = cursor {
        if index == slot {
            let next = next_of(waiters, slot);
            match previous {
                Some(before) => set_next(waiters, before, next),
                None => chain.head = next,
            }
            if chain.tail == Some(slot) {
                chain.tail = previous;
            }
            chain.len -= 1;
            return true;
        }
        previous = cursor;
        cursor = next_of(waiters, index);
    }
    false
}

#[cfg(test)]
mod tests;
