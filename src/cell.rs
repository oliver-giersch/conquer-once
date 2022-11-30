//! Generic definition and implementation of the [`OnceCell`] type.

use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ptr,
    sync::atomic::Ordering,
};

use crate::{
    state::{AtomicOnceState, BlockedState, OnceState, SwapState, TryBlockError},
    POISON_PANIC_MSG,
};

/// An internal (sealed) trait specifying the unblocking mechanism of a cell
/// locking strategy.
pub trait Unblock {
    /// Unblocks all waiting threads after setting the cell state to `READY`.
    ///
    /// # Safety
    ///
    /// Must only be called after swapping the cell with `READY` with the state
    /// returned by the swap operation.
    unsafe fn on_unblock(state: BlockedState);
}

/// An internal (sealed) trait specifying the blocking mechanism of a cell
/// locking strategy.
///
/// # Safety
///
/// The implementation of `block` must adhere to its documentation or otherwise
/// data-races could occur in other code.
pub unsafe trait Block: Unblock {
    /// Blocks the current thread until `state` is either `READY` or `POISONED`.
    ///
    /// # Panics
    ///
    /// Panics if the `state` becomes poisoned.
    fn block(state: &AtomicOnceState);
}

/// An interior mutability cell type which allows synchronized one-time
/// initialization and read-only access exclusively after initialization.
///
/// # Poisoning
///
/// A thread that panics in the course of executing its `init` function or
/// closure **poisons** the cell.
/// All subsequent accesses to a poisoned cell will propagate this and panic
/// themselves.
pub struct OnceCell<T, B> {
    /// The current initialization status.
    state: AtomicOnceState,
    /// The internal and potentially uninitialized value.
    inner: UnsafeCell<MaybeUninit<T>>,
    /// A marker for the blocking strategy (i.e. OS-level block or spin-lock)
    _marker: PhantomData<B>,
}

unsafe impl<T, B> Send for OnceCell<T, B> where T: Send {}
unsafe impl<T, B> Sync for OnceCell<T, B> where T: Send + Sync {}

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
    pub fn into_inner(mut self) -> Option<T> {
        // SAFETY: `take_inner` can not be called again after this, since we
        // forget `self` right after calling it
        let res = unsafe { self.take_inner(false) };
        mem::forget(self);
        res
    }

    /// Returns true if the [`OnceCell`] has been successfully initialized.
    ///
    /// This method will never panic or block.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        // (cell:1) this acquire load syncs-with the acq-rel swap (guard:2)
        self.state.load(Ordering::Acquire) == Ok(OnceState::Ready)
    }

    /// Returns true if the [`OnceCell`] has been poisoned during
    /// initialization.
    ///
    /// This method will never panic or block.
    ///
    /// Once this method has returned `true` all other means of accessing the
    /// [`OnceCell`] except for further calls to
    /// [`is_initialized`][OnceCell::is_initialized] or
    /// [`is_poisoned`][OnceCell::is_poisoned] will panic.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.state.load(Ordering::Relaxed).is_err()
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// an [`Err`].
    ///
    /// This method will never blocks.
    ///
    /// When this function returns with an [`Ok`] result, it is guaranteed that
    /// some initialization closure has run and completed.
    /// It is also guaranteed that any memory writes performed by the executed
    /// closure can be reliably observed by other threads at this point (there
    /// is a happens-before relation between the closure and code executing
    /// after the return).
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
        // (cell:2) this acquire load syncs-with the acq-rel swap (guard:2)
        match self.state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
            OnceState::Ready => Ok(unsafe { self.get_unchecked() }),
            OnceState::Uninit => Err(TryGetError::Uninit),
            OnceState::WouldBlock(_) => Err(TryGetError::WouldBlock),
        }
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
    /// # #[cfg(feature = "std")]
    /// use conquer_once::OnceCell;
    /// # #[cfg(not(feature = "std"))]
    /// # use conquer_once::spin::OnceCell;
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

    /// Moves the inner cell value out of the [`OnceCell`] and returns it
    /// wrapped in [`Some`], if it has been successfully initialized.
    ///
    /// # Safety
    ///
    /// If the cell is initialized, this must be called at most once.
    ///
    /// # Panics
    ///
    /// If `ignore_poisoning` is `false`, this method will panic if the
    /// [`OnceCell`] has been poisoned, otherwise it will simply return
    /// [`None`].
    #[inline]
    unsafe fn take_inner(&mut self, ignore_poisoning: bool) -> Option<T> {
        #[allow(clippy::match_wild_err_arm)]
        match self.state.load(Ordering::Relaxed) {
            Err(_) if !ignore_poisoning => panic!("{}", POISON_PANIC_MSG),
            // SAFETY: the mutable reference guarantees there can be no aliased
            // reference and since the state is `Ready`
            Ok(OnceState::Ready) =>
            {
                #[allow(unused_unsafe)]
                Some(unsafe { ptr::read(self.get_unchecked()) })
            }
            _ => None,
        }
    }
}

