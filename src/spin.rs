//! Synchronized one-time and lazy initialization primitives that use spin-locks
//! in case of concurrent accesses under contention.

use core::sync::atomic::spin_loop_hint;

use crate::internal::Internal;
use crate::state::{AtomicState, State::WouldBlock, Waiter};
use crate::{Block, POISON_PANIC_MSG};

/// TODO: Docs...
pub type Lazy<T, F = fn() -> T> = crate::lazy::Lazy<T, Spin, F>;
/// TODO: Docs...
pub type OnceCell<T> = crate::cell::OnceCell<T, Spin>;
/// TODO: Docs...
pub type Once = OnceCell<()>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Spin
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Spin;

/********** impl Internal *************************************************************************/

impl Internal for Spin {}

/********** impl Block ****************************************************************************/

impl Block for Spin {
    #[inline]
    fn block(state: &AtomicState, _: Waiter) {
        while let WouldBlock(_) = state.load().expect(POISON_PANIC_MSG) {
            spin_loop_hint()
        }
    }

    #[inline]
    fn unblock(_: Waiter) {}
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::{Lazy, OnceCell};

    static MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
        let mut map = HashMap::new();
        map.insert("A", "Hello");
        map.insert("B", "World");
        map
    });

    #[test]
    fn lazy() {
        assert_eq!(MAP.get(&"A"), Some(&"Hello"));
        assert_eq!(MAP.get(&"B"), Some(&"World"));
    }

    #[test]
    fn lazy_poisoned() {
        static POISONED: Lazy<HashMap<&str, &str>> = Lazy::new(|| panic!("explicit init panic"));

        thread::spawn(|| {
            let _map = &*POISONED;
        })
        .join()
        .unwrap_err();

        assert!(Lazy::is_poisoned(&POISONED));
    }

    #[test]
    fn once_cell_block() {
        const WAITERS: usize = 4;

        let barrier = Arc::new(Barrier::new(WAITERS + 1));
        let cell = Arc::new(OnceCell::new());

        let handles: Vec<_> = (0..WAITERS)
            .map(|id| {
                let barrier = Arc::clone(&barrier);
                let cell = Arc::clone(&cell);
                thread::spawn(move || {
                    barrier.wait();
                    // all threads block and have to wait for the main thread to
                    // initialize the cell
                    let res = cell.get_or_init(|| id + 1);
                    assert_eq!(*res, 0);
                })
            })
            .collect();

        let res = cell.get_or_init(|| {
            barrier.wait();
            0
        });

        assert_eq!(*res, 0);

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[should_panic]
    #[test]
    fn once_cell_propagate_poison_from_block() {
        const WAITERS: usize = 4;

        let barrier = Arc::new(Barrier::new(WAITERS + 1));
        let cell = Arc::new(OnceCell::new());

        let handles: Vec<_> = (0..WAITERS)
            .map(|id| {
                let barrier = Arc::clone(&barrier);
                let cell = Arc::clone(&cell);
                thread::spawn(move || {
                    barrier.wait();
                    let _res = cell.get_or_init(|| id + 1);
                })
            })
            .collect();

        let panic = panic::catch_unwind(AssertUnwindSafe(|| {
            cell.init_once(|| {
                barrier.wait();
                panic!("explicit panic")
            });
        }))
        .unwrap_err();

        for handle in handles {
            handle.join().unwrap_err();
        }

        panic::resume_unwind(panic);
    }
}
