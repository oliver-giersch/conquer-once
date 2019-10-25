//! Generic definition and implementation of the [`Once `] and [`OnceCell`]
//! type.

use core::cell::UnsafeCell;
use core::fmt;
use core::marker::PhantomData;
use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::Ordering;

use crate::state::{AtomicOnceState, OnceState, TryBlockError, WaiterQueue};
use crate::{Internal, POISON_PANIC_MSG};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Block (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A sealed trait for abstracting over different blocking strategies.
pub trait Block: Default + Internal {
    /// Blocks the current thread, until `state` is no longer blocking.
    fn block(state: &AtomicOnceState);
    /// Unblocks all waiting threads.
    fn unblock(waiter: WaiterQueue);
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
    pub const fn uninit() -> Self {
        Self {
            state: AtomicOnceState::new(),
            inner: UnsafeCell::new(MaybeUninit::uninit()),
            _marker: PhantomData,
        }
    }

    /// Creates a new [`OnceCell`] pre-initialized with `value`.
    #[inline]
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicOnceState::ready(),
            inner: UnsafeCell::new(MaybeUninit::new(value)),
            _marker: PhantomData,
        }
    }

    /// Consumes `self` and returns a [`Some(T)`](Some) if the [`OnceCell`] has
    /// previously been successfully initialized or [`None`] otherwise.
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
    /// let uninit: OnceCell<i32> = OnceCell::uninit();
    /// assert!(uninit.into_inner().is_none());
    ///
    /// let once = OnceCell::uninit();
    /// once.init_once(|| "initialized");
    /// assert_eq!(once.into_inner(), Some("initialized"));
    /// ```
    #[inline]
    pub fn into_inner(self) -> Option<T> {
        let res = unsafe { self.read_relaxed() };
        mem::forget(self);
        res
    }

    /// Returns true if the [`OnceCell`] has been successfully initialized.
    ///
    /// This method does not panic if the [`OnceCell`] is poisoned and
    /// never blocks.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        // (cell:1) this acquire load syncs-with the acq-rel swap (guard:2)
        self.state.load(Ordering::Acquire) == Ok(OnceState::Ready)
    }

    /// Returns true if the [`OnceCell`] has been poisoned during
    /// initialization.
    ///
    /// This method does not panic if the [`OnceCell`] is poisoned and
    /// never blocks.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.state.load(Ordering::Relaxed).is_err()
    }

    /// Returns a reference to the inner value without checking whether the
    /// [`OnceCell`] is actually initialized.
    ///
    /// # Safety
    ///
    /// The caller has to ensure that the cell has been successfully
    /// initialized, otherwise uninitialized memory will be read.
    ///
    /// # Examples
    ///
    /// This is one safe way to use this method, although
    /// [`try_get`][OnceCell::try_get] is the better alternative:
    ///
    /// ```
    /// use conquer_once::OnceCell;
    ///
    /// // let cell = ...
    /// # let cell = OnceCell::uninit();
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

    #[inline]
    unsafe fn read_relaxed(&self) -> Option<T> {
        match self.state.load(Ordering::Relaxed).expect(POISON_PANIC_MSG) {
            OnceState::Ready => Some(ptr::read(self.get_unchecked())),
            _ => None,
        }
    }
}

impl<T, B: Block> OnceCell<T, B> {
    /// Attempts to initialize the [`OnceCell`] with `func` if it is
    /// uninitialized.
    ///
    /// This method **blocks** if another thread has already begun initializing
    /// the [`OnceCell`] concurrently.
    ///
    /// If the initialization of the [`OnceCell`] has already been
    /// completed previously, this method returns early with minimal
    /// overhead (approximately 0.5ns in some benchmarks).
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    ///
    /// # Examples
    ///
    /// ```
    /// use conquer_once::OnceCell;
    ///
    /// let cell = OnceCell::uninit();
    /// cell.init_once(|| {
    ///     // expensive calculation
    ///     (0..1_000).map(|i| i * i).sum::<usize>()
    /// });
    ///
    /// cell.init_once(|| {
    ///     // any further or concurrent calls to `init_once` will do
    ///     // nothing and return immediately with almost no overhead.
    ///     # 0
    /// });
    ///
    /// # let exp = (0..1_000).map(|i| i * i).sum::<usize>();
    /// # assert_eq!(cell.get().copied(), Some(exp));
    /// ```
    #[inline]
    pub fn init_once(&self, func: impl FnOnce() -> T) {
        if self.is_initialized() {
            return;
        }

        let mut func = Some(func);
        if let Err(TryBlockError::WouldBlock(_)) =
            self.try_init_inner(&mut || func.take().unwrap()())
        {
            B::block(&self.state);
        }
    }

