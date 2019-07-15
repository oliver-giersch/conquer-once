//! This crate provides a set of synchronized initialization primitives which
//! are useful for lazy initialization of static variables.
//!
//! # Synchronization Primitives
//!
//! With the `std` feature enabled (which is the default setting), this crate
//! provides the [`Once`], [`OnceCell`] and [`Lazy`] types and the equivalents
//! of these types using spin-locks in the `spin` sub-module.
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
//! A cell type with interior mutability that can be only synchronized once and
//! only allows read-only access to the contained data after initialization.
//!
//! # Credits
//!
//! Inspiration for this crate is heavily drawn from [`once_cell`](https://crates.io/crates/once_cell),
//! but features clear distinctions between blocking and non-blocking operations
//! and support for `#[no_std]` environments by offering additional primitives
//! that use spin-locks instead of OS reliant blocking mechanisms out of the
//! box.
//! Unlike many other crates, support for the `#[no_std]` compatible types is
//! not mutually exclusive with using the types that rely on the presence of the
//! standard library.
//!
//! The idea for the implementation of the [`Once`]/[`OnceCell`] types is also
//! directly inspired by the implementation in the standard library.
//! The reasoning behind re-implementing [`Once`] is the fairly restricted and
//! partly unstable API of the version in the standard library.

#![cfg_attr(all(not(test), not(feature = "std")), no_std)]

#[cfg(test)]
#[macro_use]
mod tests;

pub mod spin;

mod cell;
mod lazy;
mod state;
#[cfg(feature = "std")]
mod with_std;

use crate::internal::Internal;
use crate::state::{AtomicOnceState, Waiter};
#[cfg(feature = "std")]
use crate::with_std::ParkThread;

const POISON_PANIC_MSG: &str = "OnceCell instance has been poisoned.";

#[cfg(feature = "std")]
/// A type for lazy initialization of e.g. global static variables.
///
/// This type provides the functionality as the `lazy_static!` macro.
///
/// # Examples
///
/// ```
/// use std::sync::Mutex;
///
/// use conquer_once::Lazy;
///
/// static MUTEX: Lazy<Mutex<Vec<i32>>> = Lazy::new(Mutex::default);
///
/// let mut lock = MUTEX.lock().unwrap();
///
/// lock.push(1);
/// lock.push(2);
/// lock.push(3);
///
/// assert_eq!(lock.as_slice(), &[1, 2, 3]);
/// ```
///
/// The associated [`new`](Lazy::new) function can be used with any function or
/// closure that implements `Fn() -> T`.
///
/// ```
/// use std::collections::HashMap;
///
/// use conquer_once::Lazy;
///
/// static CAPITALS: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
///     let mut map = HashMap::new();
///     map.insert("Norway", "Oslo");
///     map.insert("Belgium", "Brussels");
///     map.insert("Latvia", "Riga");
///     map
/// });
///
/// assert_eq!(CAPITALS.get(&"Norway"), Some(&"Oslo"));
/// assert_eq!(CAPITALS.get(&"Belgium"), Some(&"Brussels"));
/// assert_eq!(CAPITALS.get(&"Latvia"), Some(&"Riga"));
/// ```
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, ParkThread, F>;

#[cfg(feature = "std")]
/// An interior mutability cell type which allows synchronized one-time
/// initialization and exclusively read-only access after initialization.
///
/// # Examples
///
/// ```
/// use conquer_once::OnceCell;
///
/// #[derive(Copy, Clone)]
/// struct Configuration {
///     mode: i32,
///     threshold: u64,
///     msg: &'static str,
/// }
///
/// static CONFIG: OnceCell<Configuration> = OnceCell::new();
///
/// // producer thread
/// CONFIG.init_once(|| Configuration {
///     mode: 2,
///     threshold: 128,
///     msg: "..."
/// });
///
/// // consumer thread
/// let res = CONFIG.get().copied();
/// if let Some(config) = res {
///     assert_eq!(config.mode, 2);
///     assert_eq!(config.threshold, 128);
/// }
/// ```
pub type OnceCell<T> = crate::cell::OnceCell<T, ParkThread>;

#[cfg(feature = "std")]
/// A synchronization primitive which can be used to run a one-time global
/// initialization.
///
/// # Examples
///
/// ```
/// use conquer_once::Once;
///
/// static mut GLOBAL: usize = 0;
/// static INIT: Once = Once::new();
///
/// fn get_global() -> usize {
///     // this is safe because the `Once` ensures the `static mut` is assigned
///     // by only one thread and without data races.
///     unsafe {
///         INIT.init_once(|| {
///             GLOBAL = expensive_computation();
///         });
///         # assert_eq!(GLOBAL, 1);
///         GLOBAL
///     }
/// }
///
/// fn expensive_computation() -> usize {
///     // ...
///     # 1
/// }
/// ```
pub type Once = OnceCell<()>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Block (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A sealed trait for abstracting over different blocking strategies.
pub trait Block: Default + Internal {
    /// Blocks the current thread, until `state` is no longer blocking.
    fn block(state: &AtomicOnceState, waiter: Waiter);
    /// Unblocks all waiting threads.
    fn unblock(waiter: Waiter);
}

mod internal {
    pub trait Internal {}
}
