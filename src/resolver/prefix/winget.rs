use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// WinGet portable packages (Windows) view of one binary query.
///
/// Portable installs land a symlink per exported command in
/// `%LOCALAPPDATA%\Microsoft\WinGet\Links` (the directory winget puts on
/// PATH) and the package contents under
/// `%LOCALAPPDATA%\Microsoft\WinGet\Packages\<Publisher.Id_Source_8wekyb3d8bbwe>`.
/// The link target yields the package id, so the resolver walks files only,
/// no `winget` spawn.
///
/// No version: the `<package>.db` record is SQLite with no readable version
/// string, and a `winget list` spawn costs seconds on every query.
///
/// The links root is `$WINGET_LINKS`, else the LOCALAPPDATA default. Known
/// limit: packages installed with their directory directly on PATH and no
/// link (ripgrep does this) are not attributable here.
pub struct WingetResolver;

impl Resolver for WingetResolver {
    fn name(&self) -> &'static str {
        "winget"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let Some(links) = links_root() else {
            return SourceResult::checked(Vec::new());
        };
        for spelling in [binary_name.to_string(), format!("{binary_name}.exe")] {
            let link = links.join(&spelling);
            if !is_file(&link) {
                continue;
            }
            let Some(id) = package_info(&link) else {
                return SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Winget, &link)]);
            };
            let mut e = ResolvedEntry::new(Source::Winget);
            e.package_name = Some(id);
            e.path = Some(link);
            return SourceResult::checked(vec![e]);
        }
        SourceResult::checked(Vec::new())
    }
}

/// WinGet links root: `$WINGET_LINKS` when set and a directory, else
/// `%LOCALAPPDATA%\Microsoft\WinGet\Links`.
fn links_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("WINGET_LINKS") {
        if !dir.is_empty() && is_dir(Path::new(&dir)) {
            return Some(PathBuf::from(dir));
        }
    }
    let base = std::env::var("LOCALAPPDATA").ok()?;
    if base.is_empty() {
        return None;
    }
    let links = PathBuf::from(base).join("Microsoft").join("WinGet").join("Links");
    if is_dir(&links) {
        Some(links)
    } else {
        None
    }
}

/// Package id for a WinGet Links shim, resolved through the link to
/// `Packages/<Publisher.Id_Source_8wekyb3d8bbwe>`.
fn package_info(link: &Path) -> Option<String> {
    let target = fs::read_link(link)
        .ok()
        .or_else(|| is_file(link).then(|| link.to_path_buf()))?;
    let resolved = fs::canonicalize(&target).unwrap_or(target);
    winget_package_id(&resolved)
}

/// Package id from a path inside `Packages/<Publisher.Id_Source_8wekyb3d8bbwe>/`:
/// the segment before the source and the source hash (winget ids use dots,
/// so the first underscore-delimited segment is the whole id).
fn winget_package_id(path: &Path) -> Option<String> {
    let mut comps = path.components();
    while let Some(c) = comps.next() {
        if c.as_os_str() != "Packages" {
            continue;
        }
        let dir = comps.next()?.as_os_str().to_string_lossy().to_string();
        let mut parts = dir.split('_');
        let id = parts.next()?;
        if id.is_empty() || parts.count() < 2 {
            return None;
        }
        return Some(id.to_string());
    }
    None
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

    const ID: &str = "Example.Tool_Microsoft.Winget.Source_8wekyb3d8bbwe";

    fn temp(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&tmp);
        tmp
    }

    /// A links root with one shim, symlinked where the sandbox allows it.
    /// Returns false where links need privileges the sandbox lacks.
    fn link_shim(tmp: &Path, name: &str, target: &Path) -> bool {
        let links = tmp.join("Links");
        fs::create_dir_all(&links).unwrap();
        let link = links.join(name);
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, &link).is_ok();
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(target, &link).is_ok() {
                return true;
            }
            let status = std::process::Command::new("cmd")
                .args(["/c", "mklink", &link.to_string_lossy(), &target.to_string_lossy()])
                .status();
            status.map(|s| s.success()).unwrap_or(false)
        }
    }

    fn package_tree(tmp: &Path) -> PathBuf {
        let pkg = tmp.join("Packages").join(ID);
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("tool.exe"), b"mz").unwrap();
        pkg.join("tool.exe")
    }

    #[test]
    fn id_is_parsed_from_the_packages_dir_shape() {
        let pkg = PathBuf::from(format!(
            "/l/Microsoft/WinGet/Packages/{ID}/sub/tool.exe"
        ));
        assert_eq!(
            winget_package_id(&pkg).as_deref(),
            Some("Example.Tool")
        );
        assert_eq!(winget_package_id(Path::new("/l/tool.exe")), None);
        assert_eq!(winget_package_id(Path::new("/l/Packages/short_8wekyb3d8bbwe/tool.exe")), None);
    }

    #[test]
    fn link_resolves_to_the_package_id() {
        let tmp = temp("anywhich-winget-link");
        let target = package_tree(&tmp);
        if !link_shim(&tmp, "tool.exe", &target) {
            eprintln!("skipped: no link support in this sandbox");
            return;
        }
        assert_eq!(
            package_info(&tmp.join("Links").join("tool.exe")).as_deref(),
            Some("Example.Tool")
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn resolve_reports_id_and_link_path() {
        let tmp = temp("anywhich-winget-resolve");
        let target = package_tree(&tmp);
        if !link_shim(&tmp, "tool.exe", &target) {
            eprintln!("skipped: no link support in this sandbox");
            return;
        }

        let resolver_root = tmp.join("Links").to_string_lossy().to_string();
        // SAFETY: single-threaded test binary scope; no other test reads
        // WINGET_LINKS.
        unsafe { std::env::set_var("WINGET_LINKS", &resolver_root) };
        let result = WingetResolver.resolve("tool");
        unsafe { std::env::remove_var("WINGET_LINKS") };

        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].package_name.as_deref(),
            Some("Example.Tool")
        );
        assert_eq!(result.entries[0].package_version, None);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_link_is_checked_empty() {
        let tmp = temp("anywhich-winget-miss");
        fs::create_dir_all(tmp.join("Links")).unwrap();
        let resolver_root = tmp.to_string_lossy().to_string();
        unsafe { std::env::set_var("WINGET_LINKS", &resolver_root) };
        let result = WingetResolver.resolve("tool");
        unsafe { std::env::remove_var("WINGET_LINKS") };
        assert!(result.entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
