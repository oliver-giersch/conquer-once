macro_rules! generate_tests {
    () => {
        use std::cell::Cell;
        use std::collections::HashMap;
        use std::mem::drop;
        use std::panic::{self, AssertUnwindSafe};
        use std::sync::{Arc, Barrier, Mutex};
        use std::thread;

        use super::{Lazy, OnceCell};

        struct DropGuard<'a>(&'a Cell<u32>);

        impl Drop for DropGuard<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        static MUTEX: Lazy<Mutex<Vec<i32>>> = Lazy::new(Mutex::default);

        static MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
            let mut map = HashMap::new();
            map.insert("A", "Hello");
            map.insert("B", "World");
            map
        });

        #[test]
        fn lazy_mutex() {
            let mut lock = MUTEX.lock().unwrap();

            lock.push(1);
            lock.push(2);
            lock.push(3);

            assert_eq!(lock.as_slice(), &[1, 2, 3]);
            lock.clear();
        }

        #[test]
        fn lazy() {
            assert_eq!(MAP.get(&"A"), Some(&"Hello"));
            assert_eq!(MAP.get(&"B"), Some(&"World"));
        }

        #[test]
        fn lazy_poisoned() {
            static POISONED: Lazy<HashMap<&str, &str>> =
                Lazy::new(|| panic!("explicit init panic"));

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
            let cell = Arc::new(OnceCell::uninit());

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
            let cell = Arc::new(OnceCell::uninit());

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

        #[test]
        fn once_cell_into_inner() {
            let count = Cell::new(0);

            let cell = OnceCell::uninit();
            cell.init_once(|| DropGuard(&count));

            let inner = cell.into_inner().unwrap();
            drop(inner);
            assert_eq!(count.get(), 1);
        }

        #[test]
        fn once_cell_drop() {
            let count = Cell::new(0);

            let cell = OnceCell::uninit();
            cell.init_once(|| DropGuard(&count));

            drop(cell);
            assert_eq!(count.get(), 1);
        }
    };
}

generate_tests!();

#[test]
fn catch_panic() {
    static PANIC: Lazy<i32> = Lazy::new(|| panic!("explicit panic"));

    assert!(!Lazy::is_poisoned(&PANIC));
    assert!(std::panic::catch_unwind(|| *PANIC).is_err());
    assert!(std::panic::catch_unwind(|| *PANIC).is_err());
    assert!(Lazy::is_poisoned(&PANIC));
}
