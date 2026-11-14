//! Private validation hooks behind the gem's `Data` values, and the id
//! conversions every method shares. A value is valid or does not exist: the
//! hooks hold a hand-built value to what the core's constructors accept.
use coalesce_machines::{FlightId, Joined, Notification, Outcome, WaiterId};
use magnus::{Error, Ruby, Value};
use matryona_machines::ruby::{Symbols, integer};

const ISSUED: &str = "must be in 1..=2**64-1 with a positive sequence number (low 48 bits)";

/// An issued id of one kind: an Integer whose sequence number is not zero.
fn issued<T>(value: Value, kind: &str, from_raw: fn(u64) -> Option<T>) -> Result<T, Error> {
    from_raw(integer(value, kind)?).ok_or_else(|| {
        Error::new(
            Ruby::get_with(value).exception_range_error(),
            format!("{kind} {ISSUED}"),
        )
    })
}

/// A flight id.
pub fn flight(value: Value) -> Result<FlightId, Error> {
    issued(value, "flight", FlightId::from_raw)
}

/// A waiter id.
pub fn waiter(value: Value) -> Result<WaiterId, Error> {
    issued(value, "waiter", WaiterId::from_raw)
}

matryona_machines::symbols! {
    /// How a flight settled, as the Symbol a `Notification#outcome` holds.
    pub Outcomes for Outcome as "outcome" {
        Completed => "completed",
        Abandoned => "abandoned",
    }
}

fn unrelated(ruby: &Ruby) -> Error {
    Error::new(
        ruby.exception_range_error(),
        "waiter must be of the flight's scope and issued after it",
    )
}

/// `Leader.new`: converting the id is the whole check.
pub fn leader(flight: Value) -> Result<u64, Error> {
    self::flight(flight).map(FlightId::raw)
}

/// `Follower.new`: the core's `Joined::follower`.
pub fn follower(ruby: &Ruby, waiter: Value, flight: Value) -> Result<bool, Error> {
    let (waiter, flight) = (self::waiter(waiter)?, self::flight(flight)?);
    Joined::follower(waiter, flight)
        .map(|_| true)
        .ok_or_else(|| unrelated(ruby))
}

/// `Notification.new`: the core's `Notification::new`.
pub fn notification(
    ruby: &Ruby,
    waiter: Value,
    flight: Value,
    key: Value,
    ending: Value,
) -> Result<bool, Error> {
    let (waiter, flight) = (self::waiter(waiter)?, self::flight(flight)?);
    let key = integer(key, "key")?;
    let ending = Outcomes::parse(ruby, ending)?;
    Notification::new(waiter, flight, key, ending)
        .map(|_| true)
        .ok_or_else(|| unrelated(ruby))
}
