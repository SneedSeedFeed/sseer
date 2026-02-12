//! Memory allocation comparison: sseer vs eventsource-stream
//!
//! Run with: `cargo run --example memory_usage --features std`

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use bytes::Bytes;
use futures::stream::{self, StreamExt};
struct CountingAllocator {
    /// Total number of alloc() calls
    alloc_count: AtomicUsize,
    /// Total bytes requested via alloc()
    bytes_allocated: AtomicUsize,
    /// Currently live bytes (allocated - deallocated)
    live_bytes: AtomicIsize,
    /// Peak of live_bytes
    peak_live_bytes: AtomicIsize,
    inner: System,
}

impl CountingAllocator {
    const fn new() -> Self {
        Self {
            alloc_count: AtomicUsize::new(0),
            bytes_allocated: AtomicUsize::new(0),
            live_bytes: AtomicIsize::new(0),
            peak_live_bytes: AtomicIsize::new(0),
            inner: System,
        }
    }

    fn reset(&self) {
        self.alloc_count.store(0, Ordering::Relaxed);
        self.bytes_allocated.store(0, Ordering::Relaxed);
        self.live_bytes.store(0, Ordering::Relaxed);
        self.peak_live_bytes.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> AllocStats {
        AllocStats {
            alloc_count: self.alloc_count.load(Ordering::Relaxed),
            bytes_allocated: self.bytes_allocated.load(Ordering::Relaxed),
            peak_live_bytes: self.peak_live_bytes.load(Ordering::Relaxed).max(0) as usize,
        }
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.bytes_allocated.fetch_add(size, Ordering::Relaxed);

        let new_live = self.live_bytes.fetch_add(size as isize, Ordering::Relaxed) + size as isize;
        let mut peak = self.peak_live_bytes.load(Ordering::Relaxed);
        while new_live > peak {
            match self.peak_live_bytes.compare_exchange_weak(
                peak,
                new_live,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => peak = actual,
            }
        }
        unsafe { self.inner.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.live_bytes
            .fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { self.inner.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator::new();

#[derive(Debug, Clone, Copy)]
struct AllocStats {
    alloc_count: usize,
    bytes_allocated: usize,
    peak_live_bytes: usize,
}

const CHUNK_SIZE: usize = 128;

fn load_chunks(bytes: &[u8]) -> Vec<Bytes> {
    bytes
        .chunks(CHUNK_SIZE)
        .map(Bytes::copy_from_slice)
        .collect()
}

fn measure_sseer(chunks: &[Bytes]) -> AllocStats {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    // warm up
    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = sseer::EventStream::new(s);
        while let Some(item) = es.next().await {
            drop(item);
        }
    });

    ALLOC.reset();

    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = sseer::EventStream::new(s);
        while let Some(item) = es.next().await {
            drop(item);
        }
    });

    ALLOC.snapshot()
}

fn measure_eventsource_stream(chunks: &[Bytes]) -> AllocStats {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = eventsource_stream::EventStream::new(s);
        while let Some(item) = es.next().await {
            drop(item);
        }
    });

    ALLOC.reset();

    rt.block_on(async {
        let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
        let mut es = eventsource_stream::EventStream::new(s);
        while let Some(item) = es.next().await {
            drop(item);
        }
    });

    ALLOC.snapshot()
}

fn fmt_bytes(n: usize) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn print_row(label: &str, sseer: AllocStats, es: AllocStats) {
    println!("  {label}");
    println!(
        "    {:22} {:>12} {:>12} {:>12}",
        "sseer",
        sseer.alloc_count,
        fmt_bytes(sseer.bytes_allocated),
        fmt_bytes(sseer.peak_live_bytes),
    );
    println!(
        "    {:22} {:>12} {:>12} {:>12}",
        "eventsource-stream",
        es.alloc_count,
        fmt_bytes(es.bytes_allocated),
        fmt_bytes(es.peak_live_bytes),
    );
    println!(
        "    {:22} {:>11.1}x {:>11.1}x {:>11.1}x",
        "ratio (es/sseer)",
        es.alloc_count as f64 / sseer.alloc_count.max(1) as f64,
        es.bytes_allocated as f64 / sseer.bytes_allocated.max(1) as f64,
        es.peak_live_bytes as f64 / sseer.peak_live_bytes.max(1) as f64,
    );
    println!();
}

fn main() {
    let data_sets: &[(&str, &[u8])] = &[
        (
            "mixed (512 events)",
            include_bytes!("../bench_data/mixed.bin"),
        ),
        (
            "ai_stream (512 events)",
            include_bytes!("../bench_data/ai_stream.bin"),
        ),
    ];

    println!();
    println!("Memory allocation comparison: sseer vs eventsource-stream");
    println!("  Data sets chunked into {CHUNK_SIZE}-byte pieces");
    println!();
    println!(
        "    {:22} {:>12} {:>12} {:>12}",
        "impl", "alloc calls", "total bytes", "peak live"
    );
    println!("    {}", "-".repeat(60));

    for &(name, data_set) in data_sets {
        let chunks = load_chunks(data_set);
        let sseer = measure_sseer(&chunks);
        let es = measure_eventsource_stream(&chunks);
        print_row(name, sseer, es);
    }
}
