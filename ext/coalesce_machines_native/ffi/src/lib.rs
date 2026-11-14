//! Thin binding to the bounded in-flight coalescer.
mod answers;
mod init;
mod values;

use std::cell::RefCell;

use coalesce_machines::{Coalescer as Core, FlightId, Notification, Outcome};
use magnus::{Error, RArray, Ruby, Value, prelude::*, typed_data::Obj};
use matryona_machines::ruby::{integer, keep, symbol};
use periscope_machines::scope_of;
use values::{flight, waiter};

use answers::answer;

const FLIGHT_CAPACITY: usize = 64;
/// Waiter slots, and so the most notifications ever queued: the largest
/// `drain(max)` there is.
const WAITER_CAPACITY: usize = 256;

/// How many notifications one `drain` may take: an Integer in
/// `0..=WAITER_CAPACITY`.
fn room(ruby: &Ruby, max: Value) -> Result<usize, Error> {
    Some(integer::<usize>(max, "max")?)
        .filter(|max| *max <= WAITER_CAPACITY)
        .ok_or_else(|| {
            Error::new(
                ruby.exception_range_error(),
                format!("max must be in 0..={WAITER_CAPACITY}"),
            )
        })
}

/// `CoalesceMachines.scope_of(id)`: the scope an id was issued under.
fn scope_of_id(raw: Value) -> Result<u16, Error> {
    integer(raw, "id").map(scope_of)
}

type CoreType = Core<FLIGHT_CAPACITY, WAITER_CAPACITY>;

#[magnus::wrap(class = "CoalesceMachines::Coalescer", free_immediately, size)]
struct Coalescer {
    core: RefCell<CoreType>,
}

impl Coalescer {
    const fn new() -> Self {
        Self {
            core: RefCell::new(Core::new()),
        }
    }

    fn with_scope(scope: Value) -> Result<Self, Error> {
        Ok(Self {
            core: RefCell::new(Core::with_scope(integer(scope, "scope")?)),
        })
    }

    /// Raises `RangeError` when `issued` is beyond the sequence (the core answers `None`).
    fn resumed(ruby: &Ruby, scope: Value, issued: Value) -> Result<Self, Error> {
        let (scope, issued) = (integer(scope, "scope")?, integer(issued, "issued")?);
        Core::resumed(scope, issued)
            .map(|core| Self {
                core: RefCell::new(core),
            })
            .ok_or_else(|| {
                Error::new(
                    ruby.exception_range_error(),
                    "issued is past the last sequence of a scope (2**48 - 1)",
                )
            })
    }

    fn scope(&self) -> u16 {
        self.core.borrow().scope()
    }

    fn issued(&self) -> u64 {
        self.core.borrow().issued()
    }

    fn remaining(&self) -> u64 {
        self.core.borrow().remaining()
    }

    fn owns(&self, id: Value) -> Result<bool, Error> {
        let id = integer(id, "id")?;
        Ok(self.core.borrow().owns(id))
    }

    fn reserve(ruby: &Ruby, this: Obj<Self>) -> Result<Value, Error> {
        this.check_frozen()?;
        let reserved = this.core.borrow_mut().reserve();
        answer(ruby, reserved, |id| {
            Ok(ruby.integer_from_u64(id).as_value())
        })
    }

    /// Frozen first, then the key, then the core; the answer is built from a
    /// copy kept only once it exists (see [`keep`]).
    fn join(ruby: &Ruby, this: Obj<Self>, key: Value) -> Result<Value, Error> {
        this.check_frozen()?;
        let key = integer(key, "key")?;
        let classes = answers::classes(ruby)?;
        keep(
            ruby,
            this,
            &this.core,
            |core| core.join(key),
            |joined| answer(ruby, joined, |role| classes.joined(ruby, role)),
        )
    }

    fn settle(
        ruby: &Ruby,
        this: Obj<Self>,
        flight: Value,
        ending: Outcome,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let flight = self::flight(flight)?;
        let mut core = this.core.borrow_mut();
        let settled = match ending {
            Outcome::Completed => core.complete(flight),
            Outcome::Abandoned => core.abandon(flight),
        };
        answer(ruby, settled, |notified| {
            Ok(ruby.integer_from_u64(notified as u64).as_value())
        })
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, flight: Value) -> Result<Value, Error> {
        Self::settle(ruby, this, flight, Outcome::Completed)
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, flight: Value) -> Result<Value, Error> {
        Self::settle(ruby, this, flight, Outcome::Abandoned)
    }

    fn cancel(ruby: &Ruby, this: Obj<Self>, waiter: Value) -> Result<Value, Error> {
        this.check_frozen()?;
        let waiter = self::waiter(waiter)?;
        let cancelled = this.core.borrow_mut().cancel(waiter);
        answer(ruby, cancelled, |()| {
            Ok(symbol(ruby, "cancelled").as_value())
        })
    }

    fn poll(ruby: &Ruby, this: Obj<Self>) -> Result<Option<Value>, Error> {
        this.check_frozen()?;
        let classes = answers::classes(ruby)?;
        keep(ruby, this, &this.core, CoreType::poll, |note| {
            note.map(|note| classes.notification(ruby, &note))
                .transpose()
        })
    }

    /// Raises `RangeError` for `max` above `WAITER_CAPACITY`, before taking
    /// anything; reads into a Rust buffer on the copy, then builds the values.
    fn drain(ruby: &Ruby, this: Obj<Self>, max: Value) -> Result<RArray, Error> {
        this.check_frozen()?;
        let room = room(ruby, max)?;
        let classes = answers::classes(ruby)?;
        keep(
            ruby,
            this,
            &this.core,
            |core| {
                let mut buffer = [Notification::default(); WAITER_CAPACITY];
                let written = core.drain(&mut buffer[..room]);
                (buffer, written)
            },
            |(buffer, written)| {
                let notifications = ruby.ary_new_capa(written);
                for note in buffer.iter().take(written) {
                    notifications.push(classes.notification(ruby, note)?)?;
                }
                Ok(notifications)
            },
        )
    }

    fn flight(&self, key: Value) -> Result<Option<u64>, Error> {
        let key = integer(key, "key")?;
        Ok(self.core.borrow().flight(key).map(FlightId::raw))
    }

    fn followers(&self, flight: Value) -> Result<Option<usize>, Error> {
        let flight = self::flight(flight)?;
        Ok(self.core.borrow().followers(flight))
    }

    fn in_flight(&self) -> usize {
        self.core.borrow().in_flight()
    }

    fn waiting(&self) -> usize {
        self.core.borrow().waiting()
    }

    fn pending(&self) -> usize {
        self.core.borrow().pending()
    }

    const fn flight_capacity() -> usize {
        FLIGHT_CAPACITY
    }

    const fn waiter_capacity() -> usize {
        WAITER_CAPACITY
    }
}
