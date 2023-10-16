use std::mem;

pub const fn align(n: usize) -> usize {
    (n + mem::size_of::<libc::size_t>() - 1) & !(mem::size_of::<libc::size_t>() - 1)
}

pub const fn space(n: usize) -> usize {
    align(mem::size_of::<libc::cmsghdr>()) + align(n)
}

pub const fn capacity<T: Sized>() -> usize {
    space(mem::size_of::<T>())
}

pub fn buffer<T: Sized>() -> Vec<u8> {
    let capacity = capacity::<T>();
    unsafe {
        let data = std::alloc::alloc(std::alloc::Layout::array::<u8>(capacity).unwrap());
        Vec::from_raw_parts(data, capacity, capacity)
    }
}

pub struct RawCmsgIterator<'a> {
    buf: &'a [u8],
}

pub struct ControlMessageRef<'a> {
    pub cmsg_level: libc::c_int,
    pub cmsg_type: libc::c_int,
    pub data: &'a [u8],
}

impl<'a> Iterator for RawCmsgIterator<'a> {
    type Item = ControlMessageRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        use libc::cmsghdr;

        if self.buf.len() < mem::size_of::<cmsghdr>() {
            return None;
        }

        let buf: &'a [u8] = self.buf;

        // clippy issue:
        #[allow(clippy::cast_ptr_alignment)]
        let hdr: cmsghdr = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const cmsghdr) };
        let data_off = mem::size_of::<cmsghdr>();
        let data_end = hdr.cmsg_len;
        let next_hdr = align(hdr.cmsg_len);
        let data = &buf[data_off..data_end];
        let item = ControlMessageRef {
            cmsg_level: hdr.cmsg_level,
            cmsg_type: hdr.cmsg_type,
            data,
        };
        self.buf = if next_hdr >= buf.len() {
            &[]
        } else {
            &buf[next_hdr..]
        };

        Some(item)
    }
}

#[inline]
pub fn iter(buf: &[u8]) -> RawCmsgIterator {
    RawCmsgIterator { buf }
}
