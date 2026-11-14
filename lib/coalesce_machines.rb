# frozen_string_literal: true

require_relative 'coalesce_machines/version'

# Native is required: no algorithm fallback. It defines the Data values below
# and builds the ones the coalescer answers with, running no Ruby code once
# the coalescer has changed.
require 'coalesce_machines_native/coalesce_machines_native'

# Every value is valid or does not exist: built by hand it is held to the rule
# the coalescer's own are, through the core's constructors. An id is an
# Integer in 1..2**64-1 whose sequence number (low 48 bits) is not zero.
# TypeError for the wrong type, RangeError for a value outside that.
module CoalesceMachines
  # The caller that must run the computation and settle `flight`, a flight id.
  class Leader
    def initialize(flight:)
      CoalesceMachines.__send__(:__leader, flight)
      super
    end
  end

  # A caller sharing `flight`; wake it when its `waiter` id is drained. The
  # core's `Joined::follower`: RangeError unless `waiter` is of the flight's
  # scope and was issued after it.
  class Follower
    def initialize(waiter:, flight:)
      CoalesceMachines.__send__(:__follower, waiter, flight)
      super
    end
  end

  # One follower to wake: its waiter id, the flight that settled, the key the
  # flight was for. `outcome` is :completed or :abandoned. The core's
  # `Notification::new`: the ids are held to the rule a Follower's are.
  class Notification
    def initialize(waiter:, flight:, key:, outcome:)
      CoalesceMachines.__send__(:__notification, waiter, flight, key, outcome)
      super
    end
  end

  private_class_method :__leader, :__follower, :__notification

  # The values are sealed. `initialize` is where each validates, and a
  # subclass could override it, or `allocate` build an instance that never ran
  # it: subclassing raises TypeError and `allocate` is gone. Sealing stops
  # accidents, not reflection (a named subclass's constant is bound before
  # `inherited` raises; `Marshal.load` skips `initialize`); the coalescer only
  # builds these values and reads none back, so a forged one reaches nothing.
  [Leader, Follower, Notification].each do |value|
    value.singleton_class.undef_method(:allocate)
    value.define_singleton_method(:inherited) { |_| raise TypeError, "#{value} cannot be subclassed" }
  end
end
