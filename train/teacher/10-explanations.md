<!-- source: Rust reference, reference patterns; Rust book ch18 -->
### User
What does `let &x = v.iter().max()?;` do? I've never seen `&` on the left of a let.

### Assistant
On the left of `=`, `&x` is a pattern, not a borrow. It matches a reference and binds `x` to the value behind it. `v.iter().max()` returns `Option<&i32>`, `?` unwraps the `Option` (returning `None` early when it is empty), and `&x` takes the `i32` out of the `&i32`.

That only works when the value is `Copy`, such as numbers, `char` and `bool`. For a `String` it fails to compile, because the pattern would move the string out of something borrowed.

These three lines all give `x: i32`:

```rust
/// The largest value three ways, to show they are the same.
#[must_use]
pub fn largest(values: &[i32]) -> Option<(i32, i32, i32)> {
    let &by_pattern = values.iter().max()?;
    let by_copied = values.iter().max().copied()?;
    let by_deref = *values.iter().max()?;
    Some((by_pattern, by_copied, by_deref))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_agree() {
        assert_eq!(largest(&[3, 8, 1]), Some((8, 8, 8)));
        assert_eq!(largest(&[]), None);
    }
}
```

They compile to the same code; which one to use is a matter of style.

---

<!-- source: std cmp::Reverse docs -->
### User
How does Reverse turn a BinaryHeap into a min-heap?

### Assistant
`BinaryHeap` always pops its largest element, by `Ord`. `Reverse<T>` is a wrapper whose `Ord` is `T`'s flipped: `Reverse(1) > Reverse(2)`. So the "largest" `Reverse` the heap pops is the one holding the smallest value.

```rust
use std::{cmp::Reverse, collections::BinaryHeap};

/// The values of `values` from smallest to largest, by popping a min-heap.
#[must_use]
pub fn ascending(values: &[u32]) -> Vec<u32> {
    let mut heap: BinaryHeap<Reverse<u32>> = values.iter().copied().map(Reverse).collect();
    std::iter::from_fn(|| heap.pop().map(|Reverse(value)| value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pops_the_smallest_first() {
        assert_eq!(ascending(&[5, 1, 4, 1]), [1, 1, 4, 5]);
        assert!(Reverse(1) > Reverse(2));
    }
}
```

The pattern `|Reverse(value)| value` unwraps it again on the way out. Tuples work too: `Reverse((distance, node))` pops the smallest distance first, which is what Dijkstra's algorithm needs.

---

<!-- source: std primitive f64 docs (total_cmp); IEEE 754 NaN -->
### User
Why can't I put f64 in a BinaryHeap or call sort() on a Vec<f64>?

### Assistant
Both need `Ord`, a total order where any two values compare as less, equal or greater. `f64` only has `PartialOrd`, because `NaN` is not less than, equal to or greater than anything, itself included: `f64::NAN == f64::NAN` is `false`.

For sorting, `total_cmp` gives an order for every `f64`, with `NaN` placed after positive infinity:

```rust
/// `values` sorted from smallest to largest, any `NaN` last.
#[must_use]
pub fn sorted(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(f64::total_cmp);
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_with_nan_last() {
        let result = sorted(vec![2.5, f64::NAN, -1.0, 0.0]);
        assert_eq!(result.get(..3), Some([-1.0, 0.0, 2.5].as_slice()));
        assert!(result.last().is_some_and(|last| last.is_nan()));
    }
}
```

For a `BinaryHeap`, wrap the value in a struct that implements `Ord` with `total_cmp`, or store integers instead, such as milliseconds rather than seconds. `total_cmp` also orders `-0.0` before `0.0`, which `==` treats as equal.

---

<!-- source: Rust 2024 edition guide (let chains); Rust reference, if let -->
### User
What is `if a && let Some(x) = b` ? Is that new?

### Assistant
It is a let chain, stable since Rust 1.88 in the 2024 edition. It joins boolean conditions and `let` patterns with `&&` in one `if` or `while`, and every `let` binding is usable in the later conditions and in the body. Before let chains, the same thing needed a nested `if let` inside an `if`.

```rust
/// The first word of `text` if it is longer than `min_len` characters.
#[must_use]
pub fn long_first_word(text: &str, min_len: usize) -> Option<&str> {
    if !text.is_empty()
        && let Some(word) = text.split_whitespace().next()
        && word.chars().count() > min_len
    {
        return Some(word);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_every_condition_in_order() {
        assert_eq!(long_first_word("thunder tigress", 3), Some("thunder"));
        assert_eq!(long_first_word("cub of thor", 3), None);
        assert_eq!(long_first_word("", 0), None);
    }
}
```

The conditions run left to right and stop at the first that fails, like `&&` between booleans. Let chains need `edition = "2024"` in `Cargo.toml`; on older editions the compiler rejects them.
