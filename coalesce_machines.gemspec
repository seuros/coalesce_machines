# frozen_string_literal: true
require_relative 'lib/coalesce_machines/version'

Gem::Specification.new do |spec|
  spec.name = 'coalesce_machines'
  spec.version = CoalesceMachines::VERSION
  spec.authors = ['Abdelkader Boudih']
  spec.email = ['oss@seuros.com']
  spec.summary = 'Initial bounded in-flight coalescing policy with a native Rust core'
  spec.description = 'Local unstable leader/follower in-flight registry; emits notification ids, ' \
                     'stores no results, runs nothing.'
  spec.homepage = 'https://github.com/seuros/coalesce_machines'
  spec.license = 'MIT'
  spec.required_ruby_version = '>= 4.0.0'
  spec.metadata['source_code_uri'] = spec.homepage
  spec.metadata['rubygems_mfa_required'] = 'true'
  spec.metadata['cargo_crate_name'] = 'coalesce_machines_native'
  spec.metadata['cargo_manifest_path'] = 'ext/coalesce_machines_native/ffi/Cargo.toml'
  spec.files = Dir.glob(%w[
    lib/**/*.rb sig/**/*.rbs ext/**/*.{rs,rb,toml,md} Cargo.toml
    Cargo.lock README.md CHANGELOG.md LICENSE.txt
  ]).select { |p| File.file?(p) }
    .reject { |p| p.end_with?('/tests.rs') || p.include?('/tests/') }
  spec.require_paths = ['lib']
  spec.extensions = ['ext/coalesce_machines_native/extconf.rb']
  spec.add_dependency 'rb_sys', '~> 0.9', '>= 0.9.130'
end
