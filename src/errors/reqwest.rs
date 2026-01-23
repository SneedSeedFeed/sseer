use core::fmt::{Display, Formatter};

/// Error for a [`reqwest::RequestBuilder`] that cannot be cloned via [`RequestBuilder::try_clone`][reqwest::RequestBuilder::try_clone]
#[derive(Debug, Clone, Copy, Default)]
pub struct CantCloneError;

impl Display for CantCloneError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        "cannot clone request".fmt(f)
    }
}

impl core::error::Error for CantCloneError {}
