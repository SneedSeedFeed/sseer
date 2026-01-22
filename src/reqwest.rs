use core::{
    error::Error,
    fmt::{Display, Formatter},
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use bytes_utils::Str;
use futures_core::future::BoxFuture;
use futures_timer::Delay;

use http_body_util::BodyDataStream;
use pin_project_lite::pin_project;
use reqwest::{
    Body, Error as ReqwestError, RequestBuilder, Response,
    header::{ACCEPT, HeaderName, HeaderValue},
};

use crate::{
    constants::EMPTY_STR,
    errors::{CantCloneError, EventStreamError},
    event::Event,
    event_stream::EventStream,
    retry::{DEFAULT_RETRY, ExponentialBackoff, RetryPolicy},
};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Open,
    Event(Event),
}

impl From<Event> for StreamEvent {
    fn from(event: Event) -> Self {
        StreamEvent::Event(event)
    }
}

pin_project! {
    #[project = EventSourceProjection]
    pub struct EventSource<R> {
        builder: RequestBuilder,
        #[pin]
        connection_state: ConnectionState,
        last_event_id: Str,
        retry_policy: R
    }
}

pin_project! {
    #[project = ConnectionStateProjection]
    enum ConnectionState {
        Connecting {
            #[pin]
            future: BoxFuture<'static, Result<Response, ReqwestError>>,
            retry_state: Option<(usize, Duration)>,
        },
        Retrying {
            #[pin]
            delay: Delay,
            attempt_number: usize,
            delay_duration: Duration,
        },
        Open {
            #[pin]
            stream: EventStream<BodyDataStream<Body>>,
            retry_state: Option<(usize, Duration)>,
        },
        Closed,
    }
}

// just some reasoning on this func: this is sort of what `Response::bytes_stream` does under da hood to make its stream, but the stream it gives you is an Impl Stream thus I cant name the bugger.
// if i decide boxing is better then I can still do that in future
fn response_to_stream(response: Response) -> EventStream<BodyDataStream<Body>> {
    EventStream::new(BodyDataStream::new(Body::from(response)))
}

impl<'pin, R> EventSourceProjection<'pin, R> {
    fn initiate_connection(
        &mut self,
        retry_state: Option<(usize, Duration)>,
    ) -> Result<(), EventSourceErrorKind> {
        let req = self.builder.try_clone().unwrap().header(
            HeaderName::from_static("last-event-id"),
            HeaderValue::from_str(self.last_event_id).map_err(|_| {
                EventSourceErrorKind::InvalidLastEventId(self.last_event_id.clone())
            })?,
        );
        let res_future = Box::pin(req.send());
        *self.connection_state = ConnectionState::Connecting {
            future: res_future,
            retry_state,
        };
        Ok(())
    }

    fn handle_successful_response(
        &mut self,
        res: Response,
        retry_state: Option<(usize, Duration)>,
    ) {
        let stream = response_to_stream(res);
        *self.connection_state = ConnectionState::Open {
            stream,
            retry_state,
        };
    }

    fn start_retry(&mut self, attempt_number: usize, delay_duration: Duration) {
        self.connection_state.set(ConnectionState::Retrying {
            delay: Delay::new(delay_duration),
            attempt_number,
            delay_duration,
        })
    }

    fn handle_error(&mut self, last_retry: Option<(usize, Duration)>)
    where
        R: RetryPolicy,
    {
        if let Some(retry_delay) = self.retry_policy.retry(last_retry) {
            let retry_num = last_retry.map(|retry| retry.0).unwrap_or(1);
            self.start_retry(retry_num, retry_delay);
        } else {
            self.connection_state.set(ConnectionState::Closed);
        }
    }

    fn handle_event(&mut self, event: &Event)
    where
        R: RetryPolicy,
    {
        *self.last_event_id = event.id.clone();
        if let Some(duration) = event.retry {
            self.retry_policy.set_reconnection_time(duration)
        }
    }
}

