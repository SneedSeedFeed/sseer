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
let mut event_stream = EventStreamBytes::new(stream); // we have a special version optimised for bytes::Bytes streams
```

## No `std`?
Yes, without the `reqwest` feature it's all no_std. 

## Benches
Benches were run on my personal computer with 96GB of 6000Mhz DDR5 + Ryzen 9 9950X3D. We benchmark against eventsource-stream as a baseline since that's the main competitor and the inspiration I hoped to do better than, and the ratios are compared to it.

### parse_line
For these benches we just run the parser on a single line of different types. The main difference we get is on long lines such as the "big 1024-byte line" benchmark, since we use `memchr` instead of `nom` for the parser any benchmarks involving long lines are weighted in our favour.

| Line type | eventsource-stream | sseer |
|---|---|---|
| data field | 31.8ns | 5.2ns (**6.1x**) |
| comment | 18.9ns | 5.0ns (**3.8x**) |
| event field | 28.4ns | 7.3ns (**3.9x**) |
| id field | 23.1ns | 5.5ns (**4.2x**) |
| empty line | 17.1ns | 4.1ns (**4.1x**) |
| no value | 21.0ns | 5.2ns (**4.0x**) |
| no space | 25.8ns | 6.5ns (**3.9x**) |
| big 1024-byte line | 617.6ns | 12.0ns (**51x**) |

### event_stream
These benchmarks run the full stream implementation across some sample events in two conditions: events aligned to line boundaries (thus more individual stream items) and 'dumb' 128 byte chunks.
- mixed is just a sort of random mixed set of different line types, with no particularly long data lines. 512 events.
- ai_stream has its line lengths and ratios based on some responses I captured from OpenRouter, so is almost entirely made of data lines with some being quite long and some quite short. 512 events.
- evenish_distribution just takes our data, comment, event and id field lines we use in the parse_line benchmark and stacks them end to end 128 times and also splits into 128 byte chunks.

| Workload | Chunking | eventsource-stream | sseer (generic) | sseer (bytes) |
|---|---|---|---|---|
| mixed | unaligned | 171.5µs | 105.3µs (**1.6x**) | 105.3µs (**1.6x**) |
| mixed | line-aligned | 215.9µs | 152.2µs (**1.4x**) | 109.8µs (**2.0x**) |
| ai_stream | unaligned | 331.8µs | 75.2µs (**4.4x**) | 75.1µs (**4.4x**) |
| ai_stream | line-aligned | 200.0µs | 102.1µs (**2.0x**) | 60.2µs (**3.3x**) |
| evenish_distribution | unaligned | 53.7µs | 34.1µs (**1.6x**) | 33.0µs (**1.6x**) |

### Memory
This is available under the example with `cargo run --example memory_usage`. I just use a global allocator that tracks calls to alloc and stores some stats, it's probably not perfectly accurate but hopefully it lets you get the gist. The main advantage `sseer` has over `eventsource-stream` is that we use `bytes::Bytes` as much as possible to reduce allocation, and we also avoid allocating a buffer for the data line in cases where there's only one data line. On the stream specialised on just `bytes::Bytes` streams instead of `AsRef<[u8]>` we also avoid allocating any time a new stream item makes a complete line, hence why the line-aligned case looks so good for us

| Workload | Chunking | Metric | eventsource-stream | sseer (generic) | sseer (bytes) |
|---|---|---|---|---|---|
| mixed | unaligned (128B) | alloc calls | 4,753 | 546 (**8.7x**) | 535 (**8.9x**) |
| mixed | unaligned (128B) | total bytes | 188.1 KiB | 35.8 KiB (**5.3x**) | 34.2 KiB (**5.5x**) |
| mixed | unaligned (128B) | peak live | 488 B | 742 B (**0.7x**) | 739 B (**0.7x**) |
| mixed | line-aligned | alloc calls | 6,034 | 1,743 (**3.5x**) | 306 (**19.7x**) |
| mixed | line-aligned | total bytes | 92.8 KiB | 49.9 KiB (**1.9x**) | 11.5 KiB (**8.1x**) |
| mixed | line-aligned | peak live | 171 B | 299 B (**0.6x**) | 93 B (**1.8x**) |
| ai_stream | unaligned (128B) | alloc calls | 4,094 | 7 (**584.9x**) | 7 (**584.9x**) |
| ai_stream | unaligned (128B) | total bytes | 669.2 KiB | 7.9 KiB (**84.6x**) | 7.9 KiB (**84.6x**) |
| ai_stream | unaligned (128B) | peak live | 6.7 KiB | 6.0 KiB (**1.1x**) | 6.0 KiB (**1.1x**) |
| ai_stream | line-aligned | alloc calls | 3,576 | 1,537 (**2.3x**) | 0 (**∞**) |
| ai_stream | line-aligned | total bytes | 515.3 KiB | 123.9 KiB (**4.2x**) | 0 B (**∞**) |
| ai_stream | line-aligned | peak live | 7.3 KiB | 1.5 KiB (**4.7x**) | 0 B (**∞**) |