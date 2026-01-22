#![cfg_attr(not(feature = "std"), no_std)]

/// common constants that get used around the crate a fair bit
pub(crate) mod constants;
pub mod errors;
pub mod event;
pub mod event_stream;
pub mod parser;
#[cfg(feature = "reqwest")]
pub mod reqwest;
pub mod retry;
pub mod utf8_stream;
