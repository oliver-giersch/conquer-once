use core::ops::Deref;

use crate::cell::OnceCell;
use crate::Block;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Lazy
////////////////////////////////////////////////////////////////////////////////////////////////////

/// TODO: Docs...
#[derive(Debug)]
pub struct Lazy<T, B, F = fn() -> T> {
    cell: OnceCell<T, B>,
    init: F,
}

/********** impl inherent *************************************************************************/

impl<T, B, F> Lazy<T, B, F> {
    /// Creates a new uninitialized [`Lazy`] with the given `init` closure.
    #[inline]
    pub const fn new(init: F) -> Self {
        Self {
            cell: OnceCell::new(),
            init,
        }
    }
}

impl<T, B, F> Lazy<T, B, F>
where
    B: Block,
    F: Fn() -> T,
{
    /// TODO: Docs...
    #[inline]
    pub fn is_initialized(lazy: &Self) -> bool {
        lazy.cell.is_initialized()
    }

    /// TODO: Docs...
    #[inline]
    pub fn is_poisoned(lazy: &Self) -> bool {
        lazy.cell.is_poisoned()
    }

    /// TODO: Docs...
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
