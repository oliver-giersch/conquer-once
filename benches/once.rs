#![feature(test)]

extern crate test;

use test::Bencher;

use conquer_once::OnceCell;

#[bench]
fn get_uninit(b: &mut Bencher) {
    b.iter(|| {
        let once: OnceCell<Vec<i32>> = OnceCell::new();
        for _ in 0..1_000 {
            test::black_box(once.get());
        }
    });
}

#[bench]
fn init_once_single(b: &mut Bencher) {
    b.iter(|| {
        let once = OnceCell::new();
        once.init_once(|| vec![1; 1 << 14])
    });
}

#[bench]
fn init_once_multi(b: &mut Bencher) {
    b.iter(|| {
        let once = OnceCell::new();
        for _ in 0..1_000_000 {
            once.init_once(|| vec![1; 1 << 14]);
        }
    });
}