impl<T, B: Unblock> OnceCell<T, B> {
    /// Attempts to initialize the [`OnceCell`] with `func` if is is
    /// uninitialized and returns [`Ok(())`](Ok) only if `func` is successfully
    /// executed.
    ///
    /// This method will never block.
    ///
    /// When this function returns with an [`Ok`] or
    /// [`Err(AlreadyInit)`][TryInitError::AlreadyInit] result, it is guaranteed
    /// that *some* initialization closure has run and completed (it may not be
    /// the closure specified).
    /// It is also guaranteed that any memory writes performed by the executed
    /// closure can be reliably observed by other threads at this point (there
    /// is a *happens-before* relation between the closure and code executing
    /// after the return).
    ///
    /// # Errors
    ///
    /// This method fails, if the initialization of [`OnceCell`] has already
    /// been completed previously, in which case an
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
    /// # #[cfg(feature = "std")]
    /// use conquer_once::{OnceCell, TryInitError};
    /// # #[cfg(not(feature = "std"))]
    /// # use conquer_once::{spin::OnceCell, TryInitError};
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
        // (cell:3) this acq load syncs-with the acq-rel swap (guard:2)
        match self.state.load(Ordering::Acquire).expect(POISON_PANIC_MSG) {
            OnceState::Ready => Err(TryInitError::AlreadyInit),
            OnceState::WouldBlock(_) => Err(TryInitError::WouldBlock),
            OnceState::Uninit => {
                let mut func = Some(func);
                self.try_init_inner(&mut || func.take().unwrap()())?;
                Ok(())
            }
        }
    }

    /// This method is annotated with `#[cold]` in order to keep it out of the
    /// fast path.
    #[inline(never)]
    #[cold]
    fn try_init_inner(&self, func: &mut dyn FnMut() -> T) -> Result<&T, TryBlockError> {
        // sets the state to blocked (i.e. guarantees mutual exclusion) or
        // returns with an error.
        let guard = PanicGuard::<B>::try_block(&self.state)?;
        // SAFETY: `try_block` ensures mutual exclusion, so no aliasing of the
        // cell is possible and the raw pointer write is unproblematic (pointer
        // is trivially aligned and valid)
        unsafe {
            let inner = &mut *self.inner.get();
            inner.as_mut_ptr().write(func());
        }
        guard.disarm();

        // SAFETY: the cell was just initialized so no check is required here
        Ok(unsafe { self.get_unchecked() })
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// otherwise attempts to initialize it with `func` and return the result.
    ///
    /// This method never blocks.
    ///
    /// When this function returns with an [`Ok`] result, it is guaranteed that
    /// some initialization closure has run and completed (it may not be the
    /// closure specified).
    /// It is also guaranteed that any memory writes performed by the executed
    /// closure can be reliably observed by other threads at this point (there
    /// is a happens-before relation between the closure and code executing
    /// after the return).
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
        match self.try_get() {
            Ok(res) => Ok(res),
            Err(TryGetError::WouldBlock) => Err(WouldBlockError(())),
            Err(TryGetError::Uninit) => {
                let mut func = Some(func);
                let res = self.try_init_inner(&mut || func.take().unwrap()())?;
                Ok(res)
            }
        }
    }
}

