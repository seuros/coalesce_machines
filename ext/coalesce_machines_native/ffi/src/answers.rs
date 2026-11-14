//! The gem's `Data` classes, defined here and built here from what the core
//! answered. Building runs no Ruby code (no `new`, no `initialize`), and the
//! mutating calls build on a copy of the coalescer kept only once the answer
//! exists, so nothing the binding does can lose the work. The classes are
//! defined once, at init, and pinned.
use std::sync::OnceLock;

use coalesce_machines::{Joined, Notification, Refusal};
use magnus::value::Opaque;
use magnus::{Error, RClass, RModule, RStruct, Ruby, Value, prelude::*};

use crate::values;

/// `CoalesceMachines::Leader`, `Follower` and `Notification`.
pub struct Classes {
    leader: Opaque<RClass>,
    follower: Opaque<RClass>,
    notification: Opaque<RClass>,
}

static CLASSES: OnceLock<Classes> = OnceLock::new();

/// Define the three `Data` classes under `module`; Ruby adds their validating
/// `initialize` when it reopens them.
pub fn define(ruby: &Ruby, module: RModule) -> Result<(), Error> {
    let class = |name: &str, data: RClass| {
        // Held in a static, so pinned: `GC.compact` must not move it.
        ruby.gc_register_mark_object(data);
        module.const_set(name, data).map(|()| data.into())
    };
    let classes = Classes {
        leader: class("Leader", ruby.define_data(None, ("flight",))?)?,
        follower: class("Follower", ruby.define_data(None, ("waiter", "flight"))?)?,
        notification: class(
            "Notification",
            ruby.define_data(None, ("waiter", "flight", "key", "outcome"))?,
        )?,
    };
    CLASSES.set(classes).map_err(|_| {
        Error::new(
            ruby.exception_load_error(),
            "the coalesce_machines extension was initialised twice",
        )
    })
}

/// The classes defined at init.
pub fn classes(ruby: &Ruby) -> Result<&'static Classes, Error> {
    CLASSES.get().ok_or_else(|| {
        Error::new(
            ruby.exception_load_error(),
            "the coalesce_machines extension is not initialised",
        )
    })
}

/// A frozen instance of `class` holding `members`, in order. Allocation and
/// in-range member writes on a fresh instance do not raise.
fn instance<const N: usize>(
    ruby: &Ruby,
    class: Opaque<RClass>,
    members: [Value; N],
) -> Result<Value, Error> {
    let instance = RStruct::from_value(ruby.get_inner(class).obj_alloc()?).ok_or_else(|| {
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
            values::outcome(ruby, note.outcome()).as_value(),
        ];
        instance(ruby, self.notification, members)
    }
}

/// Ruby symbol of a refusal: the snake case of the core variant.
pub fn refusal(ruby: &Ruby, refused: Refusal) -> Value {
    ruby.to_symbol(match refused {
        Refusal::FlightsFull => "flights_full",
        Refusal::WaitersFull => "waiters_full",
        Refusal::Exhausted => "exhausted",
        Refusal::Stale => "stale",
    })
    .as_value()
}

/// The honoured answer built by `honoured`, or the refusal's symbol.
pub fn answer<T>(
    ruby: &Ruby,
    result: Result<T, Refusal>,
    honoured: impl FnOnce(T) -> Result<Value, Error>,
) -> Result<Value, Error> {
    result.map_or_else(|refused| Ok(refusal(ruby, refused)), honoured)
}
