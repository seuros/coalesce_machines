//! Scoped ids: what one coalescer issued names nothing in another.
use periscope_machines::{SEQUENCE_MAX, scope_of, scoped, sequence_of};

use super::*;

type Pair = Coalescer<2, 2>;

/// The review's sequence: identical calls on two instances, then each is
/// handed the other's ids through every method that takes one.
#[test]
fn regression_an_id_from_another_coalescer_names_nothing_here() {
    let mut near = Pair::with_scope(1);
    let mut far = Pair::with_scope(2);
    assert_eq!((near.scope(), far.scope(), Pair::new().scope()), (1, 2, 0));

    // The same joins on both: without scopes these ids would be equal.
    let (near_flight, far_flight) = (leader(&mut near, 7), leader(&mut far, 7));
    let (near_waiter, far_waiter) = (follower(&mut near, 7), follower(&mut far, 7));
    assert_eq!((near_flight.scope(), near_waiter.scope()), (1, 1));
    assert_eq!((far_flight.scope(), far_waiter.scope()), (2, 2));
    assert_eq!(
        sequence_of(near_flight.raw()),
        sequence_of(far_flight.raw())
    );
    assert_eq!(
        sequence_of(near_waiter.raw()),
        sequence_of(far_waiter.raw())
    );
    assert_ne!(near_flight, far_flight);
    assert_ne!(near_waiter, far_waiter);

    // Each refuses the other's ids, in both roles, and is left unchanged.
    for (here, flight, waiter) in [
        (&mut near, far_flight, far_waiter),
        (&mut far, near_flight, near_waiter),
    ] {
        let before = here.clone();
        assert_eq!(here.complete(flight), Err(Refusal::Stale));
        assert_eq!(here.abandon(flight), Err(Refusal::Stale));
        assert_eq!(here.cancel(waiter), Err(Refusal::Stale));
        assert_eq!(here.followers(flight), None);
        // A foreign id in the wrong role is no better.
        assert_eq!(
            here.cancel(WaiterId::from_raw(flight.raw())),
            Err(Refusal::Stale)
        );
        assert_eq!(
            here.complete(FlightId::from_raw(waiter.raw())),
            Err(Refusal::Stale)
        );
        assert_eq!(*here, before);
    }

    // Their own ids still work.
    assert_eq!(near.cancel(near_waiter), Ok(()));
    assert_eq!(far.complete(far_flight), Ok(1));
    assert_eq!(near.abandon(near_flight), Ok(0));
}

#[test]
fn coalescers_left_in_the_same_scope_still_alias() {
    // Scopes are the caller's job: nothing tells these two apart.
    let mut one = Pair::with_scope(5);
    let mut other = Pair::with_scope(5);
    let (flight, twin) = (leader(&mut one, 7), leader(&mut other, 8));
    assert_eq!(flight, twin);
    let waiter = follower(&mut one, 7);
    assert_eq!(waiter, follower(&mut other, 8));
    // An id from `one` settles and cancels the unrelated registrations in `other`.
    assert_eq!(other.cancel(waiter), Ok(()));
    assert_eq!(other.complete(flight), Ok(0));
    assert_eq!((one.in_flight(), one.waiting()), (1, 1));
}

#[test]
fn raw_values_are_the_published_layout() {
    // Scope zero is the plain counter it always was, shared by both roles.
    let mut plain = Pair::new();
    assert_eq!(leader(&mut plain, 7).raw(), 1);
    assert_eq!(follower(&mut plain, 7).raw(), 2);
    assert_eq!(leader(&mut plain, 8).raw(), 3);
    assert_eq!(follower(&mut plain, 8).raw(), 4);

    // A scope sits in the top 16 bits above the same counter.
    let mut top = Pair::with_scope(u16::MAX);
    let (flight, waiter) = (leader(&mut top, 7), follower(&mut top, 7));
    assert_eq!(flight.raw(), (0xFFFF << 48) | 1);
    assert_eq!(waiter.raw(), (0xFFFF << 48) | 2);
    assert_eq!(
        (flight.raw(), waiter.raw()),
        (scoped(u16::MAX, 1), scoped(u16::MAX, 2))
    );
    assert_eq!(
        (scope_of(flight.raw()), flight.scope()),
        (u16::MAX, u16::MAX)
    );
    let mut three = Pair::with_scope(3);
    assert_eq!(leader(&mut three, 7).raw(), 0x0003_0000_0000_0001);
}

#[test]
fn ids_of_this_scope_that_were_never_issued_are_stale() {
    let mut c = Pair::with_scope(4);
    let flight = leader(&mut c, 7);
    let waiter = follower(&mut c, 7);
    let before = c.clone();
    // Not issued yet, the scope's zero, and the far end of the sequence.
    for sequence in [0, 3, SEQUENCE_MAX] {
        let raw = scoped(4, sequence);
        assert_eq!(c.complete(FlightId::from_raw(raw)), Err(Refusal::Stale));
        assert_eq!(c.abandon(FlightId::from_raw(raw)), Err(Refusal::Stale));
        assert_eq!(c.cancel(WaiterId::from_raw(raw)), Err(Refusal::Stale));
        assert_eq!(c.followers(FlightId::from_raw(raw)), None);
    }
    assert_eq!(c, before);
    assert_eq!(c.cancel(waiter), Ok(()));
    assert_eq!(c.complete(flight), Ok(0));
}

#[test]
fn repeats_stay_stale_because_nothing_records_how_an_id_ended() {
    let mut c = Pair::with_scope(6);
    let flight = leader(&mut c, 7);
    let (gone, kept) = (follower(&mut c, 7), follower(&mut c, 7));
    assert_eq!(c.cancel(gone), Ok(()));
    assert_eq!(c.complete(flight), Ok(1));
    let before = c.clone();
    // Issued here, provably, but the coalescer no longer knows the ending.
    assert_eq!(c.cancel(gone), Err(Refusal::Stale));
    assert_eq!(c.complete(flight), Err(Refusal::Stale));
    assert_eq!(c.abandon(flight), Err(Refusal::Stale));
    assert_eq!(c, before);
    assert_eq!(c.poll().map(|note| note.waiter), Some(kept));
    assert_eq!(c.cancel(kept), Err(Refusal::Stale));
}

#[test]
fn a_refused_join_consumes_no_id() {
    let mut c = Coalescer::<1, 1>::with_scope(9);
    assert_eq!(leader(&mut c, 7).raw(), scoped(9, 1));
    assert_eq!(c.join(8), Err(Refusal::FlightsFull));
    assert_eq!(follower(&mut c, 7).raw(), scoped(9, 2));
    assert_eq!(c.join(7), Err(Refusal::WaitersFull));
    assert_eq!(c.join(8), Err(Refusal::FlightsFull));
    assert_eq!(c.poll(), None);
    assert_eq!(c.abandon(FlightId::from_raw(scoped(9, 1))), Ok(1));
    assert_eq!(leader(&mut c, 8).raw(), scoped(9, 3));
}

#[test]
fn a_spent_issuer_is_this_cores_exhausted() {
    assert_eq!(
        spent(periscope_machines::Refusal::Exhausted),
        Refusal::Exhausted
    );
}
