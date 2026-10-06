#!/usr/bin/env bash
#
# Fetches the open training corpus into corpus/, and writes corpus/MANIFEST.tsv.
#
# Every source here is picked because its licence allows reuse. The licence is
# not taken on trust: after each clone the LICENSE file is read and its first
# line recorded in the manifest. A source whose licence cannot be read is
# marked UNKNOWN and is not used by the data pipeline.
#
#   bash train/fetch_corpus.sh
#
# The corpus itself is gitignored. The manifest and this script are committed,
# so the corpus can be rebuilt from source and version alone.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORPUS="${ROOT}/corpus"
# Committed, unlike corpus/: the record of what was fetched has to survive.
MANIFEST="${ROOT}/train/corpus.manifest.tsv"

mkdir -p "${CORPUS}"

# name | url | kind | what it contributes | folders to check out (optional)
#
# kind "code" marks a repository read for its source code rather than its
# prose. A fifth field makes a sparse checkout of only those folders, so a
# large repository (the Rust or Go standard library) costs megabytes, not
# gigabytes. Licence files at the top level are always checked out.
SOURCES=(
  "trpl|https://github.com/rust-lang/book|book|The Rust Programming Language"
  "rbe|https://github.com/rust-lang/rust-by-example|book|Rust by Example"
  "nomicon|https://github.com/rust-lang/nomicon|book|The Rustonomicon"
  "async-book|https://github.com/rust-lang/async-book|book|Asynchronous Programming in Rust"
  "api-guidelines|https://github.com/rust-lang/api-guidelines|book|Rust API Guidelines"
  "rustc-dev-guide|https://github.com/rust-lang/rustc-dev-guide|book|rustc Dev Guide"
  "patterns|https://github.com/rust-unofficial/patterns|book|Rust Design Patterns"
  "too-many-lists|https://github.com/rust-unofficial/too-many-lists|book|Learn Rust With Entirely Too Many Linked Lists"
  "comprehensive-rust|https://github.com/google/comprehensive-rust|book|Comprehensive Rust, Google"
  "reference|https://github.com/rust-lang/reference|book|The Rust Reference"
  "cargo-book|https://github.com/rust-lang/cargo|book|The Cargo Book (src/doc)"
  "clippy|https://github.com/rust-lang/rust-clippy|docs|Clippy lint documentation"
  "google-styleguide|https://github.com/google/styleguide|style|Google style guides"
  "ms-style-guide|https://github.com/MicrosoftDocs/microsoft-style-guide|style|Microsoft Writing Style Guide"
  "kubernetes-website|https://github.com/kubernetes/website|docs|Kubernetes documentation (CC BY 4.0)"
  "go-website|https://github.com/golang/website|docs|Go documentation (BSD-3-Clause)"
  "perf-book|https://github.com/nnethercote/perf-book|book|The Rust Performance Book"
  "rust-cookbook|https://github.com/rust-lang-nursery/rust-cookbook|book|Rust Cookbook"
  "embedded-book|https://github.com/rust-embedded/book|book|The Embedded Rust Book"
  "tlborm|https://github.com/Veykril/tlborm|book|The Little Book of Rust Macros"
  "rust-rfcs|https://github.com/rust-lang/rfcs|docs|Rust RFCs: design documents with motivation and trade-offs"
  "tokio-website|https://github.com/tokio-rs/website|docs|Tokio tutorial and guides"
  "tigerbeetle|https://github.com/tigerbeetle/tigerbeetle|docs|TigerBeetle docs and TigerStyle (distributed database)"
  "etcd-website|https://github.com/etcd-io/website|docs|etcd documentation (Raft-based key-value store)"
  "prometheus-docs|https://github.com/prometheus/docs|docs|Prometheus documentation"
  "grpc-website|https://github.com/grpc/grpc.io|docs|gRPC documentation"
  "aosa-500lines|https://github.com/aosabook/500lines|book|500 Lines or Less (Architecture of Open Source Applications)"
  "system-design-primer|https://github.com/donnemartin/system-design-primer|book|The System Design Primer"
  "eng-practices|https://github.com/google/eng-practices|docs|Google Engineering Practices (code review)"
  "ms-api-guidelines|https://github.com/microsoft/api-guidelines|style|Microsoft REST API Guidelines"
  "twelve-factor|https://github.com/heroku/12factor|book|The Twelve-Factor App"
  "rust-cli-book|https://github.com/rust-cli/book|book|Command Line Applications in Rust"
  "rustwasm-book|https://github.com/rustwasm/book|book|Rust and WebAssembly"
  "edition-guide|https://github.com/rust-lang/edition-guide|book|The Rust Edition Guide"
  "unsafe-code-guidelines|https://github.com/rust-lang/unsafe-code-guidelines|docs|Rust unsafe code guidelines"
  "brown-rust-book|https://github.com/cognitive-engineering-lab/rust-book|book|The Rust Book, Brown University edition with quizzes"
  "cpp-core-guidelines|https://github.com/isocpp/CppCoreGuidelines|style|C++ Core Guidelines"
  "uber-go-guide|https://github.com/uber-go/guide|style|Uber Go Style Guide"
  "airbnb-javascript|https://github.com/airbnb/javascript|style|Airbnb JavaScript Style Guide"
  "gobyexample|https://github.com/mmcgrana/gobyexample|code|Go by Example: annotated Go programs|examples"
  "ods|https://github.com/patmorin/ods|code|Open Data Structures: Java implementations (java/ods is CC BY 2.5)|java/ods"
  "ripgrep|https://github.com/BurntSushi/ripgrep|code|ripgrep: fast recursive search in Rust|crates"
  "tokio|https://github.com/tokio-rs/tokio|code|tokio: async runtime for Rust|tokio/src/sync tokio/src/time tokio/src/task"
  "serde|https://github.com/serde-rs/serde|code|serde: serialization framework for Rust|serde/src serde_derive/src"
  "rust-std|https://github.com/rust-lang/rust|code|Rust standard library|library/core/src/iter library/core/src/option.rs library/core/src/result.rs library/alloc/src/collections library/std/src/sync"
  "go-std|https://github.com/golang/go|code|Go standard library|src/sort src/container src/strings src/sync src/bufio src/slices src/maps"
  "cpython-lib|https://github.com/python/cpython|code|Python standard library (pure Python modules)|Lib/heapq.py Lib/bisect.py Lib/functools.py Lib/collections Lib/textwrap.py Lib/dataclasses.py Lib/graphlib.py Lib/statistics.py"
  "leveldb|https://github.com/google/leveldb|code|LevelDB: key-value store in C++|db util table"
  "abseil-cpp|https://github.com/abseil/abseil-cpp|code|Abseil C++ libraries|absl/strings absl/container"
  "requests|https://github.com/psf/requests|code|requests: HTTP for Python|src/requests"
  "express|https://github.com/expressjs/express|code|Express: web framework for Node.js|lib"
  "guava|https://github.com/google/guava|code|Guava: core Java libraries|guava/src/com/google/common/collect guava/src/com/google/common/base"
)

