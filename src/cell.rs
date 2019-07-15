//! Generic definition and implementation of the [`Once `] and [`OnceCell`]
//! type.

use core::cell::UnsafeCell;
use core::convert::TryFrom;
use core::fmt::{self, Debug};
use core::marker::PhantomData;
use core::mem::{self, MaybeUninit};
use core::ptr;

use crate::state::{AtomicOnceState, OnceState, TryBlockError};
use crate::{Block, POISON_PANIC_MSG};

////////////////////////////////////////////////////////////////////////////////////////////////////
// OnceCell
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A cell type with interior mutability and one time initialization only.
pub struct OnceCell<T, B> {
    state: AtomicOnceState,
    inner: UnsafeCell<MaybeUninit<T>>,
    _marker: PhantomData<B>,
}

/********** impl Send + Sync **********************************************************************/

unsafe impl<T, B> Send for OnceCell<T, B> where T: Send {}
unsafe impl<T, B> Sync for OnceCell<T, B> where T: Sync {}

/********** impl inherent *************************************************************************/

impl<T, B> OnceCell<T, B> {
    /// Creates a new uninitialized [`OnceCell`].
    #[inline]
    pub const fn new() -> Self {
        Self {
            state: AtomicOnceState::new(),
            inner: UnsafeCell::new(MaybeUninit::uninit()),
            _marker: PhantomData,
        }
    }

    /// Converts `self` into a [`Some(T)`](Some) if the [`OnceCell`] has
    /// previously been successfully initialized and [`None`] otherwise.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn into_inner(self) -> Option<T> {
        let res = match self.state.load().expect(POISON_PANIC_MSG) {
            OnceState::Ready => Some(unsafe { self.read_unchecked() }),
            _ => None,
        };

        mem::forget(self);
        res
    }

    /// Returns true if the [`OnceCell`] has been successfully initialized.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.state.load() == Ok(OnceState::Ready)
    }

    /// Returns true if the [`OnceCell`] has been poisoned during
    /// initialization.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.state.load().is_err()
    }

    /// Returns a reference to the inner value without checking whether the
    /// [`OnceCell`] is actually initialized.
    ///
    /// # Safety
    ///
    /// The caller has to ensure that the cell has been successfully
    /// initialized.
    #[inline]
    pub unsafe fn get_unchecked(&self) -> &T {
        let inner = &*self.inner.get();
        &*inner.as_ptr()
    }

    /// Reads the value in the [`OnceCell`] without checking if it has been
    /// initialized and leaving the cell itself unchanged.
    ///
    /// # Safety
    ///
    /// Similar same safety invariants as with [`ptr::read`] apply to this
    /// method as well.
    #[inline]
    unsafe fn read_unchecked(&self) -> T {
        ptr::read(self.get_unchecked())
    }
}

