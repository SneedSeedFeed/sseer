use std::time::Duration;

use bytes_utils::{Str, StrMut};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    pub event: Str,
    pub data: Str,
    pub id: Str,
    pub retry: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventMut {
    pub event: StrMut,
    pub data: StrMut,
    pub id: StrMut,
    pub retry: Option<Duration>,
}

impl EventMut {
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