# Every licence file at the top of a repository, one per line. Third-party
# notices are left out: they list the licences of dependencies, not of the text.
licence_files() {
  local dir="$1"
  find "${dir}" -maxdepth 1 -type f \
    \( -iname 'licen[cs]e*' -o -iname 'copying*' \) 2>/dev/null \
    | grep -iv 'third.party' | sort
}

# Licence titles that can sit deep inside a long licence file, past the part
# the summary keeps. CPython's file starts with the history of the software.
KNOWN_TITLES=(
  "PYTHON SOFTWARE FOUNDATION LICENSE VERSION 2"
)

# A single line summarising a licence file: any known title found anywhere in
# it, then its first 400 characters.
summarise() {
  local file="$1"
  if [ -z "${file}" ]; then
    printf 'NO-LICENCE-FILE'
    return 0
  fi
  local title
  for title in "${KNOWN_TITLES[@]}"; do
    grep -qF "${title}" "${file}" && printf '%s: ' "${title}"
  done
  tr '\n' ' ' < "${file}" \
    | tr -s ' ' \
    | cut -c1-400
}

printf 'source\tkind\tcommit\tlicence_file\tlicence\n' > "${MANIFEST}"

for entry in "${SOURCES[@]}"; do
  IFS='|' read -r name url kind purpose paths <<< "${entry}"
  dir="${CORPUS}/${name}"

  if [ -d "${dir}/.git" ]; then
    printf 'have   %-22s %s\n' "${name}" "${purpose}"
  else
    printf 'clone  %-22s %s\n' "${name}" "${purpose}"
    if [ -n "${paths:-}" ]; then
      read -r -a folders <<< "${paths}"
      cloned() {
        git clone --depth 1 --filter=blob:none --sparse --quiet "${url}" "${dir}" 2>/dev/null \
          && git -C "${dir}" sparse-checkout set --no-cone "/*" "!/*/" "${folders[@]/#//}" 2>/dev/null
      }
    else
      cloned() { git clone --depth 1 --quiet "${url}" "${dir}" 2>/dev/null; }
    fi
    if ! cloned; then
      printf 'FAILED %-22s %s — not cloned, not used\n' "${name}" "${url}"
      printf '%s\t%s\t-\t-\tCLONE-FAILED\n' "${name}" "${kind}" >> "${MANIFEST}"
      continue
    fi
  fi

  commit="$(git -C "${dir}" rev-parse --short HEAD 2>/dev/null || printf 'unknown')"
  # A repository can license its code and its text differently, so every
  # licence file is recorded, and the generator refuses the source when any
  # one of them is restrictive.
  licence=""
  relative=""
  while IFS= read -r file; do
    [ -z "${file}" ] && continue
    licence="${licence}${licence:+ || }$(summarise "${file}")"
    relative="${relative}${relative:+,}${file#"${dir}/"}"
  done <<< "$(licence_files "${dir}")"
  [ -z "${licence}" ] && licence="$(summarise "")"
  [ -z "${relative}" ] && relative="-"
  printf '%s\t%s\t%s\t%s\t%s\n' "${name}" "${kind}" "${commit}" "${relative}" "${licence}" >> "${MANIFEST}"
done

printf '\nwrote %s\n' "${MANIFEST}"
column -t -s $'\t' "${MANIFEST}" | cut -c1-190
