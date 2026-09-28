use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::pathext;

/// Walk $PATH in order and return every directory that exists and is readable.
/// Unreadable or missing directories are reported separately so the output can
/// say why a check was skipped, per the not-found output contract in DESIGN.md.
pub fn path_dirs() -> (Vec<PathBuf>, Vec<String>) {
    let raw = env::var("PATH").unwrap_or_default();
    let mut usable = Vec::new();
    let mut skipped = Vec::new();

    for dir in env::split_paths(&raw) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        match fs::metadata(&dir) {
            Ok(meta) if meta.is_dir() => usable.push(dir),
            _ => skipped.push(dir.display().to_string()),
        }
    }
    (usable, skipped)
}

/// PATHEXT lookup rules for this platform: the parsed PATHEXT on Windows,
/// an empty list elsewhere. Empty means no extension expansion and no
/// extension filter, which is exactly Unix shell behavior and keeps the
/// Unix walk's syscall count identical to the pre-PATHEXT version.
#[cfg(windows)]
fn pathext_rules() -> Vec<(String, bool)> {
    pathext::parse(std::env::var("PATHEXT").ok().as_deref())
}

#[cfg(not(windows))]
fn pathext_rules() -> Vec<(String, bool)> {
    Vec::new()
}

/// Find every PATH entry for `binary_name`, ranked by PATH order.
/// Rank 0 is the one the shell would execute.
pub fn walk(binary_name: &str, dirs: &[PathBuf]) -> Vec<ResolvedEntry> {
    let exts = pathext_rules();
    walk_with(binary_name, dirs, &exts)
}

/// walk() with injected PATHEXT rules, so the Windows expansion and ordering
/// rules are testable from any platform.
pub fn walk_with(
    binary_name: &str,
    dirs: &[PathBuf],
    exts: &[(String, bool)],
) -> Vec<ResolvedEntry> {
    let names = pathext::candidate_names(binary_name, exts);
    let mut found = Vec::new();
    for (idx, dir) in dirs.iter().enumerate() {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable(&candidate, exts) {
                let mut entry = ResolvedEntry::new(Source::Path);
                entry.path = Some(candidate);
                entry.rank = idx;
                entry.active = found.is_empty();
                found.push(entry);
                break;
            }
        }
    }
    found
}

