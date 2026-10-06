<!-- source: go-website/_content/blog/unique.md; section: 00328a3fb0199f5f; licence: BSD -->
### User
My Go parser creates millions of duplicate strings. How do I intern them?

### Assistant
The simplest version is a map from each string to its first copy:

```go,ignore
var internPool map[string]string

func Intern(s string) string {
	pooled, ok := internPool[s]
	if !ok {
		pooled = strings.Clone(s)
		internPool[pooled] = pooled
	}
	return pooled
}
```

`strings.Clone` matters: if `s` is a slice of a much bigger string (a line of the file), storing it directly would keep the whole bigger string alive.

That version has three problems: it never removes anything, so the pool only grows; it is not safe to call from several goroutines; and it only handles strings. It also misses a speed-up: two Go strings are a pointer and a length, and comparing them compares contents whenever the pointers differ. If every string is canonical, equal strings share one pointer, so comparing pointers would be enough, but a `string` comparison cannot know that.

Since Go 1.23 the standard library's `unique` package does all of this. `unique.Make` returns a `Handle[T]` for any comparable `T`; equal values get equal handles, comparing handles compares one pointer, it is safe for concurrent use, and entries are dropped once no handle refers to them.

```go
package main

import (
	"fmt"
	"strings"
	"unique"
)

func main() {
	line := "GET /index.html HTTP/1.1"
	method := unique.Make(line[:3])
	again := unique.Make(strings.Clone("GET"))
	if method != again {
		panic("equal strings must give equal handles")
	}
	fmt.Println(method.Value())
}
```

Keep the `Handle` (not `Value()`) in your data structures; that is what makes comparison cheap.

---

<!-- source: go-website/_content/blog/strings.md; section: 006a91722127d42c; licence: BSD -->
### User
A Go string prints as garbage characters. How do I see what bytes it actually holds?

### Assistant
Use the `fmt` verbs that show bytes instead of rendering them as text:

- `% x` prints every byte in hex with spaces between them (`%x` without the space runs them together).
- `%q` prints a quoted Go string literal, escaping anything not printable, so stray bytes show up as `\x..`.
- `%+q` also escapes every non-ASCII character, showing valid UTF-8 as `\u` escapes, which exposes exactly which code points are there.

```go
package main

import "fmt"

func main() {
	const sample = "\xbd\xb2\x3d\xbc\x20\xe2\x8c\x98"
	checks := []struct{ format, want string }{
		{"%x", "bdb23dbc20e28c98"},
		{"% x", "bd b2 3d bc 20 e2 8c 98"},
		{"%q", `"\xbd\xb2=\xbc ⌘"`},
		{"%+q", `"\xbd\xb2=\xbc \u2318"`},
	}
	for _, check := range checks {
		got := fmt.Sprintf(check.format, sample)
		if got != check.want {
			panic(check.format + " gave " + got)
		}
		fmt.Println(check.format, got)
	}
}
```

Read with `%+q`, the string is two bytes that are not valid UTF-8, an `=` sign, another invalid byte, a space, and U+2318 (⌘), whose UTF-8 encoding is the three bytes `e2 8c 98`.

Two things to remember: indexing a string (`s[i]`) gives bytes, not characters, and these verbs work the same on `[]byte` as on `string`.

---

<!-- source: go-website/_content/ref/mod.md; section: 00afaf76d3c17864; licence: BSD -->
### User
A dependency version has a bug, so I added `exclude example.com/lib v1.2.3` to my go.mod. What does Go pick instead?

### Assistant
Since Go 1.16, an excluded version is simply ignored wherever a `require` names it. The `go` command does not substitute another version on the spot; instead, commands such as `go get` and `go mod tidy` may add a requirement on a higher version to your `go.mod` (marked `// indirect` if you don't import the module directly). So after adding the `exclude`, run `go mod tidy` and check which version it recorded.

Before Go 1.16 the behaviour was different: the `go` command listed the module's available versions and loaded the next higher one that was not excluded. That made the build depend on what had been published at the time, so the same `go.mod` could build with different versions.

Two details that surprise people:

- `exclude` only takes effect in the main module's `go.mod`. An `exclude` inside one of your dependencies' `go.mod` files is ignored, so a library cannot exclude versions for its users.
- You can exclude several versions in a block:

```text
exclude (
    golang.org/x/crypto v1.4.5
    golang.org/x/text v1.6.7
)
```
