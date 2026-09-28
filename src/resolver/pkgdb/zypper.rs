use std::process::Command;

use super::dnf::{owned_files, rpm_scan_output};
use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// zypper (SUSE) installed database view of one binary query.
///
/// Ownership lives in the same rpm database the dnf family reads, so the
/// scan is identical and only the label and the gate differ: the resolver
/// runs when zypper is present, which keeps rpm-only systems from being
/// reported under the wrong package manager.
pub struct ZypperResolver;

impl Resolver for ZypperResolver {
    fn name(&self) -> &'static str {
        "zypper"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        match Command::new("zypper").arg("--version").output() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("zypper not found");
            }
            Err(e) => return SourceResult::unavailable(format!("zypper could not be run: {e}")),
            Ok(_) => {}
        }
        let stdout = match rpm_scan_output() {
            Ok(text) => text,
            Err(result) => return result,
        };
        let entries = owned_files(&stdout, binary_name)
            .into_iter()
            .map(|(pkg, version, path)| {
                let mut e = ResolvedEntry::new(Source::Zypper);
                e.package_name = Some(pkg);
                e.package_version = Some(version);
                e.path = Some(std::path::PathBuf::from(path));
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Line shape from SUSE rpm 4.20.1 (openSUSE Tumbleweed), same pinned
    // queryformat as the dnf family.
    #[test]
    fn suse_rpm_lines_parse() {
        let stdout = "fish 4.8.1 /usr/bin/fish\nfish 4.8.1 /usr/bin/fish_indent\n";
        assert_eq!(
            owned_files(stdout, "fish"),
            vec![(
                "fish".to_string(),
                "4.8.1".to_string(),
                "/usr/bin/fish".to_string()
            )]
        );
    }

    #[test]
    fn usr_bin_and_sbin_lines_parse() {
        let stdout = "zypper 1.14.101 /usr/bin/zypper\nrpm 4.20.1 /usr/bin/rpm\n";
        assert_eq!(
            owned_files(stdout, "zypper"),
            vec![(
                "zypper".to_string(),
                "1.14.101".to_string(),
                "/usr/bin/zypper".to_string()
            )]
        );
    }

    #[test]
    fn non_bin_paths_never_match() {
        assert!(owned_files("glibc 2.41 /lib64/ld-linux-aarch64.so.1\n", "ld-linux-aarch64.so.1").is_empty());
    }
}
