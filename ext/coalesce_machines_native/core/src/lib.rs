#![no_std]
#![doc = include_str!("../README.md")]

use core::fmt;

use periscope_machines::{Issuer, scope_of, sequence_of};

mod chain;

use chain::{
    Chain, append, free_slot, is_whole, next_of, pop_front, push_back, set_outcome, unlink,
};

/// Identity of one in-flight computation, and the token that settles it.
///
/// Issued to the leader by [`Coalescer::join`]. Unique for the lifetime of the
/// coalescer that issued it: an id is never reissued, so a settled flight's id
/// is refused as [`Refusal::Stale`] forever after. It carries its issuer's
/// scope, so coalescers built with different scopes never share an id.
///
/// `FlightId::default()` is raw zero, which is never issued: a placeholder for
/// filling an output buffer, refused as [`Refusal::Stale`] everywhere.
///
/// Ids are not ordered: one carries its issuer's scope, and an order across
/// scopes would rank ids no single coalescer ever compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FlightId(u64);

/// Identity of one follower's registration on a flight.
///
/// This is the notification id: the caller parks whatever is waiting under it
/// and wakes it when a [`Notification`] carrying the same id is drained.
///
/// `WaiterId::default()` is raw zero, which is never issued: a placeholder for
/// filling an output buffer, refused as [`Refusal::Stale`] everywhere.
///
/// Not ordered, for the reason [`FlightId`] is not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct WaiterId(u64);

