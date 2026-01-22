use core::{
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use bytes::{Buf, BufMut, BytesMut};
use bytes_utils::{Str, StrMut};
use futures_core::Stream;

use crate::{
    constants::{BOM, CR, EMPTY_STR, LF, MESSAGE_STR},
    errors::EventStreamError,
    event::Event,
    parser::{FieldName, RawEventLineOwned, ValidatedEventLine, parse_line_from_buffer},
};

#[derive(Debug, Clone)]
struct EventBuilder {
    event: Str,
    id: Str,
    data_buffer: StrMut,
    retry: Option<Duration>,
    is_complete: bool,
}

impl Default for EventBuilder {
    fn default() -> Self {
        Self {
            event: EMPTY_STR,
            id: EMPTY_STR,
            data_buffer: StrMut::new(),
            retry: None,
            is_complete: false,
        }
    }
}

impl EventBuilder {
    fn add(&mut self, line: ValidatedEventLine) {
        match line {
            ValidatedEventLine::Empty => self.is_complete = true,
            ValidatedEventLine::Field {
                field_name: FieldName::Event,
                field_value: Some(field_value),
            } => {
                self.event = field_value;
            }
            ValidatedEventLine::Field {
                field_name: FieldName::Data,
                field_value,
            } => {
                if let Some(field_value) = field_value {
                    self.data_buffer.push_str(&field_value);
                }
                self.data_buffer.push('\n')
            }
            ValidatedEventLine::Field {
                field_name: FieldName::Id,
                field_value,
            } => {
                let no_null_byte = field_value
                    .as_ref()
                    .map(|field_value| memchr::memchr(0, field_value.as_bytes()).is_none())
                    .unwrap_or(true);

                if no_null_byte {
                    self.id = field_value.unwrap_or(EMPTY_STR);
                }
            }
            ValidatedEventLine::Field {
                field_name: FieldName::Retry,
                field_value,
            } => {
                if let Some(Ok(val)) = field_value.map(|val| val.parse()) {
                    self.retry = Some(Duration::from_millis(val))
                }
            }
            // Comments are ignored, fields with no name are ignored, events with no value do nothing so might as well include them here
            ValidatedEventLine::Comment
            | ValidatedEventLine::Field {
                field_name: FieldName::Ignored,
                ..
            }
            | ValidatedEventLine::Field {
                field_name: FieldName::Event,
                field_value: None,
            } => (),
        }
    }

    // Comment taken from https://github.com/jpopesculian/eventsource-stream/blob/main/src/event_stream.rs
    /// From the HTML spec
    ///
    /// 1. Set the last event ID string of the event source to the value of the last event ID buffer. The buffer does not get reset, so the last event ID string of the event source remains set to this value until the next time it is set by the server.
    /// 2. If the data buffer is an empty string, set the data buffer and the event type buffer to the empty string and return.
    /// 3. If the data buffer's last character is a U+000A LINE FEED (LF) character, then remove the last character from the data buffer.
    /// 4. Let event be the result of creating an event using MessageEvent, in the relevant Realm of the EventSource object.
    /// 5. Initialize event's type attribute to message, its data attribute to data, its origin attribute to the serialization of the origin of the event stream's final URL (i.e., the URL after redirects), and its lastEventId attribute to the last event ID string of the event source.
    /// 6. If the event type buffer has a value other than the empty string, change the type of the newly created event to equal the value of the event type buffer.
    /// 7. Set the data buffer and the event type buffer to the empty string.
    /// 8. Queue a task which, if the readyState attribute is set to a value other than CLOSED, dispatches the newly created event at the EventSource object.
    fn dispatch(&mut self) -> Option<Event> {
        let EventBuilder {
            mut event,
            id,
            mut data_buffer,
            retry,
            ..
        } = core::mem::take(self);
        // replace id
        self.id = id.clone();

        if data_buffer.is_empty() {
            return None;
        }

        if *data_buffer.as_bytes().last().unwrap() == LF {
            // This should basically be a no-op as I'm just pulling it out of the wrapper, mutating then shoving it back in again
            let mut buf = data_buffer.into_inner();
            buf.truncate(buf.len() - 1);
            // Safety: we just removed the final byte, which is known to be LF and thus can't be part of another utf-8 codepoint
            data_buffer = unsafe { StrMut::from_inner_unchecked(buf) };
        };

        if event.is_empty() {
            event = MESSAGE_STR;
        }

        Some(Event {
            event,
            data: data_buffer.freeze(),
            id,
            retry,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum EventStreamState {
    NotStarted,
    Started,
    Terminated,
}

impl EventStreamState {
    fn is_terminated(&self) -> bool {
        matches!(self, Self::Terminated)
    }

    #[allow(dead_code)]
    fn is_started(&self) -> bool {
        matches!(self, Self::Started)
    }

    fn is_not_started(&self) -> bool {
        matches!(self, Self::NotStarted)
    }
}

pin_project_lite::pin_project! {
    #[project = EventStreamProjection]
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
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: BytesMut::new(),
            builder: EventBuilder::default(),
            state: EventStreamState::NotStarted,
            last_event_id: EMPTY_STR,
        }
    }

    pub fn set_last_event_id(&mut self, id: impl Into<Str>) {
        self.last_event_id = id.into()
    }

    pub fn last_event_id(&self) -> &Str {
        &self.last_event_id
    }
}

fn parse_event<E>(
    buffer: &mut BytesMut,
    builder: &mut EventBuilder,
) -> Result<Option<Event>, EventStreamError<E>> {
    if buffer.is_empty() {
        return Ok(None);
    }
    loop {
        let event_line = match parse_line_from_buffer(buffer).map(RawEventLineOwned::validate) {
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

macro_rules! try_parse_event_buffer {
    ($this:ident) => {
        match parse_event($this.buffer, $this.builder) {
            Ok(Some(event)) => {

                // This covers for a bug where (in theory) if the first bytes we get from a stream are ":\n" followed by another starting with the BOM, we could parse the first complete line, not say we started then strip a BOM mark
                if $this.state.is_not_started() {
                    *$this.state = EventStreamState::Started;
                }

                *$this.last_event_id = event.id.clone();
                return Poll::Ready(Some(Ok(event)));
            }
            Err(e) => return Poll::Ready(Some(Err(e))),
            _ => {}
        }
    };
}

// Todo: I can probably make this more efficient
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
                    // double check the buffer, since the parser waits to see if a line is CR LF or just CR the buffer may have a totally valid line that isn't complete until the stream finishes and we can confirm it's just CR
                    if this
                        .buffer
                        .last()
                        .map(|&last| last == CR)
                        .unwrap_or_default()
                    {
                        // just chuck an LF on to make the line actually complete
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
            // more robust BOM check than the OG
            if this.state.is_not_started() {
                // check if buffer has enough length to BOM check
                if this.buffer.len() >= BOM.len() {
                    *this.state = EventStreamState::Started;
                    if this.buffer.starts_with(BOM) {
                        // advance past BOM
                        this.buffer.advance(BOM.len());
                    }
                }
            };

            try_parse_event_buffer!(this);
        }
    }
}

// tests from https://github.com/jpopesculian/eventsource-stream/blob/main/src/event_stream.rs just adjusted by AI to match my actual API
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::prelude::*;

    #[tokio::test]
    async fn valid_data_fields() {
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
    async fn spec_examples() {
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
    async fn bom_handling() {
        // BOM at start should be stripped
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

        // BOM split across chunks
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

        // Short first line without BOM
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

        // No BOM
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
    async fn trailing_cr_handling() {
        // Stream ending with CR should be treated as complete line
        assert_eq!(
            EventStream::new(futures::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\r")
            )]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec![] // No complete event without the empty line
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
