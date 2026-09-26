use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::Resolver;

/// Local pacman database view of one binary query.
///
/// The resolver reports a package as found when its file list contains an
/// entry ending in `bin/<binary_name>`; pacman owns no PATH mapping, so an
/// off-PATH binary still counts as installed.
pub struct PacmanResolver;

impl Resolver for PacmanResolver {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn resolve(&self, binary_name: &str) -> Vec<ResolvedEntry> {
        let out = Command::new("pacman").arg("-Ql").output();
        let Ok(out) = out else {
            return Vec::new();
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let names = owned_package_names(&stdout, binary_name);
        names
            .into_iter()
            .map(|pkg| {
                let mut e = ResolvedEntry::new(Source::Pacman);
                e.package_name = Some(pkg.to_string());
                e
            })
            .collect()
    }
}

/// Package names from `pacman -Ql` output whose file list contains
/// `bin/<binary_name>`. Format is one entry per line: "package path".
pub fn owned_package_names<'a>(ql_output: &'a str, binary_name: &str) -> Vec<&'a str> {
    let suffix = format!("bin/{binary_name}");
    let mut names = Vec::new();
    for line in ql_output.lines() {
        let Some((pkg, path)) = line.split_once(' ') else {
            continue;
        };
        if path.ends_with(&suffix) {
            if !names.contains(&pkg) {
                names.push(pkg);
            }
        }
    }
    names
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
        assert_eq!(owned_package_names(QL_FIXTURE, "python"), vec!["python"]);
    }

    #[test]
    fn matches_only_exact_binary_name() {
        assert_eq!(owned_package_names(QL_FIXTURE, "gdb-add-index"), vec!["gdb"]);
        assert!(owned_package_names(QL_FIXTURE, "gdbX").is_empty());
    }

    #[test]
    fn unknown_binary_yields_no_packages() {
        assert!(owned_package_names(QL_FIXTURE, "nosuchtool").is_empty());
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(owned_package_names("garbage\n\nmore garbage", "python").is_empty());
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let fixture = "\
pkg-a /usr/bin/tool
pkg-b /usr/bin/tool
pkg-b /usr/lib/other
";
        assert_eq!(owned_package_names(fixture, "tool"), vec!["pkg-a", "pkg-b"]);
    }
}
