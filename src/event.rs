//! Representation of SSE events based primarily off <https://html.spec.whatwg.org/multipage/server-sent-events.html>

use core::time::Duration;

use bytes_utils::{Str, StrMut};

/// Event with immutable fields from a [EventStream][crate::event_stream::EventStream]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    pub event: Str,
    pub data: Str,
    pub id: Str,
    pub retry: Option<Duration>,
}

/// Event with mutable fields
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventMut {
    pub event: StrMut,
    pub data: StrMut,
    pub id: StrMut,
    pub retry: Option<Duration>,
}

impl EventMut {
    /// Converts [EventMut] into [Event] by [freezing](StrMut::freeze) the [StrMut] fields
    pub fn freeze(self) -> Event {
        let Self {
            event,
            data,
            id,
            retry,
        } = self;
        Event {
            event: event.freeze(),
            data: data.freeze(),
            id: id.freeze(),
            retry,
        }
    }
}
