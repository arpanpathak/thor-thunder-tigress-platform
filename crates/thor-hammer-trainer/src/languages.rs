//! Checks code blocks in languages other than Rust with the local toolchains.
//!
//! | Fence | Built with | Run when |
//! |---|---|---|
//! | `python` | `python3 -m py_compile` | it has `assert` or `__main__` |
//! | `go` | `go vet` | it is `package main` with `func main()` |
//! | `cpp` | `g++ -std=c++20 -Wall -Wextra -Werror` | it has `int main` |
//! | `java` | `javac` | it has `static void main` |
//! | `javascript` | `node --check` | it has `assert` |
//! | `bash` | `bash -n` | never |
//! | `yaml` | Python's `yaml.safe_load_all` | never |
//! | `json` | `python3 -m json.tool` | never |
//!
//! A block is checked only when its fence names the language; untagged
//! blocks and other languages (`text`, `toml`, `console`) are left alone. As in
//! rustdoc, a fence like `rust,ignore` or `go,ignore` marks a fragment that is
//! not meant to build on its own (it needs a crate, or is cut from a larger
//! file); such blocks are counted but not built.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::LazyLock,
};

use regex::Regex;

use crate::{
    error::DataError,
    verify::{Built, Finished, TEST_LIMIT, run_limited},
};

/// A language whose blocks are checked here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    /// Python 3.
    Python,
    /// Go.
    Go,
    /// C++20.
    Cpp,
    /// Java.
    Java,
    /// JavaScript on Node.
    JavaScript,
    /// Bash.
    Shell,
    /// YAML.
    Yaml,
    /// JSON.
    Json,
}

impl Language {
    /// The language a fence tag names, or `None` for one not checked here.
    #[must_use]
    pub fn of(tag: &str) -> Option<Language> {
        match tag.to_ascii_lowercase().as_str() {
            "python" | "py" | "python3" => Some(Language::Python),
            "go" | "golang" => Some(Language::Go),
            "cpp" | "c++" | "cc" | "cxx" => Some(Language::Cpp),
            "java" => Some(Language::Java),
            "javascript" | "js" | "node" => Some(Language::JavaScript),
            "bash" | "sh" | "shell" => Some(Language::Shell),
            "yaml" | "yml" => Some(Language::Yaml),
            "json" => Some(Language::Json),
            _ => None,
        }
    }

    /// The name used in reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::Go => "go",
            Language::Cpp => "c++",
            Language::Java => "java",
            Language::JavaScript => "javascript",
            Language::Shell => "bash",
            Language::Yaml => "yaml",
            Language::Json => "json",
        }
    }
}

/// One fenced block in a checked language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The language its fence names.
    pub language: Language,
    /// The code, without its fences.
    pub code: String,
    /// True when the fence says `ignore`: a fragment not meant to build alone.
    pub ignored: bool,
}

/// A fenced block of any language: its fence's words and its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fenced {
    /// The language word of the fence, such as `rust`; empty when untagged.
    pub tag: String,
    /// True when the fence says `ignore`.
    pub ignored: bool,
    /// The code, without its fences.
    pub code: String,
}

/// Every fenced block of `text`, in order.
#[must_use]
pub fn fenced(text: &str) -> Vec<Fenced> {
    FENCED
        .iter()
        .flat_map(|pattern| pattern.captures_iter(text))
        .filter_map(|captures| {
            let info = captures.get(1)?.as_str();
            let mut words = info.split(',').map(str::trim);
            let tag = words.next().unwrap_or_default().to_string();
            let ignored = words.any(|word| word == "ignore");
            Some(Fenced {
                tag,
                ignored,
                code: captures.get(2)?.as_str().to_string(),
            })
        })
        .collect()
}

static FENCED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?ms)^[ \t]*```[ \t]*([\w+#.,-]*)[^\n]*\n(.*?)^[ \t]*```[ \t]*$").ok()
});

static JAVA_CLASS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"public\s+(?:final\s+)?(?:class|record|enum|interface)\s+(\w+)").ok()
});

/// The blocks of `text` whose fence names a language checked here.
#[must_use]
pub fn blocks(text: &str) -> Vec<Block> {
    fenced(text)
        .into_iter()
        .filter_map(|block| {
            let language = Language::of(&block.tag)?;
            Some(Block {
                language,
                code: block.code,
                ignored: block.ignored,
            })
        })
        .collect()
}

