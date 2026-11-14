//! One sequence per coalescer: flights, waiters and reserved ids never meet,
//! and a restart resumes the sequence instead of starting it again.
use matryona_machines::testing::refuses;
use std::collections::BTreeSet;

use periscope_machines::{SEQUENCE_MAX, scoped};

use super::model::Draws;
use super::*;

fn reserved<const F: usize, const W: usize>(c: &mut Coalescer<F, W>) -> u64 {
    match c.reserve() {
        Ok(id) => id,
        Err(refusal) => panic!("expected a reserved id, got {refusal:?}"),
    }
}

#[test]
fn reserved_ids_interleave_with_flight_and_waiter_ids() {
    let mut c = Coalescer::<2, 2>::with_scope(3);
    let first = reserved(&mut c);
    let flight = leader(&mut c, 7);
    let second = reserved(&mut c);
    let waiter = follower(&mut c, 7);
    let third = reserved(&mut c);
    assert_eq!(
        [first, flight.raw(), second, waiter.raw(), third],
        [1, 2, 3, 4, 5].map(|sequence| scoped(3, sequence))
    );
    assert_eq!((c.issued(), c.remaining()), (5, SEQUENCE_MAX - 5));
    // Reserving touched neither table.
    assert_eq!((c.in_flight(), c.waiting(), c.pending()), (1, 1, 0));
    assert_eq!(c.followers(flight), Some(1));
}

#[test]
fn a_reserved_id_is_refused_by_every_method_forever() {
    let mut c = Coalescer::<2, 2>::with_scope(3);
    let flight = leader(&mut c, 7);
    let mine = reserved(&mut c);
    let waiter = follower(&mut c, 7);
    // Owned, because it was issued here; live nowhere in the coalescer.
    assert!(c.owns(mine));
    names_nothing(&c, mine);
    // Still so once the slots around it were settled, drained and reused.
    assert_eq!(c.complete(flight), Ok(1));
    assert_eq!(c.poll().map(Notification::waiter), Some(waiter));
    let again = leader(&mut c, 7);
    follower(&mut c, 7);
    assert_ne!(again.raw(), mine);
    names_nothing(&c, mine);
}

#[test]
fn owns_proves_origin_for_every_kind_and_nothing_else() {
    let mut c = Coalescer::<1, 1>::with_scope(4);
    let (flight, waiter) = (leader(&mut c, 7), follower(&mut c, 7));
    let mine = reserved(&mut c);
    assert_eq!(c.cancel(waiter), Ok(()));
    assert_eq!(c.abandon(flight), Ok(0));
    // Origin, not liveness: all three were issued here.
    for raw in [flight.raw(), waiter.raw(), mine] {
        assert!(c.owns(raw));
    }
    // Not issued yet, the scope's zero, and the same sequence in another scope.
    for raw in [scoped(4, 4), scoped(4, 0), scoped(5, 1), 1] {
        assert!(!c.owns(raw));
    }
}

#[test]
fn no_id_of_any_kind_repeats_across_join_and_reserve() {
    for seed in 0..16 {
        let scope = scope_for(seed);
        let mut draws = Draws::new(seed);
        let mut c = Coalescer::<3, 4>::with_scope(scope);
        let mut seen = BTreeSet::new();
        let mut kept = Vec::new();
        let mut flights = Vec::new();
        for _ in 0..1_200 {
            let fresh = match draws.below(6) {
                0 | 1 => match c.join(draws.below(4)) {
                    Ok(Joined::Leader(flight)) => {
                        flights.push(flight);
                        Some(flight.raw())
                    }
                    Ok(Joined::Follower { waiter, .. }) => Some(waiter.raw()),
                    Err(refusal) => {
                        assert_matches!(refusal, Refusal::FlightsFull | Refusal::WaitersFull);
                        None
                    }
                },
                2 | 3 => {
                    let id = reserved(&mut c);
                    kept.push(id);
                    Some(id)
                }
                4 => {
                    if let Some(flight) = flights.pop() {
                        assert!(c.complete(flight).is_ok());
                    }
                    None
                }
                _ => {
                    c.poll();
                    None
                }
            };
            if let Some(id) = fresh {
                assert!(seen.insert(id), "id {id:#x} was issued twice");
                assert_eq!(u64::try_from(seen.len()), Ok(c.issued()));
            }
            // Whatever is live right now, no reserved id names it.
            if let Some(id) = kept.get(usize::try_from(draws.below(8)).unwrap_or(0)) {
                names_nothing(&c, *id);
            }
        }
        assert!(kept.len() > 100 && flights.len() + kept.len() < seen.len());
    }
}

