use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, Thread};

use conquer_util::BackOff;

use crate::cell::Block;
use crate::state::{
    AtomicOnceState,
    OnceState::{Ready, Uninit, WouldBlock},
    Waiter,
};
use crate::{Internal, POISON_PANIC_MSG};

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
            let state = state.load(Ordering::Acquire).expect(POISON_PANIC_MSG);
            match state {
                Ready => return,
                WouldBlock(waiter) if backoff.advise_yield() => break waiter,
                _ => {}
            }

            backoff.spin();
        };
        backoff.reset();

        let mut waiter = StackWaiter {
            thread: Cell::new(Some(thread::current())),
            ready: AtomicBool::default(),
            next: head.into(),
        };

        let mut curr = head;
        let new_head = Waiter::from(&waiter as *const StackWaiter);

        while let Err(error) = state.try_swap_waiters(curr, new_head, Ordering::AcqRel) {
            match error {
                WouldBlock(ptr) => {
                    // the waiter hasn't been shared yet, so it's still safe to mutate the next pointer
                    curr = ptr;
                    waiter.next = ptr.into();
                    backoff.spin();
                }
                Ready => return,
                Uninit => unreachable!(),
            }
        }

        while !waiter.ready.load(Ordering::Acquire) {
            thread::park();
        }

        // propagates poisoning
        assert_eq!(state.load(Ordering::Acquire).expect(POISON_PANIC_MSG), Ready);
    }

    #[inline]
    fn unblock(waiter: Waiter) {
        let mut queue: *const StackWaiter = waiter.into();
        unsafe {
            while let Some(curr) = queue.as_ref() {
                queue = curr.next;

                // there can be now data race when mutating the thread-cell as only the unblocking
                // thread will access it
                let thread = curr.thread.take().unwrap();
                // the stack waiter can be dropped once this store becomes visible, so the thread
                // MUST be taken out beforehand
                curr.ready.store(true, Ordering::Release);

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
    thread: Cell<Option<Thread>>,
    ready: AtomicBool,
    next: *const StackWaiter,
}

/********** impl From *****************************************************************************/

impl From<*const StackWaiter> for Waiter {
    #[inline]
    fn from(waiter: *const StackWaiter) -> Self {
        Self(waiter as usize)
    }
}

impl From<Waiter> for *const StackWaiter {
    #[inline]
    fn from(waiter: Waiter) -> Self {
        waiter.0 as *const StackWaiter
    }
}
