//! High-performance, `no_std`-compatible utilities for parsing and consuming
//! [Server-Sent Events](https://html.spec.whatwg.org/multipage/server-sent-events.html) (SSE) streams.
//!
//! `sseer` provides a layered API for working with SSE:
//!
//! - [`EventStream`] - a generic [`Stream`][futures_core::Stream] adapter that converts any
//!   `Stream<Item = Result<impl AsRef<[u8]>, E>>` into a stream of parsed [`Event`][event::Event]s.
//! - [`EventSource`] (requires `reqwest` feature) - a batteries-included HTTP client that
//!   wraps [`reqwest`] with automatic reconnection, retry policies, and the `Last-Event-ID` header.
//! - [`JsonStream`][json_stream::JsonStream] (requires `json` feature) - a stream adapter
//!   that deserialises each event's `data` field into a typed value via [`serde_json`].
//! - [`Utf8Stream`][utf8_stream::Utf8Stream] - validates and converts a raw byte stream into
//!   a stream of UTF-8 [`Str`][bytes_utils::Str]s, buffering incomplete multi-byte sequences across
//!   chunks.
//! - Low-level parsing via [`parser::parse_line`] and [`parser::parse_line_from_buffer`] for
//!   custom integrations.
//!
//! # Quick start with `reqwest`
//!
//! ```ignore
//! use futures::StreamExt;
//! use sseer::{EventSource, reqwest::StreamEvent};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let request = reqwest::Client::new().get("https://example.com/events");
//! let mut source = EventSource::new(request)?;
//!
//! while let Some(result) = source.next().await {
//!     match result {
//!         Ok(StreamEvent::Open) => println!("connected"),
//!         Ok(StreamEvent::Event(evt)) => {
//!             println!("{}: {}", evt.event, evt.data);
//!         }
//!         Err(e) if e.is_response_err() => break,
//!         Err(e) => eprintln!("error: {e}"),
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Without retry logic
//!
//! If you don't need automatic reconnection, such as with the OpenAI API where you typically
//! can't pick up a dropped stream (or don't want to accidentally send the same request more times), you can skip [`EventSource`]
//! and use [`response_to_stream`] to convert a [`::reqwest::Response`] directly
//! into an [`EventStream`] (the ergonomics are better I recommend it):
//!
//! ```ignore
//! use futures::StreamExt;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let response = reqwest::Client::new()
//!     .get("https://api.example.com/v1/chat/completions")
//!     .send()
//!     .await?;
//!
//! let mut stream = sseer::response_to_stream(response);
//!
//! while let Some(Ok(event)) = stream.next().await {
//!     println!("{}", event.data);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Using `EventStream` directly
//!
//! If you already have a byte stream (from any HTTP client, WebSocket, file, etc.)
//! you can use [`EventStream`] without the `reqwest` feature:
//!
//! ```rust
//! use bytes::Bytes;
//! use futures::StreamExt;
//! use sseer::EventStream;
//!
//! # #[tokio::main]
//! # async fn main() {
//! let chunks = vec![
//!     Ok::<_, std::io::Error>(Bytes::from("data: hello\n\ndata: world\n\n")),
//! ];
//! let mut stream = EventStream::new(futures::stream::iter(chunks));
//!
//! while let Some(Ok(event)) = stream.next().await {
//!     println!("{}", event.data);
//! }
//! # }
//! ```
//!
//! # Feature flags
//!
//! | Feature | Default | Description | no std? |
//! | --- | --- | --- | --- |
//! | `serde` | off | Derives [`Serialize`][::serde::Serialize] and [`Deserialize`][::serde::Deserialize] on [`Event`][event::Event] and enables `serde` support in [`bytes-utils`][bytes_utils]. | false |
//! | `std` | off | Enables standard library support in core dependencies (`bytes`, `memchr`, `futures-core`, etc.). Notably enables runtime SIMD for memchr. Turned on automatically by `reqwest` and `json`. | false |
//! | `reqwest` | off | Provides [`EventSource`] for HTTP-based SSE with automatic reconnection and configurable retry policies. | false |
//! | `json` | off | Provides [`JsonStream`][json_stream::JsonStream] for deserialising event data into typed values via [`serde_json`] and lets you choose between the default errors or [`serde_path_to_error`] for richer errors. | false |
//!
//! Without any features enabled, the crate is fully `no_std` compatible and provides
//! [`EventStream`], [`Utf8Stream`][utf8_stream::Utf8Stream], the low-level parser,
//! and retry policy types.

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

pub use event_stream::{bytes::EventStreamBytes, generic::EventStream};

#[cfg(feature = "reqwest")]
/// Convert a [`Response`][::reqwest::Response] into a [`Stream`][futures_core::Stream] via a similar mechanism to [::reqwest::Response::bytes_stream]
pub fn response_to_stream(
    response: ::reqwest::Response,
) -> event_stream::bytes::EventStreamBytes<http_body_util::BodyDataStream<::reqwest::Body>> {
    event_stream::bytes::EventStreamBytes::new(http_body_util::BodyDataStream::new(
        ::reqwest::Body::from(response),
    ))
}
