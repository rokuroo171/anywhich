use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Chocolatey (Windows) view of one binary query.
///
/// Chocolatey puts shims in its `bin` directory (on PATH by default) and
/// installs packages under `lib/<name>-<version>` (or `lib/<name>` with a
/// sidecar `.version` file for packages that opted out of versioned dirs).
/// A hit needs the shim for the binary name; the package and version are
/// attached when the lib directory can be matched.
///
/// The install root is `$CHOCOLATEY_INSTALL`, else the default
/// `C:\ProgramData\chocolatey`, the same precedence Chocolatey uses.
/// Not live-verifiable from Linux; tests are fixtures only.
pub struct ChocolateyResolver;

impl Resolver for ChocolateyResolver {
    fn name(&self) -> &'static str {
        "chocolatey"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let Some(root) = choco_root() else {
            return SourceResult::checked(Vec::new());
        };
        let shim = root.join("bin").join(format!("{binary_name}.exe"));
        if !is_file(&shim) {
            return SourceResult::checked(Vec::new());
        }
        let mut e = ResolvedEntry::stub_only(Source::Chocolatey, &shim);
        if let Some((name, version)) = owning_package(&root, binary_name) {
            e.package_name = Some(name);
            e.package_version = Some(version);
        }
        SourceResult::checked(vec![e])
    }
}

/// Chocolatey install root: `$CHOCOLATEY_INSTALL`, else the default
/// `C:/ProgramData/chocolatey`, when the directory exists.
fn choco_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("CHOCOLATEY_INSTALL") {
        if !root.is_empty() && is_dir(Path::new(&root)) {
            return Some(PathBuf::from(root));
        }
    }
    let default = PathBuf::from("C:/ProgramData/chocolatey");
    if is_dir(&default) {
        return Some(default);
    }
    None
}

/// (package name, version) from Chocolatey's lib directory for a shim named
/// `binary_name`. Shims are named after the command, so the lib package is
/// matched by exact directory name first, then by `<name>-<version>` prefix.
fn owning_package(root: &Path, binary_name: &str) -> Option<(String, String)> {
    let lib = root.join("lib");
    let exact = lib.join(binary_name);
    if is_dir(&exact) {
        if let Some(v) = sidecar_version(&exact) {
            return Some((binary_name.to_string(), v));
        }
    }
    let entries = fs::read_dir(&lib).ok()?;
    let prefix = format!("{binary_name}-");
    let mut best: Option<(String, String)> = None;
    for entry in entries.flatten() {
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = dir_name.strip_prefix(&prefix) else {
            continue;
        };
        if rest.is_empty() || !rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let version = sidecar_version(&dir).unwrap_or_else(|| rest.to_string());
        let better = match &best {
            Some((_, v)) => version_gt(&version, v),
            None => true,
        };
        if better {
            best = Some((binary_name.to_string(), version));
        }
    }
    best
}

/// Version from a `<pkg>.version` sidecar file, Chocolatey's record for
/// unversioned lib directories.
fn sidecar_version(pkg_dir: &Path) -> Option<String> {
    let pkg_name = pkg_dir.file_name()?.to_string_lossy().to_string();
    let sidecar = pkg_dir.join(format!("{pkg_name}.version"));
    let text = fs::read_to_string(sidecar).ok()?;
    let version = text.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// True when a > b, comparing dot-separated numeric segments numerically,
/// falling back to string order for non-numeric tails (a pragmatic order,
/// only used to pick which installed version to report).
fn version_gt(a: &str, b: &str) -> bool {
    let mut ia = a.split('.');
    let mut ib = b.split('.');
    loop {
        match (ia.next(), ib.next()) {
            (Some(x), Some(y)) => {
                let (na, nb) = (x.parse::<u64>(), y.parse::<u64>());
                match (na, nb) {
                    (Ok(xa), Ok(yb)) => {
                        if xa != yb {
                            return xa > yb;
                        }
                    }
                    _ => return a > b,
                }
            }
            (Some(_), None) => return true,
            (None, Some(_)) => return false,
            (None, None) => return false,
        }
    }
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
    fn version_gt_compares_numerically() {
        assert!(version_gt("1.10.0", "1.9.0"));
        assert!(!version_gt("1.9.0", "1.10.0"));
        assert!(version_gt("2.0", "1.9.9"));
        assert!(!version_gt("1.2", "1.2"));
    }

    #[test]
    fn version_gt_handles_uneven_lengths() {
        assert!(version_gt("1.2", "1.2.0") == false);
        assert!(version_gt("1.2.1", "1.2"));
    }

    #[test]
    fn sidecar_version_reads_the_record_file() {
        let tmp = std::env::temp_dir().join("anywhich-choco-sidecar");
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("tool");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("tool.version"), "3.2.1\n").unwrap();
        assert_eq!(sidecar_version(&pkg), Some("3.2.1".to_string()));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn owning_package_matches_versioned_lib_dir() {
        let tmp = std::env::temp_dir().join("anywhich-choco-lib");
        let _ = fs::remove_dir_all(&tmp);
        let lib = tmp.join("lib");
        fs::create_dir_all(lib.join("tool-1.4.0")).unwrap();
        fs::create_dir_all(lib.join("tool-1.10.0")).unwrap();
        let (name, version) = owning_package(&tmp, "tool").unwrap();
        assert_eq!(name, "tool");
        assert_eq!(version, "1.10.0");
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn owning_package_none_when_lib_has_no_match() {
        let tmp = std::env::temp_dir().join("anywhich-choco-none");
        let _ = fs::remove_dir_all(&tmp);
        let lib = tmp.join("lib");
        fs::create_dir_all(lib.join("other-1.0.0")).unwrap();
        assert!(owning_package(&tmp, "tool").is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
