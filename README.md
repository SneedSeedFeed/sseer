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
let mut event_stream = EventStream::new(stream);
```

## No `std`?
Yes, without the `reqwest` feature it's all no_std. 

## Benches
Benches were run on my personal computer with 96GB of 6000Mhz DDR5 + Ryzen 9 9950X3D.

### parse_line
For these benches we just run the parser on a single line of different types. The main difference we get is on long lines such as the "big 1024-byte line" benchmark, since we use `memchr` instead of `nom` for the parser any benchmarks involving long lines are weighted in our favour.

| Line type | sseer | eventsource-stream | ratio |
|---|---|---|---|
| data field | 5.3ns | 28.5ns | **5.4x** |
| comment | 4.8ns | 19.5ns | **4.0x** |
| event field | 7.5ns | 24.9ns | **3.3x** |
| id field | 5.5ns | 21.4ns | **3.9x** |
| empty line | 4.5ns | 15.9ns | **3.5x** |
| no value | 5.5ns | 20.4ns | **3.7x** |
| no space | 6.8ns | 22.5ns | **3.3x** |
| big 1024-byte line | 11.3ns | 761.6ns | **67x** |

### event_stream
These benchmarks run the full stream implementation across some events split into 128 byte chunks that ignore line boundaries.
- mixed is just a sort of random mixed set of different line types, with no particularly long data lines. 512 events.
- ai_stream has its line lengths and ratios based on some responses I captured from OpenRouter, so is almost entirely made of data lines with some being quite long and some quite short. 512 events.
- evenish_distribution just takes our data, comment, event and id field lines we use in the parse_line benchmark and stacks them end to end 128 times and also splits into 128 byte chunks.

| Workload | sseer | eventsource-stream | ratio |
|---|---|---|---|
| mixed | 113.6µs | 184.4µs | **1.6x** |
| ai_stream | 79.6µs | 344.7µs | **4.3x** |
| evenish_distribution | 37.1µs | 56.3µs | **1.5x** |

### Memory (512 events, 128-byte chunks)
This is available under the example with `cargo run --example memory_usage`. I just use a global allocator that tracks calls to alloc and stores some stats, it's probably not perfectly accurate but hopefully it lets you get the gist. The main advantage `sseer` has over `eventsource-stream` is that we use `bytes::Bytes` as much as possible to reduce allocation, and we also avoid allocating a buffer for the data line in cases where there's only one data line.

| Workload | Metric | sseer | eventsource-stream | ratio |
|---|---|---|---|---|
| mixed | alloc calls | 546 | 4,753 | **8.7x** |
| mixed | total bytes | 35.5 KiB | 188.1 KiB | **5.3x** |
| ai_stream | alloc calls | 7 | 4,094 | **585x** |
| ai_stream | total bytes | 7.9 KiB | 669.2 KiB | **85x** |