    /// Attempts to initialize the [`OnceCell`] with `func` if is is
    /// uninitialized and returns [`Ok(())`](Ok) only if `func` is successfully
    /// executed.
    ///
    /// This method never blocks.
    ///
    /// # Errors
    ///
    /// This method fails if the initialization of [`OnceCell`] has already been
    /// completed previously, in which case an
    /// [`AlreadyInit`][TryInitError::AlreadyInit] error is returned.
    /// If another thread is concurrently in the process of initializing it and
    /// this thread would have to block, a
    /// [`WouldBlock`][TryInitError::WouldBlock] error is returned.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    ///
    /// # Examples
    ///
    /// ```
    /// use conquer_once::{OnceCell, TryInitError};
    ///
    /// let cell = OnceCell::uninit();
    ///
    /// // .. in thread 1
    /// let res = cell.try_init_once(|| {
    ///     1
    /// });
    /// assert!(res.is_ok());
    ///
    /// // .. in thread 2
    /// let res = cell.try_init_once(|| {
    ///     2
    /// });
    /// assert_eq!(res, Err(TryInitError::AlreadyInit));
    ///
    /// # assert_eq!(cell.get().copied(), Some(1));
    /// ```
    #[inline]
    pub fn try_init_once(&self, func: impl FnOnce() -> T) -> Result<(), TryInitError> {
        if self.is_initialized() {
            return Err(TryInitError::AlreadyInit);
        }

        let mut func = Some(func);
        self.try_init_inner(&mut || func.take().unwrap()())?;
        Ok(())
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// [`None`].
    ///
    /// This method **blocks** if another thread has already begun initializing
    /// the [`OnceCell`] concurrently.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    ///
    /// # Examples
    ///
    /// ```
    /// use conquer_once::OnceCell;
    ///
    /// let cell = OnceCell::uninit();
    /// assert_eq!(cell.get(), None);
    /// cell.init_once(|| {
    ///     1
    /// });
    /// assert_eq!(cell.get(), Some(&1));
    /// ```
    #[inline]
    pub fn get(&self) -> Option<&T> {
        // (cell:2) this acquire load syncs-with the acq-rel swap (guard:2)
        match self.state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
            OnceState::Ready => Some(unsafe { self.get_unchecked() }),
            OnceState::WouldBlock(_) => {
                B::block(&self.state);
                Some(unsafe { self.get_unchecked() })
            }
            OnceState::Uninit => None,
        }
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// an [`Err`].
    ///
    /// This method never blocks.
    ///
    /// # Errors
    ///
    /// This method fails if the [`OnceCell`] is either not initialized
    /// ([`Uninit`][TryGetError::Uninit]) or is currently being
    /// initialized by some other thread
    /// ([`WouldBlock`][TryGetError::WouldBlock]).
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn try_get(&self) -> Result<&T, TryGetError> {
        // (cell:3) this acquire load syncs-with the acq-rel swap (guard:2)
        match self.state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
            OnceState::Ready => Ok(unsafe { self.get_unchecked() }),
            OnceState::Uninit => Err(TryGetError::Uninit),
            OnceState::WouldBlock(_) => Err(TryGetError::WouldBlock),
        }
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// otherwise attempts to initialize it with `func` and return the result.
    ///
    /// This method **blocks** if another thread has already begun
    /// initializing the [`OnceCell`] concurrently.
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn get_or_init(&self, func: impl FnOnce() -> T) -> &T {
        // (cell:4) this acquire load syncs-with the acq-rel swap (guard:2)
        if let OnceState::Ready = self.state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
            return unsafe { self.get_unchecked() };
        }

        let mut func = Some(func);
        match self.try_init_inner(&mut || func.take().unwrap()()) {
            Ok(inner) => inner,
            Err(TryBlockError::AlreadyInit) => unsafe { self.get_unchecked() },
            Err(TryBlockError::WouldBlock(_)) => {
                B::block(&self.state);
                unsafe { self.get_unchecked() }
            }
        }
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// otherwise attempts to initialize it with `func` and return the result.
    ///
    /// This method never blocks.
    ///
    /// # Errors
    ///
    /// This method only fails if the calling thread would have to block in case
    /// another thread is concurrently initializing the [`OnceCell`].
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn try_get_or_init(&self, func: impl FnOnce() -> T) -> Result<&T, WouldBlockError> {
        if self.is_initialized() {
            return Ok(unsafe { self.get_unchecked() });
        }

        let mut func = Some(func);
        let mut once = || func.take().unwrap()();
        Ok(self.try_init_inner(&mut once)?)
    }

    /// This method is annotated with `#[cold]` in order to keep it out of the
    /// fast path.
    #[inline(never)]
    #[cold]
    fn try_init_inner(&self, func: &mut dyn FnMut() -> T) -> Result<&T, TryBlockError> {
        let guard = PanicGuard::<B>::try_block(&self.state)?;
        unsafe {
            let inner = &mut *self.inner.get();
            inner.as_mut_ptr().write(func());
        }
        guard.disarm();

        Ok(unsafe { self.get_unchecked() })
    }
}

/********** impl Debug ****************************************************************************/

impl<T: fmt::Debug, B: Block> fmt::Debug for OnceCell<T, B> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("OnceCell").field("inner", &self.try_get().ok()).finish()
    }
}

