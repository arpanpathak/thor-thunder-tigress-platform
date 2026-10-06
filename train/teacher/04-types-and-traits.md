<!-- source: Rust API guidelines C-NEWTYPE; std convert::TryFrom docs -->
### User
I pass emails around as String and keep validating them everywhere. Better way?

### Assistant
Validate once, in a newtype that can only be built through the check. After that, a function that takes `Email` knows the value is valid without checking again.

```rust
use std::fmt;

/// Why a string is not an email address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmailError {
    /// There is no `@`, or more than one.
    NotOneAt,
    /// The part before or after the `@` is empty.
    EmptyPart,
    /// The domain has no dot.
    NoDotInDomain,
}

impl fmt::Display for EmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            EmailError::NotOneAt => "needs exactly one @",
            EmailError::EmptyPart => "needs text before and after the @",
            EmailError::NoDotInDomain => "the domain needs a dot",
        };
        f.write_str(reason)
    }
}

impl std::error::Error for EmailError {}

/// An email address that has passed [`Email::try_from`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

impl TryFrom<&str> for Email {
    type Error = EmailError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let text = text.trim();
        let (user, domain) = text.split_once('@').ok_or(EmailError::NotOneAt)?;
        if domain.contains('@') {
            return Err(EmailError::NotOneAt);
        }
        if user.is_empty() || domain.is_empty() {
            return Err(EmailError::EmptyPart);
        }
        if !domain.contains('.') {
            return Err(EmailError::NoDotInDomain);
        }
        Ok(Email(text.to_lowercase()))
    }
}

impl AsRef<str> for Email {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_normalises_an_address() -> Result<(), EmailError> {
        let email = Email::try_from(" Ada@Example.org ")?;
        assert_eq!(email.to_string(), "ada@example.org");
        Ok(())
    }

    #[test]
    fn names_what_is_wrong() {
        assert_eq!(Email::try_from("ada.example.org"), Err(EmailError::NotOneAt));
        assert_eq!(Email::try_from("a@b@c.org"), Err(EmailError::NotOneAt));
        assert_eq!(Email::try_from("@example.org"), Err(EmailError::EmptyPart));
        assert_eq!(Email::try_from("ada@localhost"), Err(EmailError::NoDotInDomain));
    }
}
```

The field is private, so code outside the module cannot build an `Email` without `try_from`. These checks are the shape of an address, not proof it exists; only sending mail to it proves that.

---

<!-- source: std iter::Iterator trait docs; u64::checked_add -->
### User
Make a Fibonacci iterator.

### Assistant
`next` returns `None` when the next number would overflow `u64`, so the iterator ends instead of panicking or wrapping.

```rust
/// The Fibonacci numbers that fit in a `u64`: 0, 1, 1, 2, 3, 5, ...
#[derive(Debug, Clone)]
pub struct Fibonacci {
    current: Option<u64>,
    next: u64,
}

impl Default for Fibonacci {
    fn default() -> Self {
        Self { current: Some(0), next: 1 }
    }
}

impl Iterator for Fibonacci {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let current = self.current?;
        self.current = self.next.checked_add(current).map(|after| {
            let next = self.next;
            self.next = after;
            next
        });
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_the_known_numbers() {
        let first: Vec<u64> = Fibonacci::default().take(10).collect();
        assert_eq!(first, [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]);
    }

    #[test]
    fn stops_before_overflowing() {
        assert_eq!(Fibonacci::default().count(), 93);
        assert_eq!(Fibonacci::default().last(), Some(7_540_113_804_746_346_429));
    }
}
```

Because it is an `Iterator`, all the adaptors work on it: `Fibonacci::default().filter(|n| n % 2 == 0).take(5)` gives the first five even ones.

---

<!-- source: functional-vs-imperative-rust.pdf (exhaustive matching); Rust reference, match expressions -->
### User
I have a door that can be open, closed or locked, with open, close, lock and unlock actions. How do I model the allowed transitions?

### Assistant
An enum for the state, an enum for the action, and one `match` on the pair. Every combination is either a listed transition or the error arm, and adding a new state makes the compiler point at the `match` until it is handled.

