<!-- source: tokio/tokio/src/sync/once_cell.rs; section: 05c8cfa1318f2111; licence: MIT -->
### User
I need to create a database pool once, lazily, from async code. `std::sync::OnceLock` can't `.await` in its initializer. How does `tokio::sync::OnceCell` handle that, and is it safe if the task doing the init gets cancelled?

### Assistant
`tokio::sync::OnceCell` is built from three fields: an `AtomicBool` that says whether the value is set, the value slot itself, and a `Semaphore` with a single permit. `get_or_init` works like this:

1. **Fast path.** If the flag says the value is set, return a reference to it. No locking at all.
2. **Otherwise, wait for the permit** with `semaphore.acquire().await`. Only one task can hold it, so only one task runs the initializer; the others wait asynchronously, without blocking a thread.
3. **The task with the permit** runs `f().await`, stores the value, and then *closes* the semaphore.
4. **Every waiting task** now gets an error from `acquire()` because the semaphore is closed, and takes that as the signal that the value is ready, so it returns the stored value.

The invariant tokio keeps is: if the semaphore is closed, the value is set.

Cancellation is the reason for the semaphore. If the initializing task is dropped halfway through `f().await` (it lost a `select!`, its request was cancelled, or `f` panicked), its permit is dropped with it, the semaphore is still open, and the next waiting task acquires the permit and runs *its own* initializer. Nothing is left half-initialized, and no task waits forever on an initializer that will never finish.

```rust,ignore
use tokio::sync::OnceCell;

static POOL: OnceCell<Pool> = OnceCell::const_new();

async fn pool() -> &'static Pool {
    POOL.get_or_init(|| async { Pool::connect("postgres://localhost/app").await }).await
}
```

If connecting can fail, use `get_or_try_init`, which leaves the cell empty on an error so a later call can try again. `std::sync::OnceLock` remains the right choice when the initializer is ordinary synchronous code: there the waiting is done by blocking threads, which inside an async runtime would stall every task on those threads.
