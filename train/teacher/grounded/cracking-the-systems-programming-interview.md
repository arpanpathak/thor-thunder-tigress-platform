<!-- source: cracking-the-systems-programming-interview/src/ch11-trees.md; section: 0039d9fc30a00749; licence: author's own -->
### User
How do I save a tree where each node can have any number of children to a file, and load the same tree back?

### Assistant
Writing the values in pre-order is not enough: the reader cannot tell where one node's children end. Write each node as two tokens, its value and its number of children. A reader can then rebuild the tree by reading a value and a count, then that many subtrees.

```rust
use std::str::FromStr;

/// A node with a value and any number of children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode<T> {
    /// The node's value.
    pub value: T,
    /// The children, in order.
    pub children: Vec<TreeNode<T>>,
}

impl<T> TreeNode<T> {
    /// A node with no children.
    #[must_use]
    pub fn leaf(value: T) -> Self {
        Self { value, children: Vec::new() }
    }

    /// Each node's value and child count, in pre-order.
    #[must_use]
    pub fn serialize(&self) -> Vec<String>
    where
        T: ToString,
    {
        let mut tokens = Vec::new();
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            tokens.push(node.value.to_string());
            tokens.push(node.children.len().to_string());
            stack.extend(node.children.iter().rev());
        }
        tokens
    }

    /// The tree written by [`TreeNode::serialize`] starting at `position`,
    /// which is moved past it. `None` when a token is missing or does not parse.
    pub fn deserialize(tokens: &[String], position: &mut usize) -> Option<Self>
    where
        T: FromStr,
    {
        let value = tokens.get(*position)?.parse().ok()?;
        let count: usize = tokens.get(*position + 1)?.parse().ok()?;
        *position += 2;
        let children = (0..count).map(|_| Self::deserialize(tokens, position)).collect::<Option<Vec<_>>>()?;
        Some(Self { value, children })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TreeNode<String> {
        let a = TreeNode { value: "a".to_string(), children: vec![TreeNode::leaf("a1".to_string()), TreeNode::leaf("a2".to_string())] };
        let b = TreeNode { value: "b".to_string(), children: vec![TreeNode::leaf("b1".to_string())] };
        TreeNode { value: "root".to_string(), children: vec![a, b] }
    }

    #[test]
    fn writes_values_with_child_counts() {
        assert_eq!(sample().serialize(), ["root", "2", "a", "2", "a1", "0", "a2", "0", "b", "1", "b1", "0"]);
    }

    #[test]
    fn reads_back_the_same_tree() {
        let tokens = sample().serialize();
        let mut position = 0;
        assert_eq!(TreeNode::<String>::deserialize(&tokens, &mut position), Some(sample()));
        assert_eq!(position, tokens.len());
    }

    #[test]
    fn refuses_cut_or_broken_input() {
        let cut: Vec<String> = ["root", "2", "a", "0"].map(String::from).to_vec();
        assert_eq!(TreeNode::<String>::deserialize(&cut, &mut 0), None);
        let numbers: Vec<String> = ["1", "x"].map(String::from).to_vec();
        assert_eq!(TreeNode::<u32>::deserialize(&numbers, &mut 0), None);
    }
}
```

`serialize` walks the tree with a `Vec` as a stack instead of recursion. It pushes the children in reverse, so the first child is on top and comes out first. The `where T: ToString` bound sits on the method, not on the `impl`: a tree of any type can exist, and only trees whose values can be written get `serialize`.

To save, join the tokens with newlines and write the file; to load, split the file into lines and call `deserialize` with position 0. That format breaks if a value contains a newline. For arbitrary text, write each value's length before it, or use `serde`. `deserialize` is recursive, so a very deep tree (a long chain) can overflow the stack; a loop with an explicit stack avoids that.

---

<!-- source: cracking-the-systems-programming-interview/src/ch12-graphs.md; section: 00d78327a798cb5b; licence: author's own -->
### User
Explain Dijkstra's algorithm, including why the same node can be in the heap more than once.

### Assistant
Dijkstra finds the lowest-cost path from one start node to every other node, as long as no edge weight is negative. (BFS answers a different question: the path with the fewest edges.)

It keeps two things: `dist`, the lowest cost found so far to each node (only the start has one at first, 0), and a min-heap of `(cost, node)` entries. Each step pops the cheapest entry. For every edge out of that node it computes the cost of reaching the neighbour through it, and if that beats the neighbour's recorded cost, records it and pushes the neighbour. That update is called relaxing the edge.

