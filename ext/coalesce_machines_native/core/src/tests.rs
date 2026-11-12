extern crate std;

use std::collections::VecDeque;
use std::vec::Vec;

use super::*;

fn leader<const F: usize, const W: usize>(c: &mut Coalescer<F, W>, key: u64) -> FlightId {
    match c.join(key) {
        Ok(Joined::Leader(flight)) => flight,
        other => panic!("expected leader for {key}, got {other:?}"),
    }
}

fn follower<const F: usize, const W: usize>(c: &mut Coalescer<F, W>, key: u64) -> WaiterId {
    match c.join(key) {
        Ok(Joined::Follower { waiter, .. }) => waiter,
        other => panic!("expected follower for {key}, got {other:?}"),
    }
}

fn drained<const F: usize, const W: usize>(c: &mut Coalescer<F, W>) -> Vec<Notification> {
    core::iter::from_fn(|| c.poll()).collect()
}

#[test]
fn followers_share_one_flight_and_are_notified_in_join_order() {
    let mut c = Coalescer::<2, 4>::new();
    let flight = leader(&mut c, 7);
    assert_eq!(c.flight(7), Some(flight));
    let first = c.join(7);
    let Ok(Joined::Follower {
        waiter: a,
        flight: shared,
    }) = first
    else {
        panic!("expected follower, got {first:?}");
    };
    assert_eq!(shared, flight);
    let b = follower(&mut c, 7);
    assert_ne!(a, b);
    assert_eq!((c.in_flight(), c.waiting(), c.pending()), (1, 2, 0));
    assert_eq!(c.followers(flight), Some(2));
    assert_eq!(c.poll(), None);

    assert_eq!(c.complete(flight), Ok(2));
    assert_eq!((c.in_flight(), c.waiting(), c.pending()), (0, 0, 2));
    assert_eq!(c.flight(7), None);
    assert_eq!(c.followers(flight), None);
    let expect = |waiter| Notification {
        waiter,
        flight,
        key: 7,
        outcome: Outcome::Completed,
    };
    assert_eq!(drained(&mut c), [expect(a), expect(b)]);
    assert_eq!(c.pending(), 0);
}

#[test]
fn a_settled_key_is_not_remembered() {
    let mut c = Coalescer::<1, 1>::new();
    let first = leader(&mut c, 7);
    assert_eq!(c.complete(first), Ok(0));
    let second = leader(&mut c, 7);
    assert_ne!(first, second);
    assert_eq!(c.complete(first), Err(Refused::Stale));
    assert_eq!(c.abandon(first), Err(Refused::Stale));
    assert_eq!(c.flight(7), Some(second));
    assert_eq!(c.abandon(second), Ok(0));
    assert_eq!(c.abandon(second), Err(Refused::Stale));
}

#[test]
fn stale_ids_are_refused_without_touching_reused_slots() {
    let mut c = Coalescer::<1, 1>::new();
    let old_flight = leader(&mut c, 1);
    let old_waiter = follower(&mut c, 1);
    assert_eq!(c.cancel(old_waiter), Ok(()));
    assert_eq!(c.abandon(old_flight), Ok(0));
    // Same slots, new generations.
    let flight = leader(&mut c, 2);
    let waiter = follower(&mut c, 2);
    let before = c.clone();
    assert_eq!(c.complete(old_flight), Err(Refused::Stale));
    assert_eq!(c.abandon(old_flight), Err(Refused::Stale));
    assert_eq!(c.cancel(old_waiter), Err(Refused::Stale));
    assert_eq!(c.followers(old_flight), None);
    for raw in [0, u64::MAX, flight.raw() + 100] {
        assert_eq!(c.complete(FlightId::from_raw(raw)), Err(Refused::Stale));
        assert_eq!(c.cancel(WaiterId::from_raw(raw)), Err(Refused::Stale));
    }
    // A flight id is not a waiter id and the reverse.
    assert_eq!(
        c.cancel(WaiterId::from_raw(flight.raw())),
        Err(Refused::Stale)
    );
    assert_eq!(
        c.complete(FlightId::from_raw(waiter.raw())),
        Err(Refused::Stale)
    );
    assert_eq!(c, before);
    assert_eq!(c.complete(FlightId::from_raw(flight.raw())), Ok(1));
}