impl<R> EventSource<R> {
    pub fn new(request: RequestBuilder) -> Result<EventSource<ExponentialBackoff>, CantCloneError> {
        let request = request.header(ACCEPT, HeaderValue::from_static("text/event-stream"));
        let req_fut = Box::pin(request.try_clone().ok_or(CantCloneError)?.send());

        Ok(EventSource {
            builder: request,
            connection_state: ConnectionState::Connecting {
                future: req_fut,
                retry_state: None,
            },
            last_event_id: EMPTY_STR,
            retry_policy: DEFAULT_RETRY,
        })
    }
}

#[derive(Debug)]
enum EventSourceErrorKind {
    InvalidLastEventId(Str),
    Transport(ReqwestError),
    Stream(EventStreamError<ReqwestError>),
    StreamEnded, // not sure how i feel about this being an error tbh, change me?
}

impl Display for EventSourceErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            EventSourceErrorKind::InvalidLastEventId(s) => s.fmt(f),
            EventSourceErrorKind::Transport(err) => err.fmt(f),
            EventSourceErrorKind::Stream(err) => err.fmt(f),
            EventSourceErrorKind::StreamEnded => "stream ended".fmt(f),
        }
    }
}

impl Error for EventSourceErrorKind {}

pub struct EventSourceError {
    retry_state: Option<(usize, Duration)>,
    kind: EventSourceErrorKind,
}

impl Display for EventSourceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let retry_state = self
            .retry_state
            .as_ref()
            .map(|(attempts, duration)| format!("attempts: '{attempts}' duration: {duration:?}"))
            .unwrap_or(String::from("no last retry"));
        write!(
            f,
            "Error '{kind}' with last retry: {retry_state}",
            kind = self.kind
        )
    }
}

impl EventSourceError {
    fn new(
        kind: impl Into<EventSourceErrorKind>,
        retry_state: impl Into<Option<(usize, Duration)>>,
    ) -> Self {
        Self {
            retry_state: retry_state.into(),
            kind: kind.into(),
        }
    }
}

impl From<ReqwestError> for EventSourceErrorKind {
    fn from(value: ReqwestError) -> Self {
        Self::Transport(value)
    }
}

impl From<EventStreamError<ReqwestError>> for EventSourceErrorKind {
    fn from(value: EventStreamError<ReqwestError>) -> Self {
        Self::Stream(value)
    }
}

impl<R> futures_core::Stream for EventSource<R>
where
    R: RetryPolicy,
{
    type Item = Result<StreamEvent, EventSourceError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.connection_state.as_mut().project() {
                ConnectionStateProjection::Connecting {
                    future,
                    retry_state,
                } => {
                    let retry_state = *retry_state;
                    match ready!(future.poll(cx)) {
                        Ok(response) => {
                            this.handle_successful_response(response, retry_state);
                            return Poll::Ready(Some(Ok(StreamEvent::Open)));
                        }
                        Err(err) => {
                            this.connection_state.set(ConnectionState::Closed);
                            return Poll::Ready(Some(Err(EventSourceError::new(err, retry_state))));
                        }
                    }
                }
                ConnectionStateProjection::Retrying {
                    delay,
                    attempt_number,
                    delay_duration,
                } => {
                    ready!(delay.poll(cx));
                    let retry_state = Some((*attempt_number, *delay_duration));
                    if let Err(err) = this.initiate_connection(retry_state) {
                        this.connection_state.set(ConnectionState::Closed);
                        return Poll::Ready(Some(Err(EventSourceError::new(err, retry_state))));
                    }
                }
                ConnectionStateProjection::Open {
                    stream,
                    retry_state,
                } => {
                    let retry_state = *retry_state;
                    match ready!(stream.poll_next(cx)) {
                        Some(Ok(event)) => {
                            this.handle_event(&event);
                            return Poll::Ready(Some(Ok(event.into())));
                        }
                        Some(Err(err)) => {
                            this.handle_error(retry_state);
                            return Poll::Ready(Some(Err(EventSourceError::new(err, retry_state))));
                        }
                        None => {
                            this.handle_error(retry_state);
                            return Poll::Ready(Some(Err(EventSourceError::new(
                                EventSourceErrorKind::StreamEnded,
                                retry_state,
                            ))));
                        }
                    }
                }
                ConnectionStateProjection::Closed => return Poll::Ready(None),
            }
        }
    }
}
