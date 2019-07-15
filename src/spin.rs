//! Synchronized one-time and lazy initialization primitives that use spin-locks
//! in case of concurrent accesses under contention.

use core::sync::atomic::spin_loop_hint;

use crate::internal::Internal;
use crate::state::{AtomicOnceState, OnceState::WouldBlock, Waiter};
use crate::{Block, POISON_PANIC_MSG};

/// A type for lazy initialization of e.g. global static variables.
///
/// This type provides the functionality as the `lazy_static!` macro.
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, Spin, F>;
/// An interior mutability cell type which allows synchronized one-time
/// initialization and exclusively read-only access after initialization.
pub type OnceCell<T> = crate::cell::OnceCell<T, Spin>;
/// A synchronization primitive which can be used to run a one-time global
/// initialization.
pub type Once = OnceCell<()>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Spin
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Spin;

/********** impl Internal *************************************************************************/

impl Internal for Spin {}

/********** impl Block ****************************************************************************/

impl Block for Spin {
    #[inline]
    fn block(state: &AtomicOnceState, _: Waiter) {
        while let WouldBlock(_) = state.load().expect(POISON_PANIC_MSG) {
            spin_loop_hint()
        }
    }

    #[inline]
    fn unblock(_: Waiter) {}
}

#[cfg(test)]
mod tests {
    generate_tests!();
}
