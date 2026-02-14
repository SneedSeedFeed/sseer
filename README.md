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
| data field | 5.3ns | 31.9ns | **6.0x** |
| comment | 4.9ns | 19.0ns | **3.9x** |
| event field | 7.3ns | 28.8ns | **3.9x** |
| id field | 5.5ns | 23.1ns | **4.2x** |
| empty line | 4.3ns | 17.3ns | **4.0x** |
| no value | 5.3ns | 21.2ns | **4.0x** |
| no space | 6.7ns | 26.3ns | **3.9x** |
| big 1024-byte line | 11.4ns | 619.0ns | **54x** |

### event_stream
These benchmarks run the full stream implementation across some sample events in two conditions: events aligned to line boundaries (thus more individual stream items) and 'dumb' 128 byte chunks.
- mixed is just a sort of random mixed set of different line types, with no particularly long data lines. 512 events.
- ai_stream has its line lengths and ratios based on some responses I captured from OpenRouter, so is almost entirely made of data lines with some being quite long and some quite short. 512 events.
- evenish_distribution just takes our data, comment, event and id field lines we use in the parse_line benchmark and stacks them end to end 128 times and also splits into 128 byte chunks.

| Workload | Chunking | sseer | eventsource-stream | ratio |
|---|---|---|---|---|
| mixed | unaligned | 107.8µs | 169.4µs | **1.6x** |
| mixed | aligned | 155.2µs | 220.4µs | **1.4x** |
| ai_stream | unaligned | 76.5µs | 343.1µs | **4.5x** |
| ai_stream | aligned | 103.6µs | 204.5µs | **2.0x** |
| evenish_distribution | unaligned | 34.5µs | 57.6µs | **1.7x** |

### Memory
This is available under the example with `cargo run --example memory_usage`. I just use a global allocator that tracks calls to alloc and stores some stats, it's probably not perfectly accurate but hopefully it lets you get the gist. The main advantage `sseer` has over `eventsource-stream` is that we use `bytes::Bytes` as much as possible to reduce allocation, and we also avoid allocating a buffer for the data line in cases where there's only one data line.

| Workload | Metric | sseer | eventsource-stream | ratio |
|---|---|---|---|---|
| mixed | alloc calls | 546 | 4,753 | **8.7x** |
| mixed | total bytes | 35.5 KiB | 188.1 KiB | **5.3x** |
| ai_stream | alloc calls | 7 | 4,094 | **585x** |
| ai_stream | total bytes | 7.9 KiB | 669.2 KiB | **85x** |