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
    assert_equal leader.flight, @flights.flight(42)

    a = @flights.join(42)
    b = @flights.join(42)
    assert_instance_of C::Follower, a
    assert_equal leader.flight, a.flight
    refute_equal a.waiter, b.waiter
    assert_equal [1, 2, 0], [@flights.in_flight, @flights.waiting, @flights.pending]
    assert_equal 2, @flights.followers(leader.flight)
    assert_empty @flights.drain(10)

    assert_equal 2, @flights.complete(leader.flight)
    assert_equal [0, 0, 2], [@flights.in_flight, @flights.waiting, @flights.pending]
    assert_nil @flights.flight(42)
    assert_nil @flights.followers(leader.flight)

    expected = [a, b].map { |f| C::Notification.new(f.waiter, leader.flight, 42, :completed) }
    assert_equal expected, @flights.drain(10)
    assert_equal 0, @flights.pending
  end

  def test_settled_keys_are_not_remembered_and_old_ids_are_stale
    first = @flights.join(7)
    assert_equal 0, @flights.complete(first.flight)
    assert_equal :stale, @flights.complete(first.flight)
    assert_equal :stale, @flights.abandon(first.flight)
    second = @flights.join(7)
    assert_instance_of C::Leader, second
    refute_equal first.flight, second.flight
    assert_equal :stale, @flights.complete(first.flight)
    assert_equal :stale, @flights.complete(0)
    assert_equal :stale, @flights.complete(2**64 - 1)
    assert_equal second.flight, @flights.flight(7)
  end

  def test_abandon_notifies_followers_who_rejoin
    leader = @flights.join(9)
    follower = @flights.join(9)
    assert_equal 1, @flights.abandon(leader.flight)
    note = @flights.drain(1).first
    assert_equal [follower.waiter, leader.flight, 9, :abandoned],
                 [note.waiter, note.flight, note.key, note.outcome]
    assert_instance_of C::Leader, @flights.join(9)
    assert_instance_of C::Follower, @flights.join(9)
  end

  def test_cancel_waiting_and_queued_followers
    leader = @flights.join(1)
    a, b, c = Array.new(3) { @flights.join(1) }
    assert_equal :cancelled, @flights.cancel(a.waiter)
    assert_equal :stale, @flights.cancel(a.waiter)
    assert_equal :stale, @flights.cancel(leader.flight)
    assert_equal 2, @flights.followers(leader.flight)
    assert_equal 2, @flights.complete(leader.flight)
    assert_equal :cancelled, @flights.cancel(b.waiter)
    assert_equal [c.waiter], @flights.drain(10).map(&:waiter)
    assert_equal :stale, @flights.cancel(c.waiter)
  end

  def test_last_follower_leaving_keeps_the_flight
    leader = @flights.join(1)
    follower = @flights.join(1)
    assert_equal :cancelled, @flights.cancel(follower.waiter)
    assert_equal 0, @flights.followers(leader.flight)
    assert_equal leader.flight, @flights.flight(1)
    assert_equal 0, @flights.complete(leader.flight)
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
    assert_equal 128, @flights.complete(leaders[0].flight)
    assert_equal :waiters_full, @flights.join(1)
    assert_instance_of C::Leader, @flights.join(64)
    assert_equal :waiters_full, @flights.join(64)
    assert_equal followers[0].waiter, @flights.drain(1).first.waiter
    assert_instance_of C::Follower, @flights.join(1)
  end

  def test_drain_is_bounded_and_resumable
    one = @flights.join(1)
    two = @flights.join(2)
    waiters = Array.new(6) { |i| @flights.join(1 + (i % 2)) }
    assert_equal 3, @flights.abandon(two.flight)
    assert_equal 3, @flights.complete(one.flight)
    order = waiters.values_at(1, 3, 5, 0, 2, 4).map(&:waiter)

    assert_empty @flights.drain(0)
    assert_equal 6, @flights.pending
    first = @flights.drain(4)
    assert_equal order.first(4), first.map(&:waiter)
    assert_equal %i[abandoned abandoned abandoned completed], first.map(&:outcome)
    assert_equal 2, @flights.pending
    assert_equal order.last(2), @flights.drain(2**64 - 1).map(&:waiter)
    assert_empty @flights.drain(4)
  end

  def test_poll_takes_the_oldest_notification
    assert_nil @flights.poll
    leader = @flights.join(9)
    a = @flights.join(9)
    b = @flights.join(9)
    assert_nil @flights.poll
    assert_equal 2, @flights.abandon(leader.flight)
    assert_equal C::Notification.new(a.waiter, leader.flight, 9, :abandoned), @flights.poll
    assert_equal 1, @flights.pending
    assert_equal [b.waiter], @flights.drain(4).map(&:waiter)
    assert_nil @flights.poll
    assert_equal :stale, @flights.cancel(a.waiter)
  end

  def test_id_layout_helpers
    assert_equal [48, (2**48) - 1], [C::SEQUENCE_BITS, C::SEQUENCE_MAX]
    assert_equal 5, C.scoped(0, 5)
    assert_equal (3 << 48) | 5, C.scoped(3, 5)
    assert_equal (65_535 << 48) | C::SEQUENCE_MAX, C.scoped(65_535, (2**64) - 1)
    assert_equal [0, 3, 65_535], [5, C.scoped(3, 5), (2**64) - 1].map { |raw| C.scope_of(raw) }
    scoped = C::Coalescer.with_scope(7)
    assert_equal C.scoped(7, 1), scoped.join(1).flight
    assert_equal 7, C.scope_of(scoped.join(1).waiter)
    assert_raises(RangeError) { C.scoped(65_536, 1) }
    assert_raises(RangeError) { C.scoped(1, 2**64) }
    assert_raises(RangeError) { C.scope_of(-1) }
    assert_raises(TypeError) { C.scoped(1.0, 1) }
    assert_raises(TypeError) { C.scoped(1, nil) }
    assert_raises(TypeError) { C.scope_of('1') }
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
    assert_equal high.flight, @flights.flight(2**64 - 1)
    follower = @flights.join(2**64 - 1)
    assert_equal 1, @flights.complete(high.flight)
    assert_equal C::Notification.new(follower.waiter, high.flight, 2**64 - 1, :completed),
                 @flights.drain(1).first
    assert_equal low.flight, @flights.flight(0)
  end

  def test_values_are_frozen_data
    leader = @flights.join(1)
    follower = @flights.join(1)
    [leader, follower].each { |value| assert_kind_of Data, value }
    assert_predicate leader, :frozen?
    assert_equal({ flight: leader.flight }, leader.to_h)
    assert_equal C::Follower.new(waiter: follower.waiter, flight: leader.flight), follower
    assert_equal [%i[flight], %i[waiter flight], %i[waiter flight key outcome]],
                 [C::Leader, C::Follower, C::Notification].map(&:members)
    assert_kind_of Integer, leader.flight
  end

  def test_freeze_guards
    leader = @flights.join(1)
    follower = @flights.join(1)
    @flights.freeze
    assert_raises(FrozenError) { @flights.join(2) }
    assert_raises(FrozenError) { @flights.complete(leader.flight) }
    assert_raises(FrozenError) { @flights.abandon(leader.flight) }
    assert_raises(FrozenError) { @flights.cancel(follower.waiter) }
    assert_raises(FrozenError) { @flights.drain(1) }
    assert_raises(FrozenError) { @flights.poll }
    assert_equal leader.flight, @flights.flight(1)
    assert_equal 1, @flights.followers(leader.flight)
    assert_equal [1, 1, 0], [@flights.in_flight, @flights.waiting, @flights.pending]
  end

  def test_scopes_keep_the_ids_of_separate_coalescers_apart
    near = C::Coalescer.with_scope(1)
    far = C::Coalescer.with_scope(2)
    assert_equal [0, 1, 2], [@flights.scope, near.scope, far.scope]
    near_leader = near.join(7)
    far_leader = far.join(7)
    near_follower = near.join(7)
    far_follower = far.join(7)
    assert_equal (1 << 48) | 1, near_leader.flight
    assert_equal (2 << 48) | 2, far_follower.waiter
    assert_equal 1, @flights.join(7).flight # scope zero keeps plain counters

    # A foreign id names nothing here, and the refusal changes nothing.
    assert_equal :stale, near.cancel(far_follower.waiter)
    assert_equal :stale, near.complete(far_leader.flight)
    assert_equal :stale, near.abandon(far_leader.flight)
    assert_nil near.followers(far_leader.flight)
    assert_equal [1, 1, 0], [near.in_flight, near.waiting, near.pending]
    assert_equal :cancelled, near.cancel(near_follower.waiter)
    assert_equal 1, far.complete(far_leader.flight)
  end

  def test_scope_must_be_a_sixteen_bit_integer
    assert_equal [0, 65_535], [C::Coalescer.with_scope(0).scope, C::Coalescer.with_scope(65_535).scope]
    assert_raises(RangeError) { C::Coalescer.with_scope(65_536) }
    assert_raises(RangeError) { C::Coalescer.with_scope(-1) }
    assert_raises(TypeError) { C::Coalescer.with_scope(1.0) }
    assert_raises(TypeError) { C::Coalescer.with_scope(nil) }
    assert_raises(ArgumentError) { C::Coalescer.with_scope }
    assert_raises(ArgumentError) { C::Coalescer.new(1) }
    assert_raises(ArgumentError) { C::Coalescer.new(scope: 1) }
  end

  def test_coalescers_are_independent_and_survive_gc_stress
    previous = GC.stress
    GC.stress = true
    other = C::Coalescer.new
    leader = @flights.join(5)
    assert_instance_of C::Leader, other.join(5)
    @flights.join(5)
    assert_equal 1, @flights.complete(leader.flight)
    GC.compact
    assert_equal 1, @flights.drain(8).size
    assert_equal 1, other.in_flight
  ensure
    GC.stress = previous
  end
end
