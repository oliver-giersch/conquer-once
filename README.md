# conquer-once

Synchronization primitives for lazy and one-time initialization using low-level
blocking mechanisms with clear distinction between blocking and non-blocking
methods and additional support for `#[no_std]` environments when using
spin-locks.

[![Build Status](https://github.com/oliver-giersch/conquer-once/workflows/Rust/badge.svg)](
https://github.com/oliver-giersch/conquer-once/actions)
[![Latest version](https://img.shields.io/crates/v/conquer-once.svg)](
https://crates.io/crates/conquer-once)
[![Documentation](https://docs.rs/conquer-once/badge.svg)](https://docs.rs/conquer-once)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](
https://github.com/oliver-giersch/conquer-once)
[![Rust 1.36+](https://img.shields.io/badge/Rust-1.36.0-orange.svg)](
https://www.rust-lang.org)

## Usage

To use this crate, add the following to your `Cargo.toml`

```
[dependencies]
conquer-once = "0.4.0"
```

## Minimum Supported Rust Version (MSRV)

The minimum supported Rust version for this crate is 1.49.0.

## Cargo Features

By default, `conquer-once` enables the `std` feature.
With this feature enabled, the crate exports the `Lazy`, `Once` and `OnceCell`
types that use an OS and standard library reliant blocking mechanism.
Without this feature, the crate is `#[no_std]` environment compatible, but only
exports the types in the crate's `spin` sub-module, which use spin-locks.

The feature can be disabled by specifying the dependency as follows:

```
[dependencies.conquer-once]
version = "0.4.0"
use-default-features = false
```

## License

`conquer-once` is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
