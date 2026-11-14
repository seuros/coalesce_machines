# Changelog

## [0.7.1](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.7.0...coalesce_machines/v0.7.1) (2026-11-14)


### Bug Fixes

* **deps:** require matryona-machines &gt;= 0.4 ([f762a91](https://github.com/seuros/coalesce_machines/commit/f762a91e08bfdf8f9db13f3c9557a417f30113bd))

## [0.7.0](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.6.0...coalesce_machines/v0.7.0) (2026-11-14)


### ⚠ BREAKING CHANGES

* adopt matryona 0.3 integer_in for drain max

### Features

* adopt matryona 0.3 integer_in for drain max ([a7903e4](https://github.com/seuros/coalesce_machines/commit/a7903e4fe7621e1ca228528ecfa729b39a04f22a))

## [0.6.0](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.5.0...coalesce_machines/v0.6.0) (2026-11-14)


### ⚠ BREAKING CHANGES

* adopt matryona-machines 0.2: pinned Data classes, symbols! tables for refusals and outcomes

### Features

* adopt matryona-machines 0.2: pinned Data classes, symbols! tables for refusals and outcomes ([afc1ee9](https://github.com/seuros/coalesce_machines/commit/afc1ee958d70b905471d935b10e82f3eb31db83d))

## [0.5.0](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.4.0...coalesce_machines/v0.5.0) (2026-11-14)


### ⚠ BREAKING CHANGES

* seal values, check exact classes natively, clone-then-keep join/poll/drain

### Features

* seal values, check exact classes natively, clone-then-keep join/poll/drain ([19b8261](https://github.com/seuros/coalesce_machines/commit/19b8261e59f48a25eb3444eea2fa674004ae024c))

## [0.4.0](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.3.0...coalesce_machines/v0.4.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* handles are issued through periscope-machines. Every instance can be built with a scope; a handle from another scope is refused before any lookup; all handle kinds of an instance share one sequence; resumed restores the sequence after a restart. The private id layout (SEQUENCE_BITS, SEQUENCE_MAX, scoped, scope_of) is gone from the public API.

### Features

* issue ids through periscope-machines; add reserve, resumed and issued so sibling cores share the one sequence ([0437f1f](https://github.com/seuros/coalesce_machines/commit/0437f1fc773c136bff287d29b6dd44a1ed41368b))

## [0.3.0](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.2.0...coalesce_machines/v0.3.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* see CONVENTIONS: one exhaustive Refusal per crate, Result from every mutator, honoured idempotent repeats, Ruby mirrors the core. No compatibility aliases. Ruby gains poll, scoped, scope_of and the sequence constants.

### Features

* exhaustive Refusal without alias, Ruby mirrors Joined and Notification fields, with_scope constructor ([de09f01](https://github.com/seuros/coalesce_machines/commit/de09f01be8793a23d6d178bb165fb929893f2ee5))

## [0.2.0](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.1.2...coalesce_machines/v0.2.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* Coalescer::drain takes &mut [Notification] instead of &mut [Option<Notification>]. Refused is kept as a deprecated alias of Refusal.

### Features

* rename Refused to Refusal and drain into a plain Notification buffer ([2d806ff](https://github.com/seuros/coalesce_machines/commit/2d806ff114038089e8f82b56a68cb74ca8765d99))

## [0.1.2](https://github.com/seuros/coalesce_machines/compare/coalesce_machines/v0.1.1...coalesce_machines/v0.1.2) (2026-11-13)


### Features

* scope ids so separate coalescers never share one ([032cc0c](https://github.com/seuros/coalesce_machines/commit/032cc0c7edf983534e67f3ecbbf35a9e15034886))

## [0.1.1](https://github.com/seuros/coalesce_machines/compare/coalesce_machines-v0.1.0...coalesce_machines/v0.1.1) (2026-11-12)


### Features

* initial extraction ([145944e](https://github.com/seuros/coalesce_machines/commit/145944ec7a7f5fb1884d179fb650e4fdbb3e97de))
