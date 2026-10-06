<!-- source: std thread::scope docs; slice::chunks docs -->
### User
Sum a big Vec<u64> using all CPU cores.

### Assistant
`thread::scope` lets the threads borrow the slice directly, with no `Arc` and no copying, because the scope waits for every thread before it returns.

```rust
use std::{num::NonZeroUsize, thread};

/// The sum of `values`, split across the machine's cores. Wraps on overflow
/// like `wrapping_add`.
#[must_use]
pub fn parallel_sum(values: &[u64]) -> u64 {
    let threads = thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let chunk_size = values.len().div_ceil(threads).max(1);
    thread::scope(|scope| {
        let workers: Vec<_> = values
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(move || chunk.iter().fold(0_u64, |sum, &value| sum.wrapping_add(value))))
            .collect();
        workers
            .into_iter()
            .filter_map(|worker| worker.join().ok())
            .fold(0, u64::wrapping_add)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_plain_sum() {
        let values: Vec<u64> = (1..=100_000).collect();
        assert_eq!(parallel_sum(&values), values.iter().sum::<u64>());
    }

    #[test]
    fn handles_empty_and_tiny_inputs() {
        assert_eq!(parallel_sum(&[]), 0);
        assert_eq!(parallel_sum(&[7]), 7);
    }
}
```

`chunks` panics on a size of 0, so `.max(1)` covers the empty slice. A thread that panics is skipped by `filter_map` and its part is missing from the sum; the closures here cannot panic, but if yours can, return a `Result` instead.

For summing alone, threads only help with tens of millions of values: below that, starting them costs more than the additions. Measure before keeping it.

---

<!-- source: std sync::mpsc docs -->
### User
I have a list of URLs (just strings for now) and a slow function that processes each. Run them on 4 worker threads and collect the results in the original order.

### Assistant
Send each result back over a channel with its position, then put them in order at the end. The workers take jobs from a shared iterator behind a `Mutex`, so a slow job does not hold up the others.

```rust
use std::{
    sync::{Mutex, mpsc},
    thread,
};

/// `process` applied to every item on `workers` threads, results in the
/// order of `items`. `None` marks an item whose worker panicked.
pub fn run_all<T, R, F>(items: Vec<T>, workers: usize, process: F) -> Vec<Option<R>>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync,
{
    let count = items.len();
    let jobs = Mutex::new(items.into_iter().enumerate());
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            let sender = sender.clone();
            let (jobs, process) = (&jobs, &process);
            scope.spawn(move || {
                while let Some((position, item)) = jobs.lock().ok().and_then(|mut jobs| jobs.next()) {
                    if sender.send((position, process(item))).is_err() {
                        return;
                    }
                }
            });
        }
    });
    drop(sender);
    let mut results: Vec<Option<R>> = std::iter::repeat_with(|| None).take(count).collect();
    for (position, result) in receiver {
        if let Some(slot) = results.get_mut(position) {
            *slot = Some(result);
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_original_order() {
        let urls: Vec<String> = (0..20).map(|n| format!("https://example.org/{n}")).collect();
        let lengths = run_all(urls.clone(), 4, |url| url.len());
        let expected: Vec<Option<usize>> = urls.iter().map(|url| Some(url.len())).collect();
        assert_eq!(lengths, expected);
    }

    #[test]
    fn zero_workers_still_runs() {
        assert_eq!(run_all(vec![1, 2], 0, |n| n * 10), [Some(10), Some(20)]);
    }
}
```

The lock is held only while taking the next job, not while processing it: `jobs.lock().ok().and_then(...)` drops the guard at the end of the `while let` condition. A poisoned lock (another worker panicked while holding it) makes `lock()` return `Err`, and the worker stops instead of panicking too.

---

<!-- source: std io::BufRead docs; std::error::Error docs -->
### User
Read a file of numbers, one per line, and return their sum. Tell me which line is bad if one isn't a number.

