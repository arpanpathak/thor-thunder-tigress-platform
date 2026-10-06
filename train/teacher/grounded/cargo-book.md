<!-- source: cargo-book/doc/book/src/reference/config.md; section: 0010a83e2b06091e; licence: Apache-2.0 -->
### User
I type `cargo run --release --example bench` all day. Can I make a shortcut for it?

### Assistant
Yes, with an alias in Cargo's configuration. Put this in `.cargo/config.toml` in the project (or in `~/.cargo/config.toml` for every project):

```toml
[alias]
rr = "run --release"
bench-example = "rr --example bench"
```

Now `cargo rr` runs `cargo run --release`, and `cargo bench-example` runs the full command. Aliases can refer to other aliases, as `bench-example` does with `rr`.

A string value is split on spaces into the subcommand and its arguments. If an argument contains a space, use an array instead: `with-feature = ["run", "--features", "a b"]`. Two limits: an alias cannot redefine a built-in command such as `build`, and the built-in short forms `b`, `c`, `d`, `t`, `r` and `rm` already exist. You can also define an alias for one shell session with an environment variable, `CARGO_ALIAS_RR="run --release"`.

---

<!-- source: cargo-book/doc/book/src/reference/resolver.md; section: 003005f6d821c9d4; licence: Apache-2.0 -->
### User
My Cargo.toml says `bitflags = "*"`, a newer bitflags came out, and `cargo build` still uses the old one. Is Cargo broken?

### Assistant
No, that is the lock file doing its job. When `Cargo.lock` exists, Cargo prefers the versions recorded in it, so builds stay reproducible: the version chosen when the lock file was written (say 1.2.1) keeps being used even after 1.3.5 is published, because 1.2.1 still matches `"*"`.

The locked version is dropped only when it stops matching the requirement in `Cargo.toml`. If you change the line to `bitflags = "1.3.0"`, then 1.2.1 no longer satisfies it, Cargo ignores that lock entry, picks the newest matching version (1.3.5) and records it in `Cargo.lock`.

To move to a newer version without touching `Cargo.toml`, ask for it:

```bash
cargo update -p bitflags
cargo update
```

The first updates only `bitflags` (and whatever it needs); the second updates every dependency within the ranges `Cargo.toml` allows. A `"*"` requirement is best avoided anyway: it accepts any future major version, including ones with breaking changes, and crates.io refuses to publish a crate with wildcard dependencies.

---

<!-- source: cargo-book/doc/book/src/reference/build-script-examples.md; section: 004a7ad3b80c0270; licence: Apache-2.0 -->
### User
My crate links to a C library and some functions only exist in newer versions of it. How do I compile those functions only when the installed library is new enough?

### Assistant
Have the build script detect the version and turn it into `cfg` flags, then guard the code with `#[cfg(...)]`. The `openssl` crates do exactly this.

The `-sys` crate that links the library declares `links = "openssl"` in its `Cargo.toml` and, in its build script, publishes what it found:

```rust,ignore
println!("cargo::metadata=version_number={openssl_version:x}");
```

Cargo turns that into the environment variable `DEP_OPENSSL_VERSION_NUMBER` for the build scripts of crates that depend on it directly. Your crate's `build.rs` reads it and emits one `cfg` per feature level:

```rust
use std::env;

/// Each `cfg` this crate checks, with the library version that introduced it.
const LEVELS: [(&str, u64); 3] = [("ossl101", 0x1000_1000), ("ossl110", 0x1010_0000), ("ossl111", 0x1010_1000)];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(ossl101,ossl110,ossl111)");
    let version = env::var("DEP_OPENSSL_VERSION_NUMBER")
        .ok()
        .and_then(|hex| u64::from_str_radix(&hex, 16).ok());
    let Some(version) = version else {
        return;
    };
    for (name, _) in LEVELS.iter().filter(|&&(_, introduced)| version >= introduced) {
        println!("cargo::rustc-cfg={name}");
    }
}
```

Then in the library:

```rust,ignore
#[cfg(ossl111)]
pub fn sha3_224() -> MessageDigest {
    unsafe { MessageDigest(ffi::EVP_sha3_224()) }
}
```

`rustc-check-cfg` declares the names so the compiler does not warn about unknown `cfg`s. A version that is missing or not valid hex emits no flags, so only the oldest API is compiled instead of the build failing.

The cost of this approach: the binary now depends on the library found on the build machine. Copy it to a machine with an older library and the newer functions may be missing at load time.
