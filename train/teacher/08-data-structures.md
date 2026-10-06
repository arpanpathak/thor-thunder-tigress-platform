<!-- source: Learning Rust With Entirely Too Many Linked Lists (a bad stack, an ok stack) -->
### User
Implement a stack as a linked list in Rust.

### Assistant
Each node owns the next one through `Option<Box<Node<T>>>`. `take()` moves the head out and leaves `None`, which is how `push` and `pop` rewire the list without fighting the borrow checker.

```rust
type Link<T> = Option<Box<Node<T>>>;

#[derive(Debug)]
struct Node<T> {
    value: T,
    next: Link<T>,
}

/// A last-in, first-out stack of linked nodes.
#[derive(Debug)]
pub struct Stack<T> {
    head: Link<T>,
    len: usize,
}

impl<T> Default for Stack<T> {
    fn default() -> Self {
        Self { head: None, len: 0 }
    }
}

impl<T> Stack<T> {
    /// Puts `value` on top.
    pub fn push(&mut self, value: T) {
        let next = self.head.take();
        self.head = Some(Box::new(Node { value, next }));
        self.len += 1;
    }

    /// Removes and returns the top value, or `None` when empty.
    pub fn pop(&mut self) -> Option<T> {
        let node = self.head.take()?;
        self.head = node.next;
        self.len -= 1;
        Some(node.value)
    }

    /// The top value, or `None` when empty.
    #[must_use]
    pub fn peek(&self) -> Option<&T> {
        self.head.as_deref().map(|node| &node.value)
    }

    /// How many values are on the stack.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the stack holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }
}

impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        let mut link = self.head.take();
        while let Some(mut node) = link {
            link = node.next.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pops_in_reverse_order() {
        let mut stack = Stack::default();
        stack.push(1);
        stack.push(2);
        assert_eq!(stack.peek(), Some(&2));
        assert_eq!((stack.pop(), stack.pop(), stack.pop()), (Some(2), Some(1), None));
        assert!(stack.is_empty());
    }

    #[test]
    fn dropping_a_long_stack_does_not_overflow() {
        let mut stack = Stack::default();
        (0..200_000).for_each(|value| stack.push(value));
        assert_eq!(stack.len(), 200_000);
    }
}
```

The hand-written `Drop` matters: the default one drops each node from inside the previous node's drop, so a long enough stack overflows the thread's stack. The loop drops one node at a time instead. In real code, `Vec<T>` with `push` and `pop` is a faster stack; a linked list is mostly worth writing to learn ownership.

---

<!-- source: trie (prefix tree); std collections::BTreeMap entry API and Iterator::try_fold docs -->
### User
Build a trie with insert, contains, starts_with and a way to list all words with a prefix.

### Assistant
Each node maps a character to a child node. A `BTreeMap` keeps the children sorted, so the listed words come out in alphabetical order.

```rust
use std::collections::BTreeMap;

/// A set of words stored by their shared prefixes.
#[derive(Debug, Default)]
pub struct Trie {
    children: BTreeMap<char, Trie>,
    ends_word: bool,
}

impl Trie {
    /// Adds `word`.
    pub fn insert(&mut self, word: &str) {
        let node = word.chars().fold(self, |node, c| node.children.entry(c).or_default());
        node.ends_word = true;
    }

    /// True when `word` was inserted.
    #[must_use]
    pub fn contains(&self, word: &str) -> bool {
        self.find(word).is_some_and(|node| node.ends_word)
    }

    /// True when some inserted word starts with `prefix`.
    #[must_use]
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.find(prefix).is_some()
    }

    /// Every inserted word that starts with `prefix`, in alphabetical order.
    #[must_use]
    pub fn words_with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut words = Vec::new();
        if let Some(node) = self.find(prefix) {
            node.collect(&mut prefix.to_string(), &mut words);
        }
        words
    }

    fn find(&self, prefix: &str) -> Option<&Trie> {
        prefix.chars().try_fold(self, |node, c| node.children.get(&c))
    }

    fn collect(&self, word: &mut String, words: &mut Vec<String>) {
        if self.ends_word {
            words.push(word.clone());
        }
        for (&c, child) in &self.children {
            word.push(c);
            child.collect(word, words);
            word.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trie() -> Trie {
        let mut trie = Trie::default();
        for word in ["car", "card", "care", "cat", "dog"] {
            trie.insert(word);
        }
        trie
    }

    #[test]
    fn tells_words_from_prefixes() {
        let trie = trie();
        assert!(trie.contains("car"));
        assert!(!trie.contains("ca"));
        assert!(trie.starts_with("ca"));
        assert!(!trie.starts_with("cow"));
    }

    #[test]
    fn lists_words_in_order() {
        assert_eq!(trie().words_with_prefix("car"), ["car", "card", "care"]);
        assert_eq!(trie().words_with_prefix("x"), Vec::<String>::new());
    }
}
```

`try_fold` walks down one character at a time and stops with `None` at the first missing child. `collect` recurses once per character of the longest word, which is fine for words but not for strings millions of characters long.

---

<!-- source: LRU cache; std collections::HashMap and BTreeMap::pop_first docs -->
### User
LRU cache in Rust without unsafe and without external crates.

### Assistant
Doubly linked lists are what make the textbook O(1) LRU, and they need `unsafe` or `Rc<RefCell<…>>` in Rust. A simpler safe design gives O(log n): stamp every use with an increasing counter, and keep a `BTreeMap` from stamp to key, so the least recently used key is always the first entry.

