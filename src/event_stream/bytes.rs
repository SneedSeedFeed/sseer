use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use ::bytes::{Buf, BufMut, Bytes, BytesMut};
use bytes_utils::Str;
use futures_core::Stream;

use crate::{
    constants::{BOM, CR, EMPTY_STR, LF},
    errors::EventStreamError,
    event::Event,
    event_stream::{EventBuilder, EventStreamState, parse_event, starts_with_bom},
    parser::{RawEventLineOwned, parse_line_from_bytes},
};

fn parse_event_bytes<E>(
    bytes: &mut Bytes,
    builder: &mut EventBuilder,
) -> Result<Option<Event>, EventStreamError<E>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    loop {
        let event_line = match parse_line_from_bytes(bytes).map(RawEventLineOwned::validate) {
            Some(Ok(event_line)) => event_line,
            Some(Err(e)) => return Err(EventStreamError::Utf8Error(e)),
            None => return Ok(None),
        };

        builder.add(event_line);

        // dispatch mutates I don't want to collapse this, for clarity
        #[allow(clippy::collapsible_if)]
        if builder.is_complete {
            if let Some(event) = builder.dispatch() {
                return Ok(Some(event));
            }
        }
    }
}

pin_project_lite::pin_project! {
    /// Like [`EventStream`][super::generic::EventStream] but specialised for streams of [`Bytes`].
    #[derive(Debug)]
    pub struct EventStreamBytes<S> {
        #[pin]
        stream: S,
        buffer: BytesMut,
        remainder: Bytes,
        builder: EventBuilder,
        state: EventStreamState,
        last_event_id: Str,
    }
}

impl<S> EventStreamBytes<S> {
    /// Create a new [`EventStreamBytes`] from a stream of [`Bytes`].
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: BytesMut::new(),
            remainder: Bytes::new(),
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

    /// Takes the buffer and the remainder
    pub fn take_buffers(self) -> (BytesMut, Bytes) {
        (self.buffer, self.remainder)
    }
}

macro_rules! try_parse_remainder {
    ($this:ident) => {
        if !$this.remainder.is_empty() {
            match parse_event_bytes::<E>($this.remainder, $this.builder) {
                Ok(Some(event)) => {
                    *$this.last_event_id = event.id.clone();
                    return Poll::Ready(Some(Ok(event)));
                }
                Ok(None) => {
                    // incomplete event left over must concat with future data
                    if !$this.remainder.is_empty() {
                        $this.buffer.extend_from_slice($this.remainder);
                        *$this.remainder = Bytes::new();
                    }
                }
                Err(e) => return Poll::Ready(Some(Err(e))),
            }
        }
    };
}

impl<S, E> Stream for EventStreamBytes<S>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    type Item = Result<Event, EventStreamError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        try_parse_remainder!(this);
        try_parse_event_buffer!(this);

        if this.state.is_terminated() {
            return Poll::Ready(None);
        }

        loop {
            let new_bytes = match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(Ok(o)) => o,
                Some(Err(e)) => return Poll::Ready(Some(Err(EventStreamError::Transport(e)))),
                None => {
                    *this.state = EventStreamState::Terminated;

                    if !this.remainder.is_empty() {
                        this.buffer.extend_from_slice(this.remainder);
                        *this.remainder = Bytes::new();
                    }

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

            if new_bytes.is_empty() {
                continue;
            }

            if this.buffer.is_empty() && this.remainder.is_empty() {
                if this.state.is_not_started() {
                    match starts_with_bom(&new_bytes) {
                        Some(true) => {
                            *this.state = EventStreamState::Started;
                            let mut b = new_bytes;
                            b.advance(BOM.len());
                            *this.remainder = b;
                        }
                        Some(false) => {
                            *this.state = EventStreamState::Started;
                            *this.remainder = new_bytes;
                        }
                        None => {
                            // potential split BOM
                            this.buffer.extend_from_slice(&new_bytes);
                            continue;
                        }
                    }
                } else {
                    *this.remainder = new_bytes;
                }

                try_parse_remainder!(this);
            } else {
                if !this.remainder.is_empty() {
                    this.buffer.extend_from_slice(this.remainder);
                    *this.remainder = Bytes::new();
                }

                this.buffer.extend_from_slice(&new_bytes);

                if this.state.is_not_started() {
                    match starts_with_bom(this.buffer) {
                        Some(true) => {
                            *this.state = EventStreamState::Started;
                            this.buffer.advance(BOM.len());
                        }
                        Some(false) => *this.state = EventStreamState::Started,
                        None => continue,
                    }
                }

                try_parse_event_buffer!(this);
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use ::bytes::Bytes;
    use futures::prelude::*;

    #[tokio::test]
    async fn bytes_valid_data_fields() {
        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![
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
            EventStreamBytes::new(futures::stream::iter(vec![
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello, world!\n")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![]
        );

        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
    async fn bytes_spec_examples() {
        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
    async fn bytes_bom_handling() {
        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
            EventStreamBytes::new(futures::stream::iter(vec![
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
            EventStreamBytes::new(futures::stream::iter(vec![
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
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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
    async fn bytes_trailing_cr_handling() {
        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\r")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![]
        );

        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![Ok::<_, ()>(
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

    #[tokio::test]
    async fn bytes_remainder_to_buffer_transition() {
        // Chunk 1 yields an event from the remainder path, then leaves a partial line
        // Chunk 2 completes that partial line via the buffer path
        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"data: hello\n\nda")),
                Ok::<_, ()>(Bytes::from_static(b"ta: world\n\n")),
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("hello"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("world"),
                    id: EMPTY_STR,
                    retry: None,
                },
            ]
        );

        // Same idea but with multiple fields spanning the boundary
        assert_eq!(
            EventStreamBytes::new(futures::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"event: ping\ndata: first\n\nevent: po")),
                Ok::<_, ()>(Bytes::from_static(b"ng\ndata: second\n\n")),
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![
                Event {
                    event: Str::from_static("ping"),
                    data: Str::from_static("first"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("pong"),
                    data: Str::from_static("second"),
                    id: EMPTY_STR,
                    retry: None,
                },
            ]
        );
    }
}
