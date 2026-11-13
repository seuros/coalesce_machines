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

  def test_scope_of_routes_an_id_to_its_coalescer
    assert_equal [0, 0, 3, 65_535], [0, 5, (3 << 48) | 5, (2**64) - 1].map { C.scope_of(it) }
    pool = [4, 9].to_h { [it, C::Coalescer.with_scope(it)] }
    leader = pool[9].join(1)
    follower = pool[9].join(1)
    assert_equal [9, 9], [C.scope_of(leader.flight), C.scope_of(follower.waiter)]
    assert_equal 1, pool.fetch(C.scope_of(leader.flight)).complete(leader.flight)
    assert_raises(RangeError) { C.scope_of(-1) }
    assert_raises(RangeError) { C.scope_of(2**64) }
    assert_raises(TypeError) { C.scope_of(1.0) }
    assert_raises(TypeError) { C.scope_of('1') }
    assert_raises(TypeError) { C.scope_of(nil) }
    # The rest of the layout lives in the periscope-machines crate, not here.
    refute_respond_to C, :scoped
    refute C.const_defined?(:SEQUENCE_BITS)
    refute C.const_defined?(:SEQUENCE_MAX)
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

  def state(flights) = [flights.in_flight, flights.waiting, flights.pending]

  # Regression: identical calls on two coalescers issue different ids, and
  # each refuses the other's through every method that takes one.
  def test_an_id_from_another_coalescer_names_nothing_here
    near = C::Coalescer.with_scope(1)
    far = C::Coalescer.with_scope(2)
    assert_equal [0, 1, 2], [@flights.scope, near.scope, far.scope]
    near_ids = [near.join(7).flight, near.join(7).waiter]
    far_ids = [far.join(7).flight, far.join(7).waiter]
    assert_equal [(1 << 48) | 1, (1 << 48) | 2], near_ids
    assert_equal [(2 << 48) | 1, (2 << 48) | 2], far_ids
    assert_equal [1, 2], [@flights.join(7).flight, @flights.join(7).waiter] # scope zero keeps plain counters

    [[near, far_ids], [far, near_ids]].each do |here, (flight, waiter)|
      assert_equal :stale, here.complete(flight)
      assert_equal :stale, here.abandon(flight)
      assert_equal :stale, here.cancel(waiter)
      assert_nil here.followers(flight)
      assert_equal [1, 1, 0], state(here)
      assert_equal 1, here.followers(here.flight(7))
    end
    assert_equal :cancelled, near.cancel(near_ids.last)
    assert_equal 1, far.complete(far_ids.first)
  end

  def test_coalescers_left_in_the_same_scope_still_alias
    one = C::Coalescer.with_scope(5)
    other = C::Coalescer.with_scope(5)
    flight = one.join(7).flight
    assert_equal flight, other.join(8).flight
    waiter = one.join(7).waiter
    assert_equal waiter, other.join(8).waiter
    # Nothing tells them apart: keeping scopes distinct is the caller's job.
    assert_equal :cancelled, other.cancel(waiter)
    assert_equal 0, other.complete(flight)
    assert_equal [1, 1, 0], state(one)
  end

  def test_repeats_stay_stale
    leader = @flights.join(7)
    gone = @flights.join(7).waiter
    assert_equal :cancelled, @flights.cancel(gone)
    assert_equal :stale, @flights.cancel(gone)
    assert_equal 0, @flights.complete(leader.flight)
    assert_equal :stale, @flights.complete(leader.flight)
    assert_equal :stale, @flights.abandon(leader.flight)
    assert_equal [0, 0, 0], state(@flights)
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

  def refused_everywhere(flights, id)
    before = [state(flights), flights.issued]
    assert_equal %i[stale stale stale], [flights.complete(id), flights.abandon(id), flights.cancel(id)]
    assert_nil flights.followers(id)
    assert_equal before, [state(flights), flights.issued]
  end

  def test_reserved_ids_share_the_one_sequence_and_name_nothing
    flights = C::Coalescer.with_scope(3)
    ids = [flights.reserve, flights.join(7).flight, flights.reserve, flights.join(7).waiter, flights.reserve]
    assert_equal((1..5).map { (3 << 48) | it }, ids)
    assert_equal [5, 2**48 - 6], [flights.issued, flights.remaining]
    assert_equal [1, 1, 0], state(flights)
    ids.values_at(0, 2, 4).each { refused_everywhere(flights, it) }
    assert(ids.all? { flights.owns?(it) })
    refute([(3 << 48) | 6, 3 << 48, (4 << 48) | 1, 1].any? { flights.owns?(it) })
    assert_equal 1, flights.complete(ids[1])
    assert flights.owns?(ids[1]) # origin, not liveness
  end

  def test_no_id_repeats_across_join_and_reserve
    seen = []
    300.times do |i|
      seen << @flights.reserve if i.even?
      role = @flights.join(i % 5)
      seen << (role.is_a?(C::Leader) ? role.flight : role.waiter)
      @flights.complete(@flights.flight(i % 5)) if (i % 7).zero?
      @flights.drain(8)
    end
    assert_equal seen.size, seen.uniq.size
    assert_equal seen.size, @flights.issued
    assert_equal (1..seen.size).to_a, seen
  end

  def test_resumed_continues_the_sequence_after_a_restart
    first = C::Coalescer.with_scope(6)
    old = [first.join(7).flight, first.join(7).waiter, first.reserve]
    second = C::Coalescer.resumed(6, first.issued)
    assert_equal [6, 3, [0, 0, 0]], [second.scope, second.issued, state(second)]
    old.each { refused_everywhere(second, it) }
    assert_equal [(6 << 48) | 4, (6 << 48) | 5], [second.join(7).flight, second.join(7).waiter]
    old.each { refused_everywhere(second, it) }
    # A fresh coalescer in the same scope would have reissued them.
    assert_equal old.first, C::Coalescer.with_scope(6).join(7).flight
    assert_equal [0, 0], [C::Coalescer.resumed(0, 0).issued, C::Coalescer.resumed(0, 0).scope]
  end

  def test_the_last_id_is_issued_then_exhausted
    last = 2**48 - 1
    flights = C::Coalescer.resumed(65_535, last - 1)
    assert_equal 1, flights.remaining
    assert_equal 2**64 - 1, flights.reserve
    assert_equal [0, last], [flights.remaining, flights.issued]
    assert_equal %i[exhausted exhausted], [flights.reserve, flights.join(1)]
    assert_equal [[0, 0, 0], last], [state(flights), flights.issued]
    spent = C::Coalescer.resumed(2, last)
    assert_equal %i[exhausted exhausted], [spent.join(1), spent.reserve]
  end

  def test_resumed_validates_its_arguments
    assert_raises(RangeError) { C::Coalescer.resumed(0, 2**48) }
    assert_raises(RangeError) { C::Coalescer.resumed(7, 2**64 - 1) }
    assert_raises(RangeError) { C::Coalescer.resumed(65_536, 0) }
    assert_raises(RangeError) { C::Coalescer.resumed(0, -1) }
    assert_raises(RangeError) { C::Coalescer.resumed(0, 2**64) }
    assert_raises(TypeError) { C::Coalescer.resumed(1.0, 0) }
    assert_raises(TypeError) { C::Coalescer.resumed(0, nil) }
    assert_raises(ArgumentError) { C::Coalescer.resumed(0) }
    assert_raises(TypeError) { @flights.owns?(nil) }
    assert_raises(RangeError) { @flights.owns?(-1) }
    @flights.freeze
    assert_raises(FrozenError) { @flights.reserve }
    assert_equal [0, 2**48 - 1, false], [@flights.issued, @flights.remaining, @flights.owns?(1)]
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