When a node is popped with its recorded cost, that cost is final: every other entry in the heap costs at least as much, and with no negative weights no later path can be cheaper.

`BinaryHeap` cannot find and lower an existing entry, so when a cheaper path to B turns up, the old entry for B stays in the heap. When that **stale** entry is popped later, its cost is above B's recorded cost, and the loop skips it.

```rust
use std::{cmp::Reverse, collections::BinaryHeap};

/// The lowest cost from `start` to each node, where `graph[node]` lists
/// `(neighbour, weight)` pairs. `None` marks a node with no path.
#[must_use]
pub fn dijkstra(graph: &[Vec<(usize, u32)>], start: usize) -> Vec<Option<u32>> {
    let mut dist: Vec<Option<u32>> = vec![None; graph.len()];
    let Some(start_cost) = dist.get_mut(start) else {
        return dist;
    };
    *start_cost = Some(0);
    let mut heap = BinaryHeap::from([Reverse((0_u32, start))]);
    while let Some(Reverse((cost, node))) = heap.pop() {
        if dist[node].is_some_and(|best| cost > best) {
            continue;
        }
        for &(neighbour, weight) in graph.get(node).into_iter().flatten() {
            let Some(through) = cost.checked_add(weight) else {
                continue;
            };
            let Some(recorded) = dist.get_mut(neighbour) else {
                continue;
            };
            if recorded.is_none_or(|best| through < best) {
                *recorded = Some(through);
                heap.push(Reverse((through, neighbour)));
            }
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_cheaper_path_with_more_edges() {
        let a_b_c_d_e = vec![vec![(1, 4), (2, 1)], vec![(3, 1)], vec![(1, 2), (3, 5)], vec![], vec![]];
        assert_eq!(dijkstra(&a_b_c_d_e, 0), [Some(0), Some(3), Some(1), Some(4), None]);
    }

    #[test]
    fn a_path_that_would_overflow_is_not_taken() {
        let graph = vec![vec![(1, u32::MAX)], vec![(2, 1)], vec![]];
        assert_eq!(dijkstra(&graph, 0), [Some(0), Some(u32::MAX), None]);
    }
}
```

On the test graph: popping A records B at 4 and C at 1. Popping C finds B at 1 + 2 = 3, which beats 4, so B is recorded at 3 and pushed again; the `(4, B)` entry is now stale. Popping B at 3 finds D at 4, beating the 6 recorded through C. Later `(4, B)` and `(6, D)` are popped and skipped. E has no edges in, so it stays `None`.

`is_some_and` is the stale check (there is a recorded cost and this one is higher); `is_none_or` is the relax check (no cost yet, or the new one is lower). `checked_add` skips a path whose cost would overflow `u32`, where `cost + weight` would panic in a debug build and wrap in release. Each edge pushes at most one entry, so the algorithm runs in O(E log E).

---

<!-- source: cracking-the-systems-programming-interview/src/ch27-tcp-close.md; section: 020f045f664f1844; licence: author's own -->
### User
My program closes TCP sockets and `read` returns 0 on the other side, so the connection is gone, right?

### Assistant
Gone for your program, not for the kernel. A TCP connection closes one direction at a time: each side sends its own `FIN` when it closes or calls `shutdown`, and the peer's `read` returns 0 when that `FIN` arrives. After your program has moved on, the kernel keeps each side in a series of states, and two of them cause production problems:

- **`TIME_WAIT`**, on the side that closed first. The socket stays for a fixed time (60 seconds on Linux) so that late packets from the old connection cannot be mistaken for a new connection on the same address and port. A busy client that opens and closes many short connections to one server can run out of local ports because they are all sitting in `TIME_WAIT`.
- **`CLOSE_WAIT`**, on the side that received the `FIN` but never closed its own socket. It lasts as long as the program keeps the socket open, which for a leaked socket means forever. A growing number of `CLOSE_WAIT` sockets is a bug in the program: some code path reads 0 and forgets to close.

You can watch both:

```bash
ss -tan state time-wait | wc -l
ss -tanp state close-wait
```

The second shows which process holds each `CLOSE_WAIT` socket. Two related details: a server restarted while old connections sit in `TIME_WAIT` can still bind its port if it sets `SO_REUSEADDR` (Rust's `TcpListener::bind` does this on Unix), and closing a socket that still has unread data sends a reset (`RST`) instead of a `FIN`, so the peer gets "connection reset" instead of a clean end of stream.
