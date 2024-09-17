// See https://github.com/rust-lang/rfcs/blob/master/text/3467-unsafe-pinned.md

use std::{marker::PhantomPinned, pin::Pin, ptr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnsafePinned<T: ?Sized> {
    #[allow(unused)]
    not_unpin: PhantomPinned,
    /// The data stored in this aliased field.
    value: T,
}

impl<T: ?Sized> UnsafePinned<T> {
    /// Constructs a new instance of `UnsafePinned` which will wrap the specified
    /// value.
    pub fn new(value: T) -> UnsafePinned<T>
    where
        T: Sized,
    {
        UnsafePinned {
            value,
            not_unpin: PhantomPinned,
        }
    }

    pub fn into_inner(self) -> T
    where
        T: Sized,
    {
        self.value
    }

    /// Get read-write access to the contents of an `UnsafePinned`.
    /// Using `UnsafePinned` without pinning is dangerous,
    /// so use `get_mut_pinned` instead whenever possible!
    pub fn get_mut_unchecked(&mut self) -> *mut T {
        ptr::addr_of_mut!(self.value)
    }

    /// Get read-write access to the contents of a pinned `UnsafePinned`.
    pub fn get_mut_pinned(self: Pin<&mut Self>) -> *mut T {
        // SAFETY: we're not using `get_unchecked_mut` to unpin anything
        unsafe { ptr::addr_of_mut!(self.get_unchecked_mut().value) }
    }

    /// Get read-only access to the contents of a shared `UnsafePinned`.
    /// Note that `&UnsafePinned<T>` is read-only if `&T` is read-only.
    /// This means that if there is mutation of the `T`, future reads from the
    /// `*const T` returned here are UB!
    ///
    /// ```rust
    /// unsafe {
    ///     let mut x = UnsafePinned::new(0);
    ///     let ref1 = &mut *addr_of_mut!(x);
    ///     let ref2 = &mut *addr_of_mut!(x);
    ///     let ptr = ref1.get(); // read-only pointer, assumes immutability
    ///     ref2.get_mut().write(1);
    ///     ptr.read(); // UB!
    /// }
    /// ```
    pub fn get(&self) -> *const T {
        self as *const _ as *const T
    }

    pub fn raw_get_mut(this: *mut Self) -> *mut T {
        this as *mut T
    }

    pub fn raw_get(this: *const Self) -> *const T {
        this as *const T
    }
}