```rust
use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

/// A map that keeps at most `capacity` entries, dropping the least recently used.
#[derive(Debug)]
pub struct Lru<K, V> {
    capacity: usize,
    clock: u64,
    entries: HashMap<K, (V, u64)>,
    by_last_use: BTreeMap<u64, K>,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    /// An empty cache for up to `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self { capacity, clock: 0, entries: HashMap::new(), by_last_use: BTreeMap::new() }
    }

    /// The value for `key`, marking it as just used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let (value, last_use) = self.entries.get_mut(key)?;
        self.by_last_use.remove(last_use);
        *last_use = self.clock;
        self.by_last_use.insert(self.clock, key.clone());
        Some(value)
    }

    /// Stores `value` under `key`, dropping the least recently used entry when full.
    pub fn put(&mut self, key: K, value: V) {
        self.clock += 1;
        if let Some((_, last_use)) = self.entries.insert(key.clone(), (value, self.clock)) {
            self.by_last_use.remove(&last_use);
        }
        self.by_last_use.insert(self.clock, key);
        while self.entries.len() > self.capacity
            && let Some((_, oldest)) = self.by_last_use.pop_first()
        {
            self.entries.remove(&oldest);
        }
    }

    /// How many entries are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_the_least_recently_used() {
        let mut cache = Lru::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        assert_eq!(cache.get(&"a"), Some(&1));
        cache.put("c", 3);
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"c"), Some(&3));
    }

    #[test]
    fn replacing_a_value_keeps_one_entry() {
        let mut cache = Lru::new(2);
        cache.put("a", 1);
        cache.put("a", 2);
        assert_eq!((cache.len(), cache.get(&"a")), (1, Some(&2)));
    }

    #[test]
    fn capacity_zero_stores_nothing() {
        let mut cache = Lru::new(0);
        cache.put("a", 1);
        assert!(cache.is_empty());
    }
}
```

`get` takes `&mut self` because reading changes which entry is the oldest. The `Some(value)` turns the `&mut V` from `get_mut` into the `&V` the signature promises. The `u64` clock would need 584 years at a billion operations per second to wrap.

---

<!-- source: std collections::VecDeque docs -->
### User
Keep the last N response times in milliseconds and give me their average.

### Assistant
A `VecDeque` drops the oldest value from the front, and a running sum makes the average O(1) instead of re-adding the window each time.

```rust
use std::collections::VecDeque;

/// The average of the most recent response times.
#[derive(Debug, Clone)]
pub struct RecentTimes {
    window: VecDeque<u32>,
    capacity: usize,
    sum: u64,
}

impl RecentTimes {
    /// Keeps the last `capacity` times; a capacity of 0 is treated as 1.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self { window: VecDeque::with_capacity(capacity), capacity, sum: 0 }
    }

    /// Records one response time, forgetting the oldest when full.
    pub fn record(&mut self, millis: u32) {
        if self.window.len() == self.capacity
            && let Some(oldest) = self.window.pop_front()
        {
            self.sum -= u64::from(oldest);
        }
        self.window.push_back(millis);
        self.sum += u64::from(millis);
    }

    /// The average of the kept times in whole milliseconds, rounded down, or
    /// `None` before the first.
    #[must_use]
    pub fn average(&self) -> Option<u32> {
        let count = u64::try_from(self.window.len()).ok().filter(|&count| count > 0)?;
        u32::try_from(self.sum / count).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_only_the_window() {
        let mut times = RecentTimes::new(3);
        assert_eq!(times.average(), None);
        for millis in [100, 200, 300, 1000] {
            times.record(millis);
        }
        assert_eq!(times.average(), Some(500));
    }

    #[test]
    fn large_times_do_not_overflow() {
        let mut times = RecentTimes::new(2);
        times.record(u32::MAX);
        times.record(u32::MAX);
        assert_eq!(times.average(), Some(u32::MAX));
    }
}
```

The sum is a `u64`, so even a full window of `u32::MAX` values cannot overflow it for any window under four billion entries.

---

<!-- source: std iter::Iterator::by_ref and Iterator::next docs -->
### User
Transpose a Vec<Vec<T>> without index loops.

### Assistant
Hold one iterator per row and take the next item of each to build every column. A ragged input (rows of different lengths) has no transpose, so it returns `None`.

```rust
/// The columns of `rows` as rows, or `None` when the rows differ in length.
#[must_use]
pub fn transpose<T: Clone>(rows: &[Vec<T>]) -> Option<Vec<Vec<T>>> {
    let width = rows.first().map_or(0, Vec::len);
    if rows.iter().any(|row| row.len() != width) {
        return None;
    }
    let mut cursors: Vec<_> = rows.iter().map(|row| row.iter()).collect();
    let columns = std::iter::repeat_with(|| cursors.iter_mut().filter_map(Iterator::next).cloned().collect())
        .take(width)
        .collect();
    Some(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_rows_and_columns() {
        let rows = vec![vec![1, 2, 3], vec![4, 5, 6]];
        assert_eq!(transpose(&rows), Some(vec![vec![1, 4], vec![2, 5], vec![3, 6]]));
    }

    #[test]
    fn ragged_and_empty_inputs() {
        assert_eq!(transpose(&[vec![1, 2], vec![3]]), None);
        assert_eq!(transpose::<u8>(&[]), Some(Vec::new()));
    }
}
```

Every row is checked first, so each cursor yields exactly `width` items and no column comes out short.
