<!-- source: aosa-500lines/cluster/cluster.markdown; section: 0135f98f910f51bd; licence: CC-BY -->
### User
In a Paxos implementation I'm reading, every message type is a `namedtuple`, like `Accepted = namedtuple('Accepted', ['slot', 'ballot_num'])`. Why not plain dicts?

### Assistant
Three reasons, all visible in use:

- The constructor refuses wrong fields. `Accepted(slot=10, balot_num=30)` raises `TypeError` at the line with the typo, where a dict would carry the misspelled key along until some other code fails to find `ballot_num`.
- They print well. A logged message reads `Accepted(slot=10, ballot_num=30)`, which matters when you are reading the log of a distributed protocol.
- They are smaller than a dict, because the field names are stored once on the class, not in every message.

```python
from collections import namedtuple

Accepted = namedtuple("Accepted", ["slot", "ballot_num"])

message = Accepted(slot=10, ballot_num=30)
assert message.ballot_num == 30
assert repr(message) == "Accepted(slot=10, ballot_num=30)"

try:
    Accepted(slot=10, balot_num=30)
except TypeError as error:
    assert "balot_num" in str(error)
else:
    raise AssertionError("a misspelled field must be refused")
```

In current Python, `typing.NamedTuple` gives the same tuple with type annotations on each field, which a type checker can then use.

### User
It also has `NULL_BALLOT = Ballot(-1, -1)` with the comment "sorts before all real ballots". How does that work?

### Assistant
Tuples compare element by element, and named tuples are tuples. `Ballot` is `namedtuple('Ballot', ['n', 'leader'])`, so ballots are ordered first by `n` and then by `leader`. Real ballots have `n` of 0 or more, so `Ballot(-1, -1)` is smaller than any of them, and code can start from it as "no ballot yet" and keep the largest ballot seen with `max` or `>` without a special case for "none".

```python
from collections import namedtuple

Ballot = namedtuple("Ballot", ["n", "leader"])
NULL_BALLOT = Ballot(-1, -1)

seen = [Ballot(2, "b"), Ballot(2, "a"), Ballot(1, "z")]
assert all(NULL_BALLOT < ballot for ballot in seen)
assert max(seen, default=NULL_BALLOT) == Ballot(2, "b")
assert max([], default=NULL_BALLOT) == NULL_BALLOT
```

The second field breaks ties: two leaders that pick the same `n` still produce ballots that compare unequal, so one of them wins.

---

<!-- source: aosa-500lines/web-server/web-server.markdown; section: 02504fcbe9e7ec2a; licence: CC-BY -->
### User
My little Python web server sends an HTML error page when a file is missing, but curl and the browser treat the request as successful. Why?

### Assistant
Because the status line still says 200. The browser does not read the words on your error page; it reads the status code. The fix is to send the error page with a status like 404, which means `send_content` needs to take the status instead of always sending 200:

```python
def handle_error(self, msg):
    content = self.Error_Page.format(path=self.path, msg=msg)
    self.send_content(content.encode(), 404)

def send_content(self, content, status=200):
    self.send_response(status)
    self.send_header("Content-Type", "text/html")
    self.send_header("Content-Length", str(len(content)))
    self.end_headers()
    self.wfile.write(content)
```

It is worth keeping two kinds of failure apart. A missing file is the *client's* mistake: answer with an error page and a 4xx status. An exception inside your handler is *your* mistake: that deserves a 500 and a log entry, not a page that blames the URL.

### User
The handler builds the path as `os.getcwd() + self.path`. Is that safe?

### Assistant
No. `self.path` comes from the client, and nothing stops it from containing `..`. A request for `/../../etc/passwd` joins to a path outside the directory you meant to serve, and `os.path.isfile` will happily say it exists.

Resolve the joined path and refuse anything that is not inside the root:

```python
import os
import tempfile


def resolve(root, url_path):
    """The file under `root` that `url_path` names, or None if it points outside `root`."""
    root = os.path.realpath(root)
    candidate = os.path.realpath(os.path.join(root, url_path.lstrip("/")))
    if os.path.commonpath([root, candidate]) != root:
        return None
    return candidate


with tempfile.TemporaryDirectory() as root:
    assert resolve(root, "/index.html") == os.path.join(os.path.realpath(root), "index.html")
    assert resolve(root, "/../../etc/passwd") is None
    assert resolve(root, "/a/../../secret") is None
```

`realpath` also follows symbolic links, so a link inside the root that points outside it is refused too. Strip any query string (`?a=1`) and percent-decode the path before resolving it, or `%2e%2e` slips past the check.

---

<!-- source: aosa-500lines/objmodel/objmodel.markdown; section: 02dc037169d1440f; licence: CC-BY -->
### User
What's the actual difference between class-based and prototype-based object models?

### Assistant
Both give objects shared behaviour, but they get it from different places.

In a class-based language (Smalltalk, Java, Python), an object is an instance of a class. Its methods live on the class, and inheritance is a relationship between classes.

In a prototype-based language (Self, JavaScript), there are no classes underneath. An object inherits directly from another object, its prototype: a property that is not found on the object is looked up on the prototype, then on the prototype's prototype, and so on.

```javascript
const assert = require("node:assert");

const animal = {
  speak() {
    return `${this.name} makes a sound`;
  },
};

const dog = Object.create(animal);
dog.name = "Rex";

assert.strictEqual(dog.speak(), "Rex makes a sound");
assert.strictEqual(Object.getPrototypeOf(dog), animal);
assert.ok(!Object.hasOwn(dog, "speak"));
```

`dog` has no `speak` of its own; the lookup finds it on `animal`. JavaScript's `class` keyword is syntax over this same mechanism: a class's methods are stored on an object that every instance has as its prototype.

### User
If I wanted to understand Python's object model by building one, would writing it in C be more realistic?

### Assistant
More realistic, yes, since production VMs are written in low-level languages such as C or C++. But a C implementation spends most of its code on memory layout and efficiency, which hides the behaviour you are trying to learn.

Writing the model in Python itself keeps the focus on the rules: how attribute lookup walks from the instance to its class and up the inheritance chain, where methods are bound, and what changes when you add prototypes or a meta-object protocol. Tests for each step make the behaviour concrete, and comparing the models shows how small the differences between language families often are. Once the behaviour is clear, the efficient version (such as storing instance attributes in a fixed layout shared by all instances of a class) is easier to understand as an optimisation of something you already know works.
