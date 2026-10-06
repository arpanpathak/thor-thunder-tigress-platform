<!-- source: functional-vs-imperative-rust.pdf (when a loop reads better than a chain) -->
### User
Make this readable.

```rust
pub fn f(v: &[(String, u32)]) -> Option<(String, u32)> {
    v.iter().fold(None, |a: Option<(String, u32)>, (n, s)| match a { Some((an, as_)) if as_ >= *s => Some((an, as_)), _ if *s >= 50 => Some((n.clone(), *s)), x => x })
}
```

### Assistant
It returns the passing student (score 50 or more) with the highest score, keeping the first one on a tie. Named types and `filter` plus a max say that directly:

```rust
/// The score a student needs to pass.
pub const PASS_MARK: u32 = 50;

/// A student and their score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grade {
    /// The student's name.
    pub name: String,
    /// The score.
    pub score: u32,
}

/// The best passing grade, the first one on a tie, or `None` when nobody passed.
#[must_use]
pub fn best_pass(grades: &[Grade]) -> Option<&Grade> {
    grades
        .iter()
        .filter(|grade| grade.score >= PASS_MARK)
        .rev()
        .max_by_key(|grade| grade.score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grade(name: &str, score: u32) -> Grade {
        Grade { name: name.to_string(), score }
    }

    #[test]
    fn picks_the_first_of_the_best_passes() {
        let grades = [grade("a", 40), grade("b", 70), grade("c", 70), grade("d", 55)];
        assert_eq!(best_pass(&grades).map(|best| best.name.as_str()), Some("b"));
    }

    #[test]
    fn nobody_passing_gives_none() {
        assert_eq!(best_pass(&[grade("a", 49)]), None);
    }
}
```

Changes besides the layout: the magic number 50 is `PASS_MARK`, the tuple is a struct with named fields, and it returns a reference instead of cloning the name. `max_by_key` keeps the last of equal maxima, so `.rev()` makes it keep the first, as your `as_ >= *s` did.

---

<!-- source: std iter::Iterator::any and Iterator::position docs -->
### User
Clean this up.

```rust
pub fn has_admin(users: &[User]) -> bool {
    let mut found = false;
    for user in users {
        if user.role == Role::Admin {
            found = true;
            break;
        }
    }
    found
}
```

### Assistant
The flag and the `break` are what `any` does:

```rust
/// What a user may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Can change everything.
    Admin,
    /// Can read and write their own data.
    Member,
}

/// A user and their role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The login name.
    pub name: String,
    /// What they may do.
    pub role: Role,
}

/// True when any user is an admin.
#[must_use]
pub fn has_admin(users: &[User]) -> bool {
    users.iter().any(|user| user.role == Role::Admin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_an_admin() {
        let user = |name: &str, role| User { name: name.to_string(), role };
        assert!(has_admin(&[user("ada", Role::Member), user("root", Role::Admin)]));
        assert!(!has_admin(&[user("ada", Role::Member)]));
        assert!(!has_admin(&[]));
    }
}
```

`any` stops at the first match, like your `break`. The relatives are `all` (every item matches), `find` (the first matching item) and `position` (its index).

---

<!-- source: functional-vs-imperative-rust.pdf (let-else over and_then chains) -->
### User
Is this idiomatic?

```rust
pub fn port_of(url: &str) -> Option<u16> {
    url.split_once("://").and_then(|(_, rest)| rest.split('/').next()).and_then(|host| host.rsplit_once(':')).and_then(|(_, port)| port.parse().ok())
}
```

### Assistant
It is valid Rust, but four `and_then` calls in a row make the reader track what each closure's tuple holds. `?` on `Option` gives each step a name and reads top to bottom:

```rust
/// The explicit port of a URL like `http://host:8080/path`, or `None` when
/// the URL has no scheme or no port.
#[must_use]
pub fn port_of(url: &str) -> Option<u16> {
    let (_scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let (_host, port) = authority.rsplit_once(':')?;
    port.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port() {
        assert_eq!(port_of("http://thor:8080/v1/chat"), Some(8080));
        assert_eq!(port_of("https://example.org/a:b"), None);
        assert_eq!(port_of("thor:8080"), None);
        assert_eq!(port_of("http://thor:99999"), None);
    }
}
```

Behaviour is unchanged. This is a quick parser, not a URL parser: it does not handle IPv6 hosts like `[::1]:80` correctly or user info like `user:pass@host`. Use the `url` crate when inputs are not under your control.

---

<!-- source: Rust API guidelines C-CUSTOM-TYPE (no boolean parameters) -->
### User
What's wrong with this signature? `pub fn export(report: &Report, true, false)` is how it gets called.

```rust
pub fn export(report: &Report, compressed: bool, include_drafts: bool) -> Vec<u8>
```

### Assistant
At the call site, `export(&report, true, false)` does not say what `true` and `false` mean, and swapping them still compiles. Two small enums make each argument name itself:

```rust
/// Whether the output is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Plain bytes.
    None,
    /// Run-length encoded bytes.
    RunLength,
}

/// Which entries are exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drafts {
    /// Only published entries.
    Exclude,
    /// Published entries and drafts.
    Include,
}

/// One entry of a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The text.
    pub text: String,
    /// True until the entry is published.
    pub draft: bool,
}

