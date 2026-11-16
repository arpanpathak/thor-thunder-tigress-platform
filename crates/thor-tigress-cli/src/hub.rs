//! The Hugging Face Hub: search, a repository's files, its `config.json`, and downloads.
//!
//! The address comes from `HF_ENDPOINT` (default `https://huggingface.co`). The token comes
//! from `HF_TOKEN`, `HUGGING_FACE_HUB_TOKEN`, or the file `huggingface-cli login` writes,
//! and is only ever sent to that address. It is never printed. With `HF_HUB_OFFLINE=1`,
//! as for Hugging Face's own tools, nothing is asked of the Hub.

use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use serde_json::Value;

use crate::{
    engine::Engine,
    error::{Outcome, ThorError, file_error},
    memory::gigabytes,
    web,
};

/// One search result, with the fields `thor-tigress` asks the Hub to include.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HubModel {
    /// The repository, such as `ORGANIZATION/NAME`.
    pub id: String,
    /// Downloads in the last 30 days.
    #[serde(default)]
    pub downloads: u64,
    /// The repository's tags, which name its format.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The models this one was made from.
    #[serde(default, rename = "baseModels")]
    pub base_models: Option<BaseModels>,
    /// For a GGUF repository, the Hub's reading of its metadata.
    #[serde(default)]
    pub gguf: Option<GgufSummary>,
}

/// The `baseModels` field of a search result.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BaseModels {
    /// The base models.
    #[serde(default)]
    pub models: Vec<BaseModel>,
}

/// One base model.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BaseModel {
    /// Its repository.
    pub id: String,
}

/// The `gguf` field of a search result.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GgufSummary {
    /// The context length written in the GGUF.
    #[serde(default)]
    pub context_length: Option<u64>,
}

impl HubModel {
    /// The first base model's repository. A GGUF repository rarely has a `config.json`, so
    /// its base model's is read instead.
    pub fn base_model(&self) -> Option<&str> {
        let base_models = self.base_models.as_ref()?;

        base_models.models.first().map(|model| model.id.as_str())
    }
}

/// One file in a repository.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RepoFile {
    /// `file` or `directory`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The path inside the repository.
    pub path: String,
    /// The size in bytes.
    #[serde(default)]
    pub size: u64,
}

/// Reads a search reply.
pub fn parse_search(text: &str) -> Outcome<Vec<HubModel>> {
    serde_json::from_str(text).map_err(|source| ThorError::Json {
        what: "the Hub's search reply".to_string(),
        source,
    })
}

/// Reads a file-list reply, keeping files and leaving out folders and any path that
/// would land outside the download folder.
pub fn parse_files(text: &str) -> Outcome<Vec<RepoFile>> {
    let entries: Vec<RepoFile> = serde_json::from_str(text).map_err(|source| ThorError::Json {
        what: "the Hub's file list".to_string(),
        source,
    })?;

    Ok(entries
        .into_iter()
        .filter(|entry| entry.kind == "file" && stays_inside(&entry.path))
        .collect())
}

