//! Private validation hooks behind the gem's `Data` values, and the id
//! conversions every method shares. A value is valid or does not exist: the
//! hooks hold a hand-built value to what the core's constructors accept.
use coalesce_machines::{FlightId, Joined, Notification, Outcome, WaiterId};
use magnus::{Error, Ruby, Symbol, TryConvert, Value};

use crate::Number;

const ISSUED: &str = "must be in 1..=2**64-1 with a positive sequence number (low 48 bits)";

/// An issued id of one kind: an Integer whose sequence number is not zero.
fn issued<T>(value: Value, kind: &str, from_raw: fn(u64) -> Option<T>) -> Result<T, Error> {
    from_raw(Number::try_convert(value)?.0).ok_or_else(|| {
        Error::new(
            Ruby::get_with(value).exception_range_error(),
            format!("{kind} {ISSUED}"),
        )
    })
}

/// A flight id.
#[derive(Clone, Copy)]
pub struct Flight(pub FlightId);

impl TryConvert for Flight {
    fn try_convert(value: Value) -> Result<Self, Error> {
        issued(value, "flight", FlightId::from_raw).map(Self)
    }
}

/// A waiter id.
#[derive(Clone, Copy)]
pub struct Waiter(pub WaiterId);

impl TryConvert for Waiter {
    fn try_convert(value: Value) -> Result<Self, Error> {
        issued(value, "waiter", WaiterId::from_raw).map(Self)
    }
}

/// How a flight ended, as `:completed` or `:abandoned`.
#[derive(Clone, Copy)]
pub struct Ending(Outcome);

impl TryConvert for Ending {
    fn try_convert(value: Value) -> Result<Self, Error> {
        let ruby = Ruby::get_with(value);
        let Some(symbol) = Symbol::from_value(value) else {
            return Err(Error::new(
                ruby.exception_type_error(),
                "outcome must be a Symbol",
            ));
        };
        match symbol.name()?.as_ref() {
            "completed" => Ok(Self(Outcome::Completed)),
            "abandoned" => Ok(Self(Outcome::Abandoned)),
            _ => Err(Error::new(
                ruby.exception_range_error(),
                "outcome must be :completed or :abandoned",
            )),
        }
    }
}

/// The symbol of an outcome.
pub fn outcome(ruby: &Ruby, outcome: Outcome) -> Symbol {
    ruby.to_symbol(match outcome {
        Outcome::Completed => "completed",
        Outcome::Abandoned => "abandoned",
    })
}

fn unrelated(ruby: &Ruby) -> Error {
    Error::new(
        ruby.exception_range_error(),
        "waiter must be of the flight's scope and issued after it",
    )
}

/// `Leader.new`: converting the id is the whole check.
pub const fn leader(flight: Flight) -> u64 {
    flight.0.raw()
}

/// `Follower.new`: the core's `Joined::follower`.
pub fn follower(ruby: &Ruby, waiter: Waiter, flight: Flight) -> Result<bool, Error> {
    Joined::follower(waiter.0, flight.0)
        .map(|_| true)
        .ok_or_else(|| unrelated(ruby))
}

/// `Notification.new`: the core's `Notification::new`.
pub fn notification(
    ruby: &Ruby,
    waiter: Waiter,
    flight: Flight,
    key: Number,
    ending: Ending,
) -> Result<bool, Error> {
    Notification::new(waiter.0, flight.0, key.0, ending.0)
        .map(|_| true)
        .ok_or_else(|| unrelated(ruby))
}
