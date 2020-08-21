use core::convert::{TryFrom, TryInto};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::POISON_PANIC_MSG;

use self::OnceState::{Ready, Uninit, WouldBlock};

const WOULD_BLOCK: usize = 0;
const UNINIT: usize = 1;
const READY: usize = 2;
const POISONED: usize = 3;

////////////////////////////////////////////////////////////////////////////////////////////////////
// AtomicState
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The concurrently and atomically mutable internal state of a [`OnceCell`].
///
/// A `WOULD_BLOCK` value is also interpreted as a `null` pointer to an empty
/// [`WaiterQueue`] indicating that there is only a single blocked thread.
#[derive(Debug)]
pub struct AtomicOnceState(AtomicUsize, PhantomData<*const ()>);

/********** impl inherent *************************************************************************/

impl AtomicOnceState {
    /// Creates a new `UNINIT` state.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self(AtomicUsize::new(UNINIT), PhantomData)
    }

    /// Creates a new `READY` state.
    #[inline]
    pub(crate) const fn ready() -> Self {
        Self(AtomicUsize::new(READY), PhantomData)
    }

    /// Loads the current state using `ordering`.
    ///
    /// A Poisoning of the state is returned as a [`PoisonError`].
    #[inline]
    pub(crate) fn load(&self, order: Ordering) -> Result<OnceState, PoisonError> {
        self.0.load(order).try_into()
    }

    #[cfg(any(test, feature = "std"))]
    /// Attempts to compare-and-swap the head of the `current` [`WaiterQueue]`
    /// with the `next` queue.
    #[inline]
    pub(crate) fn try_swap_waiters(
        &self,
        current: WaiterQueue,
        new: WaiterQueue,
        order: Ordering,
    ) -> Result<(), OnceState> {
        match self.0.compare_and_swap(current.into(), new.into(), order) {
            prev if prev == current.into() => Ok(()),
            prev => Err(prev.try_into().expect(POISON_PANIC_MSG)),
        }
    }

    /// Attempts to set the state to blocked and fails if the state is either
    /// already initialized or blocked.
    #[inline]
    pub(crate) fn try_block(&self, order: Ordering) -> Result<(), TryBlockError> {
        let prev = self.0.compare_and_swap(UNINIT, WOULD_BLOCK, order);
        match prev.try_into().expect(POISON_PANIC_MSG) {
            Uninit => Ok(()),
            Ready => Err(TryBlockError::AlreadyInit),
            WouldBlock(waiter) => Err(TryBlockError::WouldBlock(waiter)),
        }
    }

    /// Unconditionally swaps the current blocked state to `READY` and returns
    /// the queue of waiting threads.
    #[inline]
    pub(crate) fn swap_ready(&self, order: Ordering) -> WaiterQueue {
        WaiterQueue::from(self.0.swap(READY, order))
    }

    /// Unconditionally swaps the current blocked state to `POISONED` and
    /// returns the queue of waiting threads.
    #[inline]
    pub(crate) fn swap_poisoned(&self, order: Ordering) -> WaiterQueue {
        WaiterQueue::from(self.0.swap(POISONED, order))
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// State
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OnceState {
    /// Initial (uninitialized) state.
    Uninit,
    /// Ready (initialized) state.
    Ready,
    /// Blocked state with queue of waiting threads.
    WouldBlock(WaiterQueue),
}

/********** impl TryFrom **************************************************************************/

impl TryFrom<usize> for OnceState {
    type Error = PoisonError;

    #[inline]
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            POISONED => Err(PoisonError),
            UNINIT => Ok(Uninit),
            READY => Ok(Ready),
            waiter => Ok(WouldBlock(WaiterQueue::from(waiter))),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// TryBlockError
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum TryBlockError {
    AlreadyInit,
    WouldBlock(WaiterQueue),
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// PoisonError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An error type indicating a `OnceCell` has been poisoned.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PoisonError;

////////////////////////////////////////////////////////////////////////////////////////////////////
// WaiterQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

/// When using the OS reliant blocking strategy, the state encodes a pointer
/// to the first blocked thread, which forms a linked list of waiting threads.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WaiterQueue {
    pub(crate) head: *const (),
}

/********** impl From *****************************************************************************/

impl From<usize> for WaiterQueue {
    #[inline]
    fn from(state: usize) -> Self {
        Self { head: state as *const () }
    }
}

impl From<WaiterQueue> for usize {
    #[inline]
    fn from(waiter: WaiterQueue) -> Self {
        waiter.head as usize
    }
}
