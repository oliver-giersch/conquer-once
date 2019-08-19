use std::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Release},
};
use std::thread::{self, Thread};

use conquer_util::BackOff;

use crate::cell::Block;
use crate::state::{
    AtomicOnceState,
    OnceState::{Ready, WouldBlock},
    Waiter,
};

use crate::Internal;
use crate::POISON_PANIC_MSG;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ParkThread
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Blocking strategy using low-level and OS reliant parking and un-parking
/// mechanisms.
#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParkThread;

/********** impl Internal *************************************************************************/

impl Internal for ParkThread {}

/********** impl Block ****************************************************************************/

impl Block for ParkThread {
    #[inline]
    fn block(state: &AtomicOnceState) {
        let backoff = BackOff::new();
        let head = loop {
            backoff.spin();

            let state = state.load().expect(POISON_PANIC_MSG);
            match state {
                Ready => return,
                WouldBlock(waiter) if backoff.advise_yield() => break waiter,
                _ => {}
            }
        };
        backoff.reset();

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
                backoff.spin();
            } else {
                return;
            }
        }

        while !waiter.ready.load(Acquire) {
            thread::park();
        }

        assert_eq!(state.load().expect(POISON_PANIC_MSG), Ready); // propagates poisoning
    }

    #[inline]
    fn unblock(waiter: Waiter) {
        let mut queue: *mut StackWaiter = waiter.into();

        unsafe {
            while let Some(curr) = queue.as_mut() {
                queue = curr.next;

                let thread = curr.thread.take().unwrap();
                // the stack waiter could be dropped right after this store!
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
