use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, Thread};

use conquer_util::BackOff;

use crate::cell::Block;
use crate::state::{
    AtomicOnceState,
    OnceState::{Ready, Uninit, WouldBlock},
    WaiterQueue,
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
    /// Blocks (parks) the current thread until it is woken up by the thread with permission to
    /// initialize the `OnceCell`.
    #[inline]
    fn block(state: &AtomicOnceState) {
        let backoff = BackOff::new();
        // spin a little before parking the thread in case the state is quickly unlocked again
        let queue = loop {
            // (wait:1) this acquire load syncs-with the release swaps (guard:2) and the acq-rel CAS
            // (wait:2)
            let state = state.load(Ordering::Acquire).expect(POISON_PANIC_MSG);
            match state {
                Ready => return,
                WouldBlock(queue) if backoff.advise_yield() => break queue,
                _ => {}
            }

            backoff.spin();
        };
        backoff.reset();

        // create a linked list node on the current threads stack which is guaranteed to stay alive
        // while the thread is parked.
        let mut waiter = StackWaiter {
            thread: Cell::new(Some(thread::current())),
            ready: AtomicBool::default(),
            next: queue.head(),
        };

        let mut curr = queue;
        let new_head = WaiterQueue::from(&waiter as *const StackWaiter);

        // put the current thread to the front of the list of parked threads.
        // (wait:2) this acq-rel CAS syncs-with the acq-rel swap (guard:2) and the acquire loads
        // (wait:1), (guard:1) and (cell:1) through (cell:4)
        while let Err(error) = state.try_swap_waiters(curr, new_head, Ordering::AcqRel) {
            match error {
                // another parked thread succeeded in placing itself at the queue's front
                WouldBlock(queue) => {
                    // the waiter hasn't been shared yet, so it's still safe to mutate the next pointer
                    curr = queue;
                    waiter.next = queue.head();
                    backoff.spin();
                }
                // acquire-release is required here to enforce acquire ordering in the failure case,
                // which guarantees that any (non-atomic) stores to the cell's inner state preceding
                // (guard:2) have become visible, if the function returns;
                // (alternatively an explicit acquire fence could be placed into this path)
                Ready => return,
                Uninit => unreachable!(), // the state cannot become `UNINIT` again
            }
        }

        // park the thread until it is woken up by the thread that first set the state to blocked.
        // (ready:1) this acquire load syncs-with the release store (ready:2)
        while !waiter.ready.load(Ordering::Acquire) {
            thread::park();
        }

        // propagates poisoning
        // (wait:3) this acquire load syncs-with the acq-rel swap (guard:2)
        assert_eq!(state.load(Ordering::Acquire).expect(POISON_PANIC_MSG), Ready);
    }

    /// Unblocks all blocked waiting threads.
    #[inline]
    fn unblock(waiter_queue: WaiterQueue) {
        let mut curr = waiter_queue.head();
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

/// A linked list node that lives on the stack of a parked thread.
#[repr(align(4))]
struct StackWaiter {
    thread: Cell<Option<Thread>>,
    ready: AtomicBool,
    next: *const StackWaiter,
}

/********** impl From *****************************************************************************/

impl From<*const StackWaiter> for WaiterQueue {
    #[inline]
    fn from(waiter: *const StackWaiter) -> Self {
        Self { head: waiter as *const () }
    }
}

/********** ext impl Waiter ***********************************************************************/

impl WaiterQueue {
    #[inline]
    fn head(self) -> *const StackWaiter {
        self.head as *const _
    }
}
