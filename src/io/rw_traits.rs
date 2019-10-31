pub struct Write;
pub struct Read;
pub struct RW;

mod private {
    pub trait Sealed {}

    impl Sealed for super::Read {}
    impl Sealed for super::Write {}
    impl Sealed for super::RW {}
}

pub trait HasRead: private::Sealed {}
impl HasRead for Read {}
impl HasRead for RW {}

pub trait HasWrite: private::Sealed {}
impl HasWrite for Write {}
impl HasWrite for RW {}
