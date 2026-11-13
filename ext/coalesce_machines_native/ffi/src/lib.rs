//! Thin binding to the bounded in-flight coalescer.
mod init;

use std::cell::RefCell;

use coalesce_machines::{
    Coalescer as Core, FlightId, Joined, Notification, Outcome, Refused, WaiterId,
};
use magnus::{
    Error, Integer, RArray, Ruby, TryConvert, Value,
    prelude::*,
    scan_args::{get_kwargs, scan_args},
    typed_data::Obj,
};

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

/// The optional `scope:` keyword of a constructor; zero when omitted.
fn scope_keyword(ruby: &Ruby, args: &[Value]) -> Result<u16, Error> {
    let args = scan_args::<(), (), (), (), _, ()>(args)?;
    let keywords = get_kwargs::<_, (), (Option<Value>,), ()>(args.keywords, &[], &["scope"])?;
    let Some(value) = keywords.optional.0 else {
        return Ok(0);
    };
    if Integer::from_value(value).is_none() {
        return Err(Error::new(
            ruby.exception_type_error(),
            "scope must be an Integer",
        ));
    }
    u16::try_convert(value)
        .map_err(|_| Error::new(ruby.exception_range_error(), "scope must be in 0..=65535"))
}

fn refusal(ruby: &Ruby, refused: Refused) -> Value {
    ruby.to_symbol(match refused {
        Refused::FlightsFull => "flights_full",
        Refused::WaitersFull => "waiters_full",
        Refused::Exhausted => "exhausted",
        _ => "stale",
    })
    .as_value()
}

fn data(ruby: &Ruby, name: &str) -> Result<Value, Error> {
    ruby.define_module("CoalesceMachines")?.const_get(name)
}

#[magnus::wrap(class = "CoalesceMachines::Coalescer", free_immediately, size)]
struct Coalescer {
    core: RefCell<Core<FLIGHT_CAPACITY, WAITER_CAPACITY>>,
}

impl Coalescer {
    fn new(ruby: &Ruby, args: &[Value]) -> Result<Self, Error> {
        Ok(Self {
            core: RefCell::new(Core::with_scope(scope_keyword(ruby, args)?)),
        })
    }

    fn scope(&self) -> u16 {
        self.core.borrow().scope()
    }

    fn join(ruby: &Ruby, this: Obj<Self>, key: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let joined = this.core.borrow_mut().join(key.0);
        match joined {
            Ok(Joined::Leader(flight)) => {
                data(ruby, "Leader")?.funcall("new", (flight.raw(), key.0))
            }
            Ok(Joined::Follower { waiter, flight }) => {
                data(ruby, "Follower")?.funcall("new", (waiter.raw(), flight.raw(), key.0))
            }
            Err(refused) => Ok(refusal(ruby, refused)),
        }
    }

    fn settled(ruby: &Ruby, outcome: Result<usize, Refused>) -> Value {
        match outcome {
            Ok(notified) => ruby.integer_from_u64(notified as u64).as_value(),
            Err(refused) => refusal(ruby, refused),
        }
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, flight_id: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this
            .core
            .borrow_mut()
            .complete(FlightId::from_raw(flight_id.0));
        Ok(Self::settled(ruby, outcome))
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, flight_id: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this
            .core
            .borrow_mut()
            .abandon(FlightId::from_raw(flight_id.0));
        Ok(Self::settled(ruby, outcome))
    }

    fn cancel(ruby: &Ruby, this: Obj<Self>, waiter_id: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this
            .core
            .borrow_mut()
            .cancel(WaiterId::from_raw(waiter_id.0));
        Ok(match outcome {
            Ok(()) => ruby.to_symbol("cancelled").as_value(),
            Err(refused) => refusal(ruby, refused),
        })
    }

    fn drain(ruby: &Ruby, this: Obj<Self>, max: Number) -> Result<RArray, Error> {
        this.check_frozen()?;
        // Resolve the class before taking anything, so a failure loses nothing.
        let class = data(ruby, "Notification")?;
        let room = usize::try_from(max.0).map_or(WAITER_CAPACITY, |max| max.min(WAITER_CAPACITY));
        let mut buffer: [Option<Notification>; WAITER_CAPACITY] = [None; WAITER_CAPACITY];
        let written = this.core.borrow_mut().drain(&mut buffer[..room]);
        let notifications = ruby.ary_new_capa(written);
        for note in buffer.iter().take(written).flatten() {
            let outcome = ruby.to_symbol(match note.outcome {
                Outcome::Completed => "completed",
                Outcome::Abandoned => "abandoned",
            });
            let value: Value = class.funcall(
                "new",
                (note.waiter.raw(), note.flight.raw(), note.key, outcome),
            )?;
            notifications.push(value)?;
        }
        Ok(notifications)
    }

    fn flight(&self, key: Number) -> Option<u64> {
        self.core.borrow().flight(key.0).map(FlightId::raw)
    }

    fn followers(&self, flight_id: Number) -> Option<usize> {
        self.core
            .borrow()
            .followers(FlightId::from_raw(flight_id.0))
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
