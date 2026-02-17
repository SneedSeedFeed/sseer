use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use ::bytes::{Buf, BufMut, BytesMut};
use bytes_utils::Str;
use futures_core::Stream;

use crate::{
    constants::{BOM, CR, EMPTY_STR, LF},
    errors::EventStreamError,
    event::Event,
    event_stream::{EventBuilder, EventStreamState, parse_event, starts_with_bom},
};

pin_project_lite::pin_project! {
    /// [`Stream`][futures_core::Stream] that converts a stream of [`Bytes`][bytes::Bytes] into [`Event`][crate::event::Event]s
    #[project = EventStreamProjection]
    #[derive(Debug)]
    pub struct EventStream<S> {
        #[pin]
        stream: S,
        buffer: BytesMut,
        builder: EventBuilder,
        state: EventStreamState,
        last_event_id: Str,
    }
}

impl<S> EventStream<S> {
    /// Create a new [`EventStream`] from a stream of [`AsRef<[u8]>`][AsRef]
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: BytesMut::new(),
            builder: EventBuilder::default(),
            state: EventStreamState::NotStarted,
            last_event_id: EMPTY_STR,
        }
    }

    /// Set the last event id, useful for resumability
    pub fn set_last_event_id(&mut self, id: impl Into<Str>) {
        self.last_event_id = id.into()
    }

    /// Reference to the last event id given out by this stream
    pub fn last_event_id(&self) -> &Str {
        &self.last_event_id
    }

    /// Take the current buffer from the [EventStream], useful if you want to check for leftovers
    pub fn take_buffer(self) -> BytesMut {
        self.buffer
    }
}

impl<S, E, B> Stream for EventStream<S>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    type Item = Result<Event, EventStreamError<E>>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        let mut this = self.project();

        try_parse_event_buffer!(this);

        if this.state.is_terminated() {
            return Poll::Ready(None);
        };

        loop {
            let new_bytes = match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(Ok(o)) => o,
                Some(Err(e)) => return Poll::Ready(Some(Err(EventStreamError::Transport(e)))),
                None => {
                    *this.state = EventStreamState::Terminated;
                    if this
                        .buffer
                        .last()
                        .map(|&last| last == CR)
                        .unwrap_or_default()
                    {
                        this.buffer.put_u8(LF);
                    }
                    try_parse_event_buffer!(this);
                    return Poll::Ready(None);
                }
            };

            let new_bytes = new_bytes.as_ref();

            if new_bytes.is_empty() {
                continue;
            }

            this.buffer.extend_from_slice(new_bytes);

            if this.state.is_not_started() {
                match starts_with_bom(this.buffer) {
                    Some(true) => {
                        *this.state = EventStreamState::Started;
                        this.buffer.advance(BOM.len());
                    }
                    Some(false) => *this.state = EventStreamState::Started,
                    None => continue,
                }
            };

            try_parse_event_buffer!(this);
        }
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod test {
    use super::*;
    use ::bytes::Bytes;
    use futures::prelude::*;

    #[tokio::test]
    async fn generic_valid_data_fields() {
        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello, world!\n\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello, world!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"data: Hello,")),
                Ok::<_, ()>(Bytes::from_static(b" world!\n\n"))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello, world!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"data: Hello,")),
                Ok::<_, ()>(Bytes::from_static(b"")),
                Ok::<_, ()>(Bytes::from_static(b" world!\n\n"))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello, world!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello, world!\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello,\ndata: world!\n\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello,\nworld!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello,\n\ndata: world!\n\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("Hello,"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("world!"),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );
    }

    #[tokio::test]
    async fn generic_spec_examples() {
        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data: This is the first message.

data: This is the second message, it
data: has two lines.

data: This is the third message.

"
                )
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("This is the first message."),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("This is the second message, it\nhas two lines."),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("This is the third message."),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"event: add
data: 73857293

event: remove
data: 2153

event: add
data: 113411

"
                )
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("add"),
                    data: Str::from_static("73857293"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("remove"),
                    data: Str::from_static("2153"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("add"),
                    data: Str::from_static("113411"),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data: YHOO
data: +2
data: 10

"
                )
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("YHOO\n+2\n10"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b": test stream

data: first event
id: 1

data:second event
id

data:  third event

"
                )
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    id: Str::from_static("1"),
                    data: Str::from_static("first event"),
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("second event"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static(" third event"),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data

data
data

data:
"
                )
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: EMPTY_STR,
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("\n"),
                    id: EMPTY_STR,
                    retry: None,
                },
            ]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data:test

data: test

"
                )
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("test"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("test"),
                    id: EMPTY_STR,
                    retry: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn generic_bom_handling() {
        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"\xEF\xBB\xBFdata: test\n\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"\xEF\xBB")),
                Ok::<_, ()>(Bytes::from_static(b"\xBFdata: test\n\n"))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b":\n")),
                Ok::<_, ()>(Bytes::from_static(b"data: test\n\n"))
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\n\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );
    }

    #[tokio::test]
    async fn generic_trailing_cr_handling() {
        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\r")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![]
        );

        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\r\r")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );
    }
}
