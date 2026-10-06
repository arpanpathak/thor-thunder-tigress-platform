<!-- source: ods/java/ods/ChainedHashTable.java; section: 0a640429595a8920; licence: CC-BY -->
### User
In Open Data Structures' `ChainedHashTable`, the hash is `(z * x.hashCode()) >>> (w - d)` with a random odd `z`. Why that formula?

### Assistant
It is multiplicative hashing for a table of size 2^d. Read it in three steps:

1. `z * x.hashCode()` multiplies in 32-bit `int` arithmetic, which in Java wraps around, so it computes the product modulo 2^32 (`w` is 32, the number of bits in an `int`).
2. `>>> (w - d)` is an unsigned shift that keeps the top `d` bits of that product, giving a number in `0 .. 2^d`, which is exactly a slot of the table.
3. `z` is random and odd. Odd makes multiplication modulo 2^32 a one-to-one mapping, so no two hash codes collapse just because of the multiplier. Random means no fixed set of keys is bad for every table, which is what the analysis in the book relies on.

Taking the top bits instead of the bottom bits matters: the low bits of a product depend only on the low bits of the inputs, while the top bits depend on all of them. Keys whose hash codes differ only in their high bits would all land in one slot if you used `hashCode() % size`.

The table keeps `n` at most the number of slots: `add` calls `resize()` when `n + 1` would exceed `t.length`, and `resize` picks the smallest `d` with 2^d > n, then re-adds every element because each one's slot depends on `d`. So the expected length of a chain stays constant, and `add`, `find` and `remove` take O(1) expected time. Note that this version's `remove` never shrinks the table.

### User
Can you write a small version I can run, with a check that it works?

### Assistant
Here is the same design with buckets as `ArrayList`s, a fixed odd multiplier so runs are repeatable, and a `main` that fails loudly if anything is off:

```java
import java.util.ArrayList;
import java.util.List;

public class ChainedSet<T> {
    private static final int W = 32;
    private final int z;
    private List<List<T>> table;
    private int d = 1;
    private int n = 0;

    public ChainedSet(int multiplier) {
        z = multiplier | 1;
        table = buckets(1 << d);
    }

    private static <T> List<List<T>> buckets(int count) {
        List<List<T>> created = new ArrayList<>(count);
        for (int slot = 0; slot < count; slot++) {
            created.add(new ArrayList<>());
        }
        return created;
    }

    private int hash(Object x) {
        return (z * x.hashCode()) >>> (W - d);
    }

    public boolean contains(Object x) {
        return table.get(hash(x)).contains(x);
    }

    public boolean add(T x) {
        if (contains(x)) {
            return false;
        }
        if (n + 1 > table.size()) {
            resize(n + 1);
        }
        table.get(hash(x)).add(x);
        n++;
        return true;
    }

    public boolean remove(Object x) {
        if (!table.get(hash(x)).remove(x)) {
            return false;
        }
        n--;
        return true;
    }

    public int size() {
        return n;
    }

    private void resize(int needed) {
        d = 1;
        while ((1 << d) < needed) {
            d++;
        }
        List<List<T>> old = table;
        table = buckets(1 << d);
        for (List<T> bucket : old) {
            for (T x : bucket) {
                table.get(hash(x)).add(x);
            }
        }
    }

    public static void main(String[] args) {
        ChainedSet<String> set = new ChainedSet<>(0x9E3779B9);
        for (int i = 0; i < 1000; i++) {
            set.add("key" + i);
        }
        if (set.size() != 1000 || !set.contains("key999") || set.add("key5")) {
            throw new AssertionError("adding failed");
        }
        if (!set.remove("key5") || set.contains("key5") || set.size() != 999) {
            throw new AssertionError("removing failed");
        }
        if (set.table.size() != 1024) {
            throw new AssertionError("1000 elements need 1024 slots");
        }
        System.out.println("ok");
    }
}
```

`0x9E3779B9` is a common choice of odd multiplier (it comes from the golden ratio), but any odd value works for testing. Two simplifications compared with the book's code: `contains` uses `List.contains`, which calls `equals` as the original's loop does, and there is no shrinking after removals, matching the version above. For production Java, use `HashSet`, which also converts long chains to balanced trees.
