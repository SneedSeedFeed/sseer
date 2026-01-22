use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::BytesMut;
use bytes_utils::Str;
use futures_core::{future::BoxFuture, ready};
use futures_timer::Delay;
use futures_util::FutureExt;
use http_body_util::BodyDataStream;
use pin_project_lite::pin_project;
use reqwest::{
    Body, Error as ReqwestError, RequestBuilder, Response,
    header::{ACCEPT, HeaderName, HeaderValue},
};

use crate::{
    constants::EMPTY_STR,
    errors::CantCloneError,
    event::Event,
    event_stream::EventStream,
    retry::{ExponentialBackoff, RetryPolicy},
};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Open,
    Event(Event),
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

fn response_to_stream(response: Response) -> EventStream<BodyDataStream<Body>> {
    EventStream::new(BodyDataStream::new(Body::from(response)))
}

pub enum SseError {
    InvalidLastEventId(Str),
}

impl<'pin, R> EventSourceProjection<'pin, R> {
    fn initiate_connection(
        &mut self,
        retry_state: Option<(usize, Duration)>,
    ) -> Result<(), SseError> {
        let req = self.builder.try_clone().unwrap().header(
            HeaderName::from_static("last-event-id"),
            HeaderValue::from_str(self.last_event_id)
                .map_err(|_| SseError::InvalidLastEventId(self.last_event_id.clone()))?,
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
}

impl<R> EventSource<R> {
    pub fn new(request: RequestBuilder) -> Result<EventSource<ExponentialBackoff>, CantCloneError> {
        let request = request.header(ACCEPT, HeaderValue::from_static("text/event-stream"));
        let req_fut = request.try_clone().ok_or(CantCloneError)?.send().boxed();

        Ok(EventSource {
            builder: request,
            connection_state: ConnectionState::Connecting {
                future: req_fut,
                retry_state: None,
            },
            last_event_id: EMPTY_STR,
            retry_policy: crate::retry::DEFAULT_RETRY,
        })
    }
}

impl<R> futures_core::Stream for EventSource<R>
where
    R: RetryPolicy,
{
    type Item = Result<StreamEvent, reqwest::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
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
                    Err(e) => {
                        this.connection_state.set(ConnectionState::Closed);
                        return Poll::Ready(Some(Err(e)));
                    }
                }
            }
            ConnectionStateProjection::Retrying {
                delay,
                attempt_number,
                delay_duration,
            } => todo!(),
            ConnectionStateProjection::Open {
                stream,
                retry_state,
            } => {
                let retry_state = *retry_state;
                match ready!(stream.poll_next(cx)) {
                    Some(Ok(bytes)) => {}
                    Some(Err(e)) => {}
                    None => todo!(),
                }
            }
            ConnectionStateProjection::Closed => return Poll::Ready(None),
        }
        todo!()
    }
}
