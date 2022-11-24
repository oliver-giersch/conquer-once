//! Synchronized one-time and lazy initialization primitives that permit only
//! non-blocking synchronized initialization operations.

use crate::{cell::Unblock, state::BlockedState};

use self::internal::NoBlock;

/// A type for lazy initialization of e.g. global static variables, which
/// provides the same functionality as the `lazy_static!` macro.
///
/// This type does not permit any (potentially) blocking operations, only their
/// respective non-blocking counterparts and is thus `#[no_std]` compatible.
///
/// For the API of this type alias, see the API of the generic
/// [`Lazy`](crate::lazy::Lazy) type.
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, NoBlock, F>;

/// An interior mutability cell type which allows synchronized one-time
/// initialization and read-only access exclusively after initialization.
///
/// This type does not permit any (potentially) blocking operations, only their
/// respective non-blocking counterparts and is thus `#[no_std]` compatible.
///
/// For the API of this type alias, see the generic
/// [`OnceCell`](crate::doc::OnceCell) type.
pub type OnceCell<T> = crate::cell::OnceCell<T, NoBlock>;

/// A synchronization primitive which can be used to run a one-time global
/// initialization.
///
/// This type does not permit any (potentially) blocking operations, only their
/// respective non-blocking counterparts and is thus `#[no_std]` compatible.
///
/// For the API of this type alias, see the generic
/// [`OnceCell`](crate::doc::OnceCell) type.
/// This is a specialization with `T = ()`.
pub type Once = crate::cell::OnceCell<(), NoBlock>;

mod internal {
    /// "Blocking" strategy which does not actually allow blocking.
    #[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub struct NoBlock;
}

impl Unblock for NoBlock {
    #[inline(always)]
    unsafe fn on_unblock(_: BlockedState) {}
}

#[cfg(test)]
mod tests {
    generate_tests_non_blocking!();
}