#[test]
fn abandoned_followers_rejoin_and_one_of_them_leads() {
    let mut c = Coalescer::<1, 2>::new();
    let flight = leader(&mut c, 9);
    let a = follower(&mut c, 9);
    let b = follower(&mut c, 9);
    assert_eq!(c.abandon(flight), Ok(2));
    let notes = drained(&mut c);
    assert_eq!(
        notes
            .iter()
            .map(|n| (n.waiter, n.outcome))
            .collect::<Vec<_>>(),
        [(a, Outcome::Abandoned), (b, Outcome::Abandoned)]
    );
    let retry = leader(&mut c, 9);
    assert_ne!(retry, flight);
    assert!(matches!(c.join(9), Ok(Joined::Follower { flight, .. }) if flight == retry));
}

#[test]
fn cancellation_withdraws_waiting_and_queued_followers() {
    let mut c = Coalescer::<2, 5>::new();
    let flight = leader(&mut c, 1);
    let w: Vec<_> = (0..5).map(|_| follower(&mut c, 1)).collect();
    // Head, middle and tail of the waiting chain.
    assert_eq!(c.cancel(w[0]), Ok(()));
    assert_eq!(c.cancel(w[2]), Ok(()));
    assert_eq!(c.cancel(w[4]), Ok(()));
    assert_eq!(c.cancel(w[4]), Err(Refused::Stale));
    assert_eq!(c.followers(flight), Some(2));
    let late = follower(&mut c, 1);
    assert_eq!(c.complete(flight), Ok(3));
    // Queued but undrained: still cancellable, and never delivered after.
    assert_eq!(c.cancel(w[3]), Ok(()));
    assert_eq!(c.pending(), 2);
    assert_eq!(
        drained(&mut c).iter().map(|n| n.waiter).collect::<Vec<_>>(),
        [w[1], late]
    );
    // Drained: the id is spent.
    assert_eq!(c.cancel(w[1]), Err(Refused::Stale));
    assert_eq!(c.cancel(late), Err(Refused::Stale));
    assert_eq!((c.in_flight(), c.waiting(), c.pending()), (0, 0, 0));
}

#[test]
fn losing_every_follower_leaves_the_flight_in_the_air() {
    let mut c = Coalescer::<1, 1>::new();
    let flight = leader(&mut c, 1);
    let only = follower(&mut c, 1);
    assert_eq!(c.cancel(only), Ok(()));
    assert_eq!(c.followers(flight), Some(0));
    assert_eq!(c.flight(1), Some(flight));
    assert_eq!(c.complete(flight), Ok(0));
    assert_eq!(c.poll(), None);
}

#[test]
fn full_tables_refuse_without_mutation() {
    let mut c = Coalescer::<2, 2>::new();
    let one = leader(&mut c, 1);
    let _two = leader(&mut c, 2);
    let before = c.clone();
    assert_eq!(c.join(3), Err(Refused::FlightsFull));
    assert_eq!(c, before);

    let a = follower(&mut c, 1);
    let _b = follower(&mut c, 2);
    let before = c.clone();
    assert_eq!(c.join(1), Err(Refused::WaitersFull));
    assert_eq!(c.join(2), Err(Refused::WaitersFull));
    assert_eq!(c.join(3), Err(Refused::FlightsFull));
    assert_eq!(c, before);

    // An undrained notification still occupies its waiter slot.
    assert_eq!(c.complete(one), Ok(1));
    assert_eq!(c.join(2), Err(Refused::WaitersFull));
    let again = leader(&mut c, 1);
    assert_eq!(c.join(1), Err(Refused::WaitersFull));
    assert_eq!(c.poll().map(|n| n.waiter), Some(a));
    assert!(matches!(c.join(1), Ok(Joined::Follower { flight, .. }) if flight == again));
}

#[test]
fn zero_capacities_refuse_everything() {
    let mut none = Coalescer::<0, 0>::new();
    assert_eq!(none.join(1), Err(Refused::FlightsFull));
    assert_eq!(none.poll(), None);
    assert_eq!((none.flight_capacity(), none.waiter_capacity()), (0, 0));

    let mut solo = Coalescer::<1, 0>::default();
    let flight = leader(&mut solo, 1);
    assert_eq!(solo.join(1), Err(Refused::WaitersFull));
    assert_eq!(solo.complete(flight), Ok(0));
    assert_eq!((solo.flight_capacity(), solo.waiter_capacity()), (1, 0));
}

