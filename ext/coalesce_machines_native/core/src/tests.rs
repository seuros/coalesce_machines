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
    assert_eq!(c.complete(first), Err(Refusal::Stale));
    assert_eq!(c.abandon(first), Err(Refusal::Stale));
    assert_eq!(c.flight(7), Some(second));
    assert_eq!(c.abandon(second), Ok(0));
    assert_eq!(c.abandon(second), Err(Refusal::Stale));
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
    assert_eq!(c.complete(old_flight), Err(Refusal::Stale));
    assert_eq!(c.abandon(old_flight), Err(Refusal::Stale));
    assert_eq!(c.cancel(old_waiter), Err(Refusal::Stale));
    assert_eq!(c.followers(old_flight), None);
    for raw in [0, u64::MAX, flight.raw() + 100] {
        assert_eq!(c.complete(FlightId::from_raw(raw)), Err(Refusal::Stale));
        assert_eq!(c.cancel(WaiterId::from_raw(raw)), Err(Refusal::Stale));
    }
    // A flight id is not a waiter id and the reverse.
    assert_eq!(
        c.cancel(WaiterId::from_raw(flight.raw())),
        Err(Refusal::Stale)
    );
    assert_eq!(
        c.complete(FlightId::from_raw(waiter.raw())),
        Err(Refusal::Stale)
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
    assert_eq!(c.cancel(w[4]), Err(Refusal::Stale));
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
    assert_eq!(c.cancel(w[1]), Err(Refusal::Stale));
    assert_eq!(c.cancel(late), Err(Refusal::Stale));
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
    assert_eq!(c.join(3), Err(Refusal::FlightsFull));
    assert_eq!(c, before);

    let a = follower(&mut c, 1);
    let _b = follower(&mut c, 2);
    let before = c.clone();
    assert_eq!(c.join(1), Err(Refusal::WaitersFull));
    assert_eq!(c.join(2), Err(Refusal::WaitersFull));
    assert_eq!(c.join(3), Err(Refusal::FlightsFull));
    assert_eq!(c, before);

    // An undrained notification still occupies its waiter slot.
    assert_eq!(c.complete(one), Ok(1));
    assert_eq!(c.join(2), Err(Refusal::WaitersFull));
    let again = leader(&mut c, 1);
    assert_eq!(c.join(1), Err(Refusal::WaitersFull));
    assert_eq!(c.poll().map(|n| n.waiter), Some(a));
    assert!(matches!(c.join(1), Ok(Joined::Follower { flight, .. }) if flight == again));
}

#[test]
fn zero_capacities_refuse_everything() {
    let mut none = Coalescer::<0, 0>::new();
    assert_eq!(none.join(1), Err(Refusal::FlightsFull));
    assert_eq!(none.poll(), None);
    assert_eq!((none.flight_capacity(), none.waiter_capacity()), (0, 0));

    let mut solo = Coalescer::<1, 0>::default();
    let flight = leader(&mut solo, 1);
    assert_eq!(solo.join(1), Err(Refusal::WaitersFull));
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

    let mut out = [Notification::default(); 3];
    assert_eq!(c.drain(&mut out), 3);
    assert_eq!(
        out.map(|n| (n.waiter, n.flight, n.key, n.outcome)),
        [
            (b, f2, 2, Outcome::Abandoned),
            (fourth, f2, 2, Outcome::Abandoned),
            (a, f1, 1, Outcome::Completed),
        ]
    );
    assert_eq!(c.pending(), 1);

    // A flight settled between drains queues behind what is left.
    let f3 = leader(&mut c, 3);
    let fifth = follower(&mut c, 3);
    assert_eq!(c.complete(f3), Ok(1));

    let untouched = Notification {
        waiter: WaiterId::from_raw(99),
        flight: FlightId::from_raw(98),
        key: 97,
        outcome: Outcome::Completed,
    };
    let mut out = [untouched; 3];
    assert_eq!(c.drain(&mut out), 2);
    assert_eq!((out[0].waiter, out[1].waiter), (third, fifth));
    // Untouched past the count.
    assert_eq!(out[2], untouched);
    let before = out;
    assert_eq!(c.drain(&mut out), 0);
    assert_eq!(out, before);
}

