use crate::Address;

pub trait HasSender {
    fn sender(&self) -> Address;
}

pub trait HasAmount {
    fn amount(&self) -> u128;
}
