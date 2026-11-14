//! Intrusive singly linked chains of waiter slots: a flight's followers and
//! the ready queue.

use crate::{Outcome, Waiter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chain {
    pub head: Option<usize>,
    pub tail: Option<usize>,
    pub len: usize,
}

impl Chain {
    pub const EMPTY: Self = Self {
        head: None,
        tail: None,
        len: 0,
    };
}

pub fn free_slot<T>(slots: &[Option<T>]) -> Option<usize> {
    slots.iter().position(Option::is_none)
}

/// The chains only ever link slots that hold a waiter; the helpers below
/// change nothing for any other slot.
const CHAINED: &str = "a chained slot holds a waiter";

/// Change the waiter in `slot`; nothing when the slot holds none.
fn update(waiters: &mut [Option<Waiter>], slot: usize, change: impl FnOnce(&mut Waiter)) {
    match waiters.get_mut(slot).and_then(Option::as_mut) {
        Some(waiter) => change(waiter),
        None => debug_assert!(false, "{CHAINED}"),
    }
}

pub fn set_next(waiters: &mut [Option<Waiter>], slot: usize, next: Option<usize>) {
    update(waiters, slot, |waiter| waiter.next = next);
}

pub fn set_outcome(waiters: &mut [Option<Waiter>], slot: usize, outcome: Outcome) {
    update(waiters, slot, |waiter| waiter.outcome = Some(outcome));
}

/// Whether every slot `chain` links holds a waiter, its `len` counts them and
/// its `tail` is the last: what the mutating helpers above rely on. Walks at
/// most `len` links, so a cycle reads as broken rather than spinning.
pub fn is_whole(waiters: &[Option<Waiter>], chain: Chain) -> bool {
    let mut last = None;
    let mut cursor = chain.head;
    for _ in 0..chain.len {
        let Some(waiter) = cursor.and_then(|slot| waiters.get(slot)?.as_ref()) else {
            return false;
        };
        last = cursor;
        cursor = waiter.next;
    }
    cursor.is_none() && last == chain.tail
}

pub fn next_of(waiters: &[Option<Waiter>], slot: usize) -> Option<usize> {
    let waiter = waiters.get(slot).and_then(Option::as_ref);
    debug_assert!(waiter.is_some(), "{CHAINED}");
    waiter?.next
}

pub fn push_back(waiters: &mut [Option<Waiter>], chain: &mut Chain, slot: usize) {
    set_next(waiters, slot, None);
    match chain.tail {
        Some(tail) => set_next(waiters, tail, Some(slot)),
        None => chain.head = Some(slot),
    }
    chain.tail = Some(slot);
    chain.len += 1;
}

pub fn pop_front(waiters: &[Option<Waiter>], chain: &mut Chain) -> Option<usize> {
    let head = chain.head?;
    chain.head = next_of(waiters, head);
    if chain.head.is_none() {
        chain.tail = None;
    }
    chain.len -= 1;
    Some(head)
}

/// Splice all of `tail` behind `chain`, keeping both orders.
pub fn append(waiters: &mut [Option<Waiter>], chain: &mut Chain, tail: Chain) {
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
pub fn unlink(waiters: &mut [Option<Waiter>], chain: &mut Chain, slot: usize) -> bool {
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