#[test]
fn drain_is_bounded_and_resumable_across_flights() {
    let mut c = Coalescer::<3, 6>::new();
    let f1 = leader(&mut c, 1);
    let f2 = leader(&mut c, 2);
    let a = follower(&mut c, 1);
    let b = follower(&mut c, 2);
    let third = follower(&mut c, 1);
    let fourth = follower(&mut c, 2);
    // Flight 2 settles first, so its followers are queued first.
    assert_eq!(c.abandon(f2), Ok(2));
    assert_eq!(c.complete(f1), Ok(2));

    assert_eq!(c.drain(&mut []), 0);
    assert_eq!(c.pending(), 4);

    let mut out = [None; 3];
    assert_eq!(c.drain(&mut out), 3);
    assert_eq!(
        out.map(|n| n.map(|n| (n.waiter, n.flight, n.key, n.outcome))),
        [
            Some((b, f2, 2, Outcome::Abandoned)),
            Some((fourth, f2, 2, Outcome::Abandoned)),
            Some((a, f1, 1, Outcome::Completed)),
        ]
    );
    assert_eq!(c.pending(), 1);

    // A flight settled between drains queues behind what is left.
    let f3 = leader(&mut c, 3);
    let fifth = follower(&mut c, 3);
    assert_eq!(c.complete(f3), Ok(1));

    let mut out = [None; 3];
    assert_eq!(c.drain(&mut out), 2);
    assert_eq!(
        out.map(|n| n.map(|n| n.waiter)),
        [Some(third), Some(fifth), None]
    );
    assert_eq!(c.drain(&mut out), 0);
    // Untouched past the count.
    assert_eq!(
        out.map(|n| n.map(|n| n.waiter)),
        [Some(third), Some(fifth), None]
    );
}

#[test]
fn key_and_id_limits() {
    let mut c = Coalescer::<2, 2>::new();
    let low = leader(&mut c, 0);
    let high = leader(&mut c, u64::MAX);
    assert_ne!(low, high);
    assert_eq!(c.flight(0), Some(low));
    assert_eq!(c.flight(u64::MAX), Some(high));
    assert_eq!(c.flight(1), None);
    let waiter = follower(&mut c, u64::MAX);
    assert_eq!(c.complete(high), Ok(1));
    assert_eq!(
        c.poll(),
        Some(Notification {
            waiter,
            flight: high,
            key: u64::MAX,
            outcome: Outcome::Completed
        })
    );

    // The last id is issued, then the space is spent for both roles.
    let mut c = Coalescer::<2, 2>::new();
    c.issued = u64::MAX - 2;
    let flight = leader(&mut c, 1);
    let waiter = follower(&mut c, 1);
    assert_eq!((flight.raw(), waiter.raw()), (u64::MAX - 1, u64::MAX));
    let before = c.clone();
    assert_eq!(c.join(1), Err(Refused::Exhausted));
    assert_eq!(c.join(2), Err(Refused::Exhausted));
    assert_eq!(c, before);
    // What is registered still settles and drains.
    assert_eq!(c.complete(flight), Ok(1));
    assert_eq!(c.poll().map(|n| n.waiter), Some(waiter));
    assert_eq!(c.join(1), Err(Refused::Exhausted));
}

#[test]
fn refusals_describe_themselves() {
    use std::string::ToString;
    for refusal in [
        Refused::FlightsFull,
        Refused::WaitersFull,
        Refused::Exhausted,
        Refused::Stale,
    ] {
        assert_ne!(refusal.to_string(), "");
    }
}

/// Independent naive model: growable collections, linear search, no slots.
struct Model {
    flights: Vec<(u64, u64, Vec<u64>)>,
    ready: VecDeque<Notification>,
    issued: u64,
    flight_cap: usize,
    waiter_cap: usize,
}

impl Model {
    fn held(&self) -> usize {
        self.flights.iter().map(|f| f.2.len()).sum::<usize>() + self.ready.len()
    }

    fn join(&mut self, key: u64) -> Result<Joined, Refused> {
        let cap = self.waiter_cap;
        let held = self.held();
        if let Some(flight) = self.flights.iter_mut().find(|f| f.1 == key) {
            if held >= cap {
                return Err(Refused::WaitersFull);
            }
            self.issued += 1;
            flight.2.push(self.issued);
            return Ok(Joined::Follower {
                waiter: WaiterId(self.issued),
                flight: FlightId(flight.0),
            });
        }
        if self.flights.len() >= self.flight_cap {
            return Err(Refused::FlightsFull);
        }
        self.issued += 1;
        self.flights.push((self.issued, key, Vec::new()));
        Ok(Joined::Leader(FlightId(self.issued)))
    }

