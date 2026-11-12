# frozen_string_literal: true

require_relative 'coalesce_machines/version'

module CoalesceMachines
  # The caller that must run the computation and settle `flight_id`.
  Leader = Data.define(:flight_id, :key)
  # A caller sharing `flight_id`; wake it when `waiter_id` is drained.
  Follower = Data.define(:waiter_id, :flight_id, :key)
  # One follower to wake. `outcome` is :completed or :abandoned.
  Notification = Data.define(:waiter_id, :flight_id, :key, :outcome)
end

# Native is required: no algorithm fallback. It builds the Data values above,
# so they must exist first.
require 'coalesce_machines_native/coalesce_machines_native'