```rust
use std::fmt;

/// Where the door is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    /// Open.
    Open,
    /// Closed, not locked.
    Closed,
    /// Closed and locked.
    Locked,
}

/// What someone does to the door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open it.
    Open,
    /// Close it.
    Close,
    /// Lock it.
    Lock,
    /// Unlock it.
    Unlock,
}

/// An action that is not allowed in the door's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorError {
    /// The action does nothing or is impossible from this state.
    NotAllowed {
        /// The state the door was in.
        door: Door,
        /// The action tried.
        action: Action,
    },
}

impl fmt::Display for DoorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DoorError::NotAllowed { door, action } => write!(f, "cannot {action:?} a door that is {door:?}"),
        }
    }
}

impl std::error::Error for DoorError {}

impl Door {
    /// The state after `action`.
    ///
    /// # Errors
    ///
    /// [`DoorError::NotAllowed`] for an action the state does not allow, such
    /// as opening a locked door.
    pub fn apply(self, action: Action) -> Result<Door, DoorError> {
        match (self, action) {
            (Door::Closed, Action::Open) => Ok(Door::Open),
            (Door::Open, Action::Close) | (Door::Locked, Action::Unlock) => Ok(Door::Closed),
            (Door::Closed, Action::Lock) => Ok(Door::Locked),
            (door, action) => Err(DoorError::NotAllowed { door, action }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goes_through_a_full_cycle() -> Result<(), DoorError> {
        let door = [Action::Close, Action::Lock, Action::Unlock, Action::Open]
            .into_iter()
            .try_fold(Door::Open, Door::apply)?;
        assert_eq!(door, Door::Open);
        Ok(())
    }

    #[test]
    fn a_locked_door_does_not_open() {
        assert_eq!(
            Door::Locked.apply(Action::Open),
            Err(DoorError::NotAllowed { door: Door::Locked, action: Action::Open })
        );
    }
}
```

`try_fold` runs a list of actions and stops at the first one that is not allowed. The catch-all arm is a deliberate choice here: every pair not listed is an error. If some new pair should be allowed, it needs its own arm above it.

---

<!-- source: std iter::Iterator::max_by_key docs -->
### User
Write a function that finds the oldest person in a list.

### Assistant
The standard library has it: `max_by_key`.

```rust
/// A person with an age in years.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// The person's name.
    pub name: String,
    /// Age in whole years.
    pub age: u32,
}

/// The oldest person, or `None` for an empty list. With a tie, the last of
/// the oldest in the list.
#[must_use]
pub fn oldest(people: &[Person]) -> Option<&Person> {
    people.iter().max_by_key(|person| person.age)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, age: u32) -> Person {
        Person { name: name.to_string(), age }
    }

    #[test]
    fn picks_the_highest_age() {
        let people = [person("Ada", 36), person("Grace", 85), person("Alan", 41)];
        assert_eq!(oldest(&people).map(|person| person.name.as_str()), Some("Grace"));
        assert_eq!(oldest(&[]), None);
    }

    #[test]
    fn a_tie_goes_to_the_last() {
        let people = [person("first", 50), person("second", 50)];
        assert_eq!(oldest(&people).map(|person| person.name.as_str()), Some("second"));
    }
}
```

`max_by_key` returns the last maximum when several are equal; `min_by_key` returns the first minimum. If a tie should go to the first person, use `people.iter().rev().max_by_key(...)`.

---

<!-- source: std str::FromStr docs; u8::from_str_radix -->
### User
Parse a hex color like "#ff8800" into an Rgb struct.

### Assistant
Implementing `FromStr` lets callers write `"#ff8800".parse::<Rgb>()`.

```rust
use std::{fmt, num::ParseIntError, str::FromStr};

/// A color with 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    /// Red.
    pub red: u8,
    /// Green.
    pub green: u8,
    /// Blue.
    pub blue: u8,
}

/// Why a string is not a `#rrggbb` color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorError {
    /// It does not start with `#` followed by six characters.
    Shape,
    /// A channel is not two hex digits.
    Digits(ParseIntError),
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorError::Shape => f.write_str("expected # and six hex digits"),
            ColorError::Digits(error) => write!(f, "not a hex digit: {error}"),
        }
    }
}

