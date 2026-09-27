use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// xbps (Void) installed database view of one binary query.
///
/// `xbps-query -o '*/bin/<name>'` globs installed file paths and emits one
/// "pkgver: path (kind)" line per match; links carry a " -> target" suffix
/// and the version rides the pkgver ("fish-shell-4.9.3_1"). Exit codes
/// cannot separate hit from miss: a no-match exits 0 with empty output
/// (observed on xbps 0.60.7), so empty stdout is a checked-empty result and
/// only a nonzero exit or missing binary is unavailable. The db records
/// merged-usr paths (/usr/bin), and canonical-path merging absorbs that.
pub struct XbpsResolver;

impl Resolver for XbpsResolver {
    fn name(&self) -> &'static str {
        "xbps"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let pattern = format!("*/bin/{binary_name}");
        let out = Command::new("xbps-query").arg("-o").arg(&pattern).output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("xbps-query not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("xbps-query could not be run: {e}"));
            }
        };
        if !out.status.success() {
            return SourceResult::unavailable(format!("xbps-query exited with {}", out.status));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let entries = parse_owned(&stdout, binary_name)
            .into_iter()
            .map(|(pkg, version, path)| {
                let mut e = ResolvedEntry::new(Source::Xbps);
                e.package_name = Some(pkg);
                e.package_version = Some(version);
                e.path = Some(std::path::PathBuf::from(path));
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// One (package, version, path) per "pkgver: path (kind)" line whose file
/// path ends in `bin/<binary_name>`. A pkgver is name-version_revision and
/// package names may contain dashes (fish-shell), so the version is split
/// from the right. Links keep the installed path, not the link target.
pub fn parse_owned(output: &str, binary_name: &str) -> Vec<(String, String, String)> {
    let suffix = format!("bin/{binary_name}");
    let mut found: Vec<(String, String, String)> = Vec::new();
    for line in output.lines() {
        let Some((pkgver, rest)) = line.split_once(": ") else {
            continue;
        };
        let path = match rest.split_once(" -> ") {
            Some((path, _target)) => path,
            None => rest,
        };
        let path = match path.rsplit_once(" (") {
            Some((p, kind)) if kind.ends_with(')') => p,
            _ => path,
        };
        let Some((name, version)) = pkgver.rsplit_once('-') else {
            continue;
        };
        if name.is_empty() || version.is_empty() {
            continue;
        }
        if path.ends_with(&suffix) && !found.iter().any(|(n, _, _)| n == name) {
            found.push((name.to_string(), version.to_string(), path.to_string()));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines taken verbatim from `xbps-query -o` on Void (xbps 0.60.7).
    const FIXTURE: &str = "\
fish-shell-4.9.3_1: /usr/bin/fish (regular file)
fish-shell-4.9.3_1: /usr/bin/fish_indent -> /usr/bin/fish (link)
fish-shell-4.9.3_1: /usr/bin/fish_key_reader -> /usr/bin/fish (link)
";

    #[test]
    fn parses_real_probe_line() {
        assert_eq!(
            parse_owned(FIXTURE, "fish"),
            vec![(
                "fish-shell".to_string(),
                "4.9.3_1".to_string(),
                "/usr/bin/fish".to_string()
            )]
        );
    }

    #[test]
    fn link_lines_keep_the_installed_path() {
        assert_eq!(
            parse_owned(FIXTURE, "fish_indent"),
            vec![(
                "fish-shell".to_string(),
                "4.9.3_1".to_string(),
                "/usr/bin/fish_indent".to_string()
            )]
        );
    }

    #[test]
    fn dashed_package_names_keep_their_dashes() {
        let found = parse_owned(FIXTURE, "fish");
        assert_eq!(found[0].0, "fish-shell");
    }

    #[test]
    fn matches_only_exact_binary_name() {
        assert!(parse_owned(FIXTURE, "sh").is_empty());
        assert!(parse_owned(FIXTURE, "fishX").is_empty());
    }

    #[test]
    fn multiple_owners_are_all_reported() {
        let fixture = "\
pkg-a-1.0_1: /usr/bin/tool (regular file)
pkg-b-2.0_3: /usr/bin/tool (regular file)
";
        let found = parse_owned(fixture, "tool");
        let names: Vec<&str> = found.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(parse_owned("garbage\n\nmore garbage\n", "fish").is_empty());
        assert!(parse_owned("nodash: /usr/bin/fish (regular file)\n", "fish").is_empty());
        assert!(parse_owned("", "fish").is_empty());
    }
}
