use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// rpm database view of one binary query, under the dnf family.
///
/// One `rpm -qa` with an array queryformat emits "name version /path" per
/// installed file, which gives ownership, version, and a path for PATH
/// merging in a single spawn. `=` pins NAME and VERSION to the first array
/// element while FILENAMES iterates; the unpinned form dies on rpm 6 with
/// "array iterator used with different sized arrays", still exiting 0.
pub struct DnfResolver;

impl Resolver for DnfResolver {
    fn name(&self) -> &'static str {
        "dnf"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("rpm")
            .arg("-qa")
            .arg("--queryformat=[%{=NAME} %{=VERSION} %{FILENAMES}\\n]")
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
        // rpm 6 exits 0 even when it rejects the queryformat, printing the
        // error to stderr and a partial listing to stdout, which parses as a
        // truthful-looking empty result. Only stderr tells the difference.
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("incorrect format") {
            let reason = stderr.lines().next().unwrap_or_default().trim().to_string();
            return SourceResult::unavailable(format!("rpm queryformat error: {reason}"));
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

    // Lines from `rpm -qa --queryformat='[%{=NAME} %{=VERSION} %{FILENAMES}\n]'`
    // on rpm 6.0.2 (Fedora 44), plus one synthetic spaced path.
    const QF_FIXTURE: &str = "\
libgcc 16.2.1 /lib64/libgcc_s-16-20260819.so.1
libgcc 16.2.1 /lib64/libgcc_s.so.1
libgcc 16.2.1 /usr/lib/.build-id
fish 4.6.0 /usr/bin/fish
fish 4.6.0 /usr/bin/fish_indent
fish 4.6.0 /usr/bin/fish_key_reader
bash 5.3.0 /opt/my space dir/bin/tool
";

    #[test]
    fn finds_owner_with_version_and_path() {
        assert_eq!(
            owned_files(QF_FIXTURE, "fish"),
            vec![(
                "fish".to_string(),
                "4.6.0".to_string(),
                "/usr/bin/fish".to_string()
            )]
        );
    }

    #[test]
    fn matches_only_exact_binary_name() {
        let found = owned_files(QF_FIXTURE, "fish_indent");
        assert_eq!(found[0].0, "fish");
        assert!(owned_files(QF_FIXTURE, "fishX").is_empty());
    }

    #[test]
    fn directory_lines_never_match() {
        assert!(owned_files(QF_FIXTURE, "build-id").is_empty());
    }

    #[test]
    fn paths_with_spaces_survive() {
        let found = owned_files(QF_FIXTURE, "tool");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].2, "/opt/my space dir/bin/tool");
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let fixture = "pkg-a 1.0 /usr/bin/tool\npkg-b 2.0 /usr/bin/tool\n";
        let found = owned_files(fixture, "tool");
        let names: Vec<&str> = found.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(owned_files("no fields\none two\n\n", "python3").is_empty());
    }
}
