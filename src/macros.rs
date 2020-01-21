// c_str!() from the byte-strings crate is implemented via a proc macro which seems a bit excessive
macro_rules! c_str {
    ($data:expr) => {{
        let bytes = concat!($data, "\0");
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(bytes.as_bytes()) }
    }};
}

macro_rules! file_descriptor_type {
    ($type:ident) => {
        #[repr(transparent)]
        pub struct $type(RawFd);

        file_descriptor_impl!($type);

        impl FromRawFd for $type {
            unsafe fn from_raw_fd(fd: RawFd) -> Self {
                Self(fd)
            }
        }
    };
}

macro_rules! file_descriptor_impl {
    ($type:ty) => {
        impl Drop for $type {
            fn drop(&mut self) {
                if self.0 >= 0 {
                    unsafe {
                        libc::close(self.0);
                    }
                }
            }
        }

        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                self.0
            }
        }

        impl IntoRawFd for $type {
            fn into_raw_fd(mut self) -> RawFd {
                let fd = self.0;
                self.0 = -libc::EBADF;
                fd
            }
        }
    };
}

macro_rules! c_result {
    ($expr:expr) => {{
        let res = $expr;
        if res == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok::<_, ::std::io::Error>(res)
        }
    }};
}

macro_rules! c_try {
    ($expr:expr) => {
        c_result!($expr)?
    };
}

macro_rules! io_format_err {
    ($($msg:tt)*) => {
        ::std::io::Error::new(::std::io::ErrorKind::Other, format!($($msg)*))
    };
}

macro_rules! io_bail {
    ($($msg:tt)*) => {
        return Err(::std::io::Error::new(::std::io::ErrorKind::Other, format!($($msg)*)));
    };
}

macro_rules! ready {
    ($expr:expr) => {{
        use std::task::Poll;
        match $expr {
            Poll::Ready(v) => v,
            Poll::Pending => return Poll::Pending,
        }
    }};
}
