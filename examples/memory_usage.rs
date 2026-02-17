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

fn load_aligned(bytes: &[u8]) -> Vec<Bytes> {
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

fn measure(f: impl Fn()) -> AllocStats {
    // warm up
    f();

    ALLOC.reset();
    f();

    ALLOC.snapshot()
}

fn measure_sseer_generic(chunks: &[Bytes]) -> AllocStats {
    measure(|| {
        futures::executor::block_on(async {
            let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
            let mut es = sseer::event_stream::generic::EventStream::new(s);
            while let Some(item) = es.next().await {
                drop(item);
            }
        });
    })
}

fn measure_sseer_bytes(chunks: &[Bytes]) -> AllocStats {
    measure(|| {
        futures::executor::block_on(async {
            let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
            let mut es = sseer::event_stream::bytes::EventStreamBytes::new(s);
            while let Some(item) = es.next().await {
                drop(item);
            }
        });
    })
}

fn measure_eventsource_stream(chunks: &[Bytes]) -> AllocStats {
    measure(|| {
        futures::executor::block_on(async {
            let s = stream::iter(chunks.iter().cloned().map(Ok::<_, ()>));
            let mut es = eventsource_stream::EventStream::new(s);
            while let Some(item) = es.next().await {
                drop(item);
            }
        });
    })
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

fn fmt_ratio(baseline: usize, value: usize) -> String {
    if value == 0 {
        if baseline == 0 {
            "**1.0x**".to_string()
        } else {
            "**\u{221e}**".to_string()
        }
    } else {
        format!("**{:.1}x**", baseline as f64 / value as f64)
    }
}

fn print_row(
    workload: &str,
    chunking: &str,
    metric: &str,
    baseline: usize,
    generic: usize,
    bytes: usize,
    fmt: fn(usize) -> String,
) {
    println!(
        "| {workload} | {chunking} | {metric} | {} | {} ({}) | {} ({}) |",
        fmt(baseline),
        fmt(generic),
        fmt_ratio(baseline, generic),
        fmt(bytes),
        fmt_ratio(baseline, bytes),
    );
}

fn fmt_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!(
            "{},{:03},{:03}",
            n / 1_000_000,
            (n / 1_000) % 1_000,
            n % 1_000
        )
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        format!("{n}")
    }
}

fn print_section(
    workload: &str,
    chunking: &str,
    baseline: AllocStats,
    generic: AllocStats,
    bytes: AllocStats,
) {
    print_row(
        workload,
        chunking,
        "alloc calls",
        baseline.alloc_count,
        generic.alloc_count,
        bytes.alloc_count,
        fmt_count,
    );
    print_row(
        workload,
        chunking,
        "total bytes",
        baseline.bytes_allocated,
        generic.bytes_allocated,
        bytes.bytes_allocated,
        fmt_bytes,
    );
    print_row(
        workload,
        chunking,
        "peak live",
        baseline.peak_live_bytes,
        generic.peak_live_bytes,
        bytes.peak_live_bytes,
        fmt_bytes,
    );
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

    println!(
        "| Workload | Chunking | Metric | eventsource-stream | sseer (generic) | sseer (bytes) |"
    );
    println!("|---|---|---|---|---|---|");

    for &(name, data) in data_sets {
        let unaligned = load_chunks(data);
        let aligned = load_aligned(data);

        let generic_unaligned = measure_sseer_generic(&unaligned);
        let generic_aligned = measure_sseer_generic(&aligned);
        let bytes_unaligned = measure_sseer_bytes(&unaligned);
        let bytes_aligned = measure_sseer_bytes(&aligned);
        let es_unaligned = measure_eventsource_stream(&unaligned);
        let es_aligned = measure_eventsource_stream(&aligned);

        print_section(
            name,
            &format!("unaligned ({CHUNK_SIZE}B)"),
            es_unaligned,
            generic_unaligned,
            bytes_unaligned,
        );
        print_section(
            name,
            "line-aligned",
            es_aligned,
            generic_aligned,
            bytes_aligned,
        );
    }
}
