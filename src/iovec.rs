//! Send+Sync IoSlice replacement.

use std::io::{IoSlice, IoSliceMut};
use std::marker::PhantomData;

/// The standard IoSlice does not implement Send and Sync. These types do.
#[derive(Debug)]
#[repr(C)]
pub struct IoVec<'a> {
    _iov: libc::iovec,
    _phantom: PhantomData<&'a [u8]>,
}

unsafe impl Send for IoVec<'_> {}
unsafe impl Sync for IoVec<'_> {}

impl IoVec<'_> {
    pub fn new(slice: &[u8]) -> Self {
        Self {
            _iov: libc::iovec {
                iov_base: slice.as_ptr() as *mut libc::c_void,
                iov_len: slice.len(),
            },
            _phantom: PhantomData,
        }
    }
}

impl<'s> IoVec<'s> {
    pub fn from_io_slice<'a>(ioslice: &'a [IoSlice<'s>]) -> &'a [Self] {
        unsafe { &*(ioslice as *const [IoSlice] as *const [Self]) }
    }
}

impl<'s> std::ops::Deref for IoVec<'s> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { std::slice::from_raw_parts(self._iov.iov_base as *const u8, self._iov.iov_len) }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct IoVecMut<'a> {
    _iov: libc::iovec,
    _phantom: PhantomData<&'a [u8]>,
}

unsafe impl Send for IoVecMut<'_> {}
unsafe impl Sync for IoVecMut<'_> {}

impl IoVecMut<'_> {
    pub fn new(slice: &mut [u8]) -> Self {
        Self {
            _iov: libc::iovec {
                iov_base: slice.as_mut_ptr() as *mut libc::c_void,
                iov_len: slice.len(),
            },
            _phantom: PhantomData,
        }
    }
}

impl<'s> IoVecMut<'s> {
    pub fn from_io_slice_mut<'a>(ioslice: &'a mut [IoSliceMut<'s>]) -> &'a mut [Self] {
        unsafe { &mut *(ioslice as *mut [IoSliceMut] as *mut [Self]) }
    }
}

impl<'s> std::ops::Deref for IoVecMut<'s> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { std::slice::from_raw_parts(self._iov.iov_base as *const u8, self._iov.iov_len) }
    }
}

impl<'s> std::ops::DerefMut for IoVecMut<'s> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::slice::from_raw_parts_mut(self._iov.iov_base as *mut u8, self._iov.iov_len) }
    }
}
