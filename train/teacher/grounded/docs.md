<!-- source: docs/03-kubernetes-concepts.md; section: 00253dd28da5c4f2; licence: author's own -->
### User
My distributed training job needs 8 GPU workers. Sometimes 7 start and sit there holding GPUs while the 8th stays Pending. How do I stop that?

### Assistant
You need all-or-nothing placement, called gang scheduling. Without it, the seven running workers hold their GPUs while waiting for the eighth, wasting the capacity, and two half-placed jobs can block each other forever, each holding GPUs the other needs.

Kubernetes has it built in as of v1.37, behind the `GenericWorkload` feature gate (beta, disabled by default), together with the `Workload` and `PodGroup` APIs. It works in three steps:

1. The scheduler holds the job's Pods at `PreEnqueue` until their `PodGroup` exists and has at least the policy's `minCount` Pods.
2. It evaluates the whole group's placement in one scheduling cycle; a `PlacementFeasible` check tracks whether `minCount` can still be met.
3. If at least `minCount` Pods fit, they are all bound. If not, none are, and the GPUs stay free for other work.

For jobs made of several replicated parts (say, parameter servers and workers), `CompositePodGroup` (alpha in v1.37) groups the groups with a `minGroupCount`.

On a cluster without these features, use a scheduler that implements gang scheduling at the `Permit` extension point, such as Volcano or the coscheduling plugin from scheduler-plugins. `Permit` lets the scheduler hold each Pod's binding until the whole group has a place.

---

<!-- source: docs/03-kubernetes-concepts.md; section: 0325743641adc910; licence: author's own -->
### User
In one paragraph, what happens between `kubectl apply` of a Deployment and containers running?

### Assistant
The request goes to the API server, which authenticates and authorizes it, runs admission (which may change or reject the object) and stores it in etcd. Nothing has run yet: the Deployment is only desired state. The Deployment controller, watching for changes, creates a ReplicaSet, and the ReplicaSet controller creates Pods. The scheduler sees Pods with no node, picks a node for each through its filter and score plugins, and writes the choice back through the API server. The kubelet on that node, watching for Pods assigned to it, asks the container runtime to pull images and start the containers, then reports status back. From then on every controller keeps comparing what exists with what was asked for and acts on the difference, which is why a deleted Pod comes back.

### User
Where do GPUs fit into that path?

### Assistant
At two points: the node and the scheduler. A GPU is an extended resource such as `nvidia.com/gpu`. A device plugin on each node (often installed by the GPU Operator) tells the kubelet how many GPUs the node has, and the kubelet advertises them in the node's `Allocatable`. The scheduler then treats them like CPU and memory: a Pod that asks for two GPUs in its `limits` is only placed on a node with two unallocated. When the container starts, the device plugin tells the runtime which devices to make available inside it.

Dynamic Resource Allocation (DRA) is the newer model for devices that need more than a count, such as choosing a GPU by its properties or sharing one between Pods. Gang scheduling, covered separately, handles jobs whose Pods must be placed together.

---

<!-- source: docs/13-rust-guide-part3-gotchas-linkedlist-interview.md; section: 0b72697c7e7d3ee8; licence: author's own -->
### User
I have a Rust phone screen next week in an online editor. How should I work during it?

### Assistant
Talk through your reasoning, and let the compiler and tests carry part of the work.

**Before coding.** Restate the problem and confirm the constraints with an example. Ask what you need to know: can the input be empty, can numbers be negative, how large can it get, must the output keep the input order, will it run on one thread or several? Then describe your approach and its time and space complexity, for example: "I'll use a `VecDeque` for BFS because I need O(1) pops from the front and pushes to the back, and a `HashSet` for visited nodes. Time O(n), space O(n)."

**While coding.** Write the function signature first; it settles the ownership questions (borrow or take, what to return). Write two or three `assert_eq!` tests in a `#[cfg(test)]` module early and run them. When the borrow checker complains, read the whole message and say what it means and why your fix works; the interviewer is checking that you understand the error, not only that it went away. Use autocomplete to confirm method names instead of guessing, and keep functions small enough to test one at a time. Prefer clear code to clever one-liners, unless the one-liner is the common idiom, such as `*map.entry(key).or_insert(0) += 1`.

**When choosing a structure, say the trade-off out loud:**

| Choice | Trade-off |
|---|---|
| `Vec` or `LinkedList` | `Vec` almost always wins on cache locality |
| `HashMap` or `BTreeMap` | expected O(1) lookup, or ordering and range queries |
| `Mutex` or `RwLock` | depends on the ratio of reads to writes and how long the lock is held |
| `Rc<RefCell<T>>` or an arena with indices | flexible pointers with run-time checks, or faster code with indices you manage |

The problems that come up again and again are worth having written once beforehand: two sum (map from value to index), valid parentheses (a `Vec` as a stack), merge intervals (sort, then extend the last range), top-k (a `BinaryHeap`), an LRU cache, a token-bucket rate limiter, a worker pool over an `mpsc` channel, a trie, topological sort, and BFS over a grid.
