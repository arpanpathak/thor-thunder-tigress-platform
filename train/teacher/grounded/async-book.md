<!-- source: async-book/src/part-guide/io.md; section: 00d06ea218b2c94e; licence: MIT -->
### User
Inside a Tokio task I call `std::fs::read_to_string`. It works. Is there anything wrong with that?

### Assistant
Yes: it blocks the thread. `std::fs` functions are synchronous, so the operating system pauses the whole thread until the read finishes. The scheduler is not involved, so no other task can run on that thread in the meantime. If every thread of the runtime is blocked like this (and some runtimes have only one), the whole program stops making progress until the reads finish.

An async IO call, by contrast, pauses only the current task: `.await` hands the thread back to the scheduler, which runs other tasks and wakes this one when the OS reports the IO is done.

Two fixes, depending on the work:

```rust,ignore
use tokio::{fs, task};

async fn load(path: &str) -> std::io::Result<String> {
    fs::read_to_string(path).await
}

async fn checksum(path: String) -> std::io::Result<u64> {
    let bytes = task::spawn_blocking(move || std::fs::read(path)).await??;
    Ok(bytes.iter().map(|&byte| u64::from(byte)).sum())
}
```

Use the runtime's async version (`tokio::fs`) for plain IO. Use `spawn_blocking` for blocking work that has no async version, including long CPU-bound loops: it runs the closure on a separate pool of threads meant for blocking, so the async worker threads stay free. Your program works today because one small read finishes quickly; under load, or with a slow disk or network file system, those blocked threads add up.

---

<!-- source: async-book/src/03_async_await/01_chapter.md; section: 020353942a670a7f; licence: MIT -->
### User
If I call an async function and don't `.await` it, does any of its code run?

### Assistant
No. An `async fn` call, like an `async` block, only creates a value that implements `Future`; none of the body runs until something polls that future. Usually that something is `.await`, which polls it to completion, yielding the thread whenever the future cannot make progress yet. If you drop the future without awaiting it, the body never runs (the compiler warns, because futures are `#[must_use]`).

You can see this without any runtime by polling a future by hand with a waker that does nothing:

```rust
use std::{
    cell::Cell,
    future::{self, Future},
    pin::pin,
    task::{Context, Poll, Waker},
};

/// Sets `ran` once the future is polled.
pub async fn mark(ran: &Cell<bool>) {
    future::ready(()).await;
    ran.set(true);
}

/// Polls `future` once with a waker that does nothing.
pub fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
    let mut future = pin!(future);
    future.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_runs_until_the_future_is_polled() {
        let ran = Cell::new(false);
        let future = mark(&ran);
        assert!(!ran.get());
        assert_eq!(poll_once(future), Poll::Ready(()));
        assert!(ran.get());
    }
}
```

This laziness is the main difference from promises in JavaScript, which start running as soon as they are created. In Rust, if you want work to start without awaiting it right away, you hand the future to the runtime, for example with `tokio::spawn`.

---

<!-- source: async-book/src/part-reference/pinning.md; section: 033a457acd9acf3c; licence: MIT -->
### User
Why does `Future::poll` take `self: Pin<&mut Self>` instead of `&mut self`?

### Assistant
Because the futures that `async fn` produces can point into themselves. Every local variable that lives across an `.await` is stored in the future, and one of them can be a reference to another:

```rust,ignore
async fn foo() {
    let a = make_a();
    let b = &a;
    bar().await;
    use_it(b);
}
```

The compiler turns this into a struct holding both `a` and `b`, where `b` points at the struct's own `a` field. Rust has no `'self` lifetime to describe that, and for a good reason: moving the struct would copy `a` to a new address while `b` still pointed at the old one. That is unsound even with raw pointers.

If the future never moves after it starts running, the self-reference stays valid. `Pin<&mut Self>` is that promise: a pinned value will not be moved again (unless its type is `Unpin`, meaning it holds no such references). With `async`/`.await` the compiler and runtime handle pinning for you.

### User
So when will I actually have to deal with `Pin` myself?

### Assistant
When you use futures directly instead of through `.await`. The common cases are:

- polling a future yourself, or implementing `Future` by hand, where `self` inside `poll` is pinned;
- storing futures of different types together, which means boxing them as `Pin<Box<dyn Future<Output = T>>>` with `Box::pin`;
- reusing a future across loop iterations, for example in `select!`, which needs a pinned reference to it;
- working with streams, where you tend to call combinators or `poll_next` directly.

The boxed case looks like this:

```rust
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

/// A boxed future, so futures of different types fit in one `Vec`.
pub type Job = Pin<Box<dyn Future<Output = u32>>>;

/// Polls every job once with a waker that does nothing, returning the
/// outputs of the jobs that finished.
pub fn poll_all(jobs: &mut [Job]) -> Vec<u32> {
    let mut context = Context::from_waker(Waker::noop());
    jobs.iter_mut()
        .filter_map(|job| match job.as_mut().poll(&mut context) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future;

    async fn double(value: u32) -> u32 {
        future::ready(value * 2).await
    }

    #[test]
    fn polls_futures_of_different_types() {
        let mut jobs: Vec<Job> = vec![Box::pin(double(4)), Box::pin(future::ready(7)), Box::pin(future::pending())];
        assert_eq!(poll_all(&mut jobs), [8, 7]);
    }
}
```

`Box::pin` puts the future on the heap and pins it there, so the `Vec` can move the box around freely while the future itself stays put. `job.as_mut()` turns the `Pin<Box<...>>` into the `Pin<&mut ...>` that `poll` needs.
