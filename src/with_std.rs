use std::sync::atomic::{
    spin_loop_hint, AtomicBool,
    Ordering::{Acquire, Release},
};
use std::thread::{self, Thread};

use crate::state::{AtomicState, State::WouldBlock, Waiter};
use crate::Internal;
use crate::{Block, POISON_PANIC_MSG};

////////////////////////////////////////////////////////////////////////////////////////////////////
// ParkThread
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParkThread;

/********** impl Internal *************************************************************************/

impl Internal for ParkThread {}

/********** impl Block ****************************************************************************/

impl Block for ParkThread {
    #[inline]
    fn block(state: &AtomicState, head: Waiter) {
        let mut waiter = StackWaiter {
            thread: Some(thread::current()),
            ready: AtomicBool::default(),
            next: head.into(),
        };

        let mut curr = head;
        let new_head = Waiter::from(&mut waiter as *mut StackWaiter);

        while let Err(error) = state.try_swap_waiters(curr, new_head) {
            if let WouldBlock(ptr) = error {
                curr = ptr;
                waiter.next = ptr.into();
            } else {
                return;
            }
        }

        while !waiter.ready.load(Acquire) {
            spin_loop_hint();
            thread::park();
        }

        let _ = state.load().expect(POISON_PANIC_MSG); // propagates poisoning
    }

    #[inline]
    fn unblock(waiter: Waiter) {
        let mut queue: *mut StackWaiter = waiter.into();

        unsafe {
            while let Some(curr) = queue.as_mut() {
                queue = curr.next;

                let thread = curr.thread.take().unwrap();
                // the stack waiter could be dropped right after this store
                curr.ready.store(true, Release);

                thread.unpark();
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// StackWaiter
////////////////////////////////////////////////////////////////////////////////////////////////////

#[repr(align(4))]
struct StackWaiter {
    thread: Option<Thread>,
    ready: AtomicBool,
    next: *mut StackWaiter,
}

/********** impl From *****************************************************************************/

impl From<*mut StackWaiter> for Waiter {
    #[inline]
    fn from(waiter: *mut StackWaiter) -> Self {
        Self(waiter as usize)
    }
}

impl From<Waiter> for *mut StackWaiter {
    #[inline]
    fn from(waiter: Waiter) -> Self {
        waiter.0 as *mut StackWaiter
    }
}
