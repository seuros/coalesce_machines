//! Thin binding to the bounded in-flight coalescer.
mod answers;
mod init;
mod values;

use std::cell::RefCell;

use coalesce_machines::{Coalescer as Core, FlightId, Notification, Outcome};
use magnus::{Error, Integer, RArray, Ruby, TryConvert, Value, prelude::*, typed_data::Obj};
use periscope_machines::scope_of;
use values::{Flight, Waiter};

use answers::answer;

const FLIGHT_CAPACITY: usize = 64;
/// Waiter slots, and so the most notifications ever queued: the largest
/// `drain(max)` there is.
const WAITER_CAPACITY: usize = 256;

/// `TypeError` unless `value` is an Integer.
fn integer(value: Value, message: &'static str) -> Result<(), Error> {
    if Integer::from_value(value).is_some() {
        return Ok(());
    }
    Err(Error::new(
        Ruby::get_with(value).exception_type_error(),
        message,
    ))
}

#[derive(Clone, Copy)]
struct Number(u64);

impl TryConvert for Number {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value, "expected an Integer (unsigned 64-bit)")?;
        u64::try_convert(value).map(Self)
    }
}

/// A scope: an Integer in `0..=65535`.
#[derive(Clone, Copy)]
struct Scope(u16);

impl TryConvert for Scope {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value, "scope must be an Integer")?;
        u16::try_convert(value).map(Self).map_err(|_| {
            Error::new(
                Ruby::get_with(value).exception_range_error(),
                "scope must be in 0..=65535",
            )
        })
    }
}

/// How many notifications one `drain` may take: an Integer in
/// `0..=WAITER_CAPACITY`.
#[derive(Clone, Copy)]
struct Room(usize);

impl TryConvert for Room {
    fn try_convert(value: Value) -> Result<Self, Error> {
        let max = Number::try_convert(value)?.0;
        usize::try_from(max)
            .ok()
            .filter(|max| *max <= WAITER_CAPACITY)
            .map(Self)
            .ok_or_else(|| {
                Error::new(
                    Ruby::get_with(value).exception_range_error(),
                    format!("max must be in 0..={WAITER_CAPACITY}"),
                )
            })
    }
}

/// `CoalesceMachines.scope_of(id)`: the scope an id was issued under.
const fn scope_of_id(raw: Number) -> u16 {
    scope_of(raw.0)
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

    const fn with_scope(scope: Scope) -> Self {
        Self {
            core: RefCell::new(Core::with_scope(scope.0)),
        }
    }

    /// Raises `RangeError` when `issued` is beyond the sequence (the core answers `None`).
    fn resumed(ruby: &Ruby, scope: Scope, issued: Number) -> Result<Self, Error> {
        Core::resumed(scope.0, issued.0)
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

    fn owns(&self, id: Number) -> bool {
        self.core.borrow().owns(id.0)
    }

    fn reserve(ruby: &Ruby, this: Obj<Self>) -> Result<Value, Error> {
        this.check_frozen()?;
        let reserved = this.core.borrow_mut().reserve();
        answer(ruby, reserved, |id| {
            Ok(ruby.integer_from_u64(id).as_value())
        })
    }

    fn join(ruby: &Ruby, this: Obj<Self>, key: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let classes = answers::classes(ruby)?;
        Self::on_copy(ruby, this, |core| {
            answer(ruby, core.join(key.0), |role| classes.joined(ruby, role))
        })
    }

    fn settle(
        ruby: &Ruby,
        this: Obj<Self>,
        flight: Flight,
        ending: Outcome,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let mut core = this.core.borrow_mut();
        let settled = match ending {
            Outcome::Completed => core.complete(flight.0),
            Outcome::Abandoned => core.abandon(flight.0),
        };
        answer(ruby, settled, |notified| {
            Ok(ruby.integer_from_u64(notified as u64).as_value())
        })
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, flight: Flight) -> Result<Value, Error> {
        Self::settle(ruby, this, flight, Outcome::Completed)
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, flight: Flight) -> Result<Value, Error> {
        Self::settle(ruby, this, flight, Outcome::Abandoned)
    }

    fn cancel(ruby: &Ruby, this: Obj<Self>, waiter: Waiter) -> Result<Value, Error> {
        this.check_frozen()?;
        let cancelled = this.core.borrow_mut().cancel(waiter.0);
        answer(ruby, cancelled, |()| {
            Ok(ruby.to_symbol("cancelled").as_value())
        })
    }

    fn poll(ruby: &Ruby, this: Obj<Self>) -> Result<Option<Value>, Error> {
        this.check_frozen()?;
        let classes = answers::classes(ruby)?;
        Self::on_copy(ruby, this, |core| {
            core.poll()
                .map(|note| classes.notification(ruby, &note))
                .transpose()
        })
    }

    /// Raises `RangeError` for `max` above `WAITER_CAPACITY`, before taking
    /// anything; reads into a Rust buffer, then builds the values.
    fn drain(ruby: &Ruby, this: Obj<Self>, max: Room) -> Result<RArray, Error> {
        this.check_frozen()?;
        let classes = answers::classes(ruby)?;
        Self::on_copy(ruby, this, |core| {
            let mut buffer = [Notification::default(); WAITER_CAPACITY];
            let written = core.drain(&mut buffer[..max.0]);
            let notifications = ruby.ary_new_capa(written);
            for note in buffer.iter().take(written) {
                notifications.push(classes.notification(ruby, note)?)?;
            }
            Ok(notifications)
        })
    }

    /// Runs `act`, which builds a Ruby answer, on a copy of the coalescer and
    /// keeps the copy only once the answer exists. The answers are built
    /// without running Ruby code, so nothing should get in between; the
    /// receiver is still checked right before the store: frozen meanwhile
    /// raises `FrozenError`, changed meanwhile `RuntimeError`, and in both
    /// cases nothing is applied. No borrow is held while `act` runs.
    fn on_copy<R>(
        ruby: &Ruby,
        this: Obj<Self>,
        act: impl FnOnce(&mut CoreType) -> Result<R, Error>,
    ) -> Result<R, Error> {
        this.check_frozen()?;
        let before = this.core.borrow().clone();
        let mut after = before.clone();
        let answer = act(&mut after)?;
        this.check_frozen()?;
        let mut held = this.core.borrow_mut();
        if *held != before {
            return Err(Error::new(
                ruby.exception_runtime_error(),
                "the coalescer changed while the answer was built; the call was not applied",
            ));
        }
        *held = after;
        Ok(answer)
    }

    fn flight(&self, key: Number) -> Option<u64> {
        self.core.borrow().flight(key.0).map(FlightId::raw)
    }

    fn followers(&self, flight: Flight) -> Option<usize> {
        self.core.borrow().followers(flight.0)
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
