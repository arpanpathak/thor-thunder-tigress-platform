<!-- source: leveldb/util/bloom_test.cc; section: 0042606170bf0a43; licence: BSD -->
### User
LevelDB's tests create a filter with `NewBloomFilterPolicy(10)`. What does the 10 mean, and how do its tests check the filter actually works?

### Assistant
The 10 is bits per key: a filter for `n` keys gets about `10 * n` bits. LevelDB's `bloom.cc` derives the number of hash probes from it as `bits_per_key * 0.69` (ln 2 is the optimal ratio), clamped to between 1 and 30, so 10 bits per key gives 6 probes and a false-positive rate a little under 1%.

`bloom_test.cc` checks the two properties that matter, from the outside:

- **No false negatives.** Every key that was added must match. `Small` adds "hello" and "world" and asserts both match, while "x" and "foo" do not. `EmptyFilter` asserts a filter built from nothing matches nothing.
- **A false-positive rate near the design.** `FalsePositiveRate` asks about 10,000 keys that were never added (encoded integers from 1,000,000,000 up) and counts how many match anyway. `VaryingLengths` builds filters for key counts from 1 up to 10,000 and, for each one, asserts that every added key matches, that the filter is at most `length * 10 / 8 + 40` bytes, and that its false-positive rate is at most 2%. A rate above 1.25% counts as mediocre: allowed, but the test fails if mediocre filters are more than a fifth as many as good ones. That way one unlucky filter does not fail the test, while a hash that is consistently weak does.

Here is the same design, small enough to read in one go, with those checks in `main`:

```cpp
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

class BloomFilter {
 public:
  explicit BloomFilter(size_t bits_per_key)
      : bits_per_key_(bits_per_key), probes_(std::clamp<size_t>(static_cast<size_t>(bits_per_key * 0.69), 1, 30)) {}

  std::vector<uint8_t> Create(const std::vector<std::string>& keys) const {
    size_t bits = std::max<size_t>(64, keys.size() * bits_per_key_);
    std::vector<uint8_t> filter((bits + 7) / 8, 0);
    bits = filter.size() * 8;
    for (const std::string& key : keys) {
      uint32_t hash = Hash(key);
      const uint32_t delta = (hash >> 17) | (hash << 15);
      for (size_t probe = 0; probe < probes_; probe++) {
        const size_t bit = hash % bits;
        filter[bit / 8] |= static_cast<uint8_t>(1 << (bit % 8));
        hash += delta;
      }
    }
    return filter;
  }

  bool MayMatch(const std::string& key, const std::vector<uint8_t>& filter) const {
    const size_t bits = filter.size() * 8;
    if (bits == 0) return false;
    uint32_t hash = Hash(key);
    const uint32_t delta = (hash >> 17) | (hash << 15);
    for (size_t probe = 0; probe < probes_; probe++) {
      const size_t bit = hash % bits;
      if ((filter[bit / 8] & (1 << (bit % 8))) == 0) return false;
      hash += delta;
    }
    return true;
  }

 private:
  static uint32_t Hash(const std::string& key) {
    uint32_t hash = 2166136261u;
    for (unsigned char byte : key) {
      hash = (hash ^ byte) * 16777619u;
    }
    return hash;
  }

  size_t bits_per_key_;
  size_t probes_;
};

int main() {
  const BloomFilter policy(10);
  std::vector<std::string> keys;
  for (int i = 0; i < 10000; i++) keys.push_back("key" + std::to_string(i));
  const std::vector<uint8_t> filter = policy.Create(keys);
  for (const std::string& key : keys) {
    if (!policy.MayMatch(key, filter)) return 1;
  }
  int false_positives = 0;
  for (int i = 0; i < 10000; i++) {
    if (policy.MayMatch("absent" + std::to_string(i), filter)) false_positives++;
  }
  const double rate = false_positives / 10000.0;
  std::printf("%zu bytes, false-positive rate %.4f\n", filter.size(), rate);
  return rate < 0.02 ? 0 : 2;
}
```

Instead of computing six independent hashes, it computes one hash and adds a rotated copy of it (`delta`) between probes. That trick, double hashing, is what LevelDB does too, and it costs one hash per key instead of six. The FNV hash here is a stand-in; LevelDB uses its own `Hash` function with a fixed seed.
