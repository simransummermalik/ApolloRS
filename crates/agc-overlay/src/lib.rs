#![forbid(unsafe_code)]
//! Auditable compatibility overlays applied outside immutable historical files.

use agc_source::{Program, SourceId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Evidence attached to a compatibility decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// Reproducible observation or diagnostic.
    pub observation: String,
    /// Tool or corpus used to establish the observation.
    pub source: String,
}

/// One filename substitution used only while resolving an include.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IncludeAlias {
    /// Source spelling in the historical include.
    pub original: String,
    /// Existing source filename selected by the overlay.
    pub replacement: String,
    /// Why this substitution is required.
    pub rationale: String,
    /// Concrete evidence supporting the substitution.
    pub evidence: Vec<Evidence>,
}

/// One exact source-card replacement applied only in a writable staging tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceEdit {
    /// Source path relative to the historical program directory.
    pub path: String,
    /// One-based source line number anchoring the edit.
    pub line: usize,
    /// Exact historical line content, excluding its line ending.
    pub expected: String,
    /// Replacement line content, excluding its line ending.
    pub replacement: String,
    /// Why the compatibility edit is necessary.
    pub rationale: String,
    /// Concrete evidence supporting the edit.
    pub evidence: Vec<Evidence>,
}

/// Machine-readable, versioned compatibility overlay for one program.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    /// Overlay schema version.
    pub schema_version: u32,
    /// Program to which this overlay applies.
    pub program: Program,
    /// Historical source commit against which it was verified.
    pub historical_commit: String,
    /// Reference assembler and version/commit used as corroboration.
    pub reference_toolchain: String,
    /// Include substitutions.
    #[serde(default)]
    pub include_aliases: Vec<IncludeAlias>,
    /// Exact source-card substitutions made only in staging.
    #[serde(default)]
    pub source_edits: Vec<SourceEdit>,
}

/// Overlay loading, validation, and materialization failures.
#[derive(Debug, Error)]
pub enum OverlayError {
    /// Filesystem failure.
    #[error("overlay filesystem error at {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// JSON decoding failure.
    #[error("invalid overlay JSON at {path}: {source}")]
    Json {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: serde_json::Error,
    },
    /// Overlay violates an audit invariant.
    #[error("invalid overlay: {0}")]
    Invalid(String),
}

