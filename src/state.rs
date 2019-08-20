use core::convert::{TryFrom, TryInto};
use core::sync::atomic::{
    AtomicUsize,
    Ordering::{Acquire, Release},
};

use crate::POISON_PANIC_MSG;

use self::OnceState::{Ready, Uninit, WouldBlock};

const WOULD_BLOCK: usize = 0;
const UNINIT: usize = 1;
const READY: usize = 2;
const POISONED: usize = 3;

////////////////////////////////////////////////////////////////////////////////////////////////////
// AtomicState
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct AtomicOnceState(AtomicUsize);

/********** impl inherent *************************************************************************/

impl AtomicOnceState {
    #[inline]
    pub const fn new() -> Self {
        Self(AtomicUsize::new(UNINIT))
    }

    #[inline]
    pub const fn ready() -> Self {
        Self(AtomicUsize::new(READY))
    }

    #[inline]
    pub fn load(&self) -> Result<OnceState, PoisonError> {
        self.0.load(Acquire).try_into()
    }

    #[inline]
    pub fn try_swap_waiters(&self, current: Waiter, new: Waiter) -> Result<(), OnceState> {
        match self.0.compare_and_swap(current.0, new.0, Acquire) {
            prev if prev == current.0 => Ok(()),
            prev => Err(prev.try_into().expect(POISON_PANIC_MSG)),
        }
    }

    #[inline]
    pub fn try_block(&self) -> Result<(), TryBlockError> {
        let prev = self.0.compare_and_swap(UNINIT, WOULD_BLOCK, Acquire);
        match prev.try_into().expect(POISON_PANIC_MSG) {
            Uninit => Ok(()),
            Ready => Err(TryBlockError::AlreadyInit),
            WouldBlock(waiter) => Err(TryBlockError::WouldBlock(waiter)),
        }
    }

    #[inline]
    pub fn swap_ready(&self) -> Waiter {
        Waiter(self.0.swap(READY, Release))
    }

    #[inline]
    pub fn swap_poisoned(&self) -> Waiter {
        Waiter(self.0.swap(POISONED, Release))
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// State
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OnceState {
    Uninit,
    Ready,
    WouldBlock(Waiter),
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
            waiter => Ok(WouldBlock(Waiter(waiter))),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// TryBlockError
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum TryBlockError {
    AlreadyInit,
    WouldBlock(Waiter),
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// PoisonError
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PoisonError;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Waiter
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Waiter(pub usize);
