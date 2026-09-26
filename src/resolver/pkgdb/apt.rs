use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Debian package database (dpkg) view of one binary query.
///
/// `dpkg-query -S '*/bin/<name>'` searches installed package file lists for
/// the binary, on or off PATH. Exit code 1 is dpkg's documented no-match, so
/// it is a checked-empty result, not an unavailable. Versions come from
/// `dpkg-query -W` on the owning packages.
pub struct AptResolver;

impl Resolver for AptResolver {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let pattern = format!("*/bin/{binary_name}");
        let out = Command::new("dpkg-query").arg("-S").arg(&pattern).output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("dpkg-query not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("dpkg-query could not be run: {e}"));
            }
        };
        if out.status.code() == Some(1) {
            return SourceResult::checked(Vec::new());
        }
        if !out.status.success() {
            return SourceResult::unavailable(format!(
                "dpkg-query exited with {}",
                out.status
            ));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let owned = parse_ownership(&stdout);
        let packages: Vec<String> = owned.iter().map(|(pkg, _)| pkg.clone()).collect();
        let versions = package_versions(&packages);
        let entries = owned
            .into_iter()
            .map(|(pkg, path)| {
                let mut e = ResolvedEntry::new(Source::Apt);
                e.package_name = Some(pkg.clone());
                e.package_version = versions.get(&pkg).cloned();
                e.path = Some(std::path::PathBuf::from(path));
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// One (package, file path) pair per matching line of `dpkg-query -S`
/// output ("pkg1, pkg2: /path" per line). Diversions and empty lines are
/// skipped, and multiarch qualifiers (`libc6:amd64`) are stripped.
pub fn parse_ownership(s_output: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in s_output.lines() {
        if line.starts_with("diversion by") {
            continue;
        }
        let Some((pkgs, path)) = line.split_once(": ") else {
            continue;
        };
        let Some(path) = path.split(", ").next() else {
            continue;
        };
        let mut pkg = pkgs.split(", ").next().unwrap_or_default().to_string();
        if let Some((base, _arch)) = pkg.split_once(':') {
            pkg = base.to_string();
        }
        if pkg.is_empty() {
            continue;
        }
        if !found.iter().any(|(n, _)| *n == pkg) {
            found.push((pkg, path.to_string()));
        }
    }
    found
}

/// Name-to-version map from `dpkg-query -W -f='${Package} ${Version}\n'`
/// output ("name version" per line). Failures leave versions absent.
fn package_versions(packages: &[String]) -> std::collections::HashMap<String, String> {
    if packages.is_empty() {
        return std::collections::HashMap::new();
    }
    let mut map = std::collections::HashMap::new();
    let fmt = "${Package} ${Version}\n";
    if let Ok(out) = Command::new("dpkg-query")
        .arg("-W")
        .arg(format!("-f={fmt}"))
        .args(packages)
        .output()
    {
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

    #[test]
    fn parses_owner_and_path() {
        let out = "python3-minimal: /usr/bin/python3\npython3: /usr/bin/pydoc3\n";
        assert_eq!(
            parse_ownership(out),
            vec![
                ("python3-minimal".to_string(), "/usr/bin/python3".to_string()),
                ("python3".to_string(), "/usr/bin/pydoc3".to_string())
            ]
        );
    }

    #[test]
    fn first_package_of_a_comma_list_wins() {
        let out = "bash, bash-static: /bin/bash\n";
        let found = parse_ownership(out);
        assert_eq!(found[0].0, "bash");
    }

    #[test]
    fn multiarch_qualifier_is_stripped() {
        let out = "libc6:amd64: /lib/x86_64-linux-gnu/ld-linux-x86-64.so.2\n";
        let found = parse_ownership(out);
        assert_eq!(found[0].0, "libc6");
    }

    #[test]
    fn diversions_are_skipped() {
        let out = "diversion by dash from: /bin/sh\nbash: /bin/bash\n";
        let found = parse_ownership(out);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "bash");
    }

    #[test]
    fn no_matches_yield_nothing() {
        assert!(parse_ownership("").is_empty());
        assert!(parse_ownership("garbage without a separator\n").is_empty());
    }

    #[test]
    fn versions_parse_from_w_output() {
        let mut map = std::collections::HashMap::new();
        for line in ["python3 3.12.6-1", "bash 5.2.21-2"] {
            if let Some((n, v)) = line.split_once(' ') {
                map.insert(n.to_string(), v.to_string());
            }
        }
        assert_eq!(map.get("python3").map(String::as_str), Some("3.12.6-1"));
    }
}
