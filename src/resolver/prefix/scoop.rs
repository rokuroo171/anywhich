use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Scoop (Windows) view of one binary query.
///
/// Scoop puts a shim per exported command in `<scoop>\shims` (which is what
/// lands on PATH) and keeps each app's install under `<scoop>\apps\<name>\
/// current`, a junction to the versioned directory. A hit needs both: the
/// shim for the binary name and the `apps/<name>/current` record it points
/// at. The install root is `$SCOOP`, else `%USERPROFILE%\scoop`, the same
/// precedence Scoop itself uses.
///
/// Not live-verifiable from Linux; tests are fixtures only.
pub struct ScoopResolver;

impl Resolver for ScoopResolver {
    fn name(&self) -> &'static str {
        "scoop"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let Some(root) = scoop_root() else {
            return SourceResult::checked(Vec::new());
        };
        let shim = root.join("shims").join(format!("{binary_name}.exe"));
        if !is_file(&shim) {
            return SourceResult::checked(Vec::new());
        }
        let mut e = ResolvedEntry::stub_only(Source::Scoop, &shim);
        if let Some((name, version)) = current_app(&root, binary_name) {
            e.package_name = Some(name);
            e.package_version = Some(version);
        }
        SourceResult::checked(vec![e])
    }
}

/// Scoop install root: `$SCOOP`, else `$USERPROFILE/scoop`, when the
/// directory exists.
fn scoop_root() -> Option<PathBuf> {
    for var in ["SCOOP", "USERPROFILE"] {
        if let Ok(base) = std::env::var(var) {
            if base.is_empty() {
                continue;
            }
            let root = PathBuf::from(base);
            if is_dir(&root) {
                return Some(root);
            }
        }
    }
    None
}

/// (app name, version) from the `apps/<name>/current` directory for a shim
/// named `binary_name`. Scoop shims are named after the command, and the
/// `current` junction's target directory carries the version.
fn current_app(root: &Path, binary_name: &str) -> Option<(String, String)> {
    let name = fs::read_link(root.join("apps").join(binary_name).join("current"))
        .ok()
        .or_else(|| {
            let dir = root.join("apps").join(binary_name).join("current");
            if is_dir(&dir) {
                Some(dir)
            } else {
                None
            }
        })?;
    let target = fs::canonicalize(&name).unwrap_or(name);
    let version = target
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if version.is_empty() {
        return None;
    }
    Some((binary_name.to_string(), version))
}

fn is_dir(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_app_reads_version_from_junction_target() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-ver");
        let _ = fs::remove_dir_all(&tmp);
        let app_current = tmp.join("apps").join("tool").join("current");
        let versioned = tmp.join("apps").join("tool").join("1.9.0");
        fs::create_dir_all(&versioned).unwrap();
        if !make_link(&versioned, &app_current) {
            eprintln!("skipped: no link support in this sandbox");
            return;
        }
        let (name, version) = current_app(&tmp, "tool").unwrap();
        assert_eq!(name, "tool");
        assert_eq!(version, "1.9.0");
        fs::remove_dir_all(&tmp).unwrap();
    }

    /// A junction (what scoop actually creates) needs no privilege on
    /// Windows; a symlink needs Developer Mode or admin. Either serves the
    /// read path under test.
    #[cfg(windows)]
    fn make_link(target: &Path, link: &Path) -> bool {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        let status = std::process::Command::new("cmd")
            .args([
            "/c",
            "mklink",
            "/J",
            &link.to_string_lossy(),
            &target.to_string_lossy(),
        ])
            .status();
        status.map(|s| s.success()).unwrap_or(false)
    }

    #[cfg(unix)]
    fn make_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[test]
    fn current_app_none_for_missing_app() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-missing");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        assert!(current_app(&tmp, "tool").is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