impl Overlay {
    /// Loads and validates an overlay document.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, OverlayError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| OverlayError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let overlay = serde_json::from_slice(&bytes).map_err(|source| OverlayError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        Self::validate(overlay)
    }

    /// Validates schema version, paths, uniqueness, and audit evidence.
    pub fn validate(overlay: Self) -> Result<Self, OverlayError> {
        if overlay.schema_version != 1 {
            return Err(OverlayError::Invalid(format!(
                "unsupported schema version {}",
                overlay.schema_version
            )));
        }
        if overlay.historical_commit.len() != 40
            || !overlay
                .historical_commit
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(OverlayError::Invalid(
                "historical_commit must be a full 40-character Git object ID".to_owned(),
            ));
        }
        if overlay.reference_toolchain.trim().is_empty() {
            return Err(OverlayError::Invalid(
                "reference_toolchain must be identified".to_owned(),
            ));
        }
        let mut originals = BTreeSet::new();
        for alias in &overlay.include_aliases {
            validate_relative_agc_path(&alias.original)?;
            validate_relative_agc_path(&alias.replacement)?;
            if alias.original == alias.replacement {
                return Err(OverlayError::Invalid(format!(
                    "alias {} does not change the include",
                    alias.original
                )));
            }
            if !originals.insert(alias.original.clone()) {
                return Err(OverlayError::Invalid(format!(
                    "duplicate include alias {}",
                    alias.original
                )));
            }
            if alias.rationale.trim().is_empty() || alias.evidence.is_empty() {
                return Err(OverlayError::Invalid(format!(
                    "alias {} lacks rationale or evidence",
                    alias.original
                )));
            }
            if alias
                .evidence
                .iter()
                .any(|item| item.observation.trim().is_empty() || item.source.trim().is_empty())
            {
                return Err(OverlayError::Invalid(format!(
                    "alias {} contains incomplete evidence",
                    alias.original
                )));
            }
        }
        let mut edited_lines = BTreeSet::new();
        for edit in &overlay.source_edits {
            validate_relative_agc_path(&edit.path)?;
            if edit.line == 0 {
                return Err(OverlayError::Invalid(format!(
                    "source edit {} has line zero",
                    edit.path
                )));
            }
            if edit.expected == edit.replacement
                || edit.expected.contains('\r')
                || edit.expected.contains('\n')
                || edit.replacement.contains('\r')
                || edit.replacement.contains('\n')
            {
                return Err(OverlayError::Invalid(format!(
                    "source edit {}:{} is empty or contains a line ending",
                    edit.path, edit.line
                )));
            }
            if edit.rationale.trim().is_empty()
                || edit.evidence.is_empty()
                || edit
                    .evidence
                    .iter()
                    .any(|item| item.observation.trim().is_empty() || item.source.trim().is_empty())
            {
                return Err(OverlayError::Invalid(format!(
                    "source edit {}:{} lacks rationale or evidence",
                    edit.path, edit.line
                )));
            }
            if !edited_lines.insert((&edit.path, edit.line)) {
                return Err(OverlayError::Invalid(format!(
                    "duplicate source edit {}:{}",
                    edit.path, edit.line
                )));
            }
        }
        Ok(overlay)
    }

    /// Resolves one include spelling, returning the original when no alias applies.
    pub fn resolve_include<'a>(&'a self, include: &'a str) -> &'a str {
        self.include_aliases
            .iter()
            .find(|alias| alias.original == include)
            .map_or(include, |alias| alias.replacement.as_str())
    }

    /// Returns substitutions as an ordered lookup table.
    pub fn alias_map(&self) -> BTreeMap<&str, &str> {
        self.include_aliases
            .iter()
            .map(|alias| (alias.original.as_str(), alias.replacement.as_str()))
            .collect()
    }

    /// Checks that every replacement exists and every original is absent.
    pub fn verify_against(&self, program_root: &Path) -> Result<(), OverlayError> {
        for alias in &self.include_aliases {
            let original = program_root.join(&alias.original);
            let replacement = program_root.join(&alias.replacement);
            if original.exists() {
                return Err(OverlayError::Invalid(format!(
                    "original include unexpectedly exists: {}",
                    original.display()
                )));
            }
            if !replacement.is_file() {
                return Err(OverlayError::Invalid(format!(
                    "replacement include does not exist: {}",
                    replacement.display()
                )));
            }
        }
        for path in self
            .source_edits
            .iter()
            .map(|edit| edit.path.as_str())
            .collect::<BTreeSet<_>>()
        {
            let source_path = program_root.join(path);
            let bytes = fs::read(&source_path).map_err(|source| OverlayError::Io {
                path: source_path.clone(),
                source,
            })?;
            self.apply_to_source(path, &bytes)?;
        }
        Ok(())
    }

    /// Applies exact source edits for one relative path in memory.
    ///
    /// The original byte slice is not modified. Existing LF or CRLF endings are
    /// retained, and any expected-text mismatch fails rather than drifting.
    pub fn apply_to_source(
        &self,
        relative_path: &str,
        source: &[u8],
    ) -> Result<Vec<u8>, OverlayError> {
        let edits = self
            .source_edits
            .iter()
            .filter(|edit| edit.path == relative_path)
            .map(|edit| (edit.line, edit))
            .collect::<BTreeMap<_, _>>();
        if edits.is_empty() {
            return Ok(source.to_vec());
        }
        let text = std::str::from_utf8(source).map_err(|error| {
            OverlayError::Invalid(format!(
                "source edit target {relative_path} is not UTF-8: {error}"
            ))
        })?;
        let mut output = String::with_capacity(text.len() + edits.len());
        let mut matched = BTreeSet::new();
        for (index, segment) in text.split_inclusive('\n').enumerate() {
            let line = index + 1;
            let (content, ending) = if let Some(content) = segment.strip_suffix("\r\n") {
                (content, "\r\n")
            } else if let Some(content) = segment.strip_suffix('\n') {
                (content, "\n")
            } else {
                (segment, "")
            };
            if let Some(edit) = edits.get(&line) {
                if content != edit.expected {
                    return Err(OverlayError::Invalid(format!(
                        "source edit {}:{} expected {:?}, found {:?}",
                        edit.path, edit.line, edit.expected, content
                    )));
                }
                output.push_str(&edit.replacement);
                matched.insert(line);
            } else {
                output.push_str(content);
            }
            output.push_str(ending);
        }
        if matched.len() != edits.len() {
            let Some(missing) = edits.keys().find(|line| !matched.contains(line)) else {
                return Err(OverlayError::Invalid(format!(
                    "source edit accounting failed for {relative_path}"
                )));
            };
            return Err(OverlayError::Invalid(format!(
                "source edit {relative_path}:{missing} lies beyond end of file"
            )));
        }
        Ok(output.into_bytes())
    }

    /// Copies a program to a writable staging tree and creates alias files there.
    ///
    /// Historical inputs are opened read-only. Alias files are byte-for-byte
    /// copies, avoiding symlink behavior differences in assemblers.
    pub fn materialize(&self, source_root: &Path, output_root: &Path) -> Result<(), OverlayError> {
        if output_root.exists() {
            return Err(OverlayError::Invalid(format!(
                "materialization destination already exists: {}",
                output_root.display()
            )));
        }
        self.verify_against(source_root)?;
        copy_tree(source_root, output_root)?;
        for alias in &self.include_aliases {
            let replacement = output_root.join(&alias.replacement);
            let original = output_root.join(&alias.original);
            if let Some(parent) = original.parent() {
                fs::create_dir_all(parent).map_err(|source| OverlayError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::copy(&replacement, &original).map_err(|source| OverlayError::Io {
                path: original,
                source,
            })?;
        }
        for path in self
            .source_edits
            .iter()
            .map(|edit| edit.path.as_str())
            .collect::<BTreeSet<_>>()
        {
            let output = output_root.join(path);
            let bytes = fs::read(&output).map_err(|source| OverlayError::Io {
                path: output.clone(),
                source,
            })?;
            let edited = self.apply_to_source(path, &bytes)?;
            fs::write(&output, edited).map_err(|source| OverlayError::Io {
                path: output,
                source,
            })?;
        }
        Ok(())
    }
}

/// Resolves an include path relative to its containing source without escaping
/// the program root.
pub fn resolve_relative_include(
    containing: &SourceId,
    include: &str,
) -> Result<String, OverlayError> {
    validate_relative_agc_path(include)?;
    let parent = Path::new(&containing.relative_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let joined = parent.join(include);
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(OverlayError::Invalid(format!(
                        "include escapes program root: {include}"
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(OverlayError::Invalid(format!(
                    "include is not relative: {include}"
                )));
            }
        }
    }
    Ok(normalized
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn validate_relative_agc_path(path: &str) -> Result<(), OverlayError> {
    let candidate = Path::new(path);
    if path.trim().is_empty()
        || candidate.is_absolute()
        || candidate
            .extension()
            .is_none_or(|extension| extension != "agc")
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(OverlayError::Invalid(format!(
            "unsafe or non-AGC include path: {path}"
        )));
    }
    Ok(())
}

fn copy_tree(source_root: &Path, output_root: &Path) -> Result<(), OverlayError> {
    fs::create_dir(output_root).map_err(|source| OverlayError::Io {
        path: output_root.to_path_buf(),
        source,
    })?;
    let mut entries = fs::read_dir(source_root)
        .map_err(|source| OverlayError::Io {
            path: source_root.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| OverlayError::Io {
            path: source_root.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let source = entry.path();
        let output = output_root.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| OverlayError::Io {
            path: source.clone(),
            source: error,
        })?;
        if file_type.is_dir() {
            copy_tree(&source, &output)?;
        } else if file_type.is_file() {
            fs::copy(&source, &output).map_err(|error| OverlayError::Io {
                path: output,
                source: error,
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_resolution_cannot_escape_program() {
        let source = SourceId::new(Program::Luminary099, "A/B.agc");
        assert_eq!(
            resolve_relative_include(&source, "C.agc").unwrap(),
            "A/C.agc"
        );
        assert!(resolve_relative_include(&source, "../../C.agc").is_err());
    }

    #[test]
    fn source_edits_are_exact_and_preserve_line_endings() {
        let overlay = Overlay {
            schema_version: 1,
            program: Program::Comanche055,
            historical_commit: "0".repeat(40),
            reference_toolchain: "test".to_owned(),
            include_aliases: Vec::new(),
            source_edits: vec![SourceEdit {
                path: "TEST.agc".to_owned(),
                line: 2,
                expected: "CARD".to_owned(),
                replacement: " CARD".to_owned(),
                rationale: "column restoration".to_owned(),
                evidence: vec![Evidence {
                    observation: "assembler result".to_owned(),
                    source: "test".to_owned(),
                }],
            }],
        };
        assert_eq!(
            overlay
                .apply_to_source("TEST.agc", b"HEAD\r\nCARD\r\n")
                .unwrap(),
            b"HEAD\r\n CARD\r\n"
        );
        assert!(
            overlay
                .apply_to_source("TEST.agc", b"HEAD\nWRONG\n")
                .is_err()
        );
    }
}
