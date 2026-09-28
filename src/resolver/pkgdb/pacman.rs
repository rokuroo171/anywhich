use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

const LOCAL_DB: &str = "/var/lib/pacman/local";

/// Local pacman database view of one binary query.
///
/// The database is plain text on disk: one directory per installed package
/// under `/var/lib/pacman/local/<name>-<version>`, where `files` holds the
/// package's paths and `desc` holds `%NAME%` and `%VERSION%` tags. Reading
/// those files directly replaces two pacman spawns (the old `-Ql` dump plus
/// a per-package `-Q` for versions), costs the same I/O pacman itself pays,
/// and fails honest: a missing database is an unavailable with a reason,
/// instead of a pacman binary that cannot run for unrelated reasons.
pub struct PacmanResolver;

impl Resolver for PacmanResolver {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let local = PathBuf::from(LOCAL_DB);
        if !local.is_dir() {
            return SourceResult::unavailable(format!("no pacman database at {LOCAL_DB}"));
        }
        let entries = owned_files(&local, binary_name)
            .into_iter()
            .map(|(pkg, version, path)| {
                let mut e = ResolvedEntry::new(Source::Pacman);
                e.package_name = Some(pkg);
                e.package_version = version;
                e.path = Some(path);
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// One (package, version, path) per installed package whose `files` list
/// contains `bin/<binary_name>`. Paths are reconstructed with the leading
/// slash the database omits. All owners are reported, one entry per
/// package.
pub fn owned_files(local_dir: &Path, binary_name: &str) -> Vec<(String, Option<String>, PathBuf)> {
    let suffix = format!("bin/{binary_name}");
    let mut found: Vec<(String, Option<String>, PathBuf)> = Vec::new();
    let Ok(packages) = fs::read_dir(local_dir) else {
        return found;
    };
    let mut dirs: Vec<PathBuf> = packages.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let desc = fs::read_to_string(dir.join("desc")).ok();
        let Some(name) = desc.as_deref().and_then(|d| desc_field(d, "NAME")) else {
            continue;
        };
        let version = desc.as_deref().and_then(|d| desc_field(d, "VERSION"));
        let Ok(files) = fs::read_to_string(dir.join("files")) else {
            continue;
        };
        if let Some(path) = files_hit(&files, &suffix) {
            found.push((name, version, PathBuf::from(path)));
        }
    }
    found
}

/// The first file path under `%FILES%` ending in `suffix`, with the leading
/// slash the database omits. The section ends at the next `%TAG%` line, so
/// `%BACKUP%` entries never masquerade as files.
fn files_hit(files: &str, suffix: &str) -> Option<String> {
    let mut in_files = false;
    for line in files.lines() {
        if line.starts_with('%') {
            in_files = line == "%FILES%";
            continue;
        }
        if in_files {
            let path = format!("/{line}");
            if path.ends_with(suffix) {
                return Some(path);
            }
        }
    }
    None
}

/// Value of a `%TAG%` section header in a desc file, single-line fields
/// only (NAME, VERSION).
fn desc_field(desc: &str, tag: &str) -> Option<String> {
    let marker = format!("%{tag}%");
    let mut lines = desc.lines();
    while let Some(line) = lines.next() {
        if line.trim() == marker {
            return lines.next().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(local: &Path, dirname: &str, desc: &str, files: &str) {
        let dir = local.join(dirname);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("desc"), desc).unwrap();
        fs::write(dir.join("files"), files).unwrap();
    }

    fn fixture(tmp: &Path) -> PathBuf {
        let local = tmp.join("local");
        write_package(
            &local,
            "fish-4.6.0-1",
            "%NAME%\nfish\n%VERSION%\n4.6.0-1\n",
            "%FILES%\nusr/\nusr/bin/\nusr/bin/fish\nusr/bin/fish_indent\n",
        );
        write_package(
            &local,
            "gdb-15.1-1",
            "%NAME%\ngdb\n%VERSION%\n15.1-1\n",
            "%FILES%\nusr/\nusr/bin/\nusr/bin/gdb\n",
        );
        write_package(
            &local,
            "broken-1.0-1",
            "%NAME%\nbroken\n",
            "%FILES%\nusr/bin/tool\n",
        );
        local
    }

    fn temp(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn finds_owner_with_version_and_path() {
        let tmp = temp("anywhich-pacman-hit");
        let local = fixture(&tmp);
        assert_eq!(
            owned_files(&local, "fish"),
            vec![(
                "fish".to_string(),
                Some("4.6.0-1".to_string()),
                PathBuf::from("/usr/bin/fish")
            )]
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn matches_only_exact_binary_name() {
        let tmp = temp("anywhich-pacman-exact");
        let local = fixture(&tmp);
        let found = owned_files(&local, "fish_indent");
        assert_eq!(found[0].0, "fish");
        assert!(owned_files(&local, "fishX").is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn backup_section_never_matches() {
        let tmp = temp("anywhich-pacman-backup");
        let local = tmp.join("local");
        write_package(
            &local,
            "sudo-1.9.15-1",
            "%NAME%\nsudo\n%VERSION%\n1.9.15-1\n",
            "%FILES%\nusr/bin/sudo\n%BACKUP%\netc/sudoers\n",
        );
        assert!(owned_files(&local, "sudoers").is_empty());
        assert!(owned_files(&local, "sudo").len() == 1);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let tmp = temp("anywhich-pacman-multi");
        let local = tmp.join("local");
        write_package(
            &local,
            "pkg-a-1.0-1",
            "%NAME%\npkg-a\n%VERSION%\n1.0-1\n",
            "%FILES%\nusr/bin/tool\n",
        );
        write_package(
            &local,
            "pkg-b-2.0-1",
            "%NAME%\npkg-b\n%VERSION%\n2.0-1\n",
            "%FILES%\nusr/bin/tool\n",
        );
        let found = owned_files(&local, "tool");
        let names: Vec<&str> = found.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn package_without_desc_is_skipped() {
        let tmp = temp("anywhich-pacman-nodesc");
        let local = tmp.join("local");
        fs::create_dir_all(local.join("orphan-1.0-1")).unwrap();
        write_package(
            &local,
            "real-1.0-1",
            "%NAME%\nreal\n%VERSION%\n1.0-1\n",
            "%FILES%\nusr/bin/real\n",
        );
        assert_eq!(owned_files(&local, "real").len(), 1);
        assert!(owned_files(&local, "orphan").is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn desc_and_files_helpers_read_the_db_shape() {
        assert_eq!(
            desc_field("%NAME%\nfish\n%VERSION%\n4.6.0-1\n", "VERSION").as_deref(),
            Some("4.6.0-1")
        );
        assert_eq!(desc_field("%NAME%\nfish\n", "VERSION"), None);
        assert_eq!(
            files_hit("%FILES%\nusr/\nusr/bin/fish\n%BACKUP%\netc/x\n", "bin/fish").as_deref(),
            Some("/usr/bin/fish")
        );
        assert_eq!(files_hit("%BACKUP%\netc/x\n", "bin/fish"), None);
    }
}
