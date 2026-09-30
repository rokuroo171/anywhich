use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Scoop (Windows) view of one binary query.
///
/// Scoop puts a shim per exported command in `<scoop>\shims` (which is what
/// lands on PATH) and keeps each app's install under `<scoop>\apps\<name>\
/// current`, a junction to the versioned directory. Shims are `.exe` when
/// the command is an executable and `.cmd` script wrappers otherwise
/// (observed on scoop 0.5.3: script apps get .cmd), so both spellings are
/// probed, .exe first because PATHEXT would prefer it anyway. A hit needs
/// the shim for the binary name; the package and version are attached when
/// the `apps/<name>/current` record can be matched.
///
/// Two install roots, checked in Scoop's own order: the user root (`$SCOOP`
/// verbatim, else `%USERPROFILE%\scoop`) first, then the machine-wide root
/// for `scoop install -g` (`$SCOOP_GLOBAL` verbatim, else the scoop default
/// `C:\ProgramData\scoop`). User shims outrank global ones, matching shim
/// PATH order on a real machine, and each root's `apps` record is matched
/// against its own shim.
pub struct ScoopResolver;

impl Resolver for ScoopResolver {
    fn name(&self) -> &'static str {
        "scoop"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        self.resolve_in(&scoop_roots(), binary_name)
    }
}

impl ScoopResolver {
    /// resolve() with the roots injected, so the precedence rules are
    /// fixture-testable without touching the process environment.
    fn resolve_in(&self, roots: &[PathBuf], binary_name: &str) -> SourceResult {
        for root in roots {
            let Some(shim) = shim_in(root, binary_name) else {
                continue;
            };
            let mut e = ResolvedEntry::stub_only(Source::Scoop, &shim);
            if let Some((name, version)) = current_app(root, binary_name) {
                e.package_name = Some(name);
                e.package_version = Some(version);
            }
            return SourceResult::checked(vec![e]);
        }
        SourceResult::checked(Vec::new())
    }
}

/// The shim for `binary_name` in a root's shims directory: `.exe`, then
/// `.cmd`, then `.ps1`, first existing wins.
fn shim_in(root: &Path, binary_name: &str) -> Option<PathBuf> {
    let shims = root.join("shims");
    for ext in ["exe", "cmd", "ps1"] {
        let shim = shims.join(format!("{binary_name}.{ext}"));
        if is_file(&shim) {
            return Some(shim);
        }
    }
    None
}

/// Scoop's roots, user first, then machine-wide, deduped and existing-only.
fn scoop_roots() -> Vec<PathBuf> {
    roots_from(
        std::env::var("SCOOP").ok().as_deref(),
        std::env::var("USERPROFILE").ok().as_deref(),
        std::env::var("SCOOP_GLOBAL").ok().as_deref(),
        std::env::var("ProgramData").ok().as_deref(),
    )
}

