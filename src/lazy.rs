//! Generic definition and implementation of the [`Lazy`] type.

use core::borrow::Borrow;
use core::convert::AsRef;
use core::fmt;
use core::mem::ManuallyDrop;
use core::ops::Deref;
use core::ptr;

use crate::cell::{Block, OnceCell};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Lazy
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A type for lazy initialization of e.g. global static variables, which
/// provides the same functionality as the `lazy_static!` macro.
pub struct Lazy<T, B, F = fn() -> T> {
    /// The cell storing the lazily initialized value.
    cell: OnceCell<T, B>,
    /// The initialization function or closure;
    /// this is wrapped in a [`ManuallyDrop`] so that [`FnOnce`] closures can
    /// be used as well.
    init: ManuallyDrop<F>,
}

/********** impl inherent *************************************************************************/

impl<T, B, F> Lazy<T, B, F> {
    /// Creates a new uninitialized [`Lazy`] with the given `init` closure.
    ///
    /// # Examples
    ///
    /// The `init` argument can be a simple function pointer or any [`FnOnce`]
    /// closure.
    ///
    /// ```
    /// # #[cfg(feature = "std")]
    /// use conquer_once::Lazy;
    /// # #[cfg(not(feature = "std"))]
    /// # use conquer_once::spin::Lazy;
    ///
    /// static LAZY_1: Lazy<Vec<i32>> = Lazy::new(|| vec![1, 2, 3, 4, 5]);
    /// static LAZY_2: Lazy<Vec<i32>> = Lazy::new(Vec::<i32>::new);
    /// ```
    #[inline]
    pub const fn new(init: F) -> Self {
        Self { cell: OnceCell::uninit(), init: ManuallyDrop::new(init) }
    }
}

impl<T, B, F> Lazy<T, B, F>
where
    B: Block,
    F: FnOnce() -> T,
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
    /// This has the same effect as using the [`Deref`] operator on a [`Lazy`].
    ///
    /// # Panics
    ///
    /// This method panics if the `init` procedure specified during construction
    /// panics or if the [`Lazy`] is poisoned.
    #[inline]
    pub fn get_or_init(lazy: &Self) -> &T {
        lazy.cell.get_or_init(|| {
            let func = unsafe { ptr::read(&*lazy.init) };
            func()
        })
    }
}

/********** impl AsRef ****************************************************************************/

impl<T, B, F> AsRef<T> for Lazy<T, B, F>
where
    B: Block,
    F: FnOnce() -> T,
{
    #[inline]
    fn as_ref(&self) -> &T {
        Lazy::get_or_init(self)
    }
}

/********** impl Borrow ***************************************************************************/

impl<T, B, F> Borrow<T> for Lazy<T, B, F>
where
    B: Block,
    F: FnOnce() -> T,
{
    #[inline]
    fn borrow(&self) -> &T {
        Lazy::get_or_init(self)
    }
}

/********** impl Debug ****************************************************************************/

impl<T, B, F> fmt::Debug for Lazy<T, B, F>
where
    T: fmt::Debug,
    B: Block,
    F: FnOnce() -> T,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.cell, f)
    }
}

/********** impl Display **************************************************************************/

impl<T, B, F> fmt::Display for Lazy<T, B, F>
where
    T: fmt::Display,
    B: Block,
    F: FnOnce() -> T,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(Self::get_or_init(self), f)
    }
}

/********** impl Deref ****************************************************************************/

impl<T, B, F> Deref for Lazy<T, B, F>
where
    B: Block,
    F: FnOnce() -> T,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        Lazy::get_or_init(self)
    }
}
