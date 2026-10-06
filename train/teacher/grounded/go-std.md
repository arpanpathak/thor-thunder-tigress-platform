<!-- source: go-std/src/sync/oncefunc_test.go; section: 020a86591ee78368; licence: BSD -->
### User
In Go, I load a config file lazily the first time it's needed, with a `sync.Once` and two package variables for the result and the error. Is there something tidier?

### Assistant
Yes: `sync.OnceValues` (Go 1.21). It wraps a function that returns a value and an error, and gives back a function that runs it once and returns the same results on every later call, from any goroutine. No package variables, no separate `Once`.

```go
package main

import (
	"errors"
	"fmt"
	"sync"
)

type config struct{ port int }

func main() {
	loads := 0
	loadConfig := sync.OnceValues(func() (config, error) {
		loads++
		return config{port: 8080}, nil
	})

	var wg sync.WaitGroup
	for range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if cfg, err := loadConfig(); err != nil || cfg.port != 8080 {
				panic("every caller sees the same result")
			}
		}()
	}
	wg.Wait()
	if loads != 1 {
		panic("the loader must run exactly once")
	}

	broken := 0
	failing := sync.OnceValues(func() (config, error) {
		broken++
		return config{}, errors.New("no config file")
	})
	_, first := failing()
	_, second := failing()
	if broken != 1 || first == nil || first != second {
		panic("an error is cached like a value")
	}
	fmt.Println("ok")
}
```

Its relatives are `sync.OnceFunc` for a function with no result and `sync.OnceValue` for a single result. The standard library's tests pin down the behaviour you can rely on:

- the wrapped function runs exactly once, however many times or goroutines call it;
- calling the returned function allocates nothing;
- if the wrapped function panics, every call re-panics with the same value, and the function still is not run again.

The last point matters for your config: an error is cached too. If you want a failed load to be retried later, `OnceValues` is the wrong tool; keep the result behind a mutex and decide yourself when to try again.

### User
What's the Rust equivalent?

### Assistant
`std::sync::OnceLock` for a value you fill on first use, or `LazyLock` when the initializer is fixed. Both are safe to share between threads.

```rust
use std::sync::OnceLock;

/// The server settings, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The port to listen on.
    pub port: u16,
}

/// The settings, loaded on the first call and shared afterwards.
pub fn config(cell: &OnceLock<Config>, load: impl FnOnce() -> Config) -> &Config {
    cell.get_or_init(load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn loads_once_across_threads() {
        let cell = OnceLock::new();
        let loads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let loaded = config(&cell, || {
                        loads.fetch_add(1, Ordering::Relaxed);
                        Config { port: 8080 }
                    });
                    assert_eq!(loaded.port, 8080);
                });
            }
        });
        assert_eq!(loads.load(Ordering::Relaxed), 1);
    }
}
```

Two differences from Go are worth knowing. `OnceLock` has no error slot: for a load that can fail, store a `Result` in it if failures should be cached like Go does, or, to allow a retry, initialize only on success. And panics behave differently: if the initializer of a `LazyLock` panics, the lock is poisoned and every later access panics too, while a panic inside `OnceLock::get_or_init` leaves the cell empty, so the next caller runs its initializer again.
