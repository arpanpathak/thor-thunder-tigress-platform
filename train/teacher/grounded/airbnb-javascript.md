<!-- source: airbnb-javascript/README.md; section: 03080da92f007ddf; licence: MIT -->
### User
Our JS style guide says not to use `for...of` loops and to use `map`/`reduce` instead. Is that actually better, or just taste?

### Assistant
The Airbnb guide's reason is that array methods return values instead of changing variables, and pure functions that return values are easier to reason about. For a transformation, that holds: `numbers.map((n) => n + 1)` says "a new array, each one plus one" in one expression, with no counter, no empty array to fill, and no chance of an off-by-one.

```javascript
const assert = require("node:assert");

const numbers = [1, 2, 3, 4, 5];

const increasedByOne = numbers.map((number) => number + 1);
const sum = numbers.reduce((total, number) => total + number, 0);
const firstEven = numbers.find((number) => number % 2 === 0);
const allPositive = numbers.every((number) => number > 0);

assert.deepStrictEqual(increasedByOne, [2, 3, 4, 5, 6]);
assert.strictEqual(sum, 15);
assert.strictEqual(firstEven, 2);
assert.ok(allPositive);

const prices = { apple: 2, pear: 3 };
const doubled = Object.fromEntries(Object.entries(prices).map(([name, price]) => [name, price * 2]));
assert.deepStrictEqual(doubled, { apple: 4, pear: 6 });
```

For objects, the guide's route is `Object.keys`, `Object.values` or `Object.entries` to get an array first, then the same methods.

It is not a universal rule, though. A loop is still the clearer choice when the work is sequential side effects with an early exit, or when you `await` inside it: `for (const file of files) { await upload(file); }` uploads one at a time, while `files.forEach(async ...)` starts them all and does not wait. Use the methods for computing values and a loop for doing things in order.

### User
It also says not to use generators. Why?

### Assistant
The stated reason is that generators "don't transpile well to ES5". That reason is about old browsers: when code had to be compiled down to ES5, generators turned into large, slow state machines plus a runtime helper. If your build targets only current browsers or Node, that cost is gone, and the rule is mostly about consistency within the codebase.

Generators are still worth avoiding where an array method does the job, because they add a second way of iterating that readers must follow. They earn their place for lazy or infinite sequences, where building an array first is impossible or wasteful:

```javascript
const assert = require("node:assert");

function* naturals() {
  let next = 1;
  while (true) {
    yield next;
    next += 1;
  }
}

function take(count, items) {
  const taken = [];
  for (const item of items) {
    if (taken.length === count) break;
    taken.push(item);
  }
  return taken;
}

assert.deepStrictEqual(take(3, naturals()), [1, 2, 3]);
```

If you do write one, the same guide asks for `function* name()` with the `*` attached to `function`, since `function*` is one keyword, not `function` with a modifier.
