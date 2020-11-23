use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, Thread};

use conquer_util::BackOff;

use crate::cell::{Block, Unblock};
use crate::state::{
    AtomicOnceState, BlockedState,
    OnceState::{Ready, Uninit, WouldBlock},
};
use crate::POISON_PANIC_MSG;

use self::internal::ParkThread;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Lazy (type alias)
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(any(test, feature = "std"))]
/// A type for lazy initialization of e.g. global static variables, which
/// provides the same functionality as the `lazy_static!` macro.
///
/// This type uses the blocking synchronization mechanism provided by the
/// underlying operating system.
///
/// For the API of this type alias, see the API of the generic
/// [`Lazy`](crate::doc::Lazy) type.
///
/// # Examples
///
/// ```
/// use std::sync::Mutex;
///
/// # #[cfg(feature = "std")]
/// use conquer_once::Lazy;
/// # #[cfg(not(feature = "std"))]
/// # use conquer_once::spin::Lazy;
///
/// static MUTEX: Lazy<Mutex<Vec<i32>>> = Lazy::new(Mutex::default);
///
/// let mut lock = MUTEX.lock().unwrap();
///
/// lock.push(1);
/// lock.push(2);
/// lock.push(3);
///
/// assert_eq!(lock.as_slice(), &[1, 2, 3]);
/// ```
///
/// The associated [`new`](crate::lazy::Lazy::new) function can be used with any
/// function or closure that implements `Fn() -> T`.
///
/// ```
/// use std::collections::HashMap;
///
/// # #[cfg(feature = "std")]
/// use conquer_once::Lazy;
/// # #[cfg(not(feature = "std"))]
/// # use conquer_once::spin::Lazy;
///
/// static CAPITALS: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
///     let mut map = HashMap::new();
///     map.insert("Norway", "Oslo");
///     map.insert("Belgium", "Brussels");
///     map.insert("Latvia", "Riga");
///     map
/// });
///
/// assert_eq!(CAPITALS.get(&"Norway"), Some(&"Oslo"));
/// assert_eq!(CAPITALS.get(&"Belgium"), Some(&"Brussels"));
/// assert_eq!(CAPITALS.get(&"Latvia"), Some(&"Riga"));
/// ```
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, ParkThread, F>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// OnceCell (type alias)
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(any(test, feature = "std"))]
/// An interior mutability cell type which allows synchronized one-time
/// initialization and read-only access exclusively after initialization.
///
/// This type uses the blocking synchronization mechanism provided by the
/// underlying operating system.
///
/// For the API of this type alias, see the generic
/// [`OnceCell`](crate::doc::OnceCell) type.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "std")]
/// use conquer_once::OnceCell;
/// # #[cfg(not(feature = "std"))]
/// # use conquer_once::spin::OnceCell;
///
/// #[derive(Copy, Clone)]
/// struct Configuration {
///     mode: i32,
///     threshold: u64,
///     msg: &'static str,
/// }
///
/// static CONFIG: OnceCell<Configuration> = OnceCell::uninit();
///
/// // producer thread
/// CONFIG.init_once(|| Configuration {
///     mode: 2,
///     threshold: 128,
///     msg: "..."
/// });
///
/// // consumer thread
/// let res = CONFIG.get().copied();
/// if let Some(config) = res {
///     assert_eq!(config.mode, 2);
///     assert_eq!(config.threshold, 128);
/// }
/// ```
pub type OnceCell<T> = crate::cell::OnceCell<T, ParkThread>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Once (type alias)
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(any(test, feature = "std"))]
/// A synchronization primitive which can be used to run a one-time global
/// initialization.
///
/// This type uses the blocking synchronization mechanism provided by the
/// underlying operating system.
///
/// For the API of this type alias, see the generic
/// [`OnceCell`](crate::doc::OnceCell) type.
/// This is a specialization with `T = ()`.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "std")]
/// use conquer_once::Once;
/// # #[cfg(not(feature = "std"))]
/// # use conquer_once::spin::Once;
///
/// static mut GLOBAL: usize = 0;
/// static INIT: Once = Once::uninit();
///
/// fn get_global() -> usize {
///     // this is safe because the `Once` ensures the `static mut` is assigned
///     // by only one thread and without data races.
///     unsafe {
///         INIT.init_once(|| {
///             GLOBAL = expensive_computation();
///         });
///         # assert_eq!(GLOBAL, 1);
///         GLOBAL
///     }
/// }
///
/// fn expensive_computation() -> usize {
///     // ...
///     # 1
/// }
/// ```
pub type Once = OnceCell<()>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ParkThread
////////////////////////////////////////////////////////////////////////////////////////////////////