#[cfg(unix)]
fn is_executable(path: &Path, _exts: &[(String, bool)]) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// On Windows, membership in PATHEXT is what makes a file a command.
#[cfg(windows)]
fn is_executable(path: &Path, exts: &[(String, bool)]) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    match path.file_name().and_then(|n| n.to_str()) {
        Some(file_name) => pathext::is_executable_extension(file_name, exts),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_yields_no_dirs_and_no_matches() {
        // SAFETY: single-threaded test binary scope, and no other test reads PATH.
        unsafe { env::set_var("PATH", "") };
        let (dirs, skipped) = path_dirs();
        assert!(dirs.is_empty());
        assert!(skipped.is_empty());
        assert!(walk("anything", &dirs).is_empty());
    }

    #[test]
    fn missing_dirs_are_skipped_not_dropped_silently() {
        unsafe { env::set_var("PATH", "/nonexistent-anywhich-dir") };
        let (dirs, skipped) = path_dirs();
        assert!(dirs.is_empty());
        assert_eq!(skipped, vec!["/nonexistent-anywhich-dir".to_string()]);
    }

    // Unix-only: a plain extensionless name is only executable where an
    // exec bit exists. The Windows twin below exercises the same ordering
    // through the PATHEXT rules the Windows walk actually uses.
    #[cfg(unix)]
    #[test]
    fn finds_every_match_in_order_and_marks_first_active() {
        let tmp = env::temp_dir().join("anywhich-test-order");
        let a = tmp.join("a");
        let b = tmp.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        for dir in [&a, &b] {
            let bin = dir.join("tool");
            fs::write(&bin, b"#!/bin/sh\n").unwrap();
            make_executable(&bin);
        }

        let dirs = vec![a.clone(), b.clone()];
        let hits = walk("tool", &dirs);

        assert_eq!(hits.len(), 2);
        assert!(hits[0].active);
        assert!(!hits[1].active);
        assert_eq!(hits[0].path, Some(a.join("tool")));
        assert_eq!(hits[1].path, Some(b.join("tool")));
        assert_eq!(hits[0].rank, 0);
        assert_eq!(hits[1].rank, 1);

        fs::remove_file(a.join("tool")).unwrap();
        fs::remove_file(b.join("tool")).unwrap();
        fs::remove_dir_all(&tmp).unwrap();
    }

    // On a case-insensitive filesystem the walk reports the candidate name
    // it probed (spelled by PATHEXT), not the on-disk spelling, so compare
    // the extension case-insensitively.
    #[cfg(windows)]
    #[test]
    fn finds_every_match_in_order_and_marks_first_active() {
        let tmp = env::temp_dir().join("anywhich-test-order-win");
        let a = tmp.join("a");
        let b = tmp.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        for dir in [&a, &b] {
            fs::write(dir.join("tool.exe"), b"mz").unwrap();
        }

        let exts = pathext::parse(Some(".COM;.EXE;.BAT"));
        let hits = walk_with("tool", &[a.clone(), b.clone()], &exts);

        assert_eq!(hits.len(), 2);
        assert!(hits[0].active);
        assert!(!hits[1].active);
        assert_eq!(
            hits[0].path.as_ref().map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase()),
            Some("tool.exe".to_string())
        );
        assert_eq!(hits[0].path.as_ref().unwrap().parent(), Some(a.as_path()));
        assert_eq!(
            hits[1].path.as_ref().map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase()),
            Some("tool.exe".to_string())
        );
        assert_eq!(hits[1].path.as_ref().unwrap().parent(), Some(b.as_path()));
        assert_eq!(hits[0].rank, 0);
        assert_eq!(hits[1].rank, 1);

        fs::remove_dir_all(&tmp).unwrap();
    }

    // Unix-only: without an exec bit there is no way to mark a plain file
    // non-executable. On Windows the PATHEXT membership filter plays that
    // role and is covered by the extension tests below.
    #[cfg(unix)]
    #[test]
    fn non_executable_file_is_not_a_match() {
        let tmp = env::temp_dir().join("anywhich-test-perm");
        fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("notexec");
        fs::write(&bin, b"data").unwrap();

        assert!(walk("notexec", &[tmp.clone()]).is_empty());

        fs::remove_file(&bin).unwrap();
        fs::remove_dir(&tmp).unwrap();
    }

    #[test]
    fn empty_pathext_rules_expand_nothing() {
        assert_eq!(pathext::candidate_names("tool", &[]), vec!["tool"]);
    }

    #[test]
    fn pathext_expansion_finds_the_only_ext_variant() {
        let tmp = env::temp_dir().join("anywhich-test-pathtext");
        fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("tool.BAT");
        fs::write(&bin, b"data").unwrap();
        make_executable(&bin);

        let exts = pathext::parse(Some(".COM;.EXE;.BAT"));
        let hits = walk_with("tool", &[tmp.clone()], &exts);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, Some(tmp.join("tool.BAT")));
        assert!(hits[0].active);

        fs::remove_dir_all(&tmp).unwrap();
    }

    // Unix: the typed bare name outranks every PATHEXT expansion. The
    // Windows twin below asserts the equivalent through PATHEXT rules.
    #[cfg(unix)]
    #[test]
    fn pathext_order_prefers_the_typed_name_then_ext_order() {
        let tmp = env::temp_dir().join("anywhich-test-pathtext-order");
        fs::create_dir_all(&tmp).unwrap();
        for name in ["tool", "tool.EXE", "tool.BAT"] {
            let p = tmp.join(name);
            fs::write(&p, b"data").unwrap();
            make_executable(&p);
        }

        let exts = pathext::parse(Some(".COM;.EXE;.BAT"));
        let hits = walk_with("tool", &[tmp.clone()], &exts);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, Some(tmp.join("tool")));

        let hits = walk_with("tool.BAT", &[tmp.clone()], &exts);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, Some(tmp.join("tool.BAT")));

        fs::remove_dir_all(&tmp).unwrap();
    }

    // Windows: an extensionless file is not a command (PATHEXT membership
    // is the executable filter), so the typed-name preference shows up as
    // the .EXE expansion winning over the later .BAT. On a case-insensitive
    // filesystem the walked name is what PATHEXT spells, so the extension is
    // compared case-insensitively.
    #[cfg(windows)]
    #[test]
    fn pathext_order_prefers_the_typed_name_then_ext_order() {
        let tmp = env::temp_dir().join("anywhich-test-pathtext-order-win");
        fs::create_dir_all(&tmp).unwrap();
        for name in ["tool", "tool.EXE", "tool.BAT"] {
            let p = tmp.join(name);
            fs::write(&p, b"data").unwrap();
            make_executable(&p);
        }

        let exts = pathext::parse(Some(".COM;.EXE;.BAT"));
        let hits = walk_with("tool", &[tmp.clone()], &exts);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].path.as_ref().map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase()),
            Some("tool.exe".to_string())
        );

        let hits = walk_with("tool.BAT", &[tmp.clone()], &exts);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].path.as_ref().map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase()),
            Some("tool.bat".to_string())
        );

        fs::remove_dir_all(&tmp).unwrap();
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
        #[cfg(windows)]
        let _ = path;
    }
}
