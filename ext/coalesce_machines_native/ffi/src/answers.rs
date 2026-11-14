//! The gem's `Data` classes, defined here and built here from what the core
//! answered. Building runs no Ruby code (no `new`, no `initialize`), and the
//! mutating calls build on a copy of the coalescer kept only once the answer
//! exists, so nothing the binding does can lose the work. The classes are
//! defined once, at init, and pinned.
use coalesce_machines::{Joined, Notification, Refusal};
use magnus::{Error, RModule, RStruct, Ruby, Value, prelude::*};
use matryona_machines::ruby::{Class, Registry, Symbols};

use crate::values::Outcomes;

/// `CoalesceMachines::Leader`, `Follower` and `Notification`.
pub struct Classes {
    leader: Class,
    follower: Class,
    notification: Class,
}

static CLASSES: Registry<Classes> = Registry::new("coalesce_machines");

/// Define the three `Data` classes under `module` and pin them as defined,
/// with no lookup back by name; Ruby adds their validating `initialize` when
/// it reopens them.
pub fn define(ruby: &Ruby, module: RModule) -> Result<(), Error> {
    let leader = ruby.define_data(None, ("flight",))?;
    let follower = ruby.define_data(None, ("waiter", "flight"))?;
    let notification = ruby.define_data(None, ("waiter", "flight", "key", "outcome"))?;
    module.const_set("Leader", leader)?;
    module.const_set("Follower", follower)?;
    module.const_set("Notification", notification)?;
    let classes = Classes {
        leader: Class::pin(ruby, leader),
        follower: Class::pin(ruby, follower),
        notification: Class::pin(ruby, notification),
    };
    CLASSES.set(ruby, classes).map(drop)
}

/// The classes defined at init.
pub fn classes(ruby: &Ruby) -> Result<&'static Classes, Error> {
    CLASSES.get(ruby)
}

/// A frozen instance of `class` holding `members`, in order. Allocation and
/// in-range member writes on a fresh instance do not raise.
fn instance<const N: usize>(
    ruby: &Ruby,
    class: Class,
    members: [Value; N],
) -> Result<Value, Error> {
    let instance = RStruct::from_value(class.get(ruby).obj_alloc()?).ok_or_else(|| {
        Error::new(
            ruby.exception_type_error(),
            "a Data class allocated a non-Data",
        )
    })?;
    for (index, member) in members.into_iter().enumerate() {
        instance.aset(index, member)?;
    }
    instance.freeze();
    Ok(instance.as_value())
}

fn integer(ruby: &Ruby, raw: u64) -> Value {
    ruby.integer_from_u64(raw).as_value()
}

impl Classes {
    /// A `Leader` or a `Follower`.
    pub fn joined(&self, ruby: &Ruby, joined: Joined) -> Result<Value, Error> {
        match joined {
            Joined::Leader(flight) => instance(ruby, self.leader, [integer(ruby, flight.raw())]),
            Joined::Follower { waiter, flight } => {
                let members = [integer(ruby, waiter.raw()), integer(ruby, flight.raw())];
                instance(ruby, self.follower, members)
            }
        }
    }

    /// A `Notification`.
    pub fn notification(&self, ruby: &Ruby, note: &Notification) -> Result<Value, Error> {
        let members = [
            integer(ruby, note.waiter().raw()),
            integer(ruby, note.flight().raw()),
            integer(ruby, note.key()),
            Outcomes::symbol(ruby, note.outcome()).as_value(),
        ];
        instance(ruby, self.notification, members)
    }
}

matryona_machines::symbols! {
    /// A refusal as its Symbol: the snake case of the core variant.
    Refusals for Refusal as "refusal" {
        FlightsFull => "flights_full",
        WaitersFull => "waiters_full",
        Exhausted => "exhausted",
        Stale => "stale",
    }
}

/// Ruby symbol of a refusal.
pub fn refusal(ruby: &Ruby, refused: Refusal) -> Value {
    Refusals::symbol(ruby, refused).as_value()
}

/// The honoured answer built by `honoured`, or the refusal's symbol.
pub fn answer<T>(
    ruby: &Ruby,
    result: Result<T, Refusal>,
    honoured: impl FnOnce(T) -> Result<Value, Error>,
) -> Result<Value, Error> {
    result.map_or_else(|refused| Ok(refusal(ruby, refused)), honoured)
}