mod internal {
    /// Blocking strategy using low-level and OS reliant parking and un-parking
    /// mechanisms.
    #[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
    pub struct ParkThread;
}

/********** impl inherent *************************************************************************/

impl ParkThread {
    #[inline]
    pub(crate) fn try_block_spinning(
        state: &AtomicOnceState,
        back_off: &BackOff,
    ) -> Result<(), BlockedState> {
        loop {
            // (wait:1) this acquire load syncs-with the release swaps (guard:2)
            // and the acq-rel CAS (wait:2)
            match state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
                Ready => return Ok(()),
                WouldBlock(blocked) if back_off.advise_yield() => {
                    back_off.reset();
                    return Err(blocked);
                }
                _ => {}
            }

            back_off.spin();
        }
    }
}

/********** impl Unblock **************************************************************************/

unsafe impl Unblock for ParkThread {
    /// Unblocks all blocked waiting threads.
    #[inline]
    unsafe fn on_unblock(state: BlockedState) {
        let mut curr = state.as_ptr() as *const StackWaiter;
        while !curr.is_null() {
            let thread = {
                let waiter = &*curr;
                curr = waiter.next.get();
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

/********** impl Block ****************************************************************************/

unsafe impl Block for ParkThread {
    /// Blocks (parks) the current thread until it is woken up by the thread with permission to
    /// initialize the `OnceCell`.
    #[inline]
    fn block(state: &AtomicOnceState) {
        let back_off = BackOff::new();
        // spin a little before parking the thread in case the state is quickly unlocked again
        let blocked = match Self::try_block_spinning(state, &back_off) {
            Ok(_) => return,
            Err(blocked) => blocked,
        };

        // create a linked list node on the current threads stack, which is
        // guaranteed to stay alive while the thread is parked.
        let waiter = StackWaiter {
            ready: AtomicBool::new(false),
            thread: Cell::new(Some(thread::current())),
            next: Cell::new(blocked.as_ptr() as *const StackWaiter),
        };

        let mut curr = blocked;
        let head = BlockedState::from(&waiter as *const _);

        while let Err(err) = unsafe { state.try_swap_blocked(curr, head, Ordering::AcqRel) } {
            match err {
                // another parked thread succeeded in placing itself at the queue's front
                WouldBlock(queue) => {
                    // the waiter hasn't been shared yet, so it's still safe to
                    // mutate the next pointer
                    curr = queue;
                    waiter.next.set(queue.as_ptr() as *const StackWaiter);
                    back_off.spin();
                }
                // acquire-release is required here to enforce acquire ordering in the failure case,
                // which guarantees that any (non-atomic) stores to the cell's inner state preceding
                // (guard:2) have become visible, if the function returns;
                // (alternatively an explicit acquire fence could be placed into this path)
                Ready => return,
                Uninit => unreachable!("cell state can not become `UNINIT again`"),
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
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// StackWaiter
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A linked list node that lives on the stack of a parked thread.
#[repr(align(4))]
pub(crate) struct StackWaiter {
    ready: AtomicBool,
    thread: Cell<Option<Thread>>,
    next: Cell<*const StackWaiter>,
}

#[cfg(test)]
mod tests {
    generate_tests_non_blocking!();
    generate_tests!();
}
