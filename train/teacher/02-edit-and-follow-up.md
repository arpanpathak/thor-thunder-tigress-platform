<!-- source: functional-vs-imperative-rust.pdf (guard clauses); std str::split_once docs -->
### User
Flatten this. No nested ifs.

```rust
pub fn parse_line(line: &str) -> Option<(String, String)> {
    if !line.starts_with('#') {
        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim();
            if !key.is_empty() {
                let value = line[pos + 1..].trim();
                return Some((key.to_string(), value.to_string()));
            } else {
                return None;
            }
        } else {
            return None;
        }
    } else {
        return None;
    }
}
```

### Assistant
Each `else { return None; }` becomes a guard clause, and `split_once` replaces finding the index and slicing around it:

```rust
/// The key and value of a `key = value` line, or `None` for a comment, a
/// line without `=`, or an empty key.
#[must_use]
pub fn parse_line(line: &str) -> Option<(String, String)> {
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), value.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_key_and_value() {
        assert_eq!(parse_line(" port = 8080 "), Some(("port".to_string(), "8080".to_string())));
        assert_eq!(parse_line("url=a=b"), Some(("url".to_string(), "a=b".to_string())));
    }

    #[test]
    fn skips_comments_bare_words_and_empty_keys() {
        assert_eq!(parse_line("# port = 1"), None);
        assert_eq!(parse_line("port"), None);
        assert_eq!(parse_line(" = 1"), None);
    }
}
```

The behaviour is the same as before. A `#` still only counts at the very start of the line; if indented comments should count too, test `line.trim_start().starts_with('#')`.

---

<!-- source: std slice::windows docs; clippy ptr_arg -->
### User
This panics on an empty vec. Fix it and get rid of the index loop.

```rust
pub fn rising_pairs(values: &Vec<i32>) -> usize {
    let mut count = 0;
    for i in 0..values.len() - 1 {
        if values[i] < values[i + 1] {
            count += 1;
        }
    }
    count
}
```

### Assistant
`values.len() - 1` underflows when the vec is empty: a debug build panics on the subtraction, and a release build wraps to `usize::MAX` and panics on `values[0]`. `windows(2)` yields nothing for fewer than two elements, so the problem goes away with the loop:

```rust
/// How many neighbouring pairs in `values` go up.
#[must_use]
pub fn rising_pairs(values: &[i32]) -> usize {
    values
        .windows(2)
        .filter(|pair| matches!(pair, [left, right] if left < right))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_rise() {
        assert_eq!(rising_pairs(&[1, 3, 2, 4, 4]), 2);
    }

    #[test]
    fn short_inputs_have_no_pairs() {
        assert_eq!(rising_pairs(&[]), 0);
        assert_eq!(rising_pairs(&[7]), 0);
    }
}
```

The parameter is now `&[i32]`. Callers holding a `Vec<i32>` pass `&values` as before, and arrays and slices work too.

---

<!-- source: std str::split_whitespace docs -->
### User
Write tests for this. Don't change the function.

```rust
/// Splits `text` into lines of at most `width` characters, breaking only
/// between words. A word longer than `width` gets a line of its own.
#[must_use]
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}
```

### Assistant
```rust
/// Splits `text` into lines of at most `width` characters, breaking only
/// between words. A word longer than `width` gets a line of its own.
#[must_use]
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_no_lines() {
        assert_eq!(wrap("", 10), Vec::<String>::new());
        assert_eq!(wrap("   ", 10), Vec::<String>::new());
    }

    #[test]
    fn fills_each_line_up_to_the_width() {
        assert_eq!(wrap("the quick brown fox", 9), ["the quick", "brown fox"]);
    }

    #[test]
    fn a_line_may_end_exactly_at_the_width() {
        assert_eq!(wrap("ab cd", 5), ["ab cd"]);
        assert_eq!(wrap("ab cd", 4), ["ab", "cd"]);
    }

    #[test]
    fn a_long_word_gets_its_own_line() {
        assert_eq!(wrap("a incomprehensible b", 5), ["a", "incomprehensible", "b"]);
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(wrap("über straße", 11), ["über straße"]);
    }

    #[test]
    fn collapses_runs_of_whitespace() {
        assert_eq!(wrap("a  \n b", 10), ["a b"]);
    }
}
```

The last test documents a behaviour you might not want: line breaks in the input are not kept, because `split_whitespace` treats them like spaces.

---

<!-- source: std collections::HashMap entry API docs; slice::sort_by_key (stable sort) docs -->
### User
Count how often each word appears in a text, case-insensitive.

