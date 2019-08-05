//! Generic definition and implementation of the [`Once `] and [`OnceCell`]
//! type.

use core::cell::UnsafeCell;
use core::convert::TryFrom;
use core::fmt::{self, Debug};
use core::marker::PhantomData;
use core::mem::{self, MaybeUninit};
use core::ptr;

use crate::state::{AtomicOnceState, OnceState, TryBlockError, Waiter};
use crate::{Internal, POISON_PANIC_MSG};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Block (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A sealed trait for abstracting over different blocking strategies.
pub trait Block: Default + Internal {
    /// Blocks the current thread, until `state` is no longer blocking.
    fn block(state: &AtomicOnceState);
    /// Unblocks all waiting threads.
    fn unblock(waiter: Waiter);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// OnceCell
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An interior mutability cell type which allows synchronized one-time
/// initialization and read-only access exclusively after initialization.
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
    ///
    /// # Examples
    ///
    /// ```
    /// # use conquer_once::spin::OnceCell;
    ///
    /// let uninit: OnceCell<i32> = OnceCell::new();
    /// assert!(uninit.into_inner().is_none());
    ///
    /// let once = OnceCell::new();
    /// once.init_once(|| "initialized");
    /// assert_eq!(once.into_inner(), Some("initialized"));
    /// ```
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
    ///
    /// # Notes
    ///
    /// This method does not panic if the [`OnceCell`] is poisoned.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.state.load() == Ok(OnceState::Ready)
    }

    /// Returns true if the [`OnceCell`] has been poisoned during
    /// initialization.
    ///
    /// # Notes
    ///
    /// This method does not panic if the [`OnceCell`] is poisoned.
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
    ///
    /// # Examples
    ///
    /// This is a safe way to use this method:
    ///
    /// ```
    /// use conquer_once::OnceCell;
    ///
    /// // let cell = ...
    /// # let cell = OnceCell::new();
    /// # cell.init_once(|| 0);
    ///
    /// let res = if cell.is_initialized() {
    ///     Some(unsafe { cell.get_unchecked() })
    /// } else {
    ///     None
    /// };
    ///
    /// # assert_eq!(res, Some(&0));
    /// ```
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
        if self.is_initialized() {
            return;
        }

        if let Err(TryBlockError::WouldBlock(_)) = self.try_init_inner(func) {
            B::block(&self.state);
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
            OnceState::Ready => Some(unsafe { self.get_unchecked() }),
            OnceState::WouldBlock(_) => {
                B::block(&self.state);
                Some(unsafe { self.get_unchecked() })
            }
            OnceState::Uninit => None,
        }
    }

    /// Returns a reference to the inner value if the [`OnceCell`] has been
    /// successfully initialized.
    ///
    /// # Errors
    ///
    /// This method fails if the [`OnceCell`] is either not initialized or
    /// is currently being initialized by some other thread.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn try_get(&self) -> Result<&T, TryGetError> {
        match self.state.load().expect(POISON_PANIC_MSG) {
            OnceState::Ready => Ok(unsafe { self.get_unchecked() }),
            OnceState::Uninit => Err(TryGetError::Uninit),
            OnceState::WouldBlock(_) => Err(TryGetError::WouldBlock),
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
            Err(TryBlockError::WouldBlock(_)) => {
                B::block(&self.state);
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
    pub fn try_get_or_init(&self, func: impl FnOnce() -> T) -> Result<&T, TryInitError> {
        if self.is_initialized() {
            return Ok(unsafe { self.get_unchecked() });
        }

        Ok(self.try_init_inner(func)?)
    }

    /// Attempts to lock the [`OnceCell`] and execute the initialization `func`.
    ///
    /// If `func` panics, the [`OnceCell`] will be poisoned.
    ///
    /// # Errors
    ///
    /// This method returns an [`Err`] if the [`OnceCell`] is either already
    /// initialized or if the calling thread would have to block.
    ///
    /// # Notes
    ///
    /// This method is annotated with `#[cold]` in order to keep it out of the
    /// fast path.
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
            debug.field("initialized", &true).field("inner", unsafe { self.get_unchecked() });
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
// TryGetError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Possible error variants of non-blocking fallible get calls.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TryGetError {
    /// The [`OnceCell`] is currently not initialized.
    Uninit,
    /// The [`OnceCell`] is currently being initialized by another thread and
    /// the current thread would have to block.
    WouldBlock,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// PanicGuard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A guard for catching panics during the execution of the initialization
/// closure.
#[derive(Debug)]
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
        Ok(Self { has_panicked: true, state, _marker: PhantomData })
    }
}

/********** impl Drop *****************************************************************************/

impl<B: Block> Drop for PanicGuard<'_, B> {
    #[inline]
    fn drop(&mut self) {
        let waiters =
            if self.has_panicked { self.state.swap_poisoned() } else { self.state.swap_ready() };

        B::unblock(waiters);
    }
}
