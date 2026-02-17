use std::hint::black_box;

use bytes::Bytes;
use futures::stream::{self, StreamExt};

const CHUNK_SIZE: usize = 128;

/// Chop slice into [CHUNK_SIZE]-byte `Bytes` chunks, ignoring line boundaries
pub fn load_chunks(bytes: &[u8]) -> Vec<Bytes> {
    bytes
        .chunks(CHUNK_SIZE)
        .map(Bytes::copy_from_slice)
        .collect()
}

/// Split on `\n` boundaries - each chunk is one complete line (including the `\n`)
pub fn load_line_aligned_chunks(bytes: &[u8]) -> Vec<Bytes> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while let Some(pos) = memchr::memchr(b'\n', &bytes[start..]) {
        let end = start + pos + 1;
        chunks.push(Bytes::copy_from_slice(&bytes[start..end]));
        start = end;
    }
    if start < bytes.len() {
        chunks.push(Bytes::copy_from_slice(&bytes[start..]));
    }
    chunks
}

pub fn run_sseer(chunks: &[Bytes]) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = sseer::EventStream::new(s);
        while let Some(item) = es.next().await {
            let _ = black_box(item);
        }
    });
}

pub fn run_sseer_bytes_only(chunks: &[Bytes]) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = sseer::event_stream::bytes::EventStreamBytes::new(s);
        while let Some(item) = es.next().await {
            let _ = black_box(item);
        }
    });
}

pub fn run_eventsource_stream(chunks: &[Bytes]) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = eventsource_stream::EventStream::new(s);
        while let Some(item) = es.next().await {
            let _ = black_box(item);
        }
    });
}