impl<T, B: Block> OnceCell<T, B> {
    /// Initializes the [`OnceCell`] with `func` or blocks until it is fully
    /// initialized by another thread.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn init_once(&self, func: impl FnOnce() -> T) {
        if let Err(TryBlockError::WouldBlock(waiter)) = self.try_init_inner(func) {
            B::block(&self.state, waiter);
        }
    }

    /// Attempts to initialize the [`OnceCell`] with `func` and returns
    /// [`Ok(())`](Ok) if the initialization closure is successfully executed.
    ///
    /// # Errors
    ///
    /// This method fails if the [`OnceCell`] is either already initialized or
    /// already being initialized by some other thread.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn try_init_once(&self, func: impl FnOnce() -> T) -> Result<(), TryInitError> {
        if self.is_initialized() {
            return Err(TryInitError::AlreadyInit);
        }

        self.try_init_inner(func)?;
        Ok(())
    }

    /// Returns a reference to the inner value if the [`OnceCell`] has been
    /// successfully initialized or blocks until an already begun initialization
    /// is complete.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn get(&self) -> Option<&T> {
        match self.state.load().expect(POISON_PANIC_MSG) {
            OnceState::WouldBlock(waiter) => {
                B::block(&self.state, waiter);
                Some(unsafe { self.get_unchecked() })
            }
            OnceState::Ready => Some(unsafe { self.get_unchecked() }),
            _ => None,
        }
    }

    /// Returns a reference to the inner value if the [`OnceCell`] has been
    /// successfully initialized.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    pub fn try_get(&self) -> Option<&T> {
        match self.state.load().expect(POISON_PANIC_MSG) {
            OnceState::Ready => Some(unsafe { self.get_unchecked() }),
            _ => None,
        }
    }

    /// Returns a reference to the inner value if the [`OnceCell`] has been
    /// successfully initialized and otherwise attempts to call `func` in order
    /// to initialize the [`OnceCell`].
    ///
    /// This method is potentially blocking, if another thread is concurrently
    /// attempting to initialize the same [`OnceCell`].
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn get_or_init(&self, func: impl FnOnce() -> T) -> &T {
        if let OnceState::Ready = self.state.load().expect(POISON_PANIC_MSG) {
            return unsafe { self.get_unchecked() };
        }

        match self.try_init_inner(func) {
            Ok(inner) => inner,
            Err(TryBlockError::AlreadyInit) => unsafe { self.get_unchecked() },
            Err(TryBlockError::WouldBlock(waiter)) => {
                B::block(&self.state, waiter);
                unsafe { self.get_unchecked() }
            }
        }
    }

    /// Returns a reference to the inner value if the [`OnceCell`] has been
    /// successfully initialized and otherwise attempts to call `func` in order
    /// to initialize the [`OnceCell`].
    ///
    /// Instead of blocking, this method returns a [`WouldBlock`](TryInitError::WouldBlock) error,
    /// if another thread is concurrently attempting to initialize the same
    /// [`OnceCell`].
    ///
    /// # Errors
    ///
    /// This method returns an [`Err`] if the [`OnceCell`] is either not
    /// initialized or the thread would have to block.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn get_or_try_init(&self, func: impl FnOnce() -> T) -> Result<&T, TryInitError> {
        if self.is_initialized() {
            return Ok(unsafe { self.get_unchecked() });
        }

        Ok(self.try_init_inner(func)?)
    }

    #[cold]
    fn try_init_inner(&self, func: impl FnOnce() -> T) -> Result<&T, TryBlockError> {
        let guard = PanicGuard::<B>::try_from(&self.state)?;
        unsafe {
            let inner = &mut *self.inner.get();
            inner.as_mut_ptr().write(func());
        }
        guard.disarm();

        Ok(unsafe { self.get_unchecked() })
    }
}

/********** impl Debug ****************************************************************************/

impl<T: Debug, B> Debug for OnceCell<T, B> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut debug = f.debug_struct("OnceCell");
        if self.is_initialized() {
            debug
                .field("initialized", &true)
                .field("inner", unsafe { self.get_unchecked() });
        } else {
            debug.field("initialized", &false);
        }

        debug.finish()
    }
}

/********** impl Drop *****************************************************************************/

impl<T, B> Drop for OnceCell<T, B> {
    #[inline]
    fn drop(&mut self) {
        if self.is_initialized() {
            unsafe { mem::drop(self.read_unchecked()) }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// TryInitError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Possible error variants of non-blocking initialization calls.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TryInitError {
    /// The [`OnceCell`] is already initialized and the initialization procedure
    /// was not called.
    AlreadyInit,
    /// The [`OnceCell`] is currently being initialized by another thread and
    /// the current thread would have to block.
    WouldBlock,
}

impl From<TryBlockError> for TryInitError {
    #[inline]
    fn from(err: TryBlockError) -> Self {
        match err {
            TryBlockError::AlreadyInit => TryInitError::AlreadyInit,
            TryBlockError::WouldBlock(_) => TryInitError::WouldBlock,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// PanicGuard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A guard for catching panics during the execution of the initialization
/// closure.
struct PanicGuard<'a, B: Block> {
    has_panicked: bool,
    state: &'a AtomicOnceState,
    _marker: PhantomData<B>,
}

impl<'a, B: Block> PanicGuard<'a, B> {
    /// Consumes the guard and assures that no panic has occurred.
    #[inline]
    fn disarm(mut self) {
        self.has_panicked = false;
        mem::drop(self);
    }
}

/********** impl TryFrom **************************************************************************/

impl<'a, B: Block> TryFrom<&'a AtomicOnceState> for PanicGuard<'a, B> {
    type Error = TryBlockError;

    #[inline]
    fn try_from(state: &'a AtomicOnceState) -> Result<Self, Self::Error> {
        state.try_block()?;
        Ok(Self {
            has_panicked: true,
            state,
            _marker: PhantomData,
        })
    }
}

/********** impl Drop *****************************************************************************/

impl<B: Block> Drop for PanicGuard<'_, B> {
    #[inline]
    fn drop(&mut self) {
        let waiters = if self.has_panicked {
            self.state.swap_poisoned()
        } else {
            self.state.swap_ready()
        };

        B::unblock(waiters);
    }
}
