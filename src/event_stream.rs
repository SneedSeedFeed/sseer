//! [`Stream`][futures_core::Stream] that converts a stream of [`Bytes`][::bytes::Bytes] into [`Event`]s

use core::time::Duration;

use ::bytes::{BufMut, BytesMut};
use bytes_utils::{Str, StrMut};

use crate::{
    constants::{BOM, EMPTY_STR, MESSAGE_STR},
    errors::EventStreamError,
    event::Event,
    parser::{FieldName, RawEventLineOwned, ValidatedEventLine, parse_line_from_buffer},
};

#[derive(Debug, Clone)]
pub(crate) struct EventBuilder {
    event: Str,
    id: Str,
    data_buffer: EventBuilderDataBuffer,
    retry: Option<Duration>,
    is_complete: bool,
}

// this is an optimisation over using just a StrMut buffer. like 99% of the time we are just gonna have a single data line so we should just take that as the buffer's value and never add the linefeed at all
// if we get more data lines then we pay the allocation cost and lose out
#[derive(Debug, Default, Clone)]
enum EventBuilderDataBuffer {
    #[default]
    Uninit,
    Immutable(Str),
    Mutable(StrMut),
}

impl EventBuilderDataBuffer {
    fn freeze(self) -> Str {
        match self {
            EventBuilderDataBuffer::Uninit => EMPTY_STR,
            EventBuilderDataBuffer::Immutable(str) => str,
            EventBuilderDataBuffer::Mutable(str_mut) => str_mut.freeze(),
        }
    }

    fn push_str(&mut self, str: Str) {
        match self {
            EventBuilderDataBuffer::Uninit => *self = EventBuilderDataBuffer::Immutable(str),
            EventBuilderDataBuffer::Immutable(immutable_buf) => {
                let len = immutable_buf.len() + 1 + str.len(); // immutable buf + '\n' + str
                let mut buf = BytesMut::with_capacity(len);

                buf.extend_from_slice(immutable_buf.as_bytes());
                buf.put_u8(b'\n');
                buf.extend_from_slice(str.as_bytes());

                // Safety: We pushed two valid utf8 Str and a newline, all valid utf8
                let buf = unsafe { StrMut::from_inner_unchecked(buf) };
                *self = EventBuilderDataBuffer::Mutable(buf)
            }
            EventBuilderDataBuffer::Mutable(mutable_buf) => {
                mutable_buf.push('\n');
                mutable_buf.push_str(&str)
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, EventBuilderDataBuffer::Uninit)
    }
}

impl Default for EventBuilder {
    fn default() -> Self {
        Self {
            event: EMPTY_STR,
            id: EMPTY_STR,
            data_buffer: EventBuilderDataBuffer::default(),
            retry: None,
            is_complete: false,
        }
    }
}

impl EventBuilder {
    pub(crate) fn add(&mut self, line: ValidatedEventLine) {
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
                let field_value = field_value.unwrap_or(EMPTY_STR);
                self.data_buffer.push_str(field_value)
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
    #[must_use]
    pub(crate) fn dispatch(&mut self) -> Option<Event> {
        if self.data_buffer.is_empty() {
            self.event = EMPTY_STR;
            self.retry = None;
            self.is_complete = false;
            return None;
        }

        let event = if self.event.is_empty() {
            MESSAGE_STR
        } else {
            core::mem::replace(&mut self.event, EMPTY_STR)
        };

        let data = core::mem::take(&mut self.data_buffer).freeze();
        let id = self.id.clone();
        let retry = self.retry.take();
        self.is_complete = false;

        Some(Event {
            event,
            data,
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

    fn is_not_started(&self) -> bool {
        matches!(self, Self::NotStarted)
    }
}

pub(crate) const fn starts_with_bom(buf: &[u8]) -> Option<bool> {
    match buf.len() {
        0 => None,
        1 => {
            if buf[0] == BOM[0] {
                None
            } else {
                Some(false)
            }
        }
        2 => {
            if buf[0] == BOM[0] && buf[1] == BOM[1] {
                None
            } else {
                Some(false)
            }
        }
        _gte_3 => {
            if buf[0] == BOM[0] && buf[1] == BOM[1] && buf[2] == BOM[2] {
                Some(true)
            } else {
                Some(false)
            }
        }
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
                *$this.last_event_id = event.id.clone();
                return Poll::Ready(Some(Ok(event)));
            }
            Err(e) => return Poll::Ready(Some(Err(e))),
            _ => {}
        }
    };
}

pub mod bytes;
pub mod generic;
