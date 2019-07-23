//! Generic definition and implementation of the [`Lazy`] type.

use core::ops::Deref;

use crate::cell::{Block, OnceCell};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Lazy
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A type for lazy initialization of e.g. global static variables, which
/// provides the same functionality as the `lazy_static!` macro.
#[derive(Debug)]
pub struct Lazy<T, B, F = fn() -> T> {
    cell: OnceCell<T, B>,
    init: F,
}

/********** impl inherent *************************************************************************/

impl<T, B, F> Lazy<T, B, F> {
    /// Creates a new uninitialized [`Lazy`] with the given `init` closure.
    ///
    /// # Examples
    ///
    /// The `init` argument can be either a function pointer or a [`Fn`]
    /// closure.
    ///
    /// ```
    /// use conquer_once::Lazy;
    ///
    /// static LAZY_1: Lazy<Vec<i32>> = Lazy::new(|| vec![1, 2, 3, 4, 5]);
    /// static LAZY_2: Lazy<Vec<i32>> = Lazy::new(Vec::<i32>::new);
    /// ```
    #[inline]
    pub const fn new(init: F) -> Self {
        Self { cell: OnceCell::new(), init }
    }
}

impl<T, B, F> Lazy<T, B, F>
where
    B: Block,
    F: Fn() -> T,
{
    /// Returns `true` if the [`Lazy`] has been successfully initialized.
    #[inline]
    pub fn is_initialized(lazy: &Self) -> bool {
        lazy.cell.is_initialized()
    }

    /// Returns `true` if the [`Lazy`] has been poisoned.
    #[inline]
    pub fn is_poisoned(lazy: &Self) -> bool {
        lazy.cell.is_poisoned()
    }

    /// Returns a reference to the already initialized inner value or
    /// initializes it first.
    ///
    /// This has the same effect as using the `deref` operator on a [`Lazy`].
    #[inline]
    pub fn get_or_init(lazy: &Self) -> &T {
        lazy.cell.get_or_init(|| (lazy.init)())
    }
}

/********** impl Deref ****************************************************************************/

impl<T, B, F> Deref for Lazy<T, B, F>
where
    B: Block,
    F: Fn() -> T,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        Lazy::get_or_init(self)
    }
}