#[test]
fn a_resumed_coalescer_continues_the_sequence() {
    let mut first = Coalescer::<2, 2>::with_scope(6);
    let flight = leader(&mut first, 7);
    let waiter = follower(&mut first, 7);
    let mine = reserved(&mut first);
    assert_eq!(first.issued(), 3);

    // The restart: nothing in flight, the saved count, the same scope.
    let Some(mut second) = Coalescer::<2, 2>::resumed(6, first.issued()) else {
        panic!("three ids issued is within the sequence");
    };
    assert_eq!((second.scope(), second.issued()), (6, 3));
    assert_eq!(
        (second.in_flight(), second.waiting(), second.pending()),
        (0, 0, 0)
    );
    // What outlived the restart was issued, and is live nowhere.
    for raw in [flight.raw(), waiter.raw(), mine] {
        assert!(second.owns(raw));
        names_nothing(&second, raw);
    }
    assert_eq!(leader(&mut second, 7).raw(), scoped(6, 4));
    assert_eq!(follower(&mut second, 7).raw(), scoped(6, 5));
    assert_eq!(second.reserve(), Ok(scoped(6, 6)));
    // An old id still names nothing, though the same key is in flight again.
    names_nothing(&second, flight.raw());
    names_nothing(&second, waiter.raw());

    // Resuming from nothing is a new coalescer.
    assert_eq!(
        Coalescer::<2, 2>::resumed(6, 0),
        Some(Coalescer::with_scope(6))
    );
    assert_eq!(Coalescer::<2, 2>::resumed(0, 0), Some(Coalescer::new()));
}

#[test]
fn resumed_answers_none_exactly_when_the_issuer_does() {
    for issued in [
        0,
        1,
        SEQUENCE_MAX - 1,
        SEQUENCE_MAX,
        SEQUENCE_MAX + 1,
        u64::MAX,
    ] {
        for scope in [0, 1, u16::MAX] {
            let coalescer = Coalescer::<1, 1>::resumed(scope, issued);
            let issuer = periscope_machines::Issuer::resumed(scope, issued);
            assert_eq!(coalescer.is_none(), issuer.is_none());
            assert_eq!(coalescer.is_none(), issued > SEQUENCE_MAX);
            if let Some(c) = coalescer {
                assert_eq!((c.scope(), c.issued()), (scope, issued));
                assert_eq!(c.remaining(), SEQUENCE_MAX - issued);
            }
        }
    }
}

#[test]
fn the_last_id_is_reserved_then_the_sequence_is_spent() {
    let mut c: Coalescer<2, 2> = near_the_end(u16::MAX, 1);
    assert_eq!(c.remaining(), 1);
    assert_eq!(c.reserve(), Ok(u64::MAX));
    assert_eq!((c.remaining(), c.issued()), (0, SEQUENCE_MAX));
    refuses(&mut c, Refusal::Exhausted, Coalescer::reserve);
    refuses(&mut c, Refusal::Exhausted, |r| r.join(1));

    // Spent from the start: every kind is refused alike.
    let mut spent: Coalescer<2, 2> = near_the_end(2, 0);
    refuses(&mut spent, Refusal::Exhausted, Coalescer::reserve);
    refuses(&mut spent, Refusal::Exhausted, |r| r.join(1));
}
