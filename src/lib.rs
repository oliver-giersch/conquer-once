//! This crate provides a set of synchronized initialization primitives, which
//! are primarily useful for lazy and one-time initialization of static
//! variables.
//!
//! All types exported through the [`noblock`] and [`spin`] modules are fully
//! `#[no_std]` compatible.
//!
//! # Synchronization Primitives
//!
//! With the `std` cargo feature enabled (which is the default setting), this
//! crate provides the [`Once`], [`OnceCell`] and [`Lazy`] types and the
//! equivalents of these types using spin-locks in the `spin` sub-module.
//!
//! ## Lazy
//!
//! The [`Lazy`] type has the same functionality as the `lazy_static!` macro,
//! but works without any macros.
//!
//! ## Once
//!
//! This type is very similar to the `std::sync::Once` type in the standard
//! library, but features a richer API.
//!
//! ## OnceCell
//!
//! A cell type with interior mutability that can be only written to once and
//! only allows read access to the contained data after initialization.
//!
//! # Credits
//!
//! Inspiration for this crate is heavily drawn from
//! [`once_cell`](https://crates.io/crates/once_cell),
//! but features clear distinctions between blocking and non-blocking operations
//! and support for `#[no_std]` environments out of the box, by offering
//! additional synchronization primitives using spin-locks instead of OS reliant
//! blocking mechanisms.
//! Unlike many other crates, support for the `#[no_std]` compatible types is
//! not mutually exclusive with using the types relying on the presence of the
//! standard library.
//!
//! The idea for the implementation of the [`Once`]/[`OnceCell`] types is also
//! directly inspired by the implementation in the standard library.
//! The reasoning behind re-implementing [`Once`] is the fairly restricted and
//! partly unstable API of the version in the standard library.

#![cfg_attr(all(not(test), not(feature = "std")), no_std)]
#![deny(missing_docs)]
#![forbid(clippy::undocumented_unsafe_blocks)]

#[cfg(test)]
#[macro_use]
mod tests;

pub mod noblock;
pub mod spin;

/// Re-exports of internal generic type for the purpose of accessing their
/// documentation.
pub mod doc {
    pub use crate::cell::OnceCell;
    pub use crate::lazy::Lazy;
}

mod cell;
mod lazy;
mod state;

#[cfg(feature = "std")]
mod park;

pub use crate::cell::{TryGetError, TryInitError};

#[cfg(feature = "std")]
pub use crate::park::Lazy;
#[cfg(feature = "std")]
pub use crate::park::Once;
#[cfg(feature = "std")]
pub use crate::park::OnceCell;

const POISON_PANIC_MSG: &str = "OnceCell instance has been poisoned.";