/// Whether a path from the Hub stays inside the folder it is downloaded into: relative,
/// with no `..`.
fn stays_inside(path: &str) -> bool {
    Path::new(path)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

/// The search address: models tagged with the engine's format, matching `text`, most
/// downloaded first, with the fields [`HubModel`] reads.
pub fn search_url(endpoint: &str, engine: Engine, text: &str, limit: usize) -> String {
    format!(
        "{endpoint}/api/models?filter={}&search={}&sort=downloads&limit={limit}\
         &expand[]=downloads&expand[]=tags&expand[]=baseModels&expand[]=gguf",
        engine.format_tag(),
        web::encode_query(text),
    )
}

/// A connection to the Hugging Face Hub.
pub struct Hub {
    /// The Hub's address, without a trailing slash.
    pub endpoint: String,
    /// The access token, when one was found.
    token: Option<String>,
    /// Whether `HF_HUB_OFFLINE` is set. When it is, `thor-tigress` sends no request to the Hub.
    pub offline: bool,
    /// The HTTP client.
    client: ureq::Agent,
}

impl Hub {
    /// The Hub named by `HF_ENDPOINT`, with the token found in the environment or under
    /// `home`.
    pub fn from_environment(home: &Path) -> Hub {
        let endpoint =
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".to_string());

        Hub {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            token: find_token(home),
            offline: std::env::var("HF_HUB_OFFLINE")
                .is_ok_and(|value| !matches!(value.as_str(), "" | "0")),
            client: web::client(Duration::from_secs(15), Duration::from_secs(60)),
        }
    }

    /// Starts a GET request to the Hub, with the token.
    fn get(
        &self,
        url: &str,
        headers: &[(&str, String)],
    ) -> Outcome<ureq::http::Response<ureq::Body>> {
        if self.offline {
            return Err(ThorError::Condition(format!(
                "HF_HUB_OFFLINE is set; {url} was not asked"
            )));
        }

        web::get(&self.client, url, self.token.as_deref(), headers)
    }

    /// The text of a Hub reply.
    fn get_text(&self, url: &str) -> Outcome<String> {
        if self.offline {
            return Err(ThorError::Condition(format!(
                "HF_HUB_OFFLINE is set; {url} was not asked"
            )));
        }

        web::get_text(&self.client, url, self.token.as_deref())
    }

    /// Searches for models in the engine's format.
    pub fn search(&self, engine: Engine, text: &str, limit: usize) -> Outcome<Vec<HubModel>> {
        let url = search_url(&self.endpoint, engine, text, limit);
        let reply = self.get_text(&url)?;

        parse_search(&reply)
    }

    /// One repository's search fields, for a name typed by hand.
    pub fn model(&self, repo: &str) -> Outcome<HubModel> {
        let url = format!(
            "{}/api/models/{repo}?expand[]=downloads&expand[]=tags&expand[]=baseModels&expand[]=gguf",
            self.endpoint
        );
        let reply = self.get_text(&url)?;

        serde_json::from_str(&reply).map_err(|source| ThorError::Json { what: url, source })
    }

    /// Every file in a repository, with sizes.
    pub fn files(&self, repo: &str) -> Outcome<Vec<RepoFile>> {
        let url = format!(
            "{}/api/models/{repo}/tree/main?recursive=true",
            self.endpoint
        );
        let reply = self.get_text(&url)?;

        parse_files(&reply)
    }

    /// A repository's `config.json`.
    pub fn config(&self, repo: &str) -> Outcome<Value> {
        let url = format!("{}/{repo}/resolve/main/config.json", self.endpoint);
        let reply = self.get_text(&url)?;

        serde_json::from_str(&reply).map_err(|source| ThorError::Json { what: url, source })
    }

    /// The page a person reads about a repository.
    pub fn page(&self, repo: &str) -> String {
        format!("{}/{repo}", self.endpoint)
    }

    /// Downloads one file into `folder`, keeping its path inside the repository. A file
    /// already there at full size is skipped; a partial `.part` file is resumed.
    pub fn download(&self, repo: &str, file: &RepoFile, folder: &Path) -> Outcome<PathBuf> {
        let target = folder.join(&file.path);
        let partial = PathBuf::from(format!("{}.part", target.display()));
        let finished = fs::metadata(&target).is_ok_and(|metadata| metadata.len() == file.size);

        if finished {
            return Ok(target);
        }

        let parent = target.parent().unwrap_or(folder);
        fs::create_dir_all(parent).map_err(file_error(parent))?;

        let already = fs::metadata(&partial).map_or(0, |metadata| metadata.len());
        let url = format!("{}/{repo}/resolve/main/{}", self.endpoint, file.path);
        let range = [("Range", format!("bytes={already}-"))];
        let mut response = self.get(&url, &range)?;
        let resumed = response.status().as_u16() == 206;

        let mut progress = Progress {
            output: open_partial(&partial, resumed)?,
            name: &file.path,
            written: if resumed { already } else { 0 },
            total: file.size,
        };

        std::io::copy(&mut response.body_mut().as_reader(), &mut progress)
            .map_err(file_error(&partial))?;
        eprintln!();

        fs::rename(&partial, &target).map_err(file_error(&target))?;

        Ok(target)
    }
}

/// Opens a `.part` file: appended to when the server resumes a download, emptied when it
/// sends the whole file again.
fn open_partial(partial: &Path, resumed: bool) -> Outcome<fs::File> {
    fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(true)
        .truncate(!resumed)
        .open(partial)
        .map_err(file_error(partial))
}

/// A file being written that prints how far it has got, every 256 MB.
struct Progress<'name> {
    /// The `.part` file.
    output: fs::File,
    /// The file's path in the repository, for the progress line.
    name: &'name str,
    /// Bytes in the file so far.
    written: u64,
    /// Bytes expected.
    total: u64,
}

impl Write for Progress<'_> {
    /// Writes to the file and redraws the progress line when a 256 MB step is crossed.
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        const STEP: u64 = 256 * 1024 * 1024;

        let count = self.output.write(buffer)?;
        let before = self.written / STEP;
        self.written += count as u64;

        if self.written / STEP != before || self.written == self.total {
            eprint!(
                "\r  {}  {} of {}",
                self.name,
                gigabytes(self.written),
                gigabytes(self.total)
            );
        }

        Ok(count)
    }

    /// Flushes the file.
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

/// The Hub token: `HF_TOKEN`, then `HUGGING_FACE_HUB_TOKEN`, then the token file under
/// `HF_HOME` or `~/.cache/huggingface`.
fn find_token(home: &Path) -> Option<String> {
    let hf_home = std::env::var_os("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache/huggingface"));

    std::env::var("HF_TOKEN")
        .ok()
        .or_else(|| std::env::var("HUGGING_FACE_HUB_TOKEN").ok())
        .or_else(|| fs::read_to_string(hf_home.join("token")).ok())
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}
