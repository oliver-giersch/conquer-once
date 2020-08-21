pub mod helper {
    use std::cell::Cell;

    pub(crate) struct DropGuard<'a>(pub &'a Cell<u32>);

    impl Drop for DropGuard<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
}

macro_rules! generate_tests_non_blocking {
    () => {
        use std::sync::{Arc, Barrier, RwLock};
        use std::time::Duration;
        use std::thread;

        use crate::{TryInitError, TryGetError};

        use super::OnceCell;

        const THREADS: usize = 8;

        #[test]
        fn once_cell_uninit() {
            static CELL: OnceCell<usize> = OnceCell::uninit();
            assert!(!CELL.is_initialized());
            assert!(!CELL.is_poisoned());

            let cell: OnceCell<i32> = OnceCell::uninit();
            assert!(!cell.is_initialized());
            assert!(!cell.is_poisoned());
            assert_eq!(cell.into_inner(), None);
        }

        #[test]
        fn once_cell_try_init_once() {
            let barrier = Arc::new(Barrier::new(THREADS + 1));
            let cell = Arc::new(OnceCell::uninit());

            let handles: Vec<_> = (0..THREADS)
                .map(|id| {
                    let barrier = Arc::clone(&barrier);
                    let cell = Arc::clone(&cell);
                    thread::spawn(move || {
                        barrier.wait();
                        assert!(cell.try_init_once(|| id + 1).is_err());
                    })
                })
                .collect();

            let res = cell.try_init_once(|| {
                barrier.wait();
                0
            });

            assert!(res.is_ok());
            assert_eq!(cell.try_get(), Ok(&0));

            for handle in handles {
                handle.join().unwrap();
            }
        }

        #[test]
        fn once_cell_try_init_and_try_get() {
            static CELL: OnceCell<usize> = OnceCell::uninit();
            assert_eq!(CELL.try_get(), Err(TryGetError::Uninit));

            let barrier = Arc::new(Barrier::new(THREADS));
            let winner = Arc::new(RwLock::new(None));

            let handles: Vec<_> = (0..THREADS)
                .map(|id| {
                    let barrier = Arc::clone(&barrier);
                    let winner = Arc::clone(&winner);
                    thread::spawn(move || {
                        assert_eq!(CELL.try_get(), Err(TryGetError::Uninit));

                        barrier.wait();

                        let res = CELL.try_init_once(|| id);
                        match res {
                            Ok(_) => {
                                let mut guard = winner.write().unwrap();
                                *guard = Some(id);
                            }
                            Err(TryInitError::AlreadyInit) => {
                                let id = match CELL.try_get() {
                                    Ok(id) => *id,
                                    Err(_) => panic!("`try_get` should not fail after previously returning `AlreadyInit`"),
                                };

                                loop {
                                    let observed = {
                                        let reader = winner.read().unwrap();
                                        *reader
                                    };

                                    match observed {
                                        Some(observed) => {
                                            assert_eq!(id, observed);
                                            return;
                                        },
                                        None => thread::sleep(Duration::from_millis(10)),
                                    };
                                }
                            }
                            Err(TryInitError::WouldBlock) => {
                                let id = loop {
                                    match CELL.try_get() {
                                        Err(e) => {
                                            assert_eq!(e, TryGetError::WouldBlock);
                                            thread::sleep(Duration::from_millis(10));
                                        },
                                        Ok(id) => break *id,
                                    }
                                };

                                loop {
                                    let observed = {
                                        let reader = winner.read().unwrap();
                                        *reader
                                    };

                                    match observed {
                                        Some(observed) => {
                                            assert_eq!(id, observed);
                                            return;
                                        },
                                        None => thread::sleep(Duration::from_millis(10)),
                                    };
                                }
                            }
                        }
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap();
            }
        }

        #[test]
        fn recursive() {
            let cell = OnceCell::uninit();
            let res = cell.try_init_once(|| {
                let res = cell.try_init_once(|| 1);
                assert_eq!(res, Err(TryInitError::WouldBlock));
                1
            });

            assert_eq!(res, Ok(()));
            assert_eq!(cell.try_get(), Ok(&1));
        }
    };
}

macro_rules! generate_tests {
    () => {
        use std::cell::Cell;
        use std::collections::HashMap;
        use std::mem::drop;
        use std::panic::{self, AssertUnwindSafe};
        use std::sync::Mutex;

        use crate::tests::helper::DropGuard;

        use super::Lazy;

        #[test]
        #[should_panic]
        fn drop_on_panic() {
            let cell = OnceCell::uninit();
            // must not panic again when cell is dropped
            cell.init_once(|| panic!("explicit panic"));
        }

        #[test]
        fn once_cell_new() {
            let cell = OnceCell::new(1);
            assert!(cell.is_initialized());
            assert!(!cell.is_poisoned());
            assert_eq!(cell.into_inner(), Some(1));
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

        #[test]
        #[should_panic]
        fn once_cell_into_inner_poisoned() {
            let cell = Arc::new(OnceCell::uninit());

            let thread_cell = Arc::clone(&cell);
            thread::spawn(move || thread_cell.init_once(|| panic!("explicit panic")))
                .join()
                .unwrap_err();

            let cell = Arc::try_unwrap(cell).unwrap();
            assert!(cell.is_poisoned());
            cell.into_inner();
        }

        #[test]
        fn once_cell_init_once() {
            let cell = OnceCell::uninit();
            cell.init_once(|| 1);

            assert!(cell.is_initialized());
            assert!(!cell.is_poisoned());
            assert_eq!(cell.into_inner(), Some(1));
        }

        #[test]
        fn once_cell_block() {
            let barrier = Arc::new(Barrier::new(THREADS + 1));
            let cell = Arc::new(OnceCell::uninit());

            let handles: Vec<_> = (0..THREADS)
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

        #[test]
        #[should_panic]
        fn once_cell_propagate_poison_from_block() {
            let barrier = Arc::new(Barrier::new(THREADS + 1));
            let cell = Arc::new(OnceCell::uninit());

            let handles: Vec<_> = (0..THREADS)
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
        fn lazy_mutex() {
            static MUTEX: Lazy<Mutex<Vec<i32>>> = Lazy::new(Mutex::default);

            let mut lock = MUTEX.lock().unwrap();

            lock.push(1);
            lock.push(2);
            lock.push(3);

            assert_eq!(lock.as_slice(), &[1, 2, 3]);
            lock.clear();
        }

        #[test]
        fn lazy() {
            static MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
                let mut map = HashMap::new();
                map.insert("A", "Hello");
                map.insert("B", "World");
                map
            });

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
        fn catch_panic() {
            static PANIC: Lazy<i32> = Lazy::new(|| panic!("explicit panic"));

            assert!(!Lazy::is_poisoned(&PANIC));
            assert!(std::panic::catch_unwind(|| *PANIC).is_err());
            assert!(std::panic::catch_unwind(|| *PANIC).is_err());
            assert!(Lazy::is_poisoned(&PANIC));
        }
    };
}
