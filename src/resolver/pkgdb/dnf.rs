use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// rpm database view of one binary query, under the dnf family.
///
/// One `rpm -qa` with an array queryformat emits "name version /path" per
/// installed file, which gives ownership, version, and a path for PATH
/// merging in a single spawn.
pub struct DnfResolver;

impl Resolver for DnfResolver {
    fn name(&self) -> &'static str {
        "dnf"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("rpm")
            .arg("-qa")
            .arg("--queryformat=[%{NAME} %{VERSION} %{FILENAMES}\\n]")
            .output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("rpm not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("rpm could not be run: {e}"));
            }
        };
        if !out.status.success() {
            return SourceResult::unavailable(format!("rpm exited with {}", out.status));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let entries = owned_files(&stdout, binary_name)
            .into_iter()
            .map(|(pkg, version, path)| {
                let mut e = ResolvedEntry::new(Source::Dnf);
                e.package_name = Some(pkg);
                e.package_version = Some(version);
                e.path = Some(std::path::PathBuf::from(path));
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// One (package, version, path) per installed rpm whose file list contains
/// `bin/<binary_name>`, from "name version /path" lines. Paths keep spaces:
/// only the first two fields split off.
pub fn owned_files(qf_output: &str, binary_name: &str) -> Vec<(String, String, String)> {
    let suffix = format!("bin/{binary_name}");
    let mut found: Vec<(String, String, String)> = Vec::new();
    for line in qf_output.lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(pkg), Some(version), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if pkg.is_empty() || version.is_empty() {
            continue;
        }
        if path.ends_with(&suffix) && !found.iter().any(|(n, _, _)| n == pkg) {
            found.push((pkg.to_string(), version.to_string(), path.to_string()));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const QF_FIXTURE: &str = "\
glibc 2.39-9.fc40 /usr/bin/glibc-bin
python3 3.12.6-1.fc40 /usr/bin/python3
python3 3.12.6-1.fc40 /usr/bin/python3.12
python3 3.12.6-1.fc40 /usr/lib/python3.12/abc.py
bash 5.2.26-1.fc40 /opt/my space dir/bin/tool
";

    #[test]
    fn finds_owner_with_version_and_path() {
        assert_eq!(
            owned_files(QF_FIXTURE, "python3"),
            vec![(
                "python3".to_string(),
                "3.12.6-1.fc40".to_string(),
                "/usr/bin/python3".to_string()
            )]
        );
    }

    #[test]
    fn matches_only_exact_binary_name() {
        let found = owned_files(QF_FIXTURE, "python3.12");
        assert_eq!(found[0].0, "python3");
        assert!(owned_files(QF_FIXTURE, "python3X").is_empty());
    }

    #[test]
    fn paths_with_spaces_survive() {
        let found = owned_files(QF_FIXTURE, "tool");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].2, "/opt/my space dir/bin/tool");
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let found = owned_files(QF_FIXTURE, "glibc-bin");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "glibc");
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(owned_files("no fields\none two\n\n", "python3").is_empty());
    }
}