/// The selected entries, one per line, compressed as asked.
#[must_use]
pub fn export(entries: &[Entry], compression: Compression, drafts: Drafts) -> Vec<u8> {
    let lines: Vec<&str> = entries
        .iter()
        .filter(|entry| drafts == Drafts::Include || !entry.draft)
        .map(|entry| entry.text.as_str())
        .collect();
    let bytes = lines.join("\n").into_bytes();
    match compression {
        Compression::None => bytes,
        Compression::RunLength => run_length(&bytes),
    }
}

fn run_length(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunk_by(|left, right| left == right)
        .flat_map(|run| {
            run.chunks(usize::from(u8::MAX))
                .filter_map(|piece| Some([u8::try_from(piece.len()).ok()?, *piece.first()?]))
        })
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        vec![
            Entry { text: "aaab".to_string(), draft: false },
            Entry { text: "draft".to_string(), draft: true },
        ]
    }

    #[test]
    fn leaves_out_drafts_unless_asked() {
        assert_eq!(export(&entries(), Compression::None, Drafts::Exclude), b"aaab");
        assert_eq!(export(&entries(), Compression::None, Drafts::Include), b"aaab\ndraft");
    }

    #[test]
    fn compresses_runs() {
        assert_eq!(export(&entries(), Compression::RunLength, Drafts::Exclude), [3, b'a', 1, b'b']);
    }
}
```

The call now reads `export(&entries, Compression::RunLength, Drafts::Exclude)`, and passing the arguments in the wrong order is a type error. Enums also leave room for a third option later, which a `bool` does not.

---

<!-- source: functional-vs-imperative-rust.pdf (a loop when the steps have side effects) -->
### User
Rewrite this loop with iterators.

```rust
pub fn apply(balance: &mut i64, changes: &[i64], log: &mut Vec<String>) -> usize {
    let mut rejected = 0;
    for change in changes {
        if *balance + change < 0 {
            rejected += 1;
            log.push(format!("rejected {change}"));
            continue;
        }
        *balance += change;
        log.push(format!("applied {change}, balance {balance}"));
    }
    rejected
}
```

### Assistant
I would keep the loop. Each step reads and changes the balance and appends to the log, so an iterator version would be a `fold` carrying the balance, the log and the count in a tuple, which is harder to follow than what you have. Iterators fit best when each item is turned into a value without side effects.

What I would change is the overflow and the repeated `*balance + change`:

```rust
/// Applies each change that keeps `balance` at zero or above, logging every
/// decision. Returns how many changes were rejected.
pub fn apply(balance: &mut i64, changes: &[i64], log: &mut Vec<String>) -> usize {
    let mut rejected = 0;
    for &change in changes {
        let Some(after) = balance.checked_add(change).filter(|&after| after >= 0) else {
            rejected += 1;
            log.push(format!("rejected {change}"));
            continue;
        };
        *balance = after;
        log.push(format!("applied {change}, balance {after}"));
    }
    rejected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_overdrafts_and_overflow() {
        let mut balance = 10;
        let mut log = Vec::new();
        let rejected = apply(&mut balance, &[-4, -20, 5, i64::MAX], &mut log);
        assert_eq!((balance, rejected), (11, 2));
        assert_eq!(log, ["applied -4, balance 6", "rejected -20", "applied 5, balance 11", &format!("rejected {}", i64::MAX)]);
    }
}
```

`checked_add` returns `None` on overflow, so an enormous deposit is rejected instead of wrapping around or panicking, and both failure cases go through the one `else` branch, the same early `continue` your loop had.

---

<!-- source: clippy::pedantic (magic numbers are not a clippy lint; named constants per the project rules) -->
### User
Review this: `if retries > 3 { sleep(Duration::from_millis(250 * 2u64.pow(retries))) }`

### Assistant
Three numbers with no names: 3 is a retry limit, 250 a base delay in milliseconds, 2 the backoff factor. There is also a bug: with `retries` above 3, the delay keeps doubling with no ceiling, and `2u64.pow(retries)` panics in a debug build once `retries` reaches 64.

```rust
use std::time::Duration;

/// Retries without any waiting.
pub const FREE_RETRIES: u32 = 3;

/// The wait before the first delayed retry.
pub const BASE_DELAY: Duration = Duration::from_millis(250);

/// The longest wait between two retries.
pub const MAX_DELAY: Duration = Duration::from_secs(30);

/// How long to wait before retry number `retries`: nothing for the first
/// [`FREE_RETRIES`], then doubling from [`BASE_DELAY`] up to [`MAX_DELAY`].
#[must_use]
pub fn backoff(retries: u32) -> Duration {
    let Some(delayed) = retries.checked_sub(FREE_RETRIES).filter(|&delayed| delayed > 0) else {
        return Duration::ZERO;
    };
    let factor = 2_u32.checked_pow(delayed - 1).unwrap_or(u32::MAX);
    BASE_DELAY.saturating_mul(factor).min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_nothing_then_doubles_up_to_the_cap() {
        let delays: Vec<u128> = [0, 3, 4, 5, 6, 20, 200].into_iter().map(|retries| backoff(retries).as_millis()).collect();
        assert_eq!(delays, [0, 0, 250, 500, 1000, 30_000, 30_000]);
    }
}
```

`checked_pow` and `saturating_mul` make very large retry counts land on `MAX_DELAY` instead of panicking. The first delayed retry waits `BASE_DELAY` rather than `BASE_DELAY * 2^4` as in the original; if the original numbers were intended, change `delayed - 1` to `retries`.
