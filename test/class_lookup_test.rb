# frozen_string_literal: true

require 'minitest/autorun'
require 'open3'
require 'rbconfig'

# The class of the value the extension wraps (Coalescer) is fixed when it
# loads. Nothing a caller does later, redefining the constant or
# `Object.const_get`, reaches a coalescer being built. Each case runs in a
# fresh process: the class is fixed once per process, at load.
class ClassLookupTest < Minitest::Test
  LIB = File.expand_path('../lib', __dir__)

  PRELUDE = <<~RUBY
    require 'coalesce_machines'
    C = CoalesceMachines
  RUBY

  # Runs `body` after PRELUDE in a new Ruby; answers its last line of output.
  def isolated(body)
    out, err, status = Open3.capture3(RbConfig.ruby, '-I', LIB, '-e', PRELUDE + body)
    assert status.success?, "exited #{status.exitstatus}: #{err}"
    out.lines.last&.chomp
  end

  # Bug: a Coalescer constant redefined before the first coalescer made it an
  # instance of the new class, without any of the gem's methods.
  def test_a_redefined_coalescer_constant_does_not_change_the_coalescer_class
    assert_equal 'true Leader 1', isolated(<<~RUBY)
      original = C::Coalescer
      C.send(:remove_const, :Coalescer)
      C.const_set(:Coalescer, Class.new)
      flights = original.new
      leader = flights.join(42)
      puts [flights.instance_of?(original), leader.class.name.split('::').last, flights.in_flight].join(' ')
    RUBY
  end

  # Bug: building the first coalescer ran `Object.const_get`, Ruby code in
  # the middle of a native call. No Ruby runs while one is built now.
  def test_building_a_coalescer_runs_no_const_get
    assert_equal '0 Coalescer Coalescer Coalescer', isolated(<<~RUBY)
      hits = 0
      Object.define_singleton_method(:const_get) do |*args|
        hits += 1
        super(*args)
      end
      built = [C::Coalescer.new, C::Coalescer.with_scope(3), C::Coalescer.resumed(3, 5)]
      puts [hits, *built.map { it.class.name.split('::').last }].join(' ')
    RUBY
  end

  # Bug: a redefined `Object.const_get` that raised hit an `unwrap` in the
  # extension: an uncatchable fatal, and the process exited.
  def test_a_raising_const_get_does_not_kill_the_process
    assert_equal '0 3', isolated(<<~RUBY)
      Object.define_singleton_method(:const_get) { |*| raise IOError, 'lookup ran while building' }
      puts [C::Coalescer.new.in_flight, C::Coalescer.with_scope(3).scope].join(' ')
    RUBY
  end
end