/// scoop_roots() with the environment injected, so the precedence rules are
/// fixture-testable.
fn roots_from(
    scoop_env: Option<&str>,
    userprofile: Option<&str>,
    scoop_global_env: Option<&str>,
    programdata: Option<&str>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = root_from(scoop_env, userprofile) {
        roots.push(root);
    }
    if let Some(root) = global_root_from(scoop_global_env, programdata) {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// The user install root: `$SCOOP` used verbatim when it is a directory, else
/// `$USERPROFILE/scoop` when that exists. Scoop itself treats `$SCOOP` as
/// the root as-is (nothing is appended to it) and falls back to `~\scoop`.
fn root_from(scoop_env: Option<&str>, userprofile: Option<&str>) -> Option<PathBuf> {
    if let Some(base) = scoop_env {
        if !base.is_empty() {
            let root = PathBuf::from(base);
            if is_dir(&root) {
                return Some(root);
            }
        }
    }
    userprofile
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(p).join("scoop"))
        .filter(|root| is_dir(root))
}

/// The machine-wide root for global installs: `$SCOOP_GLOBAL` verbatim when
/// it is a directory, else `%ProgramData%\scoop`, Scoop's own default for
/// `scoop install -g`. Same shape as the user root on purpose: a configured
/// root that is missing falls through to the default rather than surfacing
/// "configured root missing".
fn global_root_from(scoop_global_env: Option<&str>, programdata: Option<&str>) -> Option<PathBuf> {
    if let Some(base) = scoop_global_env {
        if !base.is_empty() {
            let root = PathBuf::from(base);
            if is_dir(&root) {
                return Some(root);
            }
        }
    }
    programdata
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(p).join("scoop"))
        .filter(|root| is_dir(root))
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

    #[test]
    fn cmd_shims_are_accepted_for_script_apps() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-cmd");
        let _ = fs::remove_dir_all(&tmp);
        let shims = tmp.join("shims");
        fs::create_dir_all(&shims).unwrap();
        fs::write(shims.join("tool.cmd"), b"@echo off\n").unwrap();
        assert_eq!(
            shim_in(&tmp, "tool"),
            Some(tmp.join("shims").join("tool.cmd"))
        );
        assert_eq!(shim_in(&tmp, "other"), None);
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
    fn userprofile_fallback_appends_scoop() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-root");
        let _ = fs::remove_dir_all(&tmp);
        let scoop = tmp.join("scoop");
        fs::create_dir_all(&scoop).unwrap();
        let root = root_from(None, Some(tmp.to_str().unwrap())).unwrap();
        assert_eq!(root, scoop);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn scoop_env_wins_and_is_used_verbatim() {
        // $SCOOP is the install root as-is; nothing is appended to it, and
        // it beats the ~\scoop default when both exist.
        let tmp = std::env::temp_dir().join("anywhich-scoop-env");
        let _ = fs::remove_dir_all(&tmp);
        let custom = tmp.join("custom-root");
        let profile = tmp.join("profile");
        fs::create_dir_all(custom.join("shims")).unwrap();
        fs::create_dir_all(profile.join("scoop")).unwrap();
        let root =
            root_from(Some(custom.to_str().unwrap()), Some(profile.to_str().unwrap())).unwrap();
        assert_eq!(root, custom);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn no_root_when_neither_location_exists() {
        assert!(root_from(None, None).is_none());
        assert!(root_from(None, Some("Z:\\definitely-missing-anywhich")).is_none());
        assert!(root_from(Some(""), Some("Z:\\definitely-missing-anywhich")).is_none());
    }

    #[test]
    fn current_app_none_for_missing_app() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-missing");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        assert!(current_app(&tmp, "tool").is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn global_env_is_used_verbatim_after_the_user_root() {
        // $SCOOP_GLOBAL is the global root as-is; it never outranks the
        // user root, and nothing is appended to it.
        let tmp = std::env::temp_dir().join("anywhich-scoop-global-env");
        let _ = fs::remove_dir_all(&tmp);
        let user = tmp.join("user-root");
        let global = tmp.join("global-root");
        fs::create_dir_all(user.join("shims")).unwrap();
        fs::create_dir_all(global.join("shims")).unwrap();
        let roots = roots_from(
            Some(user.to_str().unwrap()),
            None,
            Some(global.to_str().unwrap()),
            None,
        );
        assert_eq!(roots, vec![user, global]);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn global_falls_back_to_programdata_scoop() {
        // Scoop's own default for global installs: %ProgramData%\scoop,
        // used only when $SCOOP_GLOBAL is unset, empty, or missing.
        let tmp = std::env::temp_dir().join("anywhich-scoop-global-default");
        let _ = fs::remove_dir_all(&tmp);
        let programdata = tmp.join("ProgramData");
        fs::create_dir_all(programdata.join("scoop")).unwrap();
        let root = global_root_from(None, Some(programdata.to_str().unwrap())).unwrap();
        assert_eq!(root, programdata.join("scoop"));
        assert!(global_root_from(
            Some("Z:\\definitely-missing-anywhich"),
            Some(programdata.to_str().unwrap())
        )
        .unwrap()
        == programdata.join("scoop"));
        assert!(global_root_from(None, None).is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn identical_user_and_global_roots_are_deduped() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-dedup");
        let _ = fs::remove_dir_all(&tmp);
        let shared = tmp.join("one-root");
        fs::create_dir_all(&shared).unwrap();
        let roots = roots_from(
            Some(shared.to_str().unwrap()),
            None,
            Some(shared.to_str().unwrap()),
            None,
        );
        assert_eq!(roots, vec![shared]);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn user_shim_outranks_a_global_shim() {
        // Both roots export the command; the user root's shim wins, and the
        // package identity comes from the root that produced it.
        let tmp = std::env::temp_dir().join("anywhich-scoop-shadow");
        let _ = fs::remove_dir_all(&tmp);
        let user = tmp.join("user-root");
        let global = tmp.join("global-root");
        fs::create_dir_all(user.join("shims")).unwrap();
        fs::create_dir_all(global.join("shims")).unwrap();
        fs::write(user.join("shims").join("tool.exe"), b"mz").unwrap();
        fs::write(global.join("shims").join("tool.cmd"), b"@echo off\n").unwrap();

        let r = ScoopResolver;
        let result = r.resolve_in(
            &[
                user.clone(),
                global,
            ],
            "tool",
        );
        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(user.join("shims").join("tool.exe").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn global_only_install_is_found_in_the_global_root() {
        // `scoop install -g` with no user copy: the shim comes from the
        // global root. No apps junction in the fixture, so the entry stays
        // a stub with no package identity.
        let tmp = std::env::temp_dir().join("anywhich-scoop-global-only");
        let _ = fs::remove_dir_all(&tmp);
        let global = tmp.join("global-root");
        fs::create_dir_all(global.join("shims")).unwrap();
        fs::write(global.join("shims").join("tool.exe"), b"mz").unwrap();

        let r = ScoopResolver;
        let result = r.resolve_in(&[global], "tool");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(global.join("shims").join("tool.exe").as_path())
        );
        assert!(result.entries[0].package_name.is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn no_shim_in_any_root_is_checked_empty() {
        let tmp = std::env::temp_dir().join("anywhich-scoop-none");
        let _ = fs::remove_dir_all(&tmp);
        let user = tmp.join("user-root");
        let global = tmp.join("global-root");
        fs::create_dir_all(user.join("shims")).unwrap();
        fs::create_dir_all(global.join("shims")).unwrap();

        let r = ScoopResolver;
        let result = r.resolve_in(&[user, global], "tool");
        assert!(result.entries.is_empty());
        assert!(matches!(result.status, crate::resolver::SourceStatus::Checked));
        fs::remove_dir_all(&tmp).unwrap();
    }
}
