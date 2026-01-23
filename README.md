# `sseer` (sse - er)

## What?
A collection of utilities for getting Events out of your SSE streams

## Why?
As a bit of a learning project, in making a 'real' rust crate. Also got to learn a bit about the async ecosystem and hopefully make some improvements over the original code `reqwest-eventsource` and `eventsource-stream` by Julian Popescu

## How?
With `reqwest`:
```rust
let request = client.get("https://example.com/events");
let mut event_source = EventSource::new(request)?;
while let Some(result) = event_source.next().await { 
    // your code here :3
}
```
Make an EventSource stream over your `RequestBuilder` and it turns into a `Stream`. Or if you want to handle it yourself then hit up the `event_stream` module for a `Stream` that parses bytes into `Event`s. Or use the `parser` module if you want to parse it yourself.
```rust
let mut event_stream = EventStream::new(stream)?;
```

## No `std`?
Yes, without the `reqwest` feature it's all no_std. 