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
    Body, Error as ReqwestError, RequestBuilder, Response, StatusCode,
    header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue},
};

use crate::{
    constants::EMPTY_STR,
    errors::{CantCloneError, EventStreamError},
    event::Event,
    event_stream::bytes::EventStreamBytes,
    response_to_stream,
    retry::{DEFAULT_RETRY, ExponentialBackoff, RetryPolicy},
};

/// Events emitted by [EventSource]
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A new connection has been opened
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
    #[derive(Debug)]
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
            stream: EventStreamBytes<BodyDataStream<Body>>,
            retry_state: Option<(usize, Duration)>,
        },
        Closed,
    }
}

impl std::fmt::Debug for ConnectionState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connecting { retry_state, .. } => f
                .debug_struct("Connecting")
                .field("future", &"future")
                .field("retry_state", retry_state)
                .finish(),
            Self::Retrying {
                delay,
                attempt_number,
                delay_duration,
            } => f
                .debug_struct("Retrying")
                .field("delay", delay)
                .field("attempt_number", attempt_number)
                .field("delay_duration", delay_duration)
                .finish(),
            Self::Open {
                stream,
                retry_state,
            } => f
                .debug_struct("Open")
                .field("stream", stream)
                .field("retry_state", retry_state)
                .finish(),
            Self::Closed => write!(f, "Closed"),
        }
    }
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
        response: Response,
        retry_state: Option<(usize, Duration)>,
    ) -> Result<(), EventSourceErrorKind> {
        let status = response.status();
        if !status.is_success() {
            return Err(EventSourceErrorKind::InvalidStatusCode {
                status,
                response: Box::new(response),
            });
        }

        if let Some(content_type) = response.headers().get(CONTENT_TYPE)
            && !content_type.as_bytes().starts_with(b"text/event-stream")
        {
            return Err(EventSourceErrorKind::InvalidContentType {
                status,
                content_type: content_type.clone(),
                response: Box::new(response),
            });
        }

        let stream = response_to_stream(response);
        *self.connection_state = ConnectionState::Open {
            stream,
            retry_state,
        };
        Ok(())
    }

    fn start_retry(&mut self, attempt_number: usize, delay_duration: Duration) {
        self.connection_state.set(ConnectionState::Retrying {
            delay: Delay::new(delay_duration),
            attempt_number,
            delay_duration,
        })
    }

    fn handle_error(&mut self, err: &EventSourceErrorKind, last_retry: Option<(usize, Duration)>)
    where
        R: RetryPolicy<EventSourceErrorKind>,
    {
        if let Some(retry_delay) = self.retry_policy.retry(err, last_retry) {
            let retry_num = last_retry.map(|retry| retry.0).unwrap_or(1);
            self.start_retry(retry_num, retry_delay);
        } else {
            self.connection_state.set(ConnectionState::Closed);
        }
    }

    fn handle_event(&mut self, event: &Event)
    where
        R: RetryPolicy<EventSourceErrorKind>,
    {
        *self.last_event_id = event.id.clone();
        if let Some(duration) = event.retry {
            self.retry_policy.set_reconnection_time(duration)
        }
    }
}

impl<R> EventSource<R> {
    pub fn new_with_retry(
        request: RequestBuilder,
        retry_policy: R,
    ) -> Result<Self, CantCloneError> {
        let request = request.header(ACCEPT, HeaderValue::from_static("text/event-stream"));
        let req_fut = Box::pin(request.try_clone().ok_or(CantCloneError)?.send());

        Ok(EventSource {
            builder: request,
            connection_state: ConnectionState::Connecting {
                future: req_fut,
                retry_state: None,
            },
            last_event_id: EMPTY_STR,
            retry_policy,
        })
    }
}

impl EventSource<ExponentialBackoff> {
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
    /// The last event ID contains non-visible ascii characters, unusable in a [`HeaderValue`]
    InvalidLastEventId(Str),
    /// Reqwest has had an error when getting a response
    Transport(ReqwestError),
    /// The underlying stream has ran into an error
    Stream(EventStreamError<ReqwestError>),
    /// Received a [non 2xx][reqwest::StatusCode::is_success] response
    InvalidStatusCode {
        status: StatusCode,
        response: Box<Response>, // boxed because this was a big error
    },
    /// Received a 2XX status, but the [Content-Type][CONTENT_TYPE] was not "text/event-stream"
    InvalidContentType {
        status: StatusCode,
        content_type: HeaderValue,
        response: Box<Response>,
    },
    /// The underlying stream has ran to completion
    StreamEnded, // not sure how i feel about this being an error tbh, change me?
}

