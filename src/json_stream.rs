use crate::event::Event;
use core::{
    error::Error,
    fmt::Display,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};
use futures_core::Stream;
use serde::de::DeserializeOwned;

pin_project_lite::pin_project! {
    #[derive(Debug)]
    pub struct JsonStream<T,S, DeserError = serde_json::Error> {
        #[pin]
        stream_state: JsonStreamState<S>,
        output_marker: PhantomData<fn() -> (T, DeserError)>,
    }
}

pub type DefaultJsonStream<T, S> = JsonStream<T, S, serde_json::Error>;

pub type PathErrorJsonStream<T, S> =
    JsonStream<T, S, serde_path_to_error::Error<serde_json::Error>>;

impl<T, S, DeserError> JsonStream<T, S, DeserError> {
    #[must_use]
    /// Creates a new [`JsonStream`] atop `stream` that returns type T or an error with path information via [serde_path_to_error]
    pub fn new_path(stream: S) -> PathErrorJsonStream<T, S> {
        JsonStream {
            stream_state: JsonStreamState::Active { stream },
            output_marker: PhantomData::<fn() -> (T, serde_path_to_error::Error<serde_json::Error>)>,
        }
    }

    #[must_use]
    /// Creates a new [`JsonStream`] atop `stream` that returns type T or an error
    pub fn new_default(stream: S) -> DefaultJsonStream<T, S>
    where
        T: DeserializeOwned,
    {
        JsonStream {
            stream_state: JsonStreamState::Active { stream },
            output_marker: PhantomData,
        }
    }
}

pin_project_lite::pin_project! {

    #[derive(Debug)]
    #[project = JsonStreamStateProjection]
    enum JsonStreamState<S> {
        Active {
            #[pin]
            stream: S
        },
        Inactive,
    }
}

#[derive(Debug)]
pub enum JsonStreamError<E, E2> {
    Stream(E),
    Deserialize(E2),
}

impl<E, E2> Display for JsonStreamError<E, E2>
where
    E: Display,
    E2: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonStreamError::Stream(e) => e.fmt(f),
            JsonStreamError::Deserialize(e2) => e2.fmt(f),
        }
    }
}

impl<E, E2> Error for JsonStreamError<E, E2>
where
    E: Error,
    E2: Error,
{
}

impl<T, S, E> Stream for JsonStream<T, S, serde_json::Error>
where
    S: Stream<Item = Result<Event, E>>,
    T: DeserializeOwned,
{
    type Item = Result<T, JsonStreamError<E, serde_json::Error>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let stream = match this.stream_state.as_mut().project() {
            JsonStreamStateProjection::Active { stream } => stream,
            JsonStreamStateProjection::Inactive => return Poll::Ready(None),
        };

        let Some(next) = core::task::ready!(stream.poll_next(cx)) else {
            this.stream_state.set(JsonStreamState::Inactive);
            return Poll::Ready(None);
        };
        Poll::Ready(Some(next.map_err(JsonStreamError::Stream).and_then(|o| {
            serde_json::from_str(&o.data).map_err(JsonStreamError::Deserialize)
        })))
    }
}

impl<T, S, E> Stream for JsonStream<T, S, serde_path_to_error::Error<serde_json::Error>>
where
    S: Stream<Item = Result<Event, E>>,
    T: DeserializeOwned,
{
    type Item = Result<T, JsonStreamError<E, serde_path_to_error::Error<serde_json::Error>>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let stream = match this.stream_state.as_mut().project() {
            JsonStreamStateProjection::Active { stream } => stream,
            JsonStreamStateProjection::Inactive => return Poll::Ready(None),
        };

        let Some(next) = core::task::ready!(stream.poll_next(cx)) else {
            this.stream_state.set(JsonStreamState::Inactive);
            return Poll::Ready(None);
        };
        match next {
            Ok(o) => {
                let mut deserializer = serde_json::Deserializer::from_str(&o.data);
                Poll::Ready(Some(
                    serde_path_to_error::deserialize(&mut deserializer)
                        .map_err(JsonStreamError::Deserialize),
                ))
            }
            Err(e) => Poll::Ready(Some(Err(JsonStreamError::Stream(e)))),
        }
    }
}
