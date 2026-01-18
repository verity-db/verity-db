# Performance Guidelines

VerityDB prioritizes correctness over performance, but that doesn't mean we ignore performance. This document describes our performance philosophy, optimization priorities, and guidelines for writing efficient code.

---

## Table of Contents

1. [Philosophy](#philosophy)
2. [Optimization Priority Order](#optimization-priority-order)
3. [Mechanical Sympathy](#mechanical-sympathy)
4. [Batching and Amortization](#batching-and-amortization)
5. [Zero-Copy Patterns](#zero-copy-patterns)
6. [Memory Layout](#memory-layout)
7. [I/O Patterns](#io-patterns)
8. [Benchmarking](#benchmarking)
9. [Anti-Patterns](#anti-patterns)

---

## Philosophy

### Correctness First, Then Performance

Performance optimizations are worthless if the system is incorrect. Our priority order:

1. **Correct**: The system does what it's supposed to do
2. **Clear**: The code is understandable and maintainable
3. **Fast**: The system performs well

Never sacrifice correctness or clarity for performance without explicit justification.

### Measure Before Optimizing

> "Premature optimization is the root of all evil" — Donald Knuth

Before optimizing:
1. **Profile**: Identify the actual bottleneck
2. **Measure**: Establish a baseline
3. **Optimize**: Make targeted changes
4. **Verify**: Confirm improvement without regression

### Predictable is Better Than Fast

A system with predictable latency is often better than one with lower average latency but high variance:

```
System A: p50=5ms, p99=8ms, p99.9=12ms  ← Preferred
System B: p50=3ms, p99=50ms, p99.9=500ms
```

Design for consistent performance, not peak performance.

---

## Optimization Priority Order

When optimizing, work through resources in this order:

```
1. Network      Most expensive, highest latency
      ↓
2. Disk         Order of magnitude faster than network
      ↓
3. Memory       Order of magnitude faster than disk
      ↓
4. CPU          Order of magnitude faster than memory
```

### Why This Order?

| Resource | Latency | Bandwidth | Cost to Improve |
|----------|---------|-----------|-----------------|
| Network | ~1ms | ~1 GB/s | Very expensive |
| SSD | ~100μs | ~3 GB/s | Expensive |
| Memory | ~100ns | ~50 GB/s | Moderate |
| CPU | ~1ns | N/A | Cheap (just wait) |

An optimization that reduces network round-trips will almost always beat one that reduces CPU cycles.

### Practical Implications

**Network**: Batch requests, use persistent connections, minimize round-trips
```rust
// Bad: N round-trips
for item in items {
    client.write(item).await?;
}

// Good: 1 round-trip
client.write_batch(items).await?;
```

**Disk**: Sequential I/O, batch writes, use fsync strategically
```rust
// Bad: Sync after every write
for record in records {
    file.write(&record)?;
    file.sync_all()?;  // Expensive!
}

// Good: Batch writes, sync once
for record in records {
    file.write(&record)?;
}
file.sync_all()?;
```

**Memory**: Avoid allocations in hot paths, reuse buffers
```rust
// Bad: Allocate per iteration
for item in items {
    let buffer = Vec::new();  // Allocation!
    serialize_into(&mut buffer, item);
}

// Good: Reuse buffer
let mut buffer = Vec::with_capacity(estimated_size);
for item in items {
    buffer.clear();
    serialize_into(&mut buffer, item);
}
```

**CPU**: Last resort; usually not the bottleneck
```rust
// Only optimize CPU when profiling shows it's the bottleneck
// and after all higher-priority optimizations are done
```

---

## Mechanical Sympathy

Understanding how hardware works leads to better performance.

### CPU Cache Hierarchy

```
┌─────────────────────────────────────────────────────────────┐
│                        CPU Core                              │
│  ┌─────────┐                                                │
│  │ L1 Cache│  32KB, ~4 cycles                               │
│  │  (data) │                                                │
│  └────┬────┘                                                │
│       ▼                                                     │
│  ┌─────────┐                                                │
│  │ L2 Cache│  256KB, ~12 cycles                             │
│  └────┬────┘                                                │
│       ▼                                                     │
│  ┌─────────┐                                                │
│  │ L3 Cache│  8-30MB, ~40 cycles (shared)                   │
│  └────┬────┘                                                │
│       ▼                                                     │
│  ┌─────────┐                                                │
│  │   RAM   │  ~100+ cycles                                  │
│  └─────────┘                                                │
└─────────────────────────────────────────────────────────────┘
```

**Implications**:
- Keep hot data small (fits in L1/L2)
- Access data sequentially when possible (prefetching)
- Avoid pointer chasing (linked lists are cache-hostile)

### Cache-Friendly Data Structures

```rust
// Cache-hostile: Each node is a separate allocation
struct LinkedList<T> {
    head: Option<Box<Node<T>>>,
}
struct Node<T> {
    value: T,
    next: Option<Box<Node<T>>>,
}

// Cache-friendly: Contiguous memory
struct Vec<T> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}
```

For B+trees, we use:
- Large node sizes (4KB = page size)
- Contiguous arrays within nodes
- Predictable memory access patterns

### Page Alignment

Storage is organized into 4KB pages matching OS page size:

```rust
const PAGE_SIZE: usize = 4096;

#[repr(C, align(4096))]
struct Page {
    data: [u8; PAGE_SIZE],
}
```

Benefits:
- Aligned I/O is faster
- Direct I/O (O_DIRECT) requires alignment
- Memory-mapped I/O works at page granularity

---

## Batching and Amortization

Batching amortizes fixed costs across multiple operations.

### Write Batching

```rust
// Without batching: O(n) fsyncs
async fn write_unbatched(log: &mut Log, records: Vec<Record>) -> Result<()> {
    for record in records {
        log.append(&record)?;
        log.sync()?;  // Each sync is ~10ms on HDD, ~100μs on SSD
    }
    Ok(())
}

// With batching: O(1) fsyncs
async fn write_batched(log: &mut Log, records: Vec<Record>) -> Result<()> {
    for record in records {
        log.append(&record)?;
    }
    log.sync()?;  // One sync for entire batch
    Ok(())
}
```

### Network Batching

```rust
// Consensus with batching
impl Consensus {
    async fn propose(&mut self, commands: Vec<Command>) -> Result<Vec<Position>> {
        // One network round-trip for many commands
        let batch = Batch::new(commands);
        let positions = self.replicate_batch(batch).await?;
        Ok(positions)
    }
}
```

### Read Batching (Prefetching)

```rust
// Prefetch likely-needed pages
impl BTree {
    fn get(&self, key: &Key) -> Option<Value> {
        // Prefetch siblings while traversing
        let path = self.find_path(key);
        for node in path.windows(2) {
            let parent = node[0];
            let child = node[1];
            // Prefetch sibling nodes (likely to be needed for next query)
            self.prefetch_siblings(parent, child);
        }
        self.get_from_path(path)
    }
}
```

---

## Zero-Copy Patterns

Avoid copying data when possible.

### Bytes Instead of Vec

```rust
use bytes::Bytes;

// Copying: Each read creates a new Vec
fn read_record_copying(storage: &Storage, offset: u64) -> Vec<u8> {
    let data = storage.read(offset);
    data.to_vec()  // Copy!
}

// Zero-copy: Bytes is reference-counted
fn read_record_zero_copy(storage: &Storage, offset: u64) -> Bytes {
    storage.read(offset)  // Returns Bytes, no copy
}
```

### Borrowed vs Owned APIs

```rust
// Prefer borrowing in read-only operations
impl Record {
    // Good: Returns reference
    fn data(&self) -> &[u8] {
        &self.data
    }

    // Avoid: Unnecessary copy
    fn data_owned(&self) -> Vec<u8> {
        self.data.clone()
    }
}
```

### Memory-Mapped Reads

For large sequential reads, memory mapping avoids copies:

```rust
use memmap2::MmapOptions;

fn read_segment_mmap(path: &Path) -> Result<Mmap> {
    let file = File::open(path)?;
    let mmap = unsafe { MmapOptions::new().map(&file)? };
    Ok(mmap)
}

// Access data directly from kernel page cache
let data = &mmap[offset..offset + len];
```

---

## Memory Layout

How data is laid out in memory affects performance.

### Struct Layout

```rust
// Bad: Poor alignment, padding wasted
struct BadLayout {
    a: u8,   // 1 byte + 7 padding
    b: u64,  // 8 bytes
    c: u8,   // 1 byte + 7 padding
    d: u64,  // 8 bytes
}  // Total: 32 bytes, but only 18 bytes of data

// Good: Ordered by size, minimal padding
struct GoodLayout {
    b: u64,  // 8 bytes
    d: u64,  // 8 bytes
    a: u8,   // 1 byte
    c: u8,   // 1 byte + 6 padding
}  // Total: 24 bytes, same 18 bytes of data
```

### Arena Allocation

For many small, same-lifetime allocations:

```rust
use bumpalo::Bump;

fn process_batch(records: &[Record]) {
    // Arena for batch-lifetime allocations
    let arena = Bump::new();

    for record in records {
        // Allocates from arena, no individual free
        let parsed = arena.alloc(parse_record(record));
        process(parsed);
    }
    // All allocations freed at once when arena drops
}
```

### Small String Optimization

For short strings, avoid heap allocation:

```rust
use smartstring::alias::String as SmartString;

// Standard String: Always heap-allocated
let s: std::string::String = "hello".to_string();  // Heap allocation

// SmartString: Inline for short strings
let s: SmartString = "hello".into();  // Inline, no allocation
```

---

## I/O Patterns

Efficient I/O is critical for database performance.

### Sequential vs Random

```
Sequential read:  ~500 MB/s (SSD), ~150 MB/s (HDD)
Random read:      ~50 MB/s (SSD), ~1 MB/s (HDD)
```

The append-only log is designed for sequential I/O:
- Writes are always sequential (append)
- Reads during recovery are sequential (scan)
- Random reads go through indexed projections

### Direct I/O

Bypass the kernel page cache for predictable latency:

```rust
use std::os::unix::fs::OpenOptionsExt;

let file = OpenOptions::new()
    .read(true)
    .write(true)
    .custom_flags(libc::O_DIRECT)
    .open(path)?;
```

When to use:
- Large sequential writes (log segments)
- When you manage your own cache

When NOT to use:
- Small random reads (kernel cache helps)
- Reads that benefit from prefetching

### Async I/O with io_uring (Linux)

For high-throughput I/O on Linux:

```rust
// Future: io_uring support for batched async I/O
// Reduces syscall overhead significantly
```

---

## Benchmarking

### Criterion for Micro-Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_log_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_append");

    for size in [64, 256, 1024, 4096].iter() {
        group.bench_with_input(
            BenchmarkId::new("record_size", size),
            size,
            |b, &size| {
                let mut log = Log::new_in_memory();
                let record = Record::new(vec![0u8; size]);
                b.iter(|| log.append(&record));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_log_append);
criterion_main!(benches);
```

### Latency Histograms

Track latency distribution, not just averages:

```rust
use hdrhistogram::Histogram;

struct LatencyTracker {
    histogram: Histogram<u64>,
}

impl LatencyTracker {
    fn record(&mut self, latency_ns: u64) {
        self.histogram.record(latency_ns).unwrap();
    }

    fn report(&self) {
        println!("p50:   {} ns", self.histogram.value_at_quantile(0.50));
        println!("p90:   {} ns", self.histogram.value_at_quantile(0.90));
        println!("p99:   {} ns", self.histogram.value_at_quantile(0.99));
        println!("p99.9: {} ns", self.histogram.value_at_quantile(0.999));
    }
}
```

### Load Testing

For end-to-end performance:

```bash
# Run load test with varying client counts
for clients in 1 10 50 100; do
    verity-bench --clients $clients --duration 60s --report latency.csv
done
```

---

## Anti-Patterns

### Avoid: Allocation in Hot Paths

```rust
// Bad: Allocates on every call
fn process(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();  // Allocation!
    // ...
    output
}

// Good: Reuse buffer
fn process_into(input: &[u8], output: &mut Vec<u8>) {
    output.clear();
    // ...
}
```

### Avoid: Unbounded Queues

```rust
// Bad: Can grow without limit
let (tx, rx) = unbounded_channel();

// Good: Backpressure when queue is full
let (tx, rx) = bounded_channel(1024);
```

### Avoid: Sync in Loop

```rust
// Bad: N fsyncs
for record in records {
    write(&record)?;
    fsync()?;
}

// Good: 1 fsync
for record in records {
    write(&record)?;
}
fsync()?;
```

### Avoid: String Formatting in Hot Paths

```rust
// Bad: Allocates string
tracing::debug!("Processing record: {:?}", record);

// Good: Use structured logging
tracing::debug!(record_id = %record.id, "Processing record");
```

### Avoid: Clone When Borrow Suffices

```rust
// Bad: Unnecessary clone
fn process(data: String) {
    println!("{}", data);
}
process(my_string.clone());

// Good: Borrow
fn process(data: &str) {
    println!("{}", data);
}
process(&my_string);
```

---

## Summary

VerityDB's performance philosophy:

1. **Correctness first**: Never sacrifice correctness for speed
2. **Network → Disk → Memory → CPU**: Optimize in this order
3. **Batch operations**: Amortize fixed costs
4. **Zero-copy**: Avoid unnecessary data movement
5. **Measure everything**: Profile before optimizing
6. **Predictable latency**: Prefer consistency over peak performance

When in doubt, write correct, clear code first. Optimize only when profiling shows it's necessary, and only the specific bottleneck identified.
