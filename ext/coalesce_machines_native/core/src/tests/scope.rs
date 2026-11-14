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
        // In both roles: a foreign id in the wrong role is no better.
        names_nothing(here, flight.raw());
        names_nothing(here, waiter.raw());
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
    assert_eq!(
        (flight.raw(), waiter.raw()),
        ((0xFFFF << 48) | 1, (0xFFFF << 48) | 2)
    );
    assert_eq!(
        (scope_of(flight.raw()), flight.scope()),
        (u16::MAX, u16::MAX)
    );
}

#[test]
fn ids_of_this_scope_that_were_never_issued_are_stale() {
    let mut c = Pair::with_scope(4);
    let flight = leader(&mut c, 7);
    let waiter = follower(&mut c, 7);
    // Not issued yet, the scope's zero, and the far end of the sequence.
    for sequence in [0, 3, SEQUENCE_MAX] {
        names_nothing(&c, scoped(4, sequence));
    }
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
    assert_eq!(c.poll().map(Notification::waiter), Some(kept));
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
    assert_eq!(c.abandon(fid(scoped(9, 1))), Ok(1));
    assert_eq!(leader(&mut c, 8).raw(), scoped(9, 3));
}

#[test]
fn rebuilt_values_hold_only_what_a_coalescer_could_emit() {
    let mut c = Pair::with_scope(6);
    let flight = leader(&mut c, 7);
    let waiter = follower(&mut c, 7);
    assert_eq!(
        Joined::follower(waiter, flight),
        Some(Joined::Follower { waiter, flight })
    );
    let rebuilt = Notification::new(waiter, flight, 7, Outcome::Completed);
    assert_eq!(
        rebuilt.map(|n| (n.waiter(), n.flight(), n.key(), n.outcome())),
        Some((waiter, flight, 7, Outcome::Completed))
    );
    // Another scope, the waiter not after its flight, or a placeholder.
    let foreign = wid(scoped(5, 2));
    let early = wid(flight.raw());
    let wrong = [
        (foreign, flight),
        (early, flight),
        (WaiterId::default(), flight),
        (waiter, FlightId::default()),
        (WaiterId::default(), FlightId::default()),
    ];
    for (waiter, flight) in wrong {
        assert_eq!(Joined::follower(waiter, flight), None);
        assert_eq!(
            Notification::new(waiter, flight, 7, Outcome::Abandoned),
            None
        );
    }
}
