use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const CHUNK_SIZE: usize = 1024 * 1024;

pub struct ModelFile {
    pub name: &'static str,
    pub url_path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// `base_urls[0]` is the primary source; the rest are mirrors, tried in
    /// order. A file's URL is `base + url_path`.
    pub base_urls: &'static [&'static str],
    pub files: &'static [ModelFile],
}

impl ModelSpec {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Missing,
    Partial {
        downloaded: u64,
        total: u64,
    },
    /// Every file has its expected size and a `.verified` marker, written only
    /// after its SHA-256 matched.
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download cancelled")]
    Cancelled,
    #[error("network error: {0}")]
    Network(String),
    #[error("checksum mismatch for {file}")]
    Checksum { file: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// `<data dir>/dev.gespan.shared/models`, shared by every Gespan app.
pub fn default_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("dev.gespan.shared")
        .join("models")
}

pub struct ModelStore {
    root: PathBuf,
    chunk_size: usize,
}

impl ModelStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            chunk_size: CHUNK_SIZE,
        }
    }

    /// Smaller read chunks, so tests can stop "after the first chunk" on small bodies.
    #[doc(hidden)]
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1);
        self
    }

    pub fn dir(&self, spec: &ModelSpec) -> PathBuf {
        self.root.join(spec.id)
    }

    pub fn status(&self, spec: &ModelSpec) -> ModelStatus {
        let dir = self.dir(spec);
        let (mut downloaded, mut ready) = (0, 0);
        for file in spec.files {
            let final_len = file_len(&dir.join(file.name));
            if final_len == Some(file.size) && dir.join(marker_name(file)).exists() {
                ready += 1;
                downloaded += file.size;
            } else {
                let len = final_len.or_else(|| file_len(&dir.join(part_name(file))));
                downloaded += len.unwrap_or(0).min(file.size);
            }
        }
        if ready == spec.files.len() {
            ModelStatus::Ready
        } else if downloaded == 0 {
            ModelStatus::Missing
        } else {
            ModelStatus::Partial {
                downloaded,
                total: spec.total_size(),
            }
        }
    }

    pub fn delete(&self, spec: &ModelSpec) -> io::Result<()> {
        match fs::remove_dir_all(self.dir(spec)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Downloads whatever is missing. Safe to call again after a failure: each
    /// file resumes from its `.part`, and verified files are left alone.
    pub fn download(
        &self,
        spec: &ModelSpec,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(Progress),
    ) -> Result<(), DownloadError> {
        let dir = self.dir(spec);
        fs::create_dir_all(&dir)?;
        let mut reporter = Reporter {
            total: spec.total_size(),
            done: 0,
            high_water: 0,
            sink: &mut on_progress,
        };
        for file in spec.files {
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            self.fetch_file(spec, file, &dir, cancel, &mut reporter)?;
            reporter.done += file.size;
            reporter.emit(0);
        }
        Ok(())
    }

    fn fetch_file(
        &self,
        spec: &ModelSpec,
        file: &ModelFile,
        dir: &Path,
        cancel: &CancelToken,
        reporter: &mut Reporter<'_>,
    ) -> Result<(), DownloadError> {
        let final_path = dir.join(file.name);
        let marker = dir.join(marker_name(file));
        let part = dir.join(part_name(file));

        if final_path.exists() {
            if file_len(&final_path) == Some(file.size) && marker.exists() {
                return Ok(());
            }
            // Right name but unproven: verify in place, else start over.
            if file_len(&final_path) == Some(file.size) && hash_matches(&final_path, file)? {
                File::create(&marker)?;
                return Ok(());
            }
            fs::remove_file(&final_path)?;
        }

        // A corrupt download is retried once from zero.
        for attempt in 0..2 {
            let result = self.fill_part(spec, file, &part, cancel, reporter);
            if matches!(result, Err(DownloadError::Cancelled)) {
                let _ = fs::remove_file(&part);
            }
            result?;
            if hash_matches(&part, file)? {
                fs::rename(&part, &final_path)?;
                File::create(&marker)?;
                return Ok(());
            }
            fs::remove_file(&part)?;
            if attempt == 1 {
                break;
            }
        }
        Err(DownloadError::Checksum {
            file: file.name.to_string(),
        })
    }

    /// Streams the file into `part`, trying each base URL in turn. A network
    /// failure keeps what was written, and the next source resumes from there.
    fn fill_part(
        &self,
        spec: &ModelSpec,
        file: &ModelFile,
        part: &Path,
        cancel: &CancelToken,
        reporter: &mut Reporter<'_>,
    ) -> Result<(), DownloadError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .build();
        let mut last_error = String::from("no download sources");
        for base in spec.base_urls {
            let offset = match file_len(part) {
                Some(len) if len > file.size => {
                    fs::remove_file(part)?;
                    0
                }
                Some(len) => len,
                None => 0,
            };
            if offset == file.size {
                return Ok(());
            }
            let url = format!("{base}{}", file.url_path);
            match self.stream(&agent, &url, file, part, offset, cancel, reporter) {
                Ok(()) => return Ok(()),
                Err(DownloadError::Network(e)) => last_error = format!("{url}: {e}"),
                Err(e) => return Err(e),
            }
        }
        Err(DownloadError::Network(last_error))
    }

    #[allow(clippy::too_many_arguments)]
    fn stream(
        &self,
        agent: &ureq::Agent,
        url: &str,
        file: &ModelFile,
        part: &Path,
        offset: u64,
        cancel: &CancelToken,
        reporter: &mut Reporter<'_>,
    ) -> Result<(), DownloadError> {
        let mut request = agent.get(url);
        if offset > 0 {
            request = request.set("Range", &format!("bytes={offset}-"));
        }
        let response = request
            .call()
            .map_err(|e| DownloadError::Network(e.to_string()))?;
        // A server that ignores Range answers 200 with the whole body.
        let resumed = offset > 0 && response.status() == 206;
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(part)?;
        let mut written = if resumed { offset } else { 0 };
        let mut reader = response.into_reader();
        let mut buf = vec![0u8; self.chunk_size];
        loop {
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(DownloadError::Network(e.to_string())),
            };
            out.write_all(&buf[..n])?;
            written += n as u64;
            if written > file.size {
                return Err(DownloadError::Network(format!(
                    "{url}: longer than the expected {} bytes",
                    file.size
                )));
            }
            reporter.emit(written);
        }
        out.flush()?;
        if written < file.size {
            return Err(DownloadError::Network(format!(
                "{url}: ended after {written} of {} bytes",
                file.size
            )));
        }
        Ok(())
    }
}

/// Reports a monotonic running total across files.
struct Reporter<'a> {
    total: u64,
    /// Bytes of fully finished files.
    done: u64,
    high_water: u64,
    sink: &'a mut dyn FnMut(Progress),
}

impl Reporter<'_> {
    fn emit(&mut self, current_file: u64) {
        let downloaded = self.done + current_file;
        if downloaded >= self.high_water {
            self.high_water = downloaded;
            (self.sink)(Progress {
                downloaded,
                total: self.total,
            });
        }
    }
}

fn part_name(file: &ModelFile) -> String {
    format!("{}.part", file.name)
}

fn marker_name(file: &ModelFile) -> String {
    format!("{}.verified", file.name)
}

fn file_len(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|m| m.len())
}

fn hash_matches(path: &Path, file: &ModelFile) -> io::Result<bool> {
    let mut reader = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(hex.eq_ignore_ascii_case(file.sha256))
}
