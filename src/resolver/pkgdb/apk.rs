use std::fs;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

const INSTALLED_DB: &str = "/lib/apk/db/installed";

/// apk installed database view of one binary query.
///
/// The db is plain text, blank-line separated records of one-field lines
/// ("P:", "V:", "F:", "R:"), so it is read directly with no subprocess and
/// no failure mode beyond the file being missing. Paths are emitted exactly
/// as the db records them ("/" + F: + R:); on merged-usr Alpine /bin is a
/// symlink to usr/bin, and canonical-path merging handles both that and the
/// unmerged layout.
pub struct ApkResolver;

impl Resolver for ApkResolver {
    fn name(&self) -> &'static str {
        "apk"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let Ok(db) = fs::read_to_string(INSTALLED_DB) else {
            return SourceResult::unavailable(format!("no apk database at {INSTALLED_DB}"));
        };
        let entries = owned_binaries(&db, binary_name)
            .into_iter()
            .map(|(pkg, version, path)| {
                let mut e = ResolvedEntry::new(Source::Apk);
                e.package_name = Some(pkg);
                e.package_version = version;
                e.path = Some(std::path::PathBuf::from(path));
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// One (package, version, path) per apk whose file records contain the
/// binary. "F:" sets the current directory, "R:" a file under it; tag
/// letters are case sensitive so the lowercase "p:" depends field never
/// masquerades as a package name. One entry per package, first match wins.
pub fn owned_binaries(db: &str, binary_name: &str) -> Vec<(String, Option<String>, String)> {
    let suffix = format!("bin/{binary_name}");
    let mut found: Vec<(String, Option<String>, String)> = Vec::new();
    let mut package = String::new();
    let mut version: Option<String> = None;
    let mut dir = String::new();

    for line in db.lines() {
        let Some((tag, value)) = line.split_once(':') else {
            continue;
        };
        match tag {
            "P" => package = value.to_string(),
            "V" => version = Some(value.to_string()),
            "F" => dir = format!("/{}/", value.trim_end_matches('/')),
            "R" => {
                let path = format!("{dir}{value}");
                if package.is_empty()
                    || !path.ends_with(&suffix)
                    || found.iter().any(|(n, _, _)| *n == package)
                {
                    continue;
                }
                found.push((package.clone(), version.clone(), path));
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = "\
C:Q1CgUImGk=
P:busybox
V:1.36.1-r5
A:x86_64
o:busybox
t:1700000000
D:so:libc.musl-x86_64.so.1
F:bin
R:busybox
a:0:0:755
Z:Q1H1=
F:sbin
R:acct
Z:Q1I2=

P:alpine-baselayout
V:3.4.3-r1
F:bin
F:dev
F:etc

P:busybox-static
V:1.36.1-r5
F:usr
F:usr/bin
R:busybox
a:0:0:755
Z:Q1J3=

P:python3
V:3.12.6-r0
F:usr/bin
R:python3
R:idle3
Z:Q1K4=
";

    #[test]
    fn finds_owner_with_version_and_path() {
        assert_eq!(
            owned_binaries(DB, "busybox"),
            vec![
                (
                    "busybox".to_string(),
                    Some("1.36.1-r5".to_string()),
                    "/bin/busybox".to_string()
                ),
                (
                    "busybox-static".to_string(),
                    Some("1.36.1-r5".to_string()),
                    "/usr/bin/busybox".to_string()
                ),
            ]
        );
    }

    #[test]
    fn sbin_hits_are_found() {
        let found = owned_binaries(DB, "acct");
        assert_eq!(
            found,
            vec![(
                "busybox".to_string(),
                Some("1.36.1-r5".to_string()),
                "/sbin/acct".to_string()
            )]
        );
    }

    #[test]
    fn usr_bin_hits_are_found() {
        assert_eq!(
            owned_binaries(DB, "python3"),
            vec![(
                "python3".to_string(),
                Some("3.12.6-r0".to_string()),
                "/usr/bin/python3".to_string()
            )]
        );
    }

    #[test]
    fn matches_only_exact_binary_name() {
        assert_eq!(
            owned_binaries(DB, "idle3"),
            vec![(
                "python3".to_string(),
                Some("3.12.6-r0".to_string()),
                "/usr/bin/idle3".to_string()
            )]
        );
        assert!(owned_binaries(DB, "idle").is_empty());
        assert!(owned_binaries(DB, "busyboxX").is_empty());
    }

    #[test]
    fn directories_without_files_yield_nothing() {
        assert!(owned_binaries(DB, "dev").is_empty());
        assert!(owned_binaries(DB, "etc").is_empty());
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let db = "P:pkg-a\nF:bin\nR:tool\n\nP:pkg-b\nF:bin\nR:tool\n";
        let found = owned_binaries(db, "tool");
        let names: Vec<&str> = found.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
    }

    #[test]
    fn malformed_input_yields_nothing() {
        assert!(owned_binaries("garbage\n\nmore garbage\n", "python").is_empty());
        assert!(owned_binaries("", "python").is_empty());
        assert!(owned_binaries("F:bin\nR:tool\n", "tool").is_empty());
    }
}
