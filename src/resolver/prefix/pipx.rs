use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// pipx-installed applications view of one binary query.
///
/// `pipx list --short` prints "app (version)" per line, where app is the
/// exported command name. A hit needs the app listed and the expected bin
/// link under `~/.local/bin` (pipx's default bin dir, overridable via
/// `PIPX_BIN_DIR`); both must agree, so a listed app whose link is gone is
/// not reported.
pub struct PipxResolver;

impl Resolver for PipxResolver {
    fn name(&self) -> &'static str {
        "pipx"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("pipx").args(["list", "--short"]).output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("pipx not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("pipx could not be run: {e}"));
            }
        };
        if !out.status.success() {
            return SourceResult::unavailable(format!("pipx exited with {}", out.status));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let Some(version) = listed_version(&stdout, binary_name) else {
            return SourceResult::checked(Vec::new());
        };
        let bin_dir = bin_dir();
        let link = bin_dir.join(binary_name);
        if !is_file(&link) {
            return SourceResult::checked(Vec::new());
        }
        let mut e = ResolvedEntry::new(Source::Pipx);
        e.package_name = Some(binary_name.to_string());
        if !version.is_empty() {
            e.package_version = Some(version);
        }
        e.path = Some(link);
        SourceResult::checked(vec![e])
    }
}

/// Version pipx lists for `app`, from `pipx list --short` output. One line
/// lists one package: the app name or comma-separated app names, then the
/// version in parentheses. Returns an empty string when the app is listed
/// without a version, None when it is not listed at all.
pub fn listed_version(list_output: &str, app: &str) -> Option<String> {
    for line in list_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (names_part, version) = split_names_version(line);
        for name in names_part.split(',') {
            if name.trim() == app {
                return Some(version.clone());
            }
        }
    }
    None
}

/// (app names, version) from one `pipx list --short` line. The version is
/// the trailing parenthesized group; a line without one is a bare name with
/// no version.
fn split_names_version(line: &str) -> (&str, String) {
    if line.ends_with(')') {
        if let Some(i) = line.rfind('(') {
            return (&line[..i], line[i + 1..line.len() - 1].to_string());
        }
    }
    match line.split_once(' ') {
        Some((n, v)) => (n, v.to_string()),
        None => (line, String::new()),
    }
}

/// pipx's bin dir: `$PIPX_BIN_DIR`, else `~/.local/bin`.
fn bin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PIPX_BIN_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local").join("bin")
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_FIXTURE: &str = "\
black (25.9.0)
ruff, ruff-format (0.14.0)
cowsay (6.1)
";

    #[test]
    fn finds_version_by_app_name() {
        assert_eq!(listed_version(LIST_FIXTURE, "black"), Some("25.9.0".to_string()));
    }

    #[test]
    fn finds_app_in_comma_list() {
        assert_eq!(listed_version(LIST_FIXTURE, "ruff"), Some("0.14.0".to_string()));
        assert_eq!(listed_version(LIST_FIXTURE, "ruff-format"), Some("0.14.0".to_string()));
    }

    #[test]
    fn absent_app_yields_none() {
        assert!(listed_version(LIST_FIXTURE, "nosuch").is_none());
    }

    #[test]
    fn empty_output_yields_none() {
        assert!(listed_version("", "black").is_none());
    }
}
