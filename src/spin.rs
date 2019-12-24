//! Synchronized one-time and lazy initialization primitives that use spin-locks
//! in case of concurrent accesses under contention.

use core::sync::atomic::{spin_loop_hint, Ordering};

use crate::cell::Block;
use crate::internal::Internal;
use crate::state::{AtomicOnceState, OnceState::WouldBlock, WaiterQueue};
use crate::POISON_PANIC_MSG;

/// A type for lazy initialization of e.g. global static variables, which
/// provides the same functionality as the `lazy_static!` macro.
///
/// This type uses spin-locks if the initialization is contended and is thus
/// `#[no_std]` compatible.
///
/// For the API of this type alias, see the API of the generic
/// [`Lazy`](crate::raw::Lazy) type.
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, Spin, F>;

/// An interior mutability cell type which allows synchronized one-time
/// initialization and read-only access exclusively after initialization.
///
/// This type uses spin-locks if the initialization is contended and is thus
/// `#[no_std]` compatible.
///
/// For the API of this type alias, see the generic
/// [`OnceCell`](crate::raw::OnceCell) type.
pub type OnceCell<T> = crate::cell::OnceCell<T, Spin>;

/// A synchronization primitive which can be used to run a one-time global
/// initialization.
///
/// This type uses spin-locks if the initialization is contended and is thus
/// `#[no_std]` compatible.
///
/// For the API of this type alias, see the generic
/// [`OnceCell`](crate::raw::OnceCell) type.
/// This is a specialization with `T = ()`.
pub type Once = OnceCell<()>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Spin
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Thread blocking (locking) strategy using spin-locks.
#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Spin;

/********** impl Internal *************************************************************************/

impl Internal for Spin {}

/********** impl Block ****************************************************************************/

impl Block for Spin {
    /// Spins until the [`OnceCell`] state is set to `READY` or panics if it
    /// becomes poisoned.
    #[inline]
    fn block(state: &AtomicOnceState) {
        // (spin:1) this acquire load syncs-with the acq-rel swap (guard:2)
        while let WouldBlock(_) = state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
            spin_loop_hint()
        }
    }

    /// No-op since all waiters stop spinning on their own.
    #[inline]
    fn unblock(_: WaiterQueue) {}
}

#[cfg(test)]
mod tests {
    generate_tests!();
}
