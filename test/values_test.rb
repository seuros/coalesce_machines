# frozen_string_literal: true
require 'minitest/autorun'
require 'coalesce_machines'

# Values are valid or do not exist, and the binding never loses work.
class ValuesTest < Minitest::Test
  C = CoalesceMachines

  def setup
    @flights = C::Coalescer.with_scope(3)
    @leader = @flights.join(7)
    @follower = @flights.join(7)
  end

  def test_values_the_coalescer_answers_rebuild_by_hand
    assert_equal @leader, C::Leader.new(@leader.flight)
    assert_equal @follower, C::Follower.new(@follower.waiter, @follower.flight)
    @flights.complete(@leader.flight)
    note = @flights.poll
    assert_equal note, C::Notification.new(note.waiter, note.flight, 7, :completed)
    assert_equal note, C::Notification.new(**note.to_h)
  end

  def test_an_id_with_sequence_zero_is_no_id
    [0, 3 << 48, 2**64 - 2**48].each do |raw|
      assert_raises(RangeError) { C::Leader.new(raw) }
      assert_raises(RangeError) { C::Follower.new(raw, @leader.flight) }
      assert_raises(RangeError) { C::Follower.new(@follower.waiter, raw) }
      assert_raises(RangeError) { C::Notification.new(raw, @leader.flight, 7, :completed) }
      %i[complete abandon cancel followers].each do |name|
        assert_raises(RangeError, "#{name}(#{raw})") { @flights.public_send(name, raw) }
      end
    end
    assert_equal [1, 1, 2], [@flights.in_flight, @flights.waiting, @flights.issued]
  end

  def test_a_follower_must_be_of_its_flights_scope_and_after_it
    other_scope = (4 << 48) | 2
    [[other_scope, @leader.flight], [@leader.flight, @leader.flight], [@leader.flight, @follower.waiter]]
      .each do |waiter, flight|
        assert_raises(RangeError) { C::Follower.new(waiter, flight) }
        assert_raises(RangeError) { C::Notification.new(waiter, flight, 7, :abandoned) }
      end
  end

  def test_wrong_types_raise_type_error_and_out_of_range_raises_range_error
    [nil, 1.0, '1', :one].each do |bad|
      assert_raises(TypeError) { C::Leader.new(bad) }
      assert_raises(TypeError) { C::Follower.new(bad, @leader.flight) }
      assert_raises(TypeError) { C::Notification.new(@follower.waiter, @leader.flight, bad, :completed) }
    end
    [-1, 2**64].each do |bad|
      assert_raises(RangeError) { C::Leader.new(bad) }
      assert_raises(RangeError) { C::Notification.new(@follower.waiter, @leader.flight, bad, :completed) }
    end
    assert_raises(TypeError) { C::Notification.new(@follower.waiter, @leader.flight, 7, 'completed') }
    assert_raises(TypeError) { C::Notification.new(@follower.waiter, @leader.flight, 7, nil) }
    assert_raises(RangeError) { C::Notification.new(@follower.waiter, @leader.flight, 7, :nope) }
    assert_raises(ArgumentError) { C::Leader.new }
  end

  def test_the_hooks_are_private
    %i[__leader __follower __notification].each { refute_respond_to C, it }
  end

  def test_answers_are_frozen_data_of_the_gems_own_classes
    assert_equal [%i[flight], %i[waiter flight], %i[waiter flight key outcome]],
                 [C::Leader, C::Follower, C::Notification].map(&:members)
    @flights.complete(@leader.flight)
    [@leader, @follower, @flights.poll].each do |value|
      assert_kind_of Data, value
      assert_predicate value, :frozen?
    end
    assert_equal({ flight: @leader.flight }, @leader.to_h)
  end

  # Sealing stops accidents, not reflection: Ruby binds a named subclass's
  # constant before `inherited` raises, so the class leaks. Both forms raise.
  def test_values_are_sealed_and_a_named_subclass_leaks_its_constant
    [C::Leader, C::Follower, C::Notification].each do |value|
      assert_raises(TypeError) { Class.new(value) }
      assert_raises(NoMethodError) { value.allocate }
    end
    assert_raises(TypeError) { self.class.module_eval('class LeakedLeader < C::Leader; end', __FILE__, __LINE__) }
    assert self.class.const_defined?(:LeakedLeader, false), 'the constant is bound before inherited raises'
  end

  # The coalescer builds these values and reads none back: every argument is
  # an Integer id, so a forged value (a leaked subclass skipping the check, a
  # Marshal frame edited past it) is a TypeError and changes nothing.
  def test_a_forged_value_has_no_way_into_the_coalescer
    frame = Marshal.dump(@leader)
    frame[(frame.index(":\vflight".b) + 8)..] = "i\x00".b
    edited = Marshal.load(frame)
    assert_equal 0, edited.flight
    before = [@flights.in_flight, @flights.waiting]
    assert_raises(TypeError) { @flights.complete(edited) }
    assert_raises(TypeError) { @flights.cancel(@follower) }
    assert_equal before, [@flights.in_flight, @flights.waiting]
  end

  # A frozen coalescer raises FrozenError from every mutating call.
  def test_a_frozen_coalescer_refuses_every_mutating_call
    @flights.freeze
    [-> { @flights.reserve }, -> { @flights.join(9) }, -> { @flights.complete(@leader.flight) },
     -> { @flights.abandon(@leader.flight) }, -> { @flights.cancel(@follower.waiter) },
     -> { @flights.poll }, -> { @flights.drain(1) }].each { |call| assert_raises(FrozenError) { call.call } }
    assert_equal [1, 1], [@flights.in_flight, @flights.waiting]
  end

  # The classes are pinned at init, so a compaction moves none of them: every
  # path that builds a value still works after it.
  def test_every_value_path_survives_gc_compact
    skip 'GC.compact is not available' unless GC.respond_to?(:compact)
    GC.compact
    assert_instance_of C::Leader, @flights.join(8)
    assert_instance_of C::Follower, @flights.join(8)
    @flights.complete(@leader.flight)
    assert_instance_of C::Notification, @flights.poll
    @flights.abandon(@flights.flight(8))
    assert_equal [:abandoned], @flights.drain(8).map(&:outcome)
  end

  # The binding never loses work: answers are built natively from what the
  # core returned, so Ruby code that raises cannot run once it has changed.
  def test_answers_are_built_without_running_ruby_code
    classes = [C::Leader, C::Follower, C::Notification]
    classes.each do |klass|
      klass.class_eval do
        alias_method :__checked_initialize, :initialize
        define_method(:initialize) { |**| raise 'ran Ruby code' }
      end
    end
    @flights.complete(@leader.flight)
    assert_instance_of C::Leader, @flights.join(8)
    assert_instance_of C::Follower, @flights.join(8)
    assert_equal [@follower.waiter], @flights.drain(8).map(&:waiter)
    assert_equal [1, 1, 0], [@flights.in_flight, @flights.waiting, @flights.pending]
  ensure
    classes.each do |klass|
      klass.class_eval do
        alias_method :initialize, :__checked_initialize
        remove_method :__checked_initialize
      end
    end
  end
end