/********** impl Drop *****************************************************************************/

impl<T, B> Drop for OnceCell<T, B> {
    #[inline]
    fn drop(&mut self) {
        unsafe { mem::drop(self.read_relaxed()) }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// TryInitError
////////////////////////////////////////////////////////////////////////////////////////////////////

const UNINIT_MSG: &str = "the `OnceCell` is uninitialized";
const ALREADY_INIT_MSG: &str = "the `OnceCell` has already been initialized";
const WOULD_BLOCK_MSG: &str = "the `OnceCell` is currently being initialized";

/// Possible error variants of non-blocking initialization calls.
#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub enum TryInitError {
    /// The [`OnceCell`] is already initialized and the initialization procedure
    /// was not called.
    AlreadyInit,
    /// The [`OnceCell`] is currently being initialized by another thread and
    /// the current thread would have to block.
    WouldBlock,
}

/*********** impl Display *************************************************************************/

impl fmt::Display for TryInitError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TryInitError::AlreadyInit => write!(f, "{}", ALREADY_INIT_MSG),
            TryInitError::WouldBlock => write!(f, "{}", WOULD_BLOCK_MSG),
        }
    }
}

/*********** impl From ****************************************************************************/

impl From<TryBlockError> for TryInitError {
    #[inline]
    fn from(err: TryBlockError) -> Self {
        match err {
            TryBlockError::AlreadyInit => TryInitError::AlreadyInit,
            TryBlockError::WouldBlock(_) => TryInitError::WouldBlock,
        }
    }
}

/*********** impl Error ***************************************************************************/

#[cfg(feature = "std")]
impl std::error::Error for TryInitError {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// TryGetError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Possible error variants of non-blocking fallible get calls.
#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub enum TryGetError {
    /// The [`OnceCell`] is currently not initialized.
    Uninit,
    /// The [`OnceCell`] is currently being initialized by another thread and
    /// the current thread would have to block.
    WouldBlock,
}

/*********** impl Display *************************************************************************/

impl fmt::Display for TryGetError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TryGetError::Uninit => write!(f, "{}", UNINIT_MSG),
            TryGetError::WouldBlock => write!(f, "{}", WOULD_BLOCK_MSG),
        }
    }
}

/*********** impl Error ***************************************************************************/

#[cfg(feature = "std")]
impl std::error::Error for TryGetError {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// WouldBlockError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An error indicating that a [`OnceCell`] would have to block.
#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct WouldBlockError(());

/*********** impl Display *************************************************************************/

impl fmt::Display for WouldBlockError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", WOULD_BLOCK_MSG)
    }
}

/*********** impl From ****************************************************************************/

impl From<TryBlockError> for WouldBlockError {
    #[inline]
    fn from(err: TryBlockError) -> Self {
        match err {
            TryBlockError::AlreadyInit => unreachable!(),
            TryBlockError::WouldBlock(_) => Self(()),
        }
    }
}

/*********** impl Error ***************************************************************************/

#[cfg(feature = "std")]
impl std::error::Error for WouldBlockError {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// PanicGuard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A guard for catching panics during the execution of the initialization
/// closure.
#[derive(Debug)]
struct PanicGuard<'a, B: Block> {
    state: &'a AtomicOnceState,
    poison: bool,
    _marker: PhantomData<B>,
}

impl<'a, B: Block> PanicGuard<'a, B> {
    /// Attempts to block the [`OnceCell`] and return a guard on success.
    #[inline]
    fn try_block(state: &'a AtomicOnceState) -> Result<Self, TryBlockError> {
        // (guard:1) this acquire CAS syncs-with the acq-rel swap (guard:2) and the acq-rel CAS
        // (wait:2)
        state.try_block(Ordering::Acquire)?;
        Ok(Self { state, poison: true, _marker: PhantomData })
    }

    /// Consumes the guard and assures that no panic has occurred.
    #[inline]
    fn disarm(mut self) {
        self.poison = false;
        mem::drop(self);
    }
}

/********** impl Drop *****************************************************************************/

impl<B: Block> Drop for PanicGuard<'_, B> {
    #[inline]
    fn drop(&mut self) {
        // (guard:2) this acq-rel swap syncs-with the acq-rel CAS (wait:2) and the acquire loads
        // (cell:0) through (cell:3), (wait:1) and the acquire CAS (guard:1)
        let waiter_queue = if self.poison {
            self.state.swap_poisoned(Ordering::AcqRel)
        } else {
            self.state.swap_ready(Ordering::AcqRel)
        };

        B::unblock(waiter_queue);
    }
}
