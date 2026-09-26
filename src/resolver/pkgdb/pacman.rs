use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Local pacman database view of one binary query.
///
/// A hit is any installed package whose file list contains `bin/<name>`,
/// with the file path carried for PATH merging. Versions come from a
/// follow-up `pacman -Q` and may be absent when that lookup fails.
pub struct PacmanResolver;

impl Resolver for PacmanResolver {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("pacman").arg("-Ql").output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("pacman not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("pacman could not be run: {e}"));
            }
        };
        if !out.status.success() {
            return SourceResult::unavailable(format!("pacman exited with {}", out.status));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let owned = owned_files(&stdout, binary_name);
        let versions = package_versions(&owned.iter().map(|(pkg, _)| pkg.clone()).collect::<Vec<_>>());
        let entries = owned
            .into_iter()
            .map(|(pkg, path)| {
                let mut e = ResolvedEntry::new(Source::Pacman);
                e.package_name = Some(pkg.clone());
                e.package_version = versions.get(&pkg).cloned();
                e.path = Some(std::path::PathBuf::from(path));
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// One (package, file path) pair per package whose `pacman -Ql` output lists
/// `bin/<binary_name>` ("package path" per line).
pub fn owned_files(ql_output: &str, binary_name: &str) -> Vec<(String, String)> {
    let suffix = format!("bin/{binary_name}");
    let mut found: Vec<(String, String)> = Vec::new();
    for line in ql_output.lines() {
        let Some((pkg, path)) = line.split_once(' ') else {
            continue;
        };
        if path.ends_with(&suffix) && !found.iter().any(|(n, _)| n == pkg) {
            found.push((pkg.to_string(), path.to_string()));
        }
    }
    found
}

/// Name-to-version map from `pacman -Q <pkg>...` output ("name version"
/// per line). Failures simply leave versions absent.
fn package_versions(packages: &[String]) -> std::collections::HashMap<String, String> {
    if packages.is_empty() {
        return std::collections::HashMap::new();
    }
    let mut map = std::collections::HashMap::new();
    if let Ok(out) = Command::new("pacman").arg("-Q").args(packages).output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if let Some((name, version)) = line.split_once(' ') {
                    map.insert(name.to_string(), version.to_string());
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    const QL_FIXTURE: &str = "\
python /usr/bin/
python /usr/bin/python
python /usr/bin/python3.12
python /usr/lib/python3.12/abc.py
gdb /usr/bin/gdb
gdb /usr/bin/gdb-add-index
";

    #[test]
    fn finds_the_owning_package() {
        assert_eq!(owned_files(QL_FIXTURE, "python"), vec![("python".to_string(), "/usr/bin/python".to_string())]);
    }

    #[test]
    fn matches_only_exact_binary_name() {
        assert_eq!(
            owned_files(QL_FIXTURE, "gdb-add-index"),
            vec![("gdb".to_string(), "/usr/bin/gdb-add-index".to_string())]
        );
        assert!(owned_files(QL_FIXTURE, "gdbX").is_empty());
    }

    #[test]
    fn unknown_binary_yields_no_packages() {
        assert!(owned_files(QL_FIXTURE, "nosuchtool").is_empty());
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(owned_files("garbage\n\nmore garbage", "python").is_empty());
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let fixture = "\
pkg-a /usr/bin/tool
pkg-b /usr/bin/tool
pkg-b /usr/lib/other
";
        let found = owned_files(fixture, "tool");
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
    }
}
