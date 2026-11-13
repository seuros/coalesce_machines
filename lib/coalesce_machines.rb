# frozen_string_literal: true

require_relative 'coalesce_machines/version'

module CoalesceMachines
  # The caller that must run the computation and settle `flight`, a flight id.
  Leader = Data.define(:flight)
  # A caller sharing `flight`; wake it when its `waiter` id is drained.
  Follower = Data.define(:waiter, :flight)
  # One follower to wake: its waiter id, the flight that settled, the key the
  # flight was for. `outcome` is :completed or :abandoned.
  Notification = Data.define(:waiter, :flight, :key, :outcome)
end

# Native is required: no algorithm fallback. It builds the Data values above,
# so they must exist first.
require 'coalesce_machines_native/coalesce_machines_native'
