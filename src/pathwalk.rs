use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};

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

/// Find every PATH entry for `binary_name`, ranked by PATH order.
/// Rank 0 is the one the shell would execute.
pub fn walk(binary_name: &str, dirs: &[PathBuf]) -> Vec<ResolvedEntry> {
    let mut found = Vec::new();
    for (idx, dir) in dirs.iter().enumerate() {
        let candidate = dir.join(binary_name);
        if is_executable(&candidate) {
            let mut entry = ResolvedEntry::new(Source::Path);
            entry.path = Some(candidate);
            entry.rank = idx;
            entry.active = found.is_empty();
            found.push(entry);
        }
    }
    found
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && is_executable_mode(&meta))
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_executable_mode(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_mode(_meta: &std::fs::Metadata) -> bool {
    true
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

    #[test]
    fn finds_every_match_in_order_and_marks_first_active() {
        let tmp = env::temp_dir().join("anywhich-test-order");
        let a = tmp.join("a");
        let b = tmp.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        for dir in [&a, &b] {
            let bin = dir.join("tool");
            let mut perms = fs::metadata(&dir).unwrap().permissions();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, perms).unwrap();
            fs::write(&bin, b"#!/bin/sh\n").unwrap();
            perms = fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin, perms).unwrap();
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
}
