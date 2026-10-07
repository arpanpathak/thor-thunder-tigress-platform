//! Runs the crate's programs on a small tree of input files, the way they
//! are run by hand.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use thor_hammer_trainer::error::DataError;

/// A home folder and a repository checkout, removed when dropped.
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(name: &str) -> Result<Self, DataError> {
        let tree = Self {
            root: std::env::temp_dir()
                .join(format!("thor-hammer-bin-{name}-{}", std::process::id())),
        };
        let long =
            "A page is a fixed-size block of virtual memory that the kernel maps. ".repeat(4);
        let store = "home/Projects/edgechat/convo_datastore";
        tree.write(
            &format!("{store}/readability_training.md"),
            &format!("# Set\n---\n### Instruction\nWhat is a page?\n### Response\n{long}\n"),
        )?;
        tree.write(
            &format!("{store}/clever_vs_readable/clever_vs_readable_sft.jsonl"),
            "",
        )?;
        tree.write(
            &format!("{store}/clever_vs_readable/clever_vs_readable_dpo.jsonl"),
            "",
        )?;
        tree.write(&format!("{store}/work/extracted/conversations.json"), "[]")?;
        tree.write(&format!("{store}/chat_0.md"), "no question here")?;
        tree.write(
            "home/Projects/nvidia-cloud-software-engineer-interview/book/ch01.md",
            &format!("# Memory\n\n## Pages\n\n{long}\n"),
        )?;
        tree.write(
            "repo/train/corpus.manifest.tsv",
            "source\tkind\tcommit\tlicence_file\tlicence\nlib\tcode\tabc\tLICENSE\tMIT License\n",
        )?;
        tree.write(
            "repo/corpus/lib/src/heap.py",
            &"def push(heap, item):\n    heap.append(item)\n".repeat(60),
        )?;
        tree.write(
            "repo/train/teacher/a.md",
            "<!-- source: notes -->\n### User\nQ\n\n### Assistant\nA plain answer.\n",
        )?;
        Ok(tree)
    }

    fn write(&self, path: &str, text: &str) -> Result<(), DataError> {
        let file = self.root.join(path);
        let parent = file.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent).map_err(DataError::io(parent))?;
        fs::write(&file, text).map_err(DataError::io(&file))
    }

    fn read(&self, path: &str) -> Result<String, DataError> {
        let file = self.root.join(path);
        fs::read_to_string(&file).map_err(DataError::io(&file))
    }

    fn run(&self, program: &str, arguments: &[&str]) -> Result<Output, DataError> {
        Command::new(program)
            .args(arguments)
            .current_dir(self.root.join("repo"))
            .env("HOME", self.root.join("home"))
            .output()
            .map_err(DataError::io(Path::new(program)))
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_builder_writes_the_training_set() -> Result<(), DataError> {
    let tree = Tree::new("build")?;
    let built = tree.run(env!("CARGO_BIN_EXE_thor-hammer-trainer"), &["out"])?;
    assert!(built.status.success(), "{}", text(&built.stderr));
    let defaulted = tree.run(env!("CARGO_BIN_EXE_thor-hammer-trainer"), &[])?;
    assert!(defaulted.status.success() && tree.root.join("repo/data/train.jsonl").is_file());
    assert!(
        text(&built.stdout).contains("lib                    code repository, read by the teacher")
    );
    assert_eq!(
        tree.read("repo/out/train.jsonl")?.lines().count(),
        2,
        "{}",
        tree.read("repo/out/train.jsonl")?
    );
    tree.write(
        "repo/labels/slop_flags.jsonl",
        "{\"id\":\"gone\",\"note\":\"old\",\"spans\":[]}\n",
    )?;
    let warned = tree.run(env!("CARGO_BIN_EXE_thor-hammer-trainer"), &["out"])?;
    assert!(text(&warned.stderr).contains("flag gone matched no example"));
    Ok(())
}

#[test]
fn the_builder_fails_with_the_missing_file_named() -> Result<(), DataError> {
    let tree = Tree::new("missing")?;
    fs::remove_file(
        tree.root
            .join("home/Projects/edgechat/convo_datastore/chat_0.md"),
    )
    .map_err(DataError::io(&tree.root))?;
    let failed = tree.run(env!("CARGO_BIN_EXE_thor-hammer-trainer"), &["out"])?;
    assert!(!failed.status.success());
    assert!(text(&failed.stderr).contains("chat_0.md"));
    Ok(())
}

#[test]
fn the_teacher_checks_and_queues() -> Result<(), DataError> {
    let tree = Tree::new("teacher")?;
    let checked = tree.run(
        env!("CARGO_BIN_EXE_teacher"),
        &["check", "train/teacher", "data"],
    )?;
    assert!(checked.status.success(), "{}", text(&checked.stderr));
    assert!(text(&checked.stdout).contains("| Passed every check | 1 |"));
    tree.write("repo/data/train.jsonl", "")?;
    let queued = tree.run(env!("CARGO_BIN_EXE_teacher"), &["pick", "1"])?;
    assert!(queued.status.success(), "{}", text(&queued.stderr));
    assert!(text(&queued.stdout).starts_with("1 sections queued"));
    Ok(())
}

#[test]
fn the_teacher_reports_failures_and_bad_arguments() -> Result<(), DataError> {
    let tree = Tree::new("teacher-fail")?;
    tree.write("repo/train/teacher/b.md", "### User\nno source comment\n")?;
    let failed = tree.run(env!("CARGO_BIN_EXE_teacher"), &[])?;
    assert!(!failed.status.success());
    assert!(text(&failed.stderr).contains("some entries failed"));
    let usage = tree.run(env!("CARGO_BIN_EXE_teacher"), &["pick", "many"])?;
    assert!(!usage.status.success());
    assert!(text(&usage.stderr).starts_with("usage: teacher"));
    let broken = tree.run(
        env!("CARGO_BIN_EXE_teacher"),
        &["check", "no-such-folder", "data"],
    )?;
    assert!(text(&broken.stderr).starts_with("teacher: "));
    Ok(())
}
