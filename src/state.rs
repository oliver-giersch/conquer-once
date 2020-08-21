use core::convert::{TryFrom, TryInto};
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "std")]
use crate::park::StackWaiter;
use crate::POISON_PANIC_MSG;

use self::OnceState::{Ready, Uninit, WouldBlock};

const WOULD_BLOCK: usize = 0;
const UNINIT: usize = 1;
const READY: usize = 2;
const POISONED: usize = 3;

////////////////////////////////////////////////////////////////////////////////////////////////////
// AtomicState (public but not exported)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The concurrently and atomically mutable internal state of a [`OnceCell`].
///
/// A `WOULD_BLOCK` value is also interpreted as a `null` pointer to an empty
/// [`WaiterQueue`] indicating that there is only a single blocked thread.
#[derive(Debug)]
pub struct AtomicOnceState(AtomicUsize);

/********** impl inherent *************************************************************************/

impl AtomicOnceState {
    /// Creates a new `UNINIT` state.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self(AtomicUsize::new(UNINIT))
    }

    /// Creates a new `READY` state.
    #[inline]
    pub(crate) const fn ready() -> Self {
        Self(AtomicUsize::new(READY))
    }

    /// Loads the current state using `ordering`.
    ///
    /// A Poisoning of the state is returned as a [`PoisonError`].
    #[inline]
    pub(crate) fn load(&self, order: Ordering) -> Result<OnceState, PoisonError> {
        self.0.load(order).try_into()
    }

    /// Attempts to set the state to blocked and fails if the state is either
    /// already initialized or blocked.
    #[inline]
    pub(crate) fn try_block(&self, order: Ordering) -> Result<(), TryBlockError> {
        let prev = self.0.compare_and_swap(UNINIT, WOULD_BLOCK, order);
        match prev.try_into().expect(POISON_PANIC_MSG) {
            Uninit => Ok(()),
            Ready => Err(TryBlockError::AlreadyInit),
            WouldBlock(state) => Err(TryBlockError::WouldBlock(state)),
        }
    }

    #[inline]
    pub(crate) unsafe fn unblock(&self, state: SwapState, order: Ordering) -> BlockedState {
        BlockedState(self.0.swap(state as usize, order))
    }

    #[cfg(feature = "std")]
    /// Attempts to compare-and-swap the head of the `current` [`WaiterQueue]`
    /// with the `next` queue.
    #[inline]
    pub(crate) unsafe fn try_swap_blocked(
        &self,
        current: BlockedState,
        new: BlockedState,
        success: Ordering,
    ) -> Result<(), OnceState> {
        match self.0.compare_and_swap(current.into(), new.into(), success) {
            prev if prev == current.into() => Ok(()),
            prev => Err(prev.try_into().expect(POISON_PANIC_MSG)),
        }
    }

    /*
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
    }*/
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// OnceState
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum OnceState {
    /// Initial (uninitialized) state.
    Uninit,
    /// Ready (initialized) state.
    Ready,
    /// Blocked state with queue of waiting threads.
    WouldBlock(BlockedState),
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
            state => Ok(WouldBlock(BlockedState(state))),
        }
    }
}

/********** impl From (usize) *********************************************************************/

impl From<OnceState> for usize {
    #[inline]
    fn from(state: OnceState) -> Self {
        match state {
            OnceState::Ready => READY,
            OnceState::Uninit => UNINIT,
            OnceState::WouldBlock(BlockedState(state)) => state,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// TryBlockError
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum TryBlockError {
    AlreadyInit,
    WouldBlock(BlockedState),
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// PoisonError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An error type indicating a `OnceCell` has been poisoned.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PoisonError;

////////////////////////////////////////////////////////////////////////////////////////////////////
// BlockedState (public but not exported)
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BlockedState(usize);

/********** impl inherent *************************************************************************/

impl BlockedState {
    #[cfg(feature = "std")]
    pub(crate) fn as_ptr(self) -> *const () {
        self.0 as *const _
    }
}

/********** impl From (for usize) *****************************************************************/

impl From<BlockedState> for usize {
    #[inline]
    fn from(state: BlockedState) -> Self {
        state.0
    }
}

/********** impl From (*const StackWaiter) ********************************************************/

#[cfg(feature = "std")]
impl From<*const StackWaiter> for BlockedState {
    #[inline]
    fn from(waiter: *const StackWaiter) -> Self {
        Self(waiter as usize)
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// SwapState
////////////////////////////////////////////////////////////////////////////////////////////////////

#[repr(usize)]
pub(crate) enum SwapState {
    Ready = READY,
    Poisoned = POISONED,
}