### Assistant
```rust
use std::collections::HashMap;

/// How often each word appears in `text`, ignoring case. A word is a run of
/// letters, digits and apostrophes.
#[must_use]
pub fn word_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|word| !word.is_empty()) {
        *counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_case_and_punctuation() {
        let counts = word_counts("The cat, the hat. THE end!");
        assert_eq!(counts.get("the"), Some(&3));
        assert_eq!(counts.get("cat"), Some(&1));
        assert_eq!(counts.len(), 4);
    }
}
```

### User
Now give me the top n words, most common first, ties in alphabetical order.

### Assistant
Sort by count descending, then by word:

```rust
use std::collections::HashMap;

/// How often each word appears in `text`, ignoring case. A word is a run of
/// letters, digits and apostrophes.
#[must_use]
pub fn word_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|word| !word.is_empty()) {
        *counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    counts
}

/// The `n` most common words with their counts, most common first, ties in
/// alphabetical order.
#[must_use]
pub fn top_words(text: &str, n: usize) -> Vec<(String, usize)> {
    let mut ranked: Vec<(String, usize)> = word_counts(text).into_iter().collect();
    ranked.sort_unstable_by(|(word, count), (other_word, other_count)| {
        other_count.cmp(count).then_with(|| word.cmp(other_word))
    });
    ranked.truncate(n);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_by_count_then_word() {
        let top = top_words("b a c b a d b", 3);
        assert_eq!(top, [("b".to_string(), 3), ("a".to_string(), 2), ("c".to_string(), 1)]);
    }

    #[test]
    fn asking_for_more_than_exist_returns_all() {
        assert_eq!(top_words("x y", 10).len(), 2);
    }
}
```

`truncate` does nothing when `n` is larger than the number of words, so no bounds check is needed.

### User
Use a BTreeMap so you don't need the tie-break in the sort.

### Assistant
That works because `sort_by_key` is stable: a `BTreeMap` yields the words in alphabetical order, and a stable sort by count keeps equal counts in that order.

```rust
use std::{cmp::Reverse, collections::BTreeMap};

/// How often each word appears in `text`, ignoring case, in alphabetical
/// order. A word is a run of letters, digits and apostrophes.
#[must_use]
pub fn word_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|word| !word.is_empty()) {
        *counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    counts
}

/// The `n` most common words with their counts, most common first, ties in
/// alphabetical order.
#[must_use]
pub fn top_words(text: &str, n: usize) -> Vec<(String, usize)> {
    let mut ranked: Vec<(String, usize)> = word_counts(text).into_iter().collect();
    ranked.sort_by_key(|&(_, count)| Reverse(count));
    ranked.truncate(n);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_by_count_then_word() {
        let top = top_words("b a c b a d b", 3);
        assert_eq!(top, [("b".to_string(), 3), ("a".to_string(), 2), ("c".to_string(), 1)]);
    }

    #[test]
    fn asking_for_more_than_exist_returns_all() {
        assert_eq!(top_words("x y", 10).len(), 2);
    }
}
```

The cost: `BTreeMap` inserts are O(log n) instead of O(1) on average, and `sort_by_key` allocates a buffer that `sort_unstable_by` does not. For word counts of ordinary text neither is noticeable. Switching to `sort_unstable_by_key` would break the tie order, so it has to stay `sort_by_key`.

---

<!-- source: std::error::Error and std::fmt::Display docs; Rust API guidelines C-GOOD-ERR -->
### User
Write a function that reads a config file of `key = value` lines into a HashMap. No unwrap, and a hand-written error enum.

### Assistant
Parsing is separate from reading the file, so it can be tested without files.

```rust
use std::{collections::HashMap, fmt, fs, io, path::Path};

/// Why a config could not be loaded.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Io(io::Error),
    /// A line has no `=`.
    MissingEquals {
        /// The line number, from 1.
        line: usize,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "could not read the config: {error}"),
            ConfigError::MissingEquals { line } => write!(f, "line {line}: expected key = value"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(error) => Some(error),
            ConfigError::MissingEquals { .. } => None,
        }
    }
}

/// Reads the config at `path`.
///
/// # Errors
///
/// [`ConfigError::Io`] when the file can't be read, or a parse error from [`parse`].
pub fn load(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let text = fs::read_to_string(path).map_err(ConfigError::Io)?;
    parse(&text)
}

/// Reads `key = value` lines, skipping blank lines and `#` comments.
///
/// # Errors
///
/// [`ConfigError::MissingEquals`] for a line without `=`.
pub fn parse(text: &str) -> Result<HashMap<String, String>, ConfigError> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(line, text)| {
            let (key, value) = text.split_once('=').ok_or(ConfigError::MissingEquals { line })?;
            Ok((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}
```

`collect` into `Result<HashMap<_, _>, _>` stops at the first error and returns it.

### User
Also reject duplicate keys.

### Assistant
`collect` can't see duplicates, so `parse` becomes a loop that checks each insert. `HashMap::insert` returns the old value when the key was already there, which is the check.

```rust
use std::{collections::HashMap, fmt, fs, io, path::Path};

/// Why a config could not be loaded.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Io(io::Error),
    /// A line has no `=`.
    MissingEquals {
        /// The line number, from 1.
        line: usize,
    },
    /// A key appears a second time.
    DuplicateKey {
        /// The line number of the second appearance, from 1.
        line: usize,
        /// The repeated key.
        key: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "could not read the config: {error}"),
            ConfigError::MissingEquals { line } => write!(f, "line {line}: expected key = value"),
            ConfigError::DuplicateKey { line, key } => write!(f, "line {line}: {key} is already set"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(error) => Some(error),
            ConfigError::MissingEquals { .. } | ConfigError::DuplicateKey { .. } => None,
        }
    }
}