#[test]
fn defaults_are_placeholders_the_coalescer_never_emits() {
    let blank = Notification::default();
    assert_eq!(
        (blank.waiter.raw(), blank.flight.raw(), blank.key),
        (0, 0, 0)
    );
    assert_eq!(blank.outcome, Outcome::default());
    assert_eq!(Outcome::default(), Outcome::Abandoned);
    assert_eq!(FlightId::default(), FlightId::from_raw(0));
    assert_eq!(WaiterId::default(), WaiterId::from_raw(0));

    // Raw zero is never issued, in any scope, so a placeholder names nothing.
    let mut c = Coalescer::<1, 1>::new();
    let flight = leader(&mut c, 0);
    let waiter = follower(&mut c, 0);
    assert_ne!(flight, FlightId::default());
    assert_ne!(waiter, WaiterId::default());
    let before = c.clone();
    assert_eq!(c.complete(FlightId::default()), Err(Refusal::Stale));
    assert_eq!(c.abandon(FlightId::default()), Err(Refusal::Stale));
    assert_eq!(c.cancel(WaiterId::default()), Err(Refusal::Stale));
    assert_eq!(c.followers(FlightId::default()), None);
    assert_eq!(c, before);
    assert_eq!(c.abandon(flight), Ok(1));
    assert_ne!(c.poll(), Some(blank));
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
    let mut c = Coalescer::<2, 2>::with_scope(u16::MAX);
    c.issued = SEQUENCE_MAX - 2;
    let flight = leader(&mut c, 1);
    let waiter = follower(&mut c, 1);
    assert_eq!((flight.raw(), waiter.raw()), (u64::MAX - 1, u64::MAX));
    let before = c.clone();
    assert_eq!(c.join(1), Err(Refusal::Exhausted));
    assert_eq!(c.join(2), Err(Refusal::Exhausted));
    assert_eq!(c, before);
    // What is registered still settles and drains.
    assert_eq!(c.complete(flight), Ok(1));
    assert_eq!(c.poll().map(|n| n.waiter), Some(waiter));
    assert_eq!(c.join(1), Err(Refusal::Exhausted));
}

#[test]
fn refusals_describe_themselves() {
    use std::string::ToString;
    for refusal in [
        Refusal::FlightsFull,
        Refusal::WaitersFull,
        Refusal::Exhausted,
        Refusal::Stale,
    ] {
        assert_ne!(refusal.to_string(), "");
    }
}

mod model;

#[test]
fn scopes_keep_the_ids_of_separate_coalescers_apart() {
    let mut near = Coalescer::<2, 2>::with_scope(1);
    let mut far = Coalescer::<2, 2>::with_scope(2);
    let same = Coalescer::<2, 2>::new();
    assert_eq!((near.scope(), far.scope(), same.scope()), (1, 2, 0));

    // The same joins on both: without scopes these ids would be equal.
    let (near_flight, far_flight) = (leader(&mut near, 7), leader(&mut far, 7));
    let (near_waiter, far_waiter) = (follower(&mut near, 7), follower(&mut far, 7));
    assert_eq!((near_flight.scope(), near_waiter.scope()), (1, 1));
    assert_eq!((far_flight.scope(), far_waiter.scope()), (2, 2));
    assert_eq!(near_flight.raw(), scoped(1, 1));
    assert_eq!(far_waiter.raw(), scoped(2, 2));
    assert_eq!(scope_of(far_waiter.raw()), 2);
    assert_ne!(near_flight, far_flight);
    assert_ne!(near_waiter, far_waiter);

    // A foreign id names nothing here, and the refusal changes nothing.
    let before = near.clone();
    assert_eq!(near.cancel(far_waiter), Err(Refusal::Stale));
    assert_eq!(near.complete(far_flight), Err(Refusal::Stale));
    assert_eq!(near.abandon(far_flight), Err(Refusal::Stale));
    assert_eq!(near.followers(far_flight), None);
    assert_eq!(near, before);
    assert_eq!(near.cancel(near_waiter), Ok(()));
    assert_eq!(far.complete(far_flight), Ok(1));

    // Scope zero keeps the plain counter ids of `new`.
    let mut plain = same;
    assert_eq!(leader(&mut plain, 7).raw(), 1);
}