/// What to do with one block: the file to write, how to build it, how to run it.
struct Plan {
    file: PathBuf,
    build: Command,
    run: Option<Command>,
}

fn command(program: &str, arguments: &[&str], file: &Path, folder: &Path) -> Command {
    let mut command = Command::new(program);
    command.args(arguments).arg(file).current_dir(folder);
    command
}

fn plan(block: &Block, folder: &Path) -> Plan {
    let code = block.code.as_str();
    match block.language {
        Language::Python => {
            let file = folder.join("example.py");
            let runs = code.contains("assert ") || code.contains("__main__");
            Plan {
                build: command("python3", &["-m", "py_compile"], &file, folder),
                run: runs.then(|| command("python3", &[], &file, folder)),
                file,
            }
        }
        Language::Go => {
            let file = folder.join("example.go");
            let runs = code.contains("package main") && code.contains("func main()");
            Plan {
                build: command("go", &["vet"], &file, folder),
                run: runs.then(|| command("go", &["run"], &file, folder)),
                file,
            }
        }
        Language::Cpp => {
            let file = folder.join("example.cpp");
            let program = folder.join("example-cpp");
            let flags = ["-std=c++20", "-Wall", "-Wextra", "-Werror"];
            if code.contains("int main") {
                let mut build = command("g++", &flags, &file, folder);
                build.arg("-o").arg(&program);
                Plan {
                    build,
                    run: Some(Command::new(&program)),
                    file,
                }
            } else {
                let mut build = command("g++", &flags, &file, folder);
                build.arg("-fsyntax-only");
                Plan {
                    build,
                    run: None,
                    file,
                }
            }
        }
        Language::Java => {
            let class = JAVA_CLASS
                .as_ref()
                .and_then(|pattern| pattern.captures(code))
                .and_then(|captures| captures.get(1))
                .map_or("Main", |name| name.as_str());
            let file = folder.join(format!("{class}.java"));
            let run = code.contains("static void main").then(|| {
                let mut run = Command::new("java");
                run.arg("-cp").arg(folder).arg(class).current_dir(folder);
                run
            });
            let mut build = Command::new("javac");
            build.arg("-d").arg(folder).arg(&file).current_dir(folder);
            Plan { file, build, run }
        }
        Language::JavaScript => {
            let module = code
                .lines()
                .any(|line| line.starts_with("import ") || line.starts_with("export "));
            let file = folder.join(if module { "example.mjs" } else { "example.js" });
            Plan {
                build: command("node", &["--check"], &file, folder),
                run: code
                    .contains("assert")
                    .then(|| command("node", &[], &file, folder)),
                file,
            }
        }
        Language::Shell => {
            let file = folder.join("example.sh");
            Plan {
                build: command("bash", &["-n"], &file, folder),
                run: None,
                file,
            }
        }
        Language::Yaml => {
            let file = folder.join("example.yaml");
            let load = "import sys, yaml; list(yaml.safe_load_all(open(sys.argv[1])))";
            Plan {
                build: command("python3", &["-c", load], &file, folder),
                run: None,
                file,
            }
        }
        Language::Json => {
            let file = folder.join("example.json");
            Plan {
                build: command("python3", &["-m", "json.tool"], &file, folder),
                run: None,
                file,
            }
        }
    }
}

