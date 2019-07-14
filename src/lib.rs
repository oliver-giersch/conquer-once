#![cfg_attr(all(not(test), not(feature = "std")), no_std)]

pub mod spin;

mod cell;
mod lazy;
mod state;
#[cfg(feature = "std")]
mod with_std;

use crate::internal::Internal;
use crate::state::{AtomicState, Waiter};
#[cfg(feature = "std")]
use crate::with_std::ParkThread;

const POISON_PANIC_MSG: &str = "OnceCell instance has been poisoned.";

#[cfg(feature = "std")]
/// TODO: Docs...
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, ParkThread, F>;
#[cfg(feature = "std")]
/// TODO: Docs...
pub type OnceCell<T> = crate::cell::OnceCell<T, ParkThread>;
#[cfg(feature = "std")]
/// TODO: Docs...
pub type Once = OnceCell<ParkThread>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Block (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// TODO: Docs...
pub trait Block: Default + Internal {
    /// TODO: Docs...
    fn block(state: &AtomicState, waiter: Waiter);

    /// TODO: Docs...
    fn unblock(waiter: Waiter);
}

mod internal {
    pub trait Internal {}
}
