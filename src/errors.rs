use core::{
    fmt::{Display, Formatter},
    str::Utf8Error,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct CantCloneError;

impl Display for CantCloneError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        "cannot clone request".fmt(f)
    }
}

impl core::error::Error for CantCloneError {}

macro_rules! impl_samey_error {
    ($vis:vis enum $name:ident) => {
        #[derive(Debug, PartialEq)]
        $vis enum $name<E> {
            Transport(E),
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
