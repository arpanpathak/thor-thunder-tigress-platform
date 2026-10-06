<!-- source: api-guidelines/src/macros.md; section: 07357a32a151cdc4; licence: Apache-2.0 -->
### User
I'm writing a `macro_rules!` macro that declares a set of flag constants. Should the entries be separated by commas or semicolons?

### Assistant
Semicolons. The Rust API guidelines (C-EVOCATIVE) recommend input syntax that looks like the code the macro produces. Your macro produces `const` items, and `const` items end with semicolons, so readers will expect semicolons. For the same reason, if the macro declares a struct, put the keyword `struct` before its name in the input instead of a bare name or a made-up word like `flags`.

```rust
macro_rules! flags {
    ($($name:ident = $value:expr;)*) => {
        $(
            #[doc = concat!("The `", stringify!($name), "` flag.")]
            pub const $name: u32 = $value;
        )*
    };
}

flags! {
    READ = 0b001;
    WRITE = 0b010;
    EXECUTE = 0b100;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_combine_as_bits() {
        assert_eq!(READ | WRITE, 0b011);
        assert_eq!(EXECUTE & WRITE, 0);
    }
}
```

The rule generalises: borrow keywords and punctuation from the Rust syntax closest to what the macro expands to, so someone reading the call site can guess what it declares without opening the macro.

---

<!-- source: api-guidelines/src/flexibility.md; section: 074ad89c8f2862e2; licence: Apache-2.0 -->
### User
I get "the trait `Shape` is not dyn compatible" because one method is generic. I need both `Vec<Box<dyn Shape>>` and that generic method. What can I do?

### Assistant
Add `where Self: Sized` to the generic method. That removes the method from the trait object (the vtable cannot hold a generic method, since it would need one entry per type argument), so the rest of the trait can be used as `dyn Shape`. On concrete types the method is still there.

```rust
/// A shape with an area, usable as `dyn Shape`.
pub trait Shape {
    /// The area.
    fn area(&self) -> f64;

    /// The area formatted by `format`. Generic, so it is left out of `dyn Shape`.
    fn describe_with<F: Fn(f64) -> String>(&self, format: F) -> String
    where
        Self: Sized,
    {
        format(self.area())
    }
}

/// A square with the given side.
pub struct Square(pub f64);

/// A rectangle with the given sides.
pub struct Rectangle(pub f64, pub f64);

impl Shape for Square {
    fn area(&self) -> f64 {
        self.0 * self.0
    }
}

impl Shape for Rectangle {
    fn area(&self) -> f64 {
        self.0 * self.1
    }
}

/// The area of every shape together.
#[must_use]
pub fn total_area(shapes: &[Box<dyn Shape>]) -> f64 {
    shapes.iter().map(|shape| shape.area()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_shapes_and_keeps_the_generic_method() {
        let shapes: Vec<Box<dyn Shape>> = vec![Box::new(Square(2.0)), Box::new(Rectangle(1.0, 3.0))];
        assert!((total_area(&shapes) - 7.0).abs() < 1e-12);
        assert_eq!(Square(3.0).describe_with(|area| format!("{area:.1} m²")), "9.0 m²");
    }
}
```

The standard library does the same: `Iterator` has many generic adaptor methods marked `where Self: Sized`, which is why `Box<dyn Iterator<Item = T>>` works. "Dyn compatible" is the current name for what older docs call "object safe".

### User
When should a trait be used as an object at all, versus as a generic bound?

### Assistant
Decide early, because it shapes the method signatures.

Trait objects (`dyn Trait`) are the choice when you need values of different types in one collection, and they keep code size down because each function is compiled once instead of once per type. The costs: calls go through a vtable, the value sits behind a pointer, methods cannot be generic, and `Self` can appear only as the receiver.

Generic bounds (`T: Trait`) give static dispatch: each use is compiled for the concrete type, calls can be inlined, and methods may be generic and return `Self`. The cost is code size, and a `Vec<T>` holds only one type.

`io::Read` and `io::Write` are typical object traits: code often takes `&mut dyn Write` so it can write to a file, a socket or a buffer without being compiled three times. If a trait is meant to be used as an object, its methods should take and return trait objects instead of generics.

---

<!-- source: api-guidelines/src/naming.md; section: 0d93e154ac37926e; licence: Apache-2.0 -->
### User
I have a `Celsius(f64)` wrapper. Should the conversion be called `as_fahrenheit`, `to_fahrenheit` or `into_fahrenheit`? And how should callers get the raw number?

### Assistant
`to_fahrenheit`. The Rust API guidelines (C-CONV) tie the prefix to cost and ownership:

| Prefix | Cost | Ownership |
|---|---|---|
| `as_` | free | borrowed to borrowed |
| `to_` | does work | borrowed to borrowed, borrowed to owned, or owned to owned for `Copy` types |
| `into_` | varies | owned to owned, consuming the input |

Converting a temperature is arithmetic, so it is `to_`, and because `Celsius` is `Copy` it takes `self` by value, as `f64::to_radians` does. `into_` would wrongly suggest the value is consumed. To get the wrapped number back out, the convention is `into_inner`.

```rust
/// A temperature in degrees Celsius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Celsius(f64);

impl Celsius {
    /// A temperature of `degrees` Celsius.
    #[must_use]
    pub fn new(degrees: f64) -> Self {
        Self(degrees)
    }

    /// The same temperature in degrees Fahrenheit.
    #[must_use]
    pub fn to_fahrenheit(self) -> f64 {
        self.0 * 9.0 / 5.0 + 32.0
    }

    /// The number of degrees Celsius.
    #[must_use]
    pub fn into_inner(self) -> f64 {
        self.0
    }
}

/// A name, stored as a `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(String);

impl Name {
    /// The name as a string slice, which costs nothing.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `String` inside, consuming the name.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_unwraps() {
        assert!((Celsius::new(100.0).to_fahrenheit() - 212.0).abs() < 1e-12);
        assert!((Celsius::new(-40.0).into_inner() + 40.0).abs() < 1e-12);
        let name = Name("Ada".to_string());
        assert_eq!(name.as_str(), "Ada");
        assert_eq!(name.into_string(), "Ada");
    }
}
```

`Name` shows the other two prefixes: `as_str` is a free view of the inside, and `into_string` gives up the wrapper to hand back the `String` without copying it.