/// Builds `block` in `scratch` and, when it is a program with checks, runs it.
///
/// # Errors
///
/// `DataError::Io` when the folder can't be written or a toolchain can't be started.
pub fn check(block: &Block, scratch: &Path) -> Result<Built, DataError> {
    if block.ignored {
        return Ok(Built::Ignored);
    }
    let folder = scratch.join(block.language.name());
    fs::create_dir_all(&folder).map_err(DataError::io(&folder))?;
    let folder = std::path::absolute(&folder).map_err(DataError::io(&folder))?;
    let Plan { file, build, run } = plan(block, &folder);
    fs::write(&file, &block.code).map_err(DataError::io(&file))?;
    if let Finished::Failed(output) = run_limited(build, scratch, TEST_LIMIT * 2)? {
        return Ok(Built::BuildFailed(output));
    }
    let Some(run) = run else {
        return Ok(Built::Clean {
            tests: 0,
            ran: false,
        });
    };
    Ok(Built::after_run(run_limited(run, scratch, TEST_LIMIT)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "thor-hammer-languages-{name}-{}",
            std::process::id()
        ))
    }

    fn checked(language: Language, code: &str) -> Result<Built, DataError> {
        let folder = scratch(language.name());
        let built = check(
            &Block {
                language,
                code: code.to_string(),
                ignored: false,
            },
            &folder,
        )?;
        fs::remove_dir_all(&folder).map_err(DataError::io(&folder))?;
        Ok(built)
    }

    #[test]
    fn finds_tagged_blocks_only() {
        let text = "```python\nprint(1)\n```\n\n```\nuntagged\n```\n\n```text\nplain\n```\n\n```Go\npackage main\n```\n";
        let found: Vec<Language> = blocks(text)
            .into_iter()
            .map(|block| block.language)
            .collect();
        assert_eq!(found, [Language::Python, Language::Go]);
    }

    #[test]
    fn an_ignored_fence_is_not_built() -> Result<(), DataError> {
        let text = "```go,ignore\nfmt.Println(x)\n```\n\n```rust,no_run\nfn main() {}\n```\n";
        let found = fenced(text);
        assert_eq!(
            found
                .iter()
                .map(|block| (block.tag.as_str(), block.ignored))
                .collect::<Vec<_>>(),
            [("go", true), ("rust", false)]
        );
        let ignored = blocks(text)
            .into_iter()
            .next()
            .map(|block| check(&block, &scratch("ignored")));
        assert!(matches!(ignored, Some(Ok(Built::Ignored))));
        Ok(())
    }

    #[test]
    fn runs_a_python_program_and_catches_a_failed_assert() -> Result<(), DataError> {
        assert_eq!(
            checked(Language::Python, "assert sum([1, 2]) == 3\n")?,
            Built::Clean {
                tests: 0,
                ran: true
            }
        );
        assert!(matches!(
            checked(Language::Python, "assert 1 == 2\n")?,
            Built::TestsFailed(_)
        ));
        assert!(matches!(
            checked(Language::Python, "def f(:\n")?,
            Built::BuildFailed(_)
        ));
        Ok(())
    }

    #[test]
    fn builds_go_cpp_and_java() -> Result<(), DataError> {
        let go = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n";
        assert_eq!(
            checked(Language::Go, go)?,
            Built::Clean {
                tests: 0,
                ran: true
            }
        );
        let cpp = "#include <vector>\nint main() { std::vector<int> v{1}; return v.size() == 1 ? 0 : 1; }\n";
        assert_eq!(
            checked(Language::Cpp, cpp)?,
            Built::Clean {
                tests: 0,
                ran: true
            }
        );
        let java = "public class Hello {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n";
        assert_eq!(
            checked(Language::Java, java)?,
            Built::Clean {
                tests: 0,
                ran: true
            }
        );
        assert_eq!(
            checked(Language::Cpp, "inline int twice(int x) { return 2 * x; }\n")?,
            Built::Clean {
                tests: 0,
                ran: false
            }
        );
        Ok(())
    }

    #[test]
    fn parses_data_and_scripts_without_running_them() -> Result<(), DataError> {
        assert_eq!(
            checked(Language::Yaml, "a: [1, 2]\n")?,
            Built::Clean {
                tests: 0,
                ran: false
            }
        );
        assert!(matches!(
            checked(Language::Yaml, "a: [1, 2\n")?,
            Built::BuildFailed(_)
        ));
        assert!(matches!(
            checked(Language::Json, "{\"a\": }")?,
            Built::BuildFailed(_)
        ));
        assert_eq!(
            checked(Language::Shell, "set -eu\necho hi\n")?,
            Built::Clean {
                tests: 0,
                ran: false
            }
        );
        assert!(matches!(
            checked(Language::JavaScript, "const a = ;\n")?,
            Built::BuildFailed(_)
        ));
        let tested = "const assert = require(\"node:assert\");\nassert.strictEqual(1 + 1, 2);\n";
        assert_eq!(
            checked(Language::JavaScript, tested)?,
            Built::Clean {
                tests: 0,
                ran: true
            }
        );
        Ok(())
    }

    #[test]
    fn names_every_language() {
        let names: Vec<&str> = ["py", "go", "c++", "java", "js", "sh", "yml", "json"]
            .into_iter()
            .filter_map(Language::of)
            .map(Language::name)
            .collect();
        assert_eq!(
            names,
            [
                "python",
                "go",
                "c++",
                "java",
                "javascript",
                "bash",
                "yaml",
                "json"
            ]
        );
        assert_eq!(Language::of("toml"), None);
    }
}
