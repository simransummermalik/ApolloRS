#![forbid(unsafe_code)]
//! Discovery and byte-integrity checking for the immutable historical corpus.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Supported primary Apollo flight-software programs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Program {
    /// Command Module Colossus 2A, Comanche revision 055.
    Comanche055,
    /// Lunar Module Luminary revision 099.
    Luminary099,
}

impl Program {
    /// Directory name in the historical repository.
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Comanche055 => "Comanche055",
            Self::Luminary099 => "Luminary099",
        }
    }

    /// Human-readable mission-computer role.
    pub const fn role(self) -> &'static str {
        match self {
            Self::Comanche055 => "Apollo 11 Command Module Guidance Computer",
            Self::Luminary099 => "Apollo 11 Lunar Module Guidance Computer",
        }
    }

    /// Parses a stable CLI/program identifier.
    pub fn parse(input: &str) -> Option<Self> {
        match input.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
            "comanche055" | "comanche" | "cm" => Some(Self::Comanche055),
            "luminary099" | "luminary" | "lm" => Some(Self::Luminary099),
            _ => None,
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.directory())
    }
}

/// Stable identity for a source file within one program.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceId {
    /// Program owning this source.
    pub program: Program,
    /// Slash-normalized path relative to the program directory.
    pub relative_path: String,
}

impl SourceId {
    /// Creates a normalized source identity.
    pub fn new(program: Program, relative_path: impl AsRef<Path>) -> Self {
        Self {
            program,
            relative_path: slash_path(relative_path.as_ref()),
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.program, self.relative_path)
    }
}

/// Hash and basic shape of one historical source file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Stable source identity.
    pub source: SourceId,
    /// Byte length.
    pub bytes: u64,
    /// Number of logical lines, including a final unterminated line.
    pub lines: u64,
    /// Lowercase SHA-256 digest.
    pub sha256: String,
}

/// Complete, deterministically ordered corpus manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceManifest {
    /// Git commit of the historical submodule, when available.
    pub historical_commit: Option<String>,
    /// Every `.agc` source in the two primary programs.
    pub entries: Vec<ManifestEntry>,
}

/// A loaded source file with byte-preserving text content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFile {
    /// Stable source identity.
    pub id: SourceId,
    /// On-disk path.
    pub path: PathBuf,
    /// Exact file bytes.
    pub bytes: Vec<u8>,
}

impl SourceFile {
    /// Returns UTF-8 text without normalizing line endings.
    pub fn text(&self) -> Result<&str, SourceError> {
        std::str::from_utf8(&self.bytes).map_err(|source| SourceError::Utf8 {
            path: self.path.clone(),
            source,
        })
    }
}

/// Source discovery and integrity failures.
#[derive(Debug, Error)]
pub enum SourceError {
    /// A filesystem operation failed.
    #[error("source filesystem error at {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// A historical file is not valid UTF-8.
    #[error("historical source is not UTF-8 at {path}: {source}")]
    Utf8 {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::str::Utf8Error,
    },
    /// Current bytes do not match the expected manifest.
    #[error("historical source integrity mismatch: {0}")]
    Integrity(String),
}

/// Read-only view over a checked-out historical Apollo-11 repository.
#[derive(Clone, Debug)]
pub struct HistoricalCorpus {
    root: PathBuf,
}

impl HistoricalCorpus {
    /// Creates a corpus rooted at the historical repository checkout.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the checkout root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the absolute directory for a program.
    pub fn program_root(&self, program: Program) -> PathBuf {
        self.root.join(program.directory())
    }

    /// Discovers all primary `.agc` files in deterministic order.
    pub fn discover(&self) -> Result<Vec<SourceFile>, SourceError> {
        let mut files = Vec::new();
        for program in [Program::Comanche055, Program::Luminary099] {
            let program_root = self.program_root(program);
            walk_agc(&program_root, &program_root, program, &mut files)?;
        }
        files.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(files)
    }

    /// Computes a fresh manifest from exact bytes.
    pub fn manifest(&self) -> Result<SourceManifest, SourceError> {
        let entries = self
            .discover()?
            .into_iter()
            .map(|file| ManifestEntry {
                bytes: file.bytes.len() as u64,
                lines: logical_line_count(&file.bytes),
                sha256: hex::encode(Sha256::digest(&file.bytes)),
                source: file.id,
            })
            .collect();
        Ok(SourceManifest {
            historical_commit: git_commit(&self.root),
            entries,
        })
    }

    /// Reads one source by stable identity.
    pub fn read(&self, id: &SourceId) -> Result<SourceFile, SourceError> {
        let path = self.program_root(id.program).join(&id.relative_path);
        let bytes = fs::read(&path).map_err(|source| SourceError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(SourceFile {
            id: id.clone(),
            path,
            bytes,
        })
    }

    /// Verifies exact file identities, sizes, line counts, hashes, and commit.
    pub fn verify(&self, expected: &SourceManifest) -> Result<(), SourceError> {
        let actual = self.manifest()?;
        if &actual == expected {
            Ok(())
        } else {
            Err(SourceError::Integrity(manifest_difference(
                expected, &actual,
            )))
        }
    }
}

fn walk_agc(
    directory: &Path,
    program_root: &Path,
    program: Program,
    output: &mut Vec<SourceFile>,
) -> Result<(), SourceError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| SourceError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| SourceError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| SourceError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            walk_agc(&path, program_root, program, output)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "agc") {
            let relative = path
                .strip_prefix(program_root)
                .expect("walked path must remain inside program root");
            let bytes = fs::read(&path).map_err(|source| SourceError::Io {
                path: path.clone(),
                source,
            })?;
            output.push(SourceFile {
                id: SourceId::new(program, relative),
                path,
                bytes,
            });
        }
    }
    Ok(())
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn logical_line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        0
    } else {
        bytes.iter().filter(|&&byte| byte == b'\n').count() as u64
            + u64::from(bytes.last() != Some(&b'\n'))
    }
}

fn git_commit(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", root.to_str()?, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn manifest_difference(expected: &SourceManifest, actual: &SourceManifest) -> String {
    if expected.historical_commit != actual.historical_commit {
        return format!(
            "commit expected {:?}, found {:?}",
            expected.historical_commit, actual.historical_commit
        );
    }
    let count = expected.entries.len().min(actual.entries.len());
    for index in 0..count {
        if expected.entries[index] != actual.entries[index] {
            return format!(
                "entry {index} expected {:?}, found {:?}",
                expected.entries[index], actual.entries[index]
            );
        }
    }
    format!(
        "file count expected {}, found {}",
        expected.entries.len(),
        actual.entries.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_aliases_are_explicit() {
        assert_eq!(Program::parse("LM"), Some(Program::Luminary099));
        assert_eq!(Program::parse("comanche-055"), Some(Program::Comanche055));
        assert_eq!(Program::parse("unknown"), None);
    }

    #[test]
    fn counts_unterminated_last_line() {
        assert_eq!(logical_line_count(b"a\nb"), 2);
        assert_eq!(logical_line_count(b"a\n"), 1);
        assert_eq!(logical_line_count(b""), 0);
    }
}
