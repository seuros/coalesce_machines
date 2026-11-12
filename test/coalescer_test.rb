# frozen_string_literal: true
require 'minitest/autorun'
require 'coalesce_machines'

class CoalescerTest < Minitest::Test
  C = CoalesceMachines

  def setup
    @flights = C::Coalescer.new
  end

  def test_leader_then_followers_then_notifications
    leader = @flights.join(42)
    assert_instance_of C::Leader, leader
    assert_equal 42, leader.key
    assert_equal leader.flight_id, @flights.flight(42)

    a = @flights.join(42)
    b = @flights.join(42)
    assert_instance_of C::Follower, a
    assert_equal [leader.flight_id, 42], [a.flight_id, a.key]
    refute_equal a.waiter_id, b.waiter_id
    assert_equal [1, 2, 0], [@flights.in_flight, @flights.waiting, @flights.pending]
    assert_equal 2, @flights.followers(leader.flight_id)
    assert_empty @flights.drain(10)

    assert_equal 2, @flights.complete(leader.flight_id)
    assert_equal [0, 0, 2], [@flights.in_flight, @flights.waiting, @flights.pending]
    assert_nil @flights.flight(42)
    assert_nil @flights.followers(leader.flight_id)

    expected = [a, b].map { |f| C::Notification.new(f.waiter_id, leader.flight_id, 42, :completed) }
    assert_equal expected, @flights.drain(10)
    assert_equal 0, @flights.pending
  end

  def test_settled_keys_are_not_remembered_and_old_ids_are_stale
    first = @flights.join(7)
    assert_equal 0, @flights.complete(first.flight_id)
    assert_equal :stale, @flights.complete(first.flight_id)
    assert_equal :stale, @flights.abandon(first.flight_id)
    second = @flights.join(7)
    assert_instance_of C::Leader, second
    refute_equal first.flight_id, second.flight_id
    assert_equal :stale, @flights.complete(first.flight_id)
    assert_equal :stale, @flights.complete(0)
    assert_equal :stale, @flights.complete(2**64 - 1)
    assert_equal second.flight_id, @flights.flight(7)
  end

  def test_abandon_notifies_followers_who_rejoin
    leader = @flights.join(9)
    follower = @flights.join(9)
    assert_equal 1, @flights.abandon(leader.flight_id)
    note = @flights.drain(1).first
    assert_equal [follower.waiter_id, leader.flight_id, 9, :abandoned],
                 [note.waiter_id, note.flight_id, note.key, note.outcome]
    assert_instance_of C::Leader, @flights.join(9)
    assert_instance_of C::Follower, @flights.join(9)
  end

  def test_cancel_waiting_and_queued_followers
    leader = @flights.join(1)
    a, b, c = Array.new(3) { @flights.join(1) }
    assert_equal :cancelled, @flights.cancel(a.waiter_id)
    assert_equal :stale, @flights.cancel(a.waiter_id)
    assert_equal :stale, @flights.cancel(leader.flight_id)
    assert_equal 2, @flights.followers(leader.flight_id)
    assert_equal 2, @flights.complete(leader.flight_id)
    assert_equal :cancelled, @flights.cancel(b.waiter_id)
    assert_equal [c.waiter_id], @flights.drain(10).map(&:waiter_id)
    assert_equal :stale, @flights.cancel(c.waiter_id)
  end

  def test_last_follower_leaving_keeps_the_flight
    leader = @flights.join(1)
    follower = @flights.join(1)
    assert_equal :cancelled, @flights.cancel(follower.waiter_id)
    assert_equal 0, @flights.followers(leader.flight_id)
    assert_equal leader.flight_id, @flights.flight(1)
    assert_equal 0, @flights.complete(leader.flight_id)
  end

  def test_capacities_and_refusals
    assert_equal 64, C::Coalescer::FLIGHT_CAPACITY
    assert_equal 256, C::Coalescer::WAITER_CAPACITY
    assert_equal [64, 256], [@flights.flight_capacity, @flights.waiter_capacity]

    leaders = Array.new(64) { |key| @flights.join(key) }
    assert_equal :flights_full, @flights.join(64)
    assert_equal 64, @flights.in_flight
    followers = Array.new(256) { |i| @flights.join(i % 2) }
    assert_equal :waiters_full, @flights.join(0)
    assert_equal :flights_full, @flights.join(64)
    assert_equal 256, @flights.waiting

    # Undrained notifications still hold their waiter slots.
    assert_equal 128, @flights.complete(leaders[0].flight_id)
    assert_equal :waiters_full, @flights.join(1)
    assert_instance_of C::Leader, @flights.join(64)
    assert_equal :waiters_full, @flights.join(64)
    assert_equal followers[0].waiter_id, @flights.drain(1).first.waiter_id
    assert_instance_of C::Follower, @flights.join(1)
  end

  def test_drain_is_bounded_and_resumable
    one = @flights.join(1)
    two = @flights.join(2)
    waiters = Array.new(6) { |i| @flights.join(1 + (i % 2)) }
    assert_equal 3, @flights.abandon(two.flight_id)
    assert_equal 3, @flights.complete(one.flight_id)
    order = waiters.values_at(1, 3, 5, 0, 2, 4).map(&:waiter_id)

    assert_empty @flights.drain(0)
    assert_equal 6, @flights.pending
    first = @flights.drain(4)
    assert_equal order.first(4), first.map(&:waiter_id)
    assert_equal %i[abandoned abandoned abandoned completed], first.map(&:outcome)
    assert_equal 2, @flights.pending
    assert_equal order.last(2), @flights.drain(2**64 - 1).map(&:waiter_id)
    assert_empty @flights.drain(4)
  end

  def test_strict_inputs_before_mutation
    bad = [1.0, Float::INFINITY, Float::NAN, nil, '1', :one, Object.new]
    one_arg = %i[join complete abandon cancel drain flight followers]
    one_arg.each do |name|
      bad.each { |value| assert_raises(TypeError, "#{name}(#{value.inspect})") { @flights.public_send(name, value) } }
      [-1, 2**64].each { |n| assert_raises(RangeError, "#{name}(#{n})") { @flights.public_send(name, n) } }
    end
    assert_equal [0, 0, 0], [@flights.in_flight, @flights.waiting, @flights.pending]
    assert_raises(TypeError) { C::Coalescer.allocate }
  end

  def test_u64_keys
    low = @flights.join(0)
    high = @flights.join(2**64 - 1)
    assert_instance_of C::Leader, high
    assert_equal 2**64 - 1, high.key
    assert_equal high.flight_id, @flights.flight(2**64 - 1)
    follower = @flights.join(2**64 - 1)
    assert_equal 1, @flights.complete(high.flight_id)
    assert_equal C::Notification.new(follower.waiter_id, high.flight_id, 2**64 - 1, :completed),
                 @flights.drain(1).first
    assert_equal low.flight_id, @flights.flight(0)
  end

  def test_values_are_frozen_data
    leader = @flights.join(1)
    follower = @flights.join(1)
    [leader, follower].each { |value| assert_kind_of Data, value }
    assert_predicate leader, :frozen?
    assert_equal({ flight_id: leader.flight_id, key: 1 }, leader.to_h)
    assert_equal C::Follower.new(waiter_id: follower.waiter_id, flight_id: leader.flight_id, key: 1), follower
    assert_kind_of Integer, leader.flight_id
  end

  def test_freeze_guards
    leader = @flights.join(1)
    follower = @flights.join(1)
    @flights.freeze
    assert_raises(FrozenError) { @flights.join(2) }
    assert_raises(FrozenError) { @flights.complete(leader.flight_id) }
    assert_raises(FrozenError) { @flights.abandon(leader.flight_id) }
    assert_raises(FrozenError) { @flights.cancel(follower.waiter_id) }
    assert_raises(FrozenError) { @flights.drain(1) }
    assert_equal leader.flight_id, @flights.flight(1)
    assert_equal 1, @flights.followers(leader.flight_id)
    assert_equal [1, 1, 0], [@flights.in_flight, @flights.waiting, @flights.pending]
  end

  def test_coalescers_are_independent_and_survive_gc_stress
    previous = GC.stress
    GC.stress = true
    other = C::Coalescer.new
    leader = @flights.join(5)
    assert_instance_of C::Leader, other.join(5)
    @flights.join(5)
    assert_equal 1, @flights.complete(leader.flight_id)
    GC.compact
    assert_equal 1, @flights.drain(8).size
    assert_equal 1, other.in_flight
  ensure
    GC.stress = previous
  end
end
