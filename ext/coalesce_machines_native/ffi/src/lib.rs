//! Thin binding to the bounded in-flight coalescer.
mod init;

use std::cell::RefCell;

use coalesce_machines::{
    Coalescer as Core, FlightId, Joined, Notification, Outcome, Refusal, WaiterId, scope_of, scoped,
};
use magnus::{Error, Integer, RArray, Ruby, TryConvert, Value, prelude::*, typed_data::Obj};

const FLIGHT_CAPACITY: usize = 64;
const WAITER_CAPACITY: usize = 256;

#[derive(Clone, Copy)]
struct Number(u64);

impl TryConvert for Number {
    fn try_convert(value: Value) -> Result<Self, Error> {
        if Integer::from_value(value).is_none() {
            return Err(Error::new(
                Ruby::get_with(value).exception_type_error(),
                "expected an Integer (unsigned 64-bit)",
            ));
        }
        u64::try_convert(value).map(Self)
    }
}

/// A scope: an Integer in `0..=65535`.
#[derive(Clone, Copy)]
struct Scope(u16);

impl TryConvert for Scope {
    fn try_convert(value: Value) -> Result<Self, Error> {
        let ruby = Ruby::get_with(value);
        if Integer::from_value(value).is_none() {
            return Err(Error::new(
                ruby.exception_type_error(),
                "scope must be an Integer",
            ));
        }
        u16::try_convert(value)
            .map(Self)
            .map_err(|_| Error::new(ruby.exception_range_error(), "scope must be in 0..=65535"))
    }
}

/// Ruby symbol of a refusal: the snake case of the core variant.
fn refusal(ruby: &Ruby, refused: Refusal) -> Value {
    ruby.to_symbol(match refused {
        Refusal::FlightsFull => "flights_full",
        Refusal::WaitersFull => "waiters_full",
        Refusal::Exhausted => "exhausted",
        Refusal::Stale => "stale",
    })
    .as_value()
}

fn data(ruby: &Ruby, name: &str) -> Result<Value, Error> {
    ruby.define_module("CoalesceMachines")?.const_get(name)
}

fn notification(ruby: &Ruby, class: Value, note: Notification) -> Result<Value, Error> {
    let outcome = ruby.to_symbol(match note.outcome {
        Outcome::Completed => "completed",
        Outcome::Abandoned => "abandoned",
    });
    class.funcall(
        "new",
        (note.waiter.raw(), note.flight.raw(), note.key, outcome),
    )
}

/// `CoalesceMachines.scoped(scope, sequence)`: the raw id of that scope and sequence.
const fn scoped_id(scope: Scope, sequence: Number) -> u64 {
    scoped(scope.0, sequence.0)
}

/// `CoalesceMachines.scope_of(raw)`: the scope a raw id was issued under.
const fn scope_of_id(raw: Number) -> u16 {
    scope_of(raw.0)
}

#[magnus::wrap(class = "CoalesceMachines::Coalescer", free_immediately, size)]
struct Coalescer {
    core: RefCell<Core<FLIGHT_CAPACITY, WAITER_CAPACITY>>,
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

    fn scope(&self) -> u16 {
        self.core.borrow().scope()
    }

    fn join(ruby: &Ruby, this: Obj<Self>, key: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let joined = this.core.borrow_mut().join(key.0);
        match joined {
            Ok(Joined::Leader(flight)) => data(ruby, "Leader")?.funcall("new", (flight.raw(),)),
            Ok(Joined::Follower { waiter, flight }) => {
                data(ruby, "Follower")?.funcall("new", (waiter.raw(), flight.raw()))
            }
            Err(refused) => Ok(refusal(ruby, refused)),
        }
    }

    fn settled(ruby: &Ruby, outcome: Result<usize, Refusal>) -> Value {
        match outcome {
            Ok(notified) => ruby.integer_from_u64(notified as u64).as_value(),
            Err(refused) => refusal(ruby, refused),
        }
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, flight: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this
            .core
            .borrow_mut()
            .complete(FlightId::from_raw(flight.0));
        Ok(Self::settled(ruby, outcome))
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, flight: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().abandon(FlightId::from_raw(flight.0));
        Ok(Self::settled(ruby, outcome))
    }

    fn cancel(ruby: &Ruby, this: Obj<Self>, waiter: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().cancel(WaiterId::from_raw(waiter.0));
        Ok(match outcome {
            Ok(()) => ruby.to_symbol("cancelled").as_value(),
            Err(refused) => refusal(ruby, refused),
        })
    }

    fn poll(ruby: &Ruby, this: Obj<Self>) -> Result<Option<Value>, Error> {
        this.check_frozen()?;
        // Resolve the class before taking anything, so a failure loses nothing.
        let class = data(ruby, "Notification")?;
        let polled = this.core.borrow_mut().poll();
        polled
            .map(|note| notification(ruby, class, note))
            .transpose()
    }

    fn drain(ruby: &Ruby, this: Obj<Self>, max: Number) -> Result<RArray, Error> {
        this.check_frozen()?;
        // Resolve the class before taking anything, so a failure loses nothing.
        let class = data(ruby, "Notification")?;
        let room = usize::try_from(max.0).map_or(WAITER_CAPACITY, |max| max.min(WAITER_CAPACITY));
        let mut buffer = [Notification::default(); WAITER_CAPACITY];
        let written = this.core.borrow_mut().drain(&mut buffer[..room]);
        let notifications = ruby.ary_new_capa(written);
        for note in buffer.iter().take(written) {
            notifications.push(notification(ruby, class, *note)?)?;
        }
        Ok(notifications)
    }

    fn flight(&self, key: Number) -> Option<u64> {
        self.core.borrow().flight(key.0).map(FlightId::raw)
    }

    fn followers(&self, flight: Number) -> Option<usize> {
        self.core.borrow().followers(FlightId::from_raw(flight.0))
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
