use std::io;

pub fn io_err_other<E: ToString>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}