impl Display for EventSourceErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            EventSourceErrorKind::InvalidLastEventId(s) => s.fmt(f),
            EventSourceErrorKind::Transport(err) => err.fmt(f),
            EventSourceErrorKind::Stream(err) => err.fmt(f),
            EventSourceErrorKind::InvalidStatusCode { status, .. } => write!(
                f,
                "got non 2XX status code {status}: '{canonical}'",
                canonical = status.canonical_reason().unwrap_or("no canonical reason")
            ),
            EventSourceErrorKind::InvalidContentType {
                status,
                content_type,
                ..
            } => write!(
                f,
                "got invalid content-type '{content_type}' on status '{status}' request",
                content_type = content_type
                    .to_str()
                    .unwrap_or("unable to read content-type as str")
            ),
            EventSourceErrorKind::StreamEnded => "stream ended".fmt(f),
        }
    }
}

impl Error for EventSourceErrorKind {}

#[derive(Debug)]
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

impl std::error::Error for EventSourceError {}

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

    /// Was this error caused by [Response::status] being a non 2XX
    pub fn is_status_code(&self) -> bool {
        matches!(self.kind, EventSourceErrorKind::InvalidStatusCode { .. })
    }

    /// Was this error caused by [Content-Type][CONTENT_TYPE] not being "text/event-stream"
    pub fn is_content_type(&self) -> bool {
        matches!(self.kind, EventSourceErrorKind::InvalidContentType { .. })
    }

    /// Was the error caused by an invalid [`Response`]
    pub fn is_response_err(&self) -> bool {
        matches!(
            self.kind,
            EventSourceErrorKind::InvalidContentType { .. }
                | EventSourceErrorKind::InvalidStatusCode { .. }
        )
    }

    /// Get the status code that caused this error, if it was caused by an invalid [`Response`]
    pub fn status_code(&self) -> Option<StatusCode> {
        match &self.kind {
            EventSourceErrorKind::InvalidStatusCode { status, .. }
            | EventSourceErrorKind::InvalidContentType { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Gets a reference to the underlying response if this error was caused by an invalid [`Response`]
    pub fn response(&self) -> Option<&Response> {
        match &self.kind {
            EventSourceErrorKind::InvalidStatusCode { response, .. }
            | EventSourceErrorKind::InvalidContentType { response, .. } => Some(response),

            _ => None,
        }
    }

    /// If this error comes from a [`Response`], return the raw response, collected status code and the [Content-Type][CONTENT_TYPE] header if it has been extracted already
    pub fn into_response_err(self) -> Option<(Response, StatusCode, Option<HeaderValue>)> {
        match self.kind {
            EventSourceErrorKind::InvalidStatusCode { response, status } => {
                Some((*response, status, None))
            }
            EventSourceErrorKind::InvalidContentType {
                status,
                content_type,
                response,
            } => Some((*response, status, Some(content_type))),
            _ => None,
        }
    }

    /// Is this error because the stream has stopped?
    pub fn is_stream_ended(&self) -> bool {
        matches!(self.kind, EventSourceErrorKind::StreamEnded)
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
    R: RetryPolicy<EventSourceErrorKind>,
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
                            if let Err(kind) =
                                this.handle_successful_response(response, retry_state)
                            {
                                return Poll::Ready(Some(Err(EventSourceError::new(
                                    kind,
                                    retry_state,
                                ))));
                            };
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
                    let retry_state = Some((*attempt_number + 1, *delay_duration));
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
                            let err_kind = err.into();
                            this.handle_error(&err_kind, retry_state);
                            return Poll::Ready(Some(Err(EventSourceError::new(
                                err_kind,
                                retry_state,
                            ))));
                        }
                        None => {
                            let err_kind = EventSourceErrorKind::StreamEnded;
                            this.handle_error(&err_kind, retry_state);
                            return Poll::Ready(Some(Err(EventSourceError::new(
                                err_kind,
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
