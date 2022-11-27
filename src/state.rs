use core::{
    convert::{TryFrom, TryInto},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(feature = "std")]
use crate::park::StackWaiter;
use crate::POISON_PANIC_MSG;

use self::OnceState::{Ready, Uninit, WouldBlock};

const WOULD_BLOCK: usize = 0;
const UNINIT: usize = 1;
const READY: usize = 2;
const POISONED: usize = 3;

/// The concurrently and atomically mutable internal state of a [`OnceCell`].
///
/// A `WOULD_BLOCK` value is also interpreted as a `null` pointer to an empty
/// [`WaiterQueue`] indicating that there is only a single blocked thread.
#[derive(Debug)]
pub struct AtomicOnceState(AtomicUsize);

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
        let prev = match self.0.compare_exchange(UNINIT, WOULD_BLOCK, order, Ordering::Relaxed) {
            Ok(prev) => prev,
            Err(prev) => prev,
        };

        match prev.try_into().expect(POISON_PANIC_MSG) {
            Uninit => Ok(()),
            Ready => Err(TryBlockError::AlreadyInit),
            WouldBlock(state) => Err(TryBlockError::WouldBlock(state)),
        }
    }

    /// Unblocks the state by replacing it with either `Ready` or `Poisoned`.
    ///
    /// # Safety
    ///
    /// Must not be called if unblocking might cause data races due to
    /// un-synchronized reads and/or writes.
    #[inline]
    pub(crate) unsafe fn unblock(&self, state: SwapState, order: Ordering) -> BlockedState {
        BlockedState(self.0.swap(state as usize, order))
    }
}

#[cfg(feature = "std")]
impl AtomicOnceState {
    /// Attempts to compare-and-swap the head of the `current` [`WaiterQueue]`
    /// with the `new` queue.
    ///
    /// NOTE: This must be used instead of `try_block` for the `ParkThread`
    /// strategy implementation.
    ///
    /// # Safety
    ///
    /// The caller has to ensure that `new` is a valid pointer to a `StackWaiter`.
    #[inline]
    pub(crate) unsafe fn try_enqueue_waiter(
        &self,
        current: BlockedState,
        new: BlockedState,
        success: Ordering,
    ) -> Result<(), OnceState> {
        let prev =
            match self.0.compare_exchange(current.into(), new.into(), success, Ordering::Relaxed) {
                Ok(prev) => prev,
                Err(prev) => prev,
            };

        match prev {
            prev if prev == current.into() => Ok(()),
            prev => Err(prev.try_into().expect(POISON_PANIC_MSG)),
        }
    }
}

/// The state of the (atomic) synchronization variable at the time a thread
/// attempted to initiate its `init` closure but was blocked from doing so
/// because another thread was already in this mutually exclusive section.
///
/// Note: This is not actually exported but needs to be `pub` because it appears
/// in the API of the (sealed public) `Unblock` trait.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BlockedState(usize);

impl From<BlockedState> for usize {
    #[inline]
    fn from(state: BlockedState) -> Self {
        state.0
    }
}

#[cfg(feature = "std")]
impl BlockedState {
    pub(crate) fn as_ptr(self) -> *const StackWaiter {
        self.0 as *const _
    }
}

#[cfg(feature = "std")]
impl From<*const StackWaiter> for BlockedState {
    #[inline]
    fn from(waiter: *const StackWaiter) -> Self {
        Self(waiter as usize)
    }
}

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum OnceState {
    /// Initial (uninitialized) state.
    Uninit,
    /// Ready (initialized) state.
    Ready,
    /// Blocked state with queue of waiting threads.
    WouldBlock(BlockedState),
}

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

pub(crate) enum TryBlockError {
    AlreadyInit,
    WouldBlock(BlockedState),
}

/// An error type indicating a `OnceCell` has been poisoned.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct PoisonError;

#[repr(usize)]
pub(crate) enum SwapState {
    Ready = READY,
    Poisoned = POISONED,
}
