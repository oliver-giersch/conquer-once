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
            // (wait:1) this acquire load syncs-with the release swaps (guard:2) and the acq-rel CAS
            // (wait:2)
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
            next: head.into_stack_waiter(),
        };

        let mut curr = head;
        let new_head = Waiter::from(&waiter as *const StackWaiter);

        // (wait:2) this acq-rel CAS syncs-with the acq-rel swap (guard:2) and the acquire loads
        // (wait:1), (guard:1) and (cell:1) through (cell:4)
        while let Err(error) = state.try_swap_waiters(curr, new_head, Ordering::AcqRel) {
            match error {
                WouldBlock(ptr) => {
                    // the waiter hasn't been shared yet, so it's still safe to mutate the next pointer
                    curr = ptr;
                    waiter.next = ptr.into_stack_waiter();
                    backoff.spin();
                }
                // acquire-release is required here to enforce acquire ordering in the failure case,
                // which guarantees that any (non-atomic) stores to the cell's inner state preceding
                // (guard:2) have become visible, if the function returns;
                // (alternatively an explicit acquire fence could be placed into this path)
                Ready => return,
                Uninit => unreachable!(),
            }
        }

        // (ready:1) this acquire load syncs-with the release store (ready:2)
        while !waiter.ready.load(Ordering::Acquire) {
            thread::park();
        }

        // propagates poisoning
        // (wait:3) this acquire load syncs-with the acq-rel swap (guard:2)
        assert_eq!(state.load(Ordering::Acquire).expect(POISON_PANIC_MSG), Ready);
    }

    #[inline]
    fn unblock(waiter_queue: Waiter) {
        let mut curr = waiter_queue.into_stack_waiter();
        while !curr.is_null() {
            let thread = unsafe {
                let waiter = &*curr;
                curr = waiter.next;
                // there can be now data race when mutating the thread-cell as only the unblocking
                // thread will access it, the stack waiter can dropped as soon as the following
                // store becomes visible, so the thread MUST be taken out first
                let thread = waiter.thread.take().unwrap();
                // (ready:2) this release store syncs-with the acquire load (ready:1)
                waiter.ready.store(true, Ordering::Release);
                thread
            };

            thread.unpark();
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
        Self(waiter as *const ())
    }
}

/********** ext impl Waiter ***********************************************************************/

impl Waiter {
    #[inline]
    fn into_stack_waiter(self) -> *const StackWaiter {
        self.0 as *const _
    }
}