    fn settle(&mut self, id: u64, outcome: Outcome) -> Result<usize, Refused> {
        let at = self
            .flights
            .iter()
            .position(|f| f.0 == id)
            .ok_or(Refused::Stale)?;
        let (flight, key, waiters) = self.flights.remove(at);
        for waiter in &waiters {
            self.ready.push_back(Notification {
                waiter: WaiterId(*waiter),
                flight: FlightId(flight),
                key,
                outcome,
            });
        }
        Ok(waiters.len())
    }

    fn cancel(&mut self, id: u64) -> Result<(), Refused> {
        for flight in &mut self.flights {
            if let Some(at) = flight.2.iter().position(|w| *w == id) {
                flight.2.remove(at);
                return Ok(());
            }
        }
        let at = self
            .ready
            .iter()
            .position(|n| n.waiter.0 == id)
            .ok_or(Refused::Stale)?;
        self.ready.remove(at);
        Ok(())
    }
}

struct Draws(u64);

impl Draws {
    fn below(&mut self, bound: u64) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % bound
    }
}

fn agree<const F: usize, const W: usize>(c: &Coalescer<F, W>, model: &Model, keys: u64) {
    assert_eq!(c.in_flight(), model.flights.len());
    assert_eq!(c.pending(), model.ready.len());
    assert_eq!(c.waiting(), model.held() - model.ready.len());
    for key in 0..keys {
        let expected = model.flights.iter().find(|f| f.1 == key);
        assert_eq!(c.flight(key), expected.map(|f| FlightId(f.0)));
    }
    for id in 0..=model.issued + 1 {
        let expected = model.flights.iter().find(|f| f.0 == id);
        assert_eq!(c.followers(FlightId(id)), expected.map(|f| f.2.len()));
    }
}

fn run_against_model<const F: usize, const W: usize>(seed: u64, keys: u64, steps: usize) {
    let mut draws = Draws(seed);
    let mut c = Coalescer::<F, W>::new();
    let mut model = Model {
        flights: Vec::new(),
        ready: VecDeque::new(),
        issued: 0,
        flight_cap: F,
        waiter_cap: W,
    };
    let mut refusals = 0_usize;
    for _ in 0..steps {
        let before = c.clone();
        // Any id ever issued, plus one that never was: live, stale and wrong-role alike.
        let id = draws.below(model.issued + 2);
        let refused = match draws.below(10) {
            0..=3 => {
                let key = draws.below(keys);
                let (got, want) = (c.join(key), model.join(key));
                assert_eq!(got, want);
                got.is_err()
            }
            4 => {
                let got = c.complete(FlightId(id));
                assert_eq!(got, model.settle(id, Outcome::Completed));
                got.is_err()
            }
            5 => {
                let got = c.abandon(FlightId(id));
                assert_eq!(got, model.settle(id, Outcome::Abandoned));
                got.is_err()
            }
            6 | 7 => {
                let got = c.cancel(WaiterId(id));
                assert_eq!(got, model.cancel(id));
                got.is_err()
            }
            8 => {
                assert_eq!(c.poll(), model.ready.pop_front());
                false
            }
            _ => {
                let mut out = [None; 3];
                let room = usize::try_from(draws.below(4)).unwrap_or(0);
                let written = c.drain(&mut out[..room]);
                assert_eq!(written, room.min(model.ready.len()));
                for (slot, entry) in out.iter().enumerate() {
                    let want = (slot < written).then(|| model.ready.pop_front()).flatten();
                    assert_eq!(*entry, want);
                }
                false
            }
        };
        if refused {
            refusals += 1;
            assert_eq!(c, before, "a refused call mutated state");
        }
        agree(&c, &model, keys);
    }
    assert!(refusals > 0, "the run never exercised a refusal");
    assert!(model.issued > 0);
}

#[test]
fn randomized_operations_match_the_naive_model() {
    for seed in 0..24 {
        run_against_model::<3, 5>(seed, 5, 1_500);
    }
    // Tight tables: capacity refusals dominate.
    for seed in 100..112 {
        run_against_model::<1, 1>(seed, 3, 800);
        run_against_model::<2, 0>(seed, 3, 400);
    }
    // Roomy tables: long chains, many keys.
    for seed in 200..206 {
        run_against_model::<8, 32>(seed, 12, 1_500);
    }
}
