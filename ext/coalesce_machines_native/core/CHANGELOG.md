# Changelog

## [0.4.0](https://github.com/seuros/coalesce_machines/compare/coalesce-machines-v0.3.0...coalesce-machines-v0.4.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* Coalescer::drain takes &mut [Notification] instead of &mut [Option<Notification>]. Refused is kept as a deprecated alias of Refusal.

### Features

* rename Refused to Refusal and drain into a plain Notification buffer ([2d806ff](https://github.com/seuros/coalesce_machines/commit/2d806ff114038089e8f82b56a68cb74ca8765d99))

## [0.3.0](https://github.com/seuros/coalesce_machines/compare/coalesce-machines-v0.2.0...coalesce-machines-v0.3.0) (2026-11-13)


### Features

* scope ids so separate coalescers never share one ([032cc0c](https://github.com/seuros/coalesce_machines/commit/032cc0c7edf983534e67f3ecbbf35a9e15034886))

## [0.2.0](https://github.com/seuros/coalesce_machines/compare/coalesce-machines-v0.1.0...coalesce-machines-v0.2.0) (2026-11-12)


### Features

* initial extraction ([145944e](https://github.com/seuros/coalesce_machines/commit/145944ec7a7f5fb1884d179fb650e4fdbb3e97de))