impl std::error::Error for ColorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ColorError::Shape => None,
            ColorError::Digits(error) => Some(error),
        }
    }
}

impl FromStr for Rgb {
    type Err = ColorError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.strip_prefix('#').filter(|digits| digits.len() == 6).ok_or(ColorError::Shape)?;
        let channel = |range| {
            let pair = digits.get(range).ok_or(ColorError::Shape)?;
            u8::from_str_radix(pair, 16).map_err(ColorError::Digits)
        };
        Ok(Rgb { red: channel(0..2)?, green: channel(2..4)?, blue: channel(4..6)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_channel() -> Result<(), ColorError> {
        assert_eq!("#ff8800".parse::<Rgb>()?, Rgb { red: 255, green: 136, blue: 0 });
        assert_eq!("#0A0b0C".parse::<Rgb>()?, Rgb { red: 10, green: 11, blue: 12 });
        Ok(())
    }

    #[test]
    fn rejects_bad_shapes_and_digits() {
        assert_eq!("ff8800".parse::<Rgb>(), Err(ColorError::Shape));
        assert_eq!("#ff88".parse::<Rgb>(), Err(ColorError::Shape));
        assert!(matches!("#gg8800".parse::<Rgb>(), Err(ColorError::Digits(_))));
    }

    #[test]
    fn multibyte_text_is_a_shape_error_not_a_panic() {
        assert_eq!("#aé€".parse::<Rgb>(), Err(ColorError::Shape));
    }
}
```

`digits.get(range)` returns `None` instead of panicking when a range would split a multi-byte character, which the last test covers: `"aé€"` is six bytes, and the range `0..2` ends inside `é`.

---

<!-- source: Rust book ch05 (struct update syntax); std Default trait docs -->
### User
My config struct has 12 fields and I only want to change 2 of them in a test. Do I really have to write all 12?

### Assistant
No. Implement `Default` and use struct update syntax: name the fields you change and take the rest from `..Config::default()`.

```rust
/// Server settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The address to listen on.
    pub host: String,
    /// The port to listen on.
    pub port: u16,
    /// Requests served at once.
    pub workers: usize,
    /// Whether to log each request.
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { host: "127.0.0.1".to_string(), port: 8080, workers: 4, verbose: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_only_the_named_fields() {
        let config = Config { port: 9000, verbose: true, ..Config::default() };
        assert_eq!(config.port, 9000);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.workers, 4);
    }
}
```

If every field's own default is right (empty strings, zero, `false`), `#[derive(Default)]` replaces the hand-written `impl`.

---

<!-- source: Rust book ch17 (trait objects) and ch06 (enums) -->
### User
I'm making shapes with a Shape trait and Vec<Box<dyn Shape>>. Is that the right way?

### Assistant
It depends on who adds shapes. If the set of shapes is fixed and known to you, an enum is simpler: no boxing, `match` checks that every shape is handled, and adding an operation is one function. A trait object fits when other crates must add their own shapes, which an enum cannot allow.

For a fixed set:

```rust
use std::f64::consts::PI;

/// A shape in the plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A circle with its radius.
    Circle {
        /// The radius.
        radius: f64,
    },
    /// A rectangle with its sides.
    Rectangle {
        /// The width.
        width: f64,
        /// The height.
        height: f64,
    },
}

impl Shape {
    /// The area.
    #[must_use]
    pub fn area(self) -> f64 {
        match self {
            Shape::Circle { radius } => PI * radius * radius,
            Shape::Rectangle { width, height } => width * height,
        }
    }
}

/// The area of all `shapes` together.
#[must_use]
pub fn total_area(shapes: &[Shape]) -> f64 {
    shapes.iter().map(|shape| shape.area()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_areas() {
        let shapes = [Shape::Circle { radius: 1.0 }, Shape::Rectangle { width: 2.0, height: 3.0 }];
        assert!((total_area(&shapes) - (PI + 6.0)).abs() < 1e-12);
    }
}
```

The test compares floats with a tolerance, because rounding can make `PI + 6.0` and the computed sum differ in the last bit.
