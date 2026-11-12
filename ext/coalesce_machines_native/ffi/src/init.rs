//! Private native entry point.
use crate::{Coalescer, FLIGHT_CAPACITY, WAITER_CAPACITY};
use magnus::{Error, Ruby, function, method, prelude::*};

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("CoalesceMachines")?;
    let class = module.define_class("Coalescer", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.const_set("FLIGHT_CAPACITY", FLIGHT_CAPACITY)?;
    class.const_set("WAITER_CAPACITY", WAITER_CAPACITY)?;
    class.define_singleton_method("new", function!(Coalescer::new, 0))?;
    class.define_method("join", method!(Coalescer::join, 1))?;
    class.define_method("complete", method!(Coalescer::complete, 1))?;
    class.define_method("abandon", method!(Coalescer::abandon, 1))?;
    class.define_method("cancel", method!(Coalescer::cancel, 1))?;
    class.define_method("drain", method!(Coalescer::drain, 1))?;
    class.define_method("flight", method!(Coalescer::flight, 1))?;
    class.define_method("followers", method!(Coalescer::followers, 1))?;
    class.define_method("in_flight", method!(Coalescer::in_flight, 0))?;
    class.define_method("waiting", method!(Coalescer::waiting, 0))?;
    class.define_method("pending", method!(Coalescer::pending, 0))?;
    class.define_method("flight_capacity", function!(Coalescer::flight_capacity, 0))?;
    class.define_method("waiter_capacity", function!(Coalescer::waiter_capacity, 0))?;
    Ok(())
}
