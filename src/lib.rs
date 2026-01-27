#![cfg_attr(not(feature = "std"), no_std)]

pub(crate) mod constants;
pub mod errors;
pub mod event;
pub mod event_stream;
pub mod parser;
#[cfg(feature = "reqwest")]
pub mod reqwest;
pub mod retry;
pub mod utf8_stream;

#[cfg(feature = "json")]
pub mod json_stream;
// if the reqwest feature is enabled, this is what someone wants
#[cfg(feature = "reqwest")]
pub use reqwest::EventSource;

pub use event_stream::EventStream;
