<!-- source: cracking-the-systems-engineering-book/src/03-namespaces-and-cgroups.md; section: 0106ce71dd67e714; licence: author's own -->
### User
A container keeps getting OOM-killed. Where in cgroups v2 do I look, and how do I limit its CPU?

### Assistant
Everything is under `/sys/fs/cgroup`, in the directory of the cgroup the process belongs to. Find it first:

```bash
cat /proc/self/cgroup
cat /sys/fs/cgroup/memory.max
cat /sys/fs/cgroup/memory.current
cat /sys/fs/cgroup/memory.events
```

(Inside the container, `/sys/fs/cgroup` is usually already that cgroup; on the host, prefix the path from `/proc/<pid>/cgroup`.)

- `memory.max` is the hard limit. Going over it gets a process killed.
- `memory.current` is what the cgroup uses now.
- `memory.events` has the counters that answer your question: `oom_kill` counts kills, `max` counts how often the hard limit was hit, and `high` counts throttling at `memory.high`.

`memory.high` is the softer limit below `memory.max`: above it the kernel throttles the cgroup and reclaims memory hard, which gives you a slowdown you can see before the kills start.

CPU is limited with `cpu.max`, written as `MAX PERIOD` in microseconds. `50000 100000` means 50 ms of CPU time in every 100 ms, which is half of one CPU; `200000 100000` is two CPUs. The other limits worth knowing are `pids.max` (tasks and threads), `io.max` (bandwidth and IOPS) and `cpuset.cpus` / `cpuset.mems` (which CPUs and memory nodes it may use).

### User
Can I put my process in a cgroup that already has child cgroups?

### Assistant
No. cgroups v2 has a "no internal processes" rule: a cgroup that has children cannot also hold processes directly (the root cgroup is the exception). Processes live in the leaves. To give a child cgroup a controller, the parent lists it in `cgroup.subtree_control`, for example `echo +memory +cpu > cgroup.subtree_control`.

Also keep in mind that charging follows the process: a process is charged to the cgroup it is in, and a child created by `fork` starts in its parent's cgroup unless something moves it. That is why every process a container starts counts against the container's limits. `systemd-cgls` shows the whole tree with the processes in each cgroup.

---

<!-- source: cracking-the-systems-engineering-book/src/06-debugging-a-sick-node.md; section: 1c50b67fa20b51d9; licence: author's own -->
### User
Pods requesting `nvidia.com/gpu` stay Pending on a GPU node. How should I debug it?

### Assistant
Check the layers from the bottom up, in a fixed order, so you rule out configuration before blaming hardware.

1. **Driver on the host.** `nvidia-smi` on the node. If it fails, the driver module is not loaded, or the node needs a reboot after a driver install.
2. **Device plugin running.**
   ```bash
   kubectl get pods -n gpu-operator -l app=nvidia-device-plugin-daemonset
   kubectl logs -n gpu-operator -l app=nvidia-device-plugin-daemonset
   ```
3. **Node advertises GPUs.** `kubectl describe node <node> | grep -A5 'nvidia.com/gpu'`. If `Allocatable` has no `nvidia.com/gpu`, the plugin never finished registering, and the scheduler sees no GPUs: that alone explains Pending.
4. **Pod request is valid.** `nvidia.com/gpu` is an extended resource, so it must be in `limits`, and if you also set `requests` it must equal the limit.
5. **GPU visible inside the container.** `kubectl exec <pod> -- nvidia-smi`. If the binary or device is missing there, the container toolkit or GPU Operator did not set up the runtime hook.
6. **Hardware or driver faults**, only after the five above pass:
   ```bash
   dmesg | grep -i xid
   nvidia-smi -q -d ECC
   nvidia-smi -q -d PAGE_RETIREMENT
   ```

Xid messages are the driver's reports of GPU faults, and the number identifies the kind of fault. Xid errors together with ECC errors point to hardware or the driver rather than Kubernetes. For Pending pods specifically, steps 3 and 4 are the usual culprits: the scheduler never places a pod on a node that does not advertise enough of the resource.

---

<!-- source: cracking-the-systems-engineering-book/src/01-processes-and-tasks.md; section: 2e913bc0d5f34c28; licence: author's own -->
### User
What actually happens when my program calls `read()`, and why are system calls considered expensive?

### Assistant
User code cannot touch hardware or kernel memory, so `read()` asks the kernel to do it:

1. The C library (or a raw `syscall` instruction) puts the system call number and arguments in registers.
2. It executes `syscall` on x86-64 or `svc` on AArch64.
3. The CPU traps into kernel mode.
4. The kernel checks the arguments, copies data from user memory if needed, does the work on the task's kernel stack, and copies results back.
5. Control returns to user mode, with the result in a register.

If the call has to wait (no data on the socket yet), the kernel puts the task to sleep and runs another one.

The expense is the privilege crossing itself: saving the user context, switching stacks, running the kernel entry and exit paths. That cost is paid even when the call does nothing useful, so you can measure it with the cheapest call there is:

```rust
use std::{hint::black_box, process, time::Instant};

const ITERATIONS: u32 = 1_000_000;
const WARM_UP: u32 = 10_000;

fn main() {
    for _ in 0..WARM_UP {
        black_box(process::id());
    }
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(process::id());
    }
    let elapsed = start.elapsed();
    let per_call_ns = elapsed.as_secs_f64() * 1e9 / f64::from(ITERATIONS);
    println!("{ITERATIONS} getpid() calls in {elapsed:?}, {per_call_ns:.1} ns per call");
}
```

On Linux, `std::process::id()` calls `getpid`, which current glibc does not cache, so each iteration is one real system call. Build with `--release`. Expect a figure in the low hundreds of nanoseconds that varies by around ten percent between runs on the same machine, so always report it with the machine it came from.

### User
My server does one `read()` per small message and `strace -c` shows millions of calls. What can I do?

### Assistant
Cross into the kernel less often. A server that reads 8 bytes per call spends more on the crossings than on copying the data. The options, from simplest to most involved:

- Read bigger chunks into a user-space buffer (`BufReader` in Rust) and parse messages out of it.
- Batch at the system call level: `recvmmsg` and `sendmmsg` move many UDP datagrams per call.
- Use `io_uring`, where you queue many operations in shared memory and the kernel completes them without one crossing per operation.
- Map files with `mmap` instead of reading them.
- For extreme packet rates, user-space networking such as DPDK, which bypasses the kernel's network stack entirely.

`strace -c` is the right first tool because it counts the crossings the kernel actually handled, per system call, so you can see which call dominates before changing anything. It slows the program down a lot while tracing, so use it for counts, not timing.