### Assistant
```rust
use std::{
    fmt,
    fs::File,
    io::{self, BufRead, BufReader},
    num::ParseIntError,
    path::Path,
};

/// Why the numbers could not be summed.
#[derive(Debug)]
pub enum SumError {
    /// The file could not be opened or read.
    Io(io::Error),
    /// A line is not a whole number.
    NotANumber {
        /// The line number, from 1.
        line: usize,
        /// What the parser said.
        source: ParseIntError,
    },
    /// The sum does not fit in an `i64`.
    Overflow,
}

impl fmt::Display for SumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SumError::Io(error) => write!(f, "could not read: {error}"),
            SumError::NotANumber { line, source } => write!(f, "line {line}: {source}"),
            SumError::Overflow => f.write_str("the sum is too large for i64"),
        }
    }
}

impl std::error::Error for SumError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SumError::Io(error) => Some(error),
            SumError::NotANumber { source, .. } => Some(source),
            SumError::Overflow => None,
        }
    }
}

/// The sum of the numbers in the file at `path`, one per line. Blank lines
/// are skipped.
///
/// # Errors
///
/// [`SumError::Io`] for a read failure, [`SumError::NotANumber`] for the first
/// bad line, and [`SumError::Overflow`] when the total does not fit.
pub fn sum_file(path: &Path) -> Result<i64, SumError> {
    let file = File::open(path).map_err(SumError::Io)?;
    sum_lines(BufReader::new(file))
}

/// The sum of the numbers read from `reader`, one per line.
///
/// # Errors
///
/// The same as [`sum_file`].
pub fn sum_lines(reader: impl BufRead) -> Result<i64, SumError> {
    let mut total: i64 = 0;
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(SumError::Io)?;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let number: i64 = text.parse().map_err(|source| SumError::NotANumber { line: index + 1, source })?;
        total = total.checked_add(number).ok_or(SumError::Overflow)?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_every_line() -> Result<(), SumError> {
        assert_eq!(sum_lines("1\n-2\n\n 40 \n".as_bytes())?, 39);
        Ok(())
    }

    #[test]
    fn names_the_bad_line() {
        let error = sum_lines("1\n2\nthree\n".as_bytes()).err().map(|error| error.to_string());
        assert_eq!(error.as_deref(), Some("line 3: invalid digit found in string"));
    }

    #[test]
    fn reports_overflow_instead_of_wrapping() {
        let text = format!("{}\n1\n", i64::MAX);
        assert!(matches!(sum_lines(text.as_bytes()), Err(SumError::Overflow)));
    }
}
```

`sum_lines` takes any `BufRead`, so the tests pass a byte string instead of creating files, and `sum_file` only adds opening the file.

---

<!-- source: std sync::Mutex and sync::PoisonError docs -->
### User
How do I lock a Mutex without unwrap?

### Assistant
`lock()` only fails when another thread panicked while holding the lock; the data is then "poisoned". You have two honest options, depending on whether the data can still be trusted after such a panic.

```rust
use std::{
    collections::HashMap,
    fmt,
    sync::{Mutex, PoisonError},
};

/// A failure to use the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheError {
    /// A thread panicked while changing the cache, so its contents may be half-written.
    Poisoned,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Poisoned => f.write_str("a thread panicked while holding the cache lock"),
        }
    }
}

impl std::error::Error for CacheError {}

/// Names by id, shared between threads.
#[derive(Debug, Default)]
pub struct Names {
    by_id: Mutex<HashMap<u32, String>>,
    lookups: Mutex<u64>,
}

impl Names {
    /// Stores a name, refusing to touch a cache a panic may have left half-written.
    ///
    /// # Errors
    ///
    /// [`CacheError::Poisoned`] when another thread panicked while holding the lock.
    pub fn insert(&self, id: u32, name: &str) -> Result<(), CacheError> {
        let mut by_id = self.by_id.lock().map_err(|_| CacheError::Poisoned)?;
        by_id.insert(id, name.to_string());
        Ok(())
    }

    /// Counts a lookup. A counter can't be left half-written, so a poisoned
    /// lock is taken over instead of failing.
    pub fn count_lookup(&self) {
        let mut lookups = self.lookups.lock().unwrap_or_else(PoisonError::into_inner);
        *lookups += 1;
    }

    /// The lookups counted so far.
    #[must_use]
    pub fn lookups(&self) -> u64 {
        *self.lookups.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_counts() -> Result<(), CacheError> {
        let names = Names::default();
        names.insert(1, "Ada")?;
        names.count_lookup();
        assert_eq!(names.lookups(), 1);
        Ok(())
    }
}
```

Return an error (`map_err`) when a half-finished update could break an invariant, like a map and a count that must agree. Take the data back with `PoisonError::into_inner` when every single write leaves it valid, like a plain counter.

---

<!-- source: std sync::mpsc::Receiver::recv_timeout docs -->
### User
Wait for a result from a worker thread, but give up after 2 seconds.

### Assistant
`recv_timeout` on a channel does exactly that. The worker keeps running after the timeout; Rust has no way to kill a thread from outside, so a worker that may hang needs its own way to stop, such as checking a flag.

```rust
use std::{
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

/// How waiting for a worker ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waited<T> {
    /// The worker answered in time.
    Done(T),
    /// The time ran out first.
    TimedOut,
    /// The worker ended without answering, for example by panicking.
    Gone,
}

/// Runs `work` on a new thread and waits up to `limit` for its result.
pub fn with_timeout<T, F>(limit: Duration, work: F) -> Waited<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || sender.send(work()));
    match receiver.recv_timeout(limit) {
        Ok(value) => Waited::Done(value),
        Err(RecvTimeoutError::Timeout) => Waited::TimedOut,
        Err(RecvTimeoutError::Disconnected) => Waited::Gone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_a_fast_answer() {
        assert_eq!(with_timeout(Duration::from_secs(2), || 6 * 7), Waited::Done(42));
    }

    #[test]
    fn gives_up_on_a_slow_one() {
        let slow = || {
            thread::sleep(Duration::from_millis(500));
            1
        };
        assert_eq!(with_timeout(Duration::from_millis(50), slow), Waited::TimedOut);
    }
}
```

The `Waited` enum keeps the three outcomes apart: a timeout and a crashed worker usually need different handling, and a `Result<T, ()>` would merge them.
