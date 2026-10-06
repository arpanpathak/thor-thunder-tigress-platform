<!-- source: comprehensive-rust/src/unsafe-deep-dive/ffi/abs.md; section: 01cc5af0c5432acc; licence: Apache-2.0 -->
### User
How do I call the C library's `abs` from Rust?

### Assistant
Declare it in an `extern "C"` block whose signature matches the C declaration, `int abs(int j);`. Cargo links the C standard library by default, so the symbol is already available.

```rust
use std::ffi::c_int;

unsafe extern "C" {
    fn abs(x: c_int) -> c_int;
}

/// The absolute value of `x` from the C library, or `None` for `c_int::MIN`.
///
/// C leaves `abs(INT_MIN)` undefined, because the result does not fit in an
/// `int`; every other input is safe to pass, which is checked before the call.
#[must_use]
pub fn c_abs(x: c_int) -> Option<c_int> {
    if x == c_int::MIN {
        return None;
    }
    Some(unsafe { abs(x) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_into_libc() {
        assert_eq!(c_abs(-42), Some(42));
        assert_eq!(c_abs(7), Some(7));
        assert_eq!(c_abs(c_int::MIN), None);
    }
}
```

Three details:

- Use `std::ffi::c_int`, not `i32`. The C standard allows `int` to be narrower than 32 bits; `c_int` is whatever `int` is on the target, so the signature stays correct when the code is ported.
- The block itself must be written `unsafe extern` (required since the 2024 edition). It is your promise that the declarations match the real C functions; the compiler cannot check that.
- Inside the block you may mark a function `safe fn abs(x: c_int) -> c_int;`, which lets callers skip `unsafe`. Do that only when every possible input is safe. For `abs` it is not, because of `INT_MIN`, so the wrapper above keeps the call `unsafe` and rules out the one bad input first.

---

<!-- source: comprehensive-rust/src/idiomatic/leveraging-the-type-system/newtype-pattern.md; section: 02eb70896a417a12; licence: Apache-2.0 -->
### User
For a user id, should I write `type UserId = u64;` or `struct UserId(u64);`?

### Assistant
The struct, if you want the compiler to keep ids apart. A type alias is only a second name for `u64`: any `u64` is accepted where a `UserId` is expected, including a message id or a byte count. A newtype is a distinct type, so passing a plain number is a compile error.

```rust
/// A unique user identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(u64);

/// A message identifier: another name for `u64`, with no checking.
pub type MessageId = u64;

/// The id of the built-in administrator.
pub const ADMIN: UserId = UserId(0);

/// True for the administrator.
#[must_use]
pub fn is_admin(user: UserId) -> bool {
    user == ADMIN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_compare_but_aliases_mix_freely() {
        assert!(is_admin(UserId(0)));
        assert_ne!(UserId(1), UserId(2));
        let message: MessageId = 7;
        let bytes: u64 = message;
        assert_eq!(bytes, 7);
    }
}
```

`is_admin(1)` fails with "mismatched types: expected `UserId`, found integer", which is the point.

A newtype starts with no behaviour at all: not even `==` works until you derive or implement it. Add only what makes sense for the meaning. Comparing two user ids is reasonable, so `PartialEq` is derived; adding two user ids is meaningless, so there is no `Add`.

---

<!-- source: comprehensive-rust/src/iterators/iterator.md; section: 040d52ed0b20acc0; licence: Apache-2.0 -->
### User
I wrote an iterator over a slice with an index field `i` and `self.slice[self.i]`. Is there a cleaner way to write `next`?

### Assistant
Yes: keep the remaining part of the slice instead of an index, and take the first element off it each time. `split_first` returns the first element and the rest, or `None` when the slice is empty, so `?` ends the iteration and there is no index to keep in range.

```rust
/// Yields the elements of a slice in order.
#[derive(Debug, Clone)]
pub struct SliceIter<'s> {
    remaining: &'s [i32],
}

impl<'s> SliceIter<'s> {
    /// An iterator over `slice`.
    #[must_use]
    pub fn new(slice: &'s [i32]) -> Self {
        Self { remaining: slice }
    }
}

impl<'s> Iterator for SliceIter<'s> {
    type Item = &'s i32;

    fn next(&mut self) -> Option<Self::Item> {
        let (first, rest) = self.remaining.split_first()?;
        self.remaining = rest;
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yields_every_element_once() {
        let collected: Vec<&i32> = SliceIter::new(&[2, 4, 6, 8]).collect();
        assert_eq!(collected, [&2, &4, &6, &8]);
        assert_eq!(SliceIter::new(&[]).next(), None);
    }

    #[test]
    fn adaptors_work_on_it() {
        let sum: i32 = SliceIter::new(&[1, 2, 3, 4]).filter(|&&n| n % 2 == 0).sum();
        assert_eq!(sum, 6);
    }
}
```

Two properties are worth knowing. The iterator is lazy: `SliceIter::new` only stores the slice, and no element is touched until `next` is called. And because it implements `Iterator`, every adaptor (`filter`, `map`, `sum`, `zip`) works on it without more code. Iterators do not have to end, either: `0..` keeps producing numbers until it overflows.
