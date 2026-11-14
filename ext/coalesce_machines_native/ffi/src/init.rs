//! Private native entry point.
use crate::{Coalescer, FLIGHT_CAPACITY, WAITER_CAPACITY, answers, scope_of_id, values};
use magnus::{Error, Ruby, function, method, prelude::*};

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("CoalesceMachines")?;
    module.define_singleton_method("scope_of", function!(scope_of_id, 1))?;
    answers::define(ruby, module)?;
    // Private validation hooks of the Data values, made private in Ruby.
    module.define_singleton_method("__leader", function!(values::leader, 1))?;
    module.define_singleton_method("__follower", function!(values::follower, 2))?;
    module.define_singleton_method("__notification", function!(values::notification, 4))?;
    let class = module.define_class("Coalescer", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.const_set("FLIGHT_CAPACITY", FLIGHT_CAPACITY)?;
    class.const_set("WAITER_CAPACITY", WAITER_CAPACITY)?;
    class.define_singleton_method("new", function!(Coalescer::new, 0))?;
    class.define_singleton_method("with_scope", function!(Coalescer::with_scope, 1))?;
    class.define_singleton_method("resumed", function!(Coalescer::resumed, 2))?;
    class.define_method("scope", method!(Coalescer::scope, 0))?;
    class.define_method("issued", method!(Coalescer::issued, 0))?;
    class.define_method("remaining", method!(Coalescer::remaining, 0))?;
    class.define_method("owns?", method!(Coalescer::owns, 1))?;
    class.define_method("reserve", method!(Coalescer::reserve, 0))?;
    class.define_method("join", method!(Coalescer::join, 1))?;
    class.define_method("complete", method!(Coalescer::complete, 1))?;
    class.define_method("abandon", method!(Coalescer::abandon, 1))?;
    class.define_method("cancel", method!(Coalescer::cancel, 1))?;
    class.define_method("poll", method!(Coalescer::poll, 0))?;
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