macro_rules! raw_id {
    ($name:ident) => {
        impl $name {
            /// Rebuild an id from [`Self::raw`], e.g. after it crossed an FFI or queue boundary.
            ///
            /// `None` when the sequence number is zero, which no coalescer issues.
            /// Anything else is accepted here: an id the coalescer never issued,
            /// or already retired, is refused as stale by the coalescer.
            #[must_use]
            pub const fn from_raw(raw: u64) -> Option<Self> {
                if sequence_of(raw) == 0 {
                    None
                } else {
                    Some(Self(raw))
                }
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

impl Joined {
    /// A follower's role, rebuilt from its two ids.
    ///
    /// `None` unless both come from one coalescer as a follower's would: the
    /// same scope, and the waiter issued after the flight it follows. The
    /// ids are not looked up; whether they are live is the coalescer's answer.
    #[must_use]
    pub const fn follower(waiter: WaiterId, flight: FlightId) -> Option<Self> {
        if follows(waiter, flight) {
            Some(Self::Follower { waiter, flight })
        } else {
            None
        }
    }
}

/// Whether `waiter` could have joined `flight`: both issued (sequence above
/// zero) in one scope, the waiter after the flight.
const fn follows(waiter: WaiterId, flight: FlightId) -> bool {
    let (waiter, flight) = (waiter.0, flight.0);
    scope_of(waiter) == scope_of(flight)
        && sequence_of(flight) != 0
        && sequence_of(waiter) > sequence_of(flight)
}

/// Why a call changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    waiter: WaiterId,
    flight: FlightId,
    key: u64,
    outcome: Outcome,
}

impl Notification {
    /// A notification, rebuilt from its parts, e.g. after it crossed an FFI
    /// or queue boundary.
    ///
    /// `None` unless `waiter` and `flight` are ids a coalescer could have
    /// emitted together: see [`Joined::follower`]. So the placeholder
    /// [`Notification::default`] is not one this builds.
    #[must_use]
    pub const fn new(
        waiter: WaiterId,
        flight: FlightId,
        key: u64,
        outcome: Outcome,
    ) -> Option<Self> {
        if follows(waiter, flight) {
            Some(Self {
                waiter,
                flight,
                key,
                outcome,
            })
        } else {
            None
        }
    }

    /// The follower's id, as returned by [`Coalescer::join`].
    #[must_use]
    pub const fn waiter(self) -> WaiterId {
        self.waiter
    }

    /// The flight that settled.
    #[must_use]
    pub const fn flight(self) -> FlightId {
        self.flight
    }

    /// The key the flight was computing.
    #[must_use]
    pub const fn key(self) -> u64 {
        self.key
    }

    /// How the flight ended.
    #[must_use]
    pub const fn outcome(self) -> Outcome {
        self.outcome
    }
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
    /// The next slot of whichever chain holds this one.
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
    /// The one sequence of this coalescer: flight, waiter and reserved ids alike.
    ids: Issuer,
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
    /// Ids come from a [`periscope_machines::Issuer`]: the scope in the top 16
    /// bits, a 48-bit sequence counted from one below. Two coalescers in the
    /// same scope issue the same ones, and an id handed to the wrong coalescer
    /// can name a live registration there. Give each coalescer whose ids could
    /// meet its own scope: an id from another scope is then always
    /// [`Refusal::Stale`], refused before any table is read.
    /// The coalescer cannot pick a scope itself; keeping them distinct is the
    /// caller's job.
    #[must_use]
    pub const fn with_scope(scope: u16) -> Self {
        Self::issuing(Issuer::new(scope))
    }

    /// Start with nothing in flight, the id sequence continuing after `issued`.
    ///
    /// For a coalescer that replaces an earlier one in the same `scope`, after
    /// a restart: built with [`Self::with_scope`] it would count from one again,
    /// and an id that outlived the restart could name a new registration. Save
    /// [`Self::issued`] before handing an id out and resume from it, or give
    /// each incarnation a scope of its own. Every id up to `issued` is then
    /// [`Refusal::Stale`]: issued, and no longer live.
    ///
    /// `None` exactly when [`periscope_machines::Issuer::resumed`] answers
    /// `None`: `issued` is beyond [`periscope_machines::SEQUENCE_MAX`].
    #[must_use]
    pub const fn resumed(scope: u16, issued: u64) -> Option<Self> {
        match Issuer::resumed(scope, issued) {
            Some(ids) => Some(Self::issuing(ids)),
            None => None,
        }
    }

    /// The scope carried by every id this coalescer issues.
    #[must_use]
    pub const fn scope(&self) -> u16 {
        self.ids.scope()
    }

    /// How many ids this coalescer has issued, of every kind: flights,
    /// waiters and reserved ids. The count to save for [`Self::resumed`].
    #[must_use]
    pub const fn issued(&self) -> u64 {
        self.ids.issued()
    }

    /// How many ids are left before [`Refusal::Exhausted`].
    ///
    /// For a core built on this one that needs more than one id for a single
    /// step: check here first, so a step refused half way consumes none.
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.ids.remaining()
    }

    /// Whether `raw` is an id of this coalescer's sequence: a flight, a
    /// waiter or a reserved id. It proves origin, not liveness, and an id
    /// from another scope is never owned.
    #[must_use]
    pub const fn owns(&self, raw: u64) -> bool {
        self.ids.owns(raw)
    }

    /// Take the next id of this coalescer's sequence for a handle of the
    /// caller's own.
    ///
    /// This is the seam for a core built on top of a coalescer that issues
    /// handles of its own (a lease, a ticket): drawn from here, they share
    /// the one sequence, so no handle of the outer core ever equals a flight
    /// or waiter id and a handle rebuilt as the wrong kind names nothing.
    /// A reserved id is never a flight or a waiter: [`Self::complete`],
    /// [`Self::abandon`] and [`Self::cancel`] answer [`Refusal::Stale`] for it
    /// and [`Self::followers`] answers `None`, forever, and it is never issued
    /// again. What it names is the caller's table to keep.
    ///
    /// # Errors
    ///
    /// [`Refusal::Exhausted`] when no id is left. Nothing changes.
    pub const fn reserve(&mut self) -> Result<u64, Refusal> {
        match self.ids.issue() {
            Ok(id) => Ok(id),
            Err(refusal) => Err(spent(refusal)),
        }
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
        if let Some((home, flight)) = self.computing(key) {
            let slot = free_slot(&self.waiters).ok_or(Refusal::WaitersFull)?;
            // The last thing that can refuse, so a refused join consumes no id.
            let id = self.ids.issue().map_err(spent)?;
            self.waiters[slot] = Some(Waiter {
                id,
                key,
                flight: flight.id,
                home,
                outcome: None,
                next: None,
            });
            let mut followers = flight.followers;
            push_back(&mut self.waiters, &mut followers, slot);
            self.flights[home] = Some(Flight {
                followers,
                ..flight
            });
            return Ok(Joined::Follower {
                waiter: WaiterId(id),
                flight: FlightId(flight.id),
            });
        }
        let slot = free_slot(&self.flights).ok_or(Refusal::FlightsFull)?;
        let id = self.ids.issue().map_err(spent)?;
        self.flights[slot] = Some(Flight {
            id,
            key,
            followers: Chain::EMPTY,
        });
        self.in_flight += 1;
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
    /// [`Refusal::Stale`] when `flight` is not in flight, or was issued in
    /// another scope. Nothing changes.
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
    /// [`Refusal::Stale`] when `flight` is not in flight, or was issued in
    /// another scope. Nothing changes.
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
    /// [`Refusal::Stale`] when `waiter` is not registered, or was issued in
    /// another scope. Nothing changes.
    pub fn cancel(&mut self, waiter: WaiterId) -> Result<(), Refusal> {
        if !self.ids.owns(waiter.0) {
            return Err(Refusal::Stale);
        }
        let (slot, found) = self
            .waiters
            .iter()
            .enumerate()
            .find_map(|(slot, entry)| Some((slot, entry.filter(|w| w.id == waiter.0)?)))
            .ok_or(Refusal::Stale)?;
        let chain = match found.outcome {
            Some(_) => Some(&mut self.ready),
            None => self
                .flights
                .get_mut(found.home)
                .and_then(Option::as_mut)
                .map(|flight| &mut flight.followers),
        };
        // `unlink` changes nothing when the chain does not hold the slot.
        if !chain.is_some_and(|chain| unlink(&mut self.waiters, chain, slot)) {
            debug_assert!(
                false,
                "a registered waiter is linked in its flight's chain or the ready queue"
            );
            return Err(Refusal::Stale);
        }
        self.waiters[slot] = None;
        Ok(())
    }

    /// Take the oldest queued notification, freeing its waiter slot.
    pub fn poll(&mut self) -> Option<Notification> {
        let slot = self.ready.head?;
        // Read the whole notification before taking anything.
        let Some(Waiter {
            id,
            flight,
            key,
            outcome: Some(outcome),
            ..
        }) = self.waiters[slot]
        else {
            debug_assert!(false, "a queued slot holds a settled waiter");
            return None;
        };
        pop_front(&self.waiters, &mut self.ready);
        self.waiters[slot] = None;
        Some(Notification {
            waiter: WaiterId(id),
            flight: FlightId(flight),
            key,
            outcome,
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
        self.computing(key).map(|(_, flight)| FlightId(flight.id))
    }

    /// Followers still waiting on `flight`, or `None` when it is not in flight.
    #[must_use]
    pub fn followers(&self, flight: FlightId) -> Option<usize> {
        Some(self.live_flight(flight)?.1.followers.len)
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

    const fn issuing(ids: Issuer) -> Self {
        Self {
            flights: [None; FLIGHTS],
            waiters: [None; WAITERS],
            ready: Chain::EMPTY,
            in_flight: 0,
            ids,
        }
    }

    fn settle(&mut self, flight: FlightId, outcome: Outcome) -> Result<usize, Refusal> {
        let (slot, settled) = self.live_flight(flight).ok_or(Refusal::Stale)?;
        let followers = settled.followers;
        // Every check comes first: past this point nothing can stop half way.
        let (Some(in_flight), true, true) = (
            self.in_flight.checked_sub(1),
            is_whole(&self.waiters, followers),
            is_whole(&self.waiters, self.ready),
        ) else {
            debug_assert!(
                false,
                "a live flight is counted, and its followers and the ready queue link waiters"
            );
            return Err(Refusal::Stale);
        };
        let mut cursor = followers.head;
        while let Some(index) = cursor {
            set_outcome(&mut self.waiters, index, outcome);
            cursor = next_of(&self.waiters, index);
        }
        append(&mut self.waiters, &mut self.ready, followers);
        // Taking the flight is last, so a refusal above leaves it in the air.
        self.flights[slot] = None;
        self.in_flight = in_flight;
        Ok(followers.len)
    }

    /// The slot and record of the flight computing `key`, if any.
    fn computing(&self, key: u64) -> Option<(usize, Flight)> {
        self.find(|flight| flight.key == key)
    }

    /// The first live flight `wanted` picks, with its slot.
    fn find(&self, wanted: impl Fn(&Flight) -> bool) -> Option<(usize, Flight)> {
        self.flights
            .iter()
            .enumerate()
            .find_map(|(slot, entry)| entry.filter(&wanted).map(|flight| (slot, flight)))
    }

    /// The slot and record of a live flight. An id this coalescer never
    /// issued, one from another scope included, is turned away before the
    /// table is read.
    fn live_flight(&self, flight: FlightId) -> Option<(usize, Flight)> {
        if !self.ids.owns(flight.0) {
            return None;
        }
        self.find(|live| live.id == flight.0)
    }
}

/// The issuer's only refusal is this core's [`Refusal::Exhausted`].
const fn spent(refusal: periscope_machines::Refusal) -> Refusal {
    match refusal {
        periscope_machines::Refusal::Exhausted => Refusal::Exhausted,
    }
}

#[cfg(test)]
mod tests;
