//! [`Error`][core::error::Error] implementations used across the crate

use core::{
    fmt::{Display, Formatter},
    str::Utf8Error,
};

#[cfg(feature = "reqwest")]
pub mod reqwest;
#[cfg(feature = "reqwest")]
pub use reqwest::CantCloneError;

macro_rules! impl_samey_error {
    ($vis:vis enum $name:ident) => {
        #[derive(Debug, PartialEq)]
        $vis enum $name<E> {
            /// Something went wrong with the underlying stream
            Transport(E),
            /// The stream had invalid utf8
            Utf8Error(Utf8Error),
        }

        impl<E> From<Utf8Error> for $name<E> {
            fn from(value: Utf8Error) -> Self {
                Self::Utf8Error(value)
            }
        }

        impl<E> Display for $name<E>
        where
            E: Display,
        {
            fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
                match self {
                    $name::Transport(e) => e.fmt(f),
                    $name::Utf8Error(e) => e.fmt(f),
                }
            }
        }

        impl<E> core::error::Error for $name<E> where E: core::error::Error {}
    };
}

impl_samey_error!(pub enum EventStreamError);
impl_samey_error!(pub enum Utf8StreamError);
