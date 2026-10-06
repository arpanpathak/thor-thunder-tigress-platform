<!-- source: cpython-lib/Lib/bisect.py; section: 62b3401c8413af98; licence: PSF-2.0 -->
### User
What's the difference between `bisect_left` and `bisect_right`?

### Assistant
They differ only when the value is already in the list. Both return the position at which `x` keeps the list sorted, and CPython's `bisect.py` documents each by an invariant:

- `bisect_left(a, x)` returns `i` such that every element of `a[:i]` is `< x` and every element of `a[i:]` is `>= x`. Inserting there puts `x` before any equal elements.
- `bisect_right(a, x)` returns `i` such that every element of `a[:i]` is `<= x` and every element of `a[i:]` is `> x`. Inserting there puts `x` after any equal elements.

That makes the pair useful beyond insertion: the equal elements sit exactly between the two positions, so counting a value or taking a range of values costs two binary searches.

```python
from bisect import bisect_left, bisect_right, insort

scores = [10, 20, 20, 20, 30]
assert bisect_left(scores, 20) == 1
assert bisect_right(scores, 20) == 4
assert bisect_right(scores, 20) - bisect_left(scores, 20) == 3

assert scores[bisect_left(scores, 15):bisect_right(scores, 25)] == [20, 20, 20]

insort(scores, 25)
assert scores == [10, 20, 20, 20, 25, 30]
```

Both are O(log n) comparisons, but `insort` is O(n) overall because `list.insert` shifts the elements after the position. For a list you insert into constantly, a heap or a sorted container may fit better.

### User
I passed `key=` to `bisect_right` with a list of tuples and got a TypeError. What am I doing wrong?

### Assistant
With `key=`, the bisect functions compare `x` against `key(a[mid])`, and they do **not** apply `key` to `x`. So `x` must already be a key, not a whole item. The `insort` functions are different: they need the whole item to insert it, so they compute `key(x)` themselves. You can see it in the source:

```python
if key is None:
    lo = bisect_right(a, x, lo, hi)
else:
    lo = bisect_right(a, key(x), lo, hi, key=key)
a.insert(lo, x)
```

So search with the key value and insert with the item:

```python
from bisect import bisect_right, insort
from operator import itemgetter

events = [(1, "boot"), (5, "load"), (9, "stop")]
by_time = itemgetter(0)

assert bisect_right(events, 5, key=by_time) == 2

insort(events, (7, "ready"), key=by_time)
assert events == [(1, "boot"), (5, "load"), (7, "ready"), (9, "stop")]
```

Your TypeError came from passing the tuple: Python then compares a tuple with the integer `key(a[mid])` returns, and `<` between them is not defined. The `key` parameter exists since Python 3.10.

### User
What's the equivalent in Rust?

### Assistant
`slice::partition_point`. It takes a predicate that is true for a prefix of the slice and returns where it stops being true, so both bisections are one line:

```rust
/// The first position whose value is not less than `x`, like Python's `bisect_left`.
#[must_use]
pub fn bisect_left(sorted: &[i32], x: i32) -> usize {
    sorted.partition_point(|&value| value < x)
}

/// The first position whose value is greater than `x`, like Python's `bisect_right`.
#[must_use]
pub fn bisect_right(sorted: &[i32], x: i32) -> usize {
    sorted.partition_point(|&value| value <= x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_python_behaviour() {
        let scores = [10, 20, 20, 20, 30];
        assert_eq!((bisect_left(&scores, 20), bisect_right(&scores, 20)), (1, 4));
        assert_eq!((bisect_left(&scores, 5), bisect_right(&scores, 99)), (0, 5));
    }

    #[test]
    fn insertion_keeps_the_order() {
        let mut scores = vec![10, 20, 30];
        scores.insert(bisect_right(&scores, 25), 25);
        assert_eq!(scores, [10, 20, 25, 30]);
    }
}
```

A key works the same way: compare the key inside the predicate, `events.partition_point(|&(time, _)| time <= 5)`. Note that `slice::binary_search` is not a replacement: when the value occurs several times, it may return the position of any of them.