/// Reads the config at `path`.
///
/// # Errors
///
/// [`ConfigError::Io`] when the file can't be read, or a parse error from [`parse`].
pub fn load(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let text = fs::read_to_string(path).map_err(ConfigError::Io)?;
    parse(&text)
}

/// Reads `key = value` lines, skipping blank lines and `#` comments.
///
/// # Errors
///
/// [`ConfigError::MissingEquals`] for a line without `=`, and
/// [`ConfigError::DuplicateKey`] for a key set twice.
pub fn parse(text: &str) -> Result<HashMap<String, String>, ConfigError> {
    let mut config = HashMap::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(ConfigError::MissingEquals { line: line_number })?;
        let key = key.trim().to_string();
        if config.insert(key.clone(), value.trim().to_string()).is_some() {
            return Err(ConfigError::DuplicateKey { line: line_number, key });
        }
    }
    Ok(config)
}
```

### User
Add tests.

### Assistant
The tests call `parse`, so they need no files. `load` only adds the file read, and its error is covered by asking for a path that does not exist.

```rust
use std::{collections::HashMap, fmt, fs, io, path::Path};

/// Why a config could not be loaded.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Io(io::Error),
    /// A line has no `=`.
    MissingEquals {
        /// The line number, from 1.
        line: usize,
    },
    /// A key appears a second time.
    DuplicateKey {
        /// The line number of the second appearance, from 1.
        line: usize,
        /// The repeated key.
        key: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "could not read the config: {error}"),
            ConfigError::MissingEquals { line } => write!(f, "line {line}: expected key = value"),
            ConfigError::DuplicateKey { line, key } => write!(f, "line {line}: {key} is already set"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(error) => Some(error),
            ConfigError::MissingEquals { .. } | ConfigError::DuplicateKey { .. } => None,
        }
    }
}

/// Reads the config at `path`.
///
/// # Errors
///
/// [`ConfigError::Io`] when the file can't be read, or a parse error from [`parse`].
pub fn load(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let text = fs::read_to_string(path).map_err(ConfigError::Io)?;
    parse(&text)
}

/// Reads `key = value` lines, skipping blank lines and `#` comments.
///
/// # Errors
///
/// [`ConfigError::MissingEquals`] for a line without `=`, and
/// [`ConfigError::DuplicateKey`] for a key set twice.
pub fn parse(text: &str) -> Result<HashMap<String, String>, ConfigError> {
    let mut config = HashMap::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(ConfigError::MissingEquals { line: line_number })?;
        let key = key.trim().to_string();
        if config.insert(key.clone(), value.trim().to_string()).is_some() {
            return Err(ConfigError::DuplicateKey { line: line_number, key });
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keys_and_skips_comments() -> Result<(), ConfigError> {
        let config = parse("# server\nhost = thor\n\nport=8080\n")?;
        assert_eq!(config.get("host").map(String::as_str), Some("thor"));
        assert_eq!(config.get("port").map(String::as_str), Some("8080"));
        assert_eq!(config.len(), 2);
        Ok(())
    }

    #[test]
    fn names_the_line_without_equals() {
        let error = parse("a = 1\nb\n").err().map(|error| error.to_string());
        assert_eq!(error.as_deref(), Some("line 2: expected key = value"));
    }

    #[test]
    fn names_the_line_of_a_repeated_key() {
        let error = parse("a = 1\n# again\na = 2\n").err().map(|error| error.to_string());
        assert_eq!(error.as_deref(), Some("line 3: a is already set"));
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let result = load(Path::new("/nonexistent/config.txt"));
        assert!(matches!(result, Err(ConfigError::Io(_))));
    }
}
```
