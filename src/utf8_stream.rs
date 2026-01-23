use bytes::BytesMut;
use bytes_utils::Str;

use core::pin::Pin;
use core::task::ready;
use futures_core::stream::Stream;
use futures_core::task::{Context, Poll};
use pin_project_lite::pin_project;

pub use crate::errors::Utf8StreamError;

pin_project! {
    pub struct Utf8Stream<S> {
        #[pin]
        state: Utf8StreamState<S>,
    }
}

pin_project! {
    #[project = Utf8StreamProjection]
    pub enum Utf8StreamState<S> {
        Active { #[pin] stream: S, buffer: BytesMut },
        Terminated,
    }
}

impl<S> Utf8Stream<S> {
    pub fn new(stream: S) -> Self {
        let state = Utf8StreamState::Active {
            stream,
            buffer: BytesMut::new(),
        };
        Self { state }
    }
}

impl<S, E, B> Stream for Utf8Stream<S>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    type Item = Result<Str, Utf8StreamError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let (stream_res, buffer) = match this.state.as_mut().project() {
            Utf8StreamProjection::Active { stream, buffer } => {
                (ready!(stream.poll_next(cx)), buffer)
            }
            Utf8StreamProjection::Terminated => return Poll::Ready(None),
        };

        match stream_res {
            Some(Ok(bytes)) => {
                buffer.extend_from_slice(bytes.as_ref());

                match str::from_utf8(buffer) {
                    Ok(_) => {
                        // Safety: we just checked the buffer is valid utf8
                        let byte_str =
                            unsafe { Str::from_inner_unchecked(buffer.split().freeze()) };
                        Poll::Ready(Some(Ok(byte_str)))
                    }
                    Err(e) => {
                        let valid_to = e.valid_up_to();
                        let valid_utf8 = buffer.split_to(valid_to).freeze();
                        // safety: we just split off the valid section of utf8
                        let byte_str = unsafe { Str::from_inner_unchecked(valid_utf8) };
                        Poll::Ready(Some(Ok(byte_str)))
                    }
                }
            }
            Some(Err(err)) => Poll::Ready(Some(Err(Utf8StreamError::Transport(err)))),
            None => {
                // this drops the borrow on the buffer so we can set the stream to terminated. split is O(1) and this dance is safer than trying shenanigans with MaybeUninit (if possible)
                let buffer = buffer.split().freeze();
                this.state.set(Utf8StreamState::Terminated);

                if buffer.is_empty() {
                    Poll::Ready(None)
                } else {
                    match str::from_utf8(&buffer) {
                        Ok(_) => {
                            // Safety: we just checked the buffer is valid utf8
                            let byte_str = unsafe { Str::from_inner_unchecked(buffer) };
                            Poll::Ready(Some(Ok(byte_str)))
                        }
                        Err(e) => Poll::Ready(Some(Err(Utf8StreamError::Utf8Error(e)))),
                    }
                }
            }
        }
    }
}

// Tests copied from https://github.com/jpopesculian/eventsource-stream/blob/main/src/utf8_stream.rs and rewritten using claude (I promise the rest of the repo isnt AI slop, I like coding :3)
#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::prelude::*;

    #[tokio::test]
    async fn valid_streams() {
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(Bytes::from(
                "Hello, world!"
            ))]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Str::from("Hello, world!")]
        );

        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(Bytes::from(""))]))
                .try_collect::<Vec<_>>()
                .await
                .unwrap(),
            vec![Str::from("")]
        );

        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from("Hello")),
                Ok::<_, ()>(Bytes::from(", world!"))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Str::from("Hello"), Str::from(", world!")]
        );

        // Single emoji in one chunk
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(Bytes::from(vec![
                240, 159, 145, 141
            ])),]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Str::from("👍")]
        );

        // Emoji split across two chunks
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from(vec![240, 159])),
                Ok::<_, ()>(Bytes::from(vec![145, 141]))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Str::from(""), Str::from("👍")]
        );

        // Emoji split across chunks followed by complete emoji
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from(vec![240, 159])),
                Ok::<_, ()>(Bytes::from(vec![145, 141, 240, 159, 145, 141]))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Str::from(""), Str::from("👍👍")]
        );

        // Multiple chunks with mixed ASCII and multi-byte characters
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from("Hello ")),
                Ok::<_, ()>(Bytes::from(vec![240, 159])),
                Ok::<_, ()>(Bytes::from(vec![145, 141])),
                Ok::<_, ()>(Bytes::from(" world!"))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Str::from("Hello "),
                Str::from(""),
                Str::from("👍"),
                Str::from(" world!")
            ]
        );
    }

    #[tokio::test]
    async fn invalid_streams() {
        // Incomplete UTF-8 sequence at end of stream
        let results = Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(Bytes::from(vec![
            240, 159,
        ]))]))
        .collect::<Vec<_>>()
        .await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], Ok(Str::from("")));
        assert!(matches!(results[1], Err(Utf8StreamError::Utf8Error(_))));

        // Valid emoji followed by incomplete sequence at end
        let results = Utf8Stream::new(futures::stream::iter(vec![
            Ok::<_, ()>(Bytes::from(vec![240, 159])),
            Ok::<_, ()>(Bytes::from(vec![145, 141, 240, 159, 145])),
        ]))
        .collect::<Vec<_>>()
        .await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Ok(Str::from("")));
        assert_eq!(results[1], Ok(Str::from("👍")));
        assert!(matches!(results[2], Err(Utf8StreamError::Utf8Error(_))));

        // Invalid UTF-8 byte in middle of stream
        let results = Utf8Stream::new(futures::stream::iter(vec![
            Ok::<_, ()>(Bytes::from("Hello ")),
            Ok::<_, ()>(Bytes::from(vec![0xFF])), // Invalid UTF-8
        ]))
        .collect::<Vec<_>>()
        .await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Ok(Str::from("Hello ")));
        assert_eq!(results[1], Ok(Str::from(""))); // Empty valid portion
        assert!(matches!(results[2], Err(Utf8StreamError::Utf8Error(_))));
    }

    #[tokio::test]
    async fn transport_errors() {
        let results = Utf8Stream::new(futures::stream::iter(vec![
            Ok::<_, &str>(Bytes::from("Hello")),
            Err("transport error"),
            Ok::<_, &str>(Bytes::from("world")),
        ]))
        .collect::<Vec<_>>()
        .await;
        assert_eq!(results.len(), 3); // Changed from 2 to 3
        assert_eq!(results[0], Ok(Str::from("Hello")));
        assert!(matches!(
            results[1],
            Err(Utf8StreamError::Transport("transport error"))
        ));
        assert_eq!(results[2], Ok(Str::from("world"))); // Stream continues after error
    }
}