impl<T, B: Block> OnceCell<T, B> {
    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// [`None`].
    ///
    /// This method **blocks** if another thread has already begun initializing
    /// the [`OnceCell`] concurrently.
    /// See [`try_get`][OnceCell::try_get] for a non-blocking alternative.
    ///
    /// When this function returns with [`Some`], it is guaranteed that some
    /// initialization closure has run and completed.
    /// It is also guaranteed that any memory writes performed by the executed
    /// closure can be reliably observed by other threads at this point (there
    /// is a happens-before relation between the closure and code executing
    /// after the return).
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "std")]
    /// use conquer_once::OnceCell;
    /// # #[cfg(not(feature = "std"))]
    /// # use conquer_once::spin::OnceCell;
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
        match self.try_get() {
            Ok(res) => Some(res),
            Err(TryGetError::WouldBlock) => {
                B::block(&self.state);
                // SAFETY: `block` only returns when the state is set to
                // `INITIALIZED` and acts as an acquire barrier (initialization
                // happens-before block returns)
                Some(unsafe { self.get_unchecked() })
            }
            Err(TryGetError::Uninit) => None,
        }
    }

    /// Attempts to initialize the [`OnceCell`] with `func` if it is
    /// uninitialized.
    ///
    /// This method **blocks** if another thread has already begun initializing
    /// the [`OnceCell`] concurrently.
    ///
    /// If the initialization of the [`OnceCell`] has already been
    /// completed previously, this method returns early with minimal
    /// overhead.
    ///
    /// When this function returns, it is guaranteed that some initialization
    /// closure has run and completed (it may not be the closure specified).
    /// It is also guaranteed that any memory writes performed by the executed
    /// closure can be reliably observed by other threads at this point (there
    /// is a happens-before relation between the closure and code executing
    /// after the return).
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "std")]
    /// use conquer_once::OnceCell;
    /// # #[cfg(not(feature = "std"))]
    /// # use conquer_once::spin::OnceCell;
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
        if let Err(TryInitError::WouldBlock) = self.try_init_once(func) {
            // block the current thread if the cell is currently being
            // initialized
            B::block(&self.state);
        }
    }

    /// Returns a reference to the [`OnceCell`]'s initialized inner state or
    /// otherwise attempts to initialize it with `func` and return the result.
    ///
    /// This method **blocks** if another thread has already begun
    /// initializing the [`OnceCell`] concurrently.
    /// See [`try_get_or_init`][OnceCell::try_get_or_init] for a non-blocking
    /// alternative.
    ///
    /// When this function returns, it is guaranteed that some initialization
    /// closure has run and completed (it may not be the closure specified).
    /// It is also guaranteed that any memory writes performed by the executed
    /// closure can be reliably observed by other threads at this point (there
    /// is a happens-before relation between the closure and code executing
    /// after the return).
    ///
    /// # Panics
    ///
    /// This method panics if the [`OnceCell`] has been poisoned.
    #[inline]
    pub fn get_or_init(&self, func: impl FnOnce() -> T) -> &T {
        match self.try_get_or_init(func) {
            Ok(res) => res,
            Err(_) => {
                B::block(&self.state);
                // SAFETY: `block` only returns when the state is set to
                // `INITIALIZED` and acts as an acquire barrier (initialization
                // happens-before block returns)
                unsafe { self.get_unchecked() }
            }
        }
    }
}

impl<T: fmt::Debug, B> fmt::Debug for OnceCell<T, B> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("OnceCell").field("inner", &self.try_get().ok()).finish()
    }
}

impl<T, B> Drop for OnceCell<T, B> {
    #[inline]
    fn drop(&mut self) {
        // drop must never panic, so poisoning is ignored
        // SAFETY: take_inner cannot be called again after drop has been called
        mem::drop(unsafe { self.take_inner(true) })
    }
}

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

impl fmt::Display for TryInitError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TryInitError::AlreadyInit => write!(f, "{}", ALREADY_INIT_MSG),
            TryInitError::WouldBlock => write!(f, "{}", WOULD_BLOCK_MSG),
        }
    }
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

#[cfg(feature = "std")]
impl std::error::Error for TryInitError {}

/// Possible error variants of non-blocking fallible get calls.
#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub enum TryGetError {
    /// The [`OnceCell`] is currently not initialized.
    Uninit,
    /// The [`OnceCell`] is currently being initialized by another thread and
    /// the current thread would have to block.
    WouldBlock,
}

impl fmt::Display for TryGetError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TryGetError::Uninit => write!(f, "{}", UNINIT_MSG),
            TryGetError::WouldBlock => write!(f, "{}", WOULD_BLOCK_MSG),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TryGetError {}

/// An error indicating that a [`OnceCell`] would have to block.
#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct WouldBlockError(());

impl fmt::Display for WouldBlockError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", WOULD_BLOCK_MSG)
    }
}

impl From<TryBlockError> for WouldBlockError {
    #[inline]
    fn from(err: TryBlockError) -> Self {
        match err {
            TryBlockError::AlreadyInit => unreachable!(),
            TryBlockError::WouldBlock(_) => Self(()),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for WouldBlockError {}

/// A guard for catching panics during the execution of the initialization
/// closure.
#[derive(Debug)]
struct PanicGuard<'a, B: Unblock> {
    /// The state of the associated [`OnceCell`].
    state: &'a AtomicOnceState,
    /// Flag for indicating if a panic has occurred during the caller supplied
    /// arbitrary closure.
    poison: bool,
    /// A marker for the [`OnceCell`]'s blocking strategy.
    _marker: PhantomData<B>,
}

impl<'a, B: Unblock> PanicGuard<'a, B> {
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

impl<B: Unblock> Drop for PanicGuard<'_, B> {
    #[inline]
    fn drop(&mut self) {
        let swap = if self.poison { SwapState::Poisoned } else { SwapState::Ready };
        unsafe {
            // (guard:2) this acq-rel swap syncs-with the acq-rel CAS (wait:2)
            // and the acquire loads (cell:1), (cell:2), (wait:1) and the
            // acquire CAS (guard:1)
            let prev = self.state.unblock(swap, Ordering::AcqRel);
            B::on_unblock(prev);
        }
    }
}
