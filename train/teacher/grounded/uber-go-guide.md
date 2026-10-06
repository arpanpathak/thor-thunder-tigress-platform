<!-- source: uber-go-guide/src/error-wrap.md; section: 0296aa6b92d31e25; licence: Apache-2.0 -->
### User
In Go, when a call returns an error, should I return it as is, or wrap it with `fmt.Errorf`? And `%w` or `%v`?

### Assistant
Three choices, by what the caller needs:

- **Return it unchanged** when you have nothing to add. The caller keeps the original type and message.
- **Wrap with `%w`** to add context and keep the cause reachable: `errors.Is` and `errors.As` still find the original error. This is the usual default, but it makes the wrapped error part of your function's contract, so if callers can match on it, document and test that.
- **Wrap with `%v`** to add context and hide the cause: the message keeps the text, but callers can no longer match the original. You can switch to `%w` later without breaking anyone; going the other way can break callers.

Keep the context short. "failed to" adds nothing, and it piles up as the error travels up the stack: `failed to x: failed to y: failed to create new store: the error` says less, in more words, than `x: y: new store: the error`.

```go
package main

import (
	"errors"
	"fmt"
	"io/fs"
)

func openStore() error {
	return fs.ErrNotExist
}

func newStoreWrapped() error {
	if err := openStore(); err != nil {
		return fmt.Errorf("new store: %w", err)
	}
	return nil
}

func newStoreHidden() error {
	if err := openStore(); err != nil {
		return fmt.Errorf("new store: %v", err)
	}
	return nil
}

func main() {
	wrapped := fmt.Errorf("start service: %w", newStoreWrapped())
	hidden := newStoreHidden()
	if wrapped.Error() != "start service: new store: file does not exist" {
		panic(wrapped)
	}
	if !errors.Is(wrapped, fs.ErrNotExist) {
		panic("%w keeps the cause matchable through every layer")
	}
	if errors.Is(hidden, fs.ErrNotExist) {
		panic("%v hides the cause")
	}
	fmt.Println(wrapped)
}
```

Once an error leaves your program, for example in a log line or a response to another service, make it obvious that it is an error, with an `err` field or a "failed" prefix there; inside the program the short form is enough.
