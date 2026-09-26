use std::process::Command;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Flatpak installed-app view of one binary query.
///
/// Flatpak apps do not land executables on PATH the way native packages do;
/// they are matched by app id or exported binary name. A hit is either a
/// matching app id or an exact exported-name match.
pub struct FlatpakResolver;

impl Resolver for FlatpakResolver {
    fn name(&self) -> &'static str {
        "flatpak"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("flatpak")
            .args(["list", "--app", "--columns=application"])
            .output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("flatpak not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("flatpak could not be run: {e}"));
            }
        };
        if !out.status.success() {
            return SourceResult::unavailable(format!("flatpak exited with {}", out.status));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let apps = matching_app_ids(&stdout, binary_name);
        let entries = apps
            .into_iter()
            .map(|app| {
                let mut e = ResolvedEntry::new(Source::Flatpak);
                e.package_name = Some(app.to_string());
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// App ids from `flatpak list --columns=application` output that match a
/// query. A line matches when the app id equals the query, or when the query
/// equals the last segment (the exported binary-style short name, so
/// `org.mozilla.firefox` is found by `firefox`).
pub fn matching_app_ids<'a>(list_output: &'a str, query: &str) -> Vec<&'a str> {
    let mut ids = Vec::new();
    for line in list_output.lines() {
        let id = line.trim();
        if id.is_empty() {
            continue;
        }
        let short = id.rsplit('.').next().unwrap_or(id);
        if id == query || short.eq_ignore_ascii_case(query) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_FIXTURE: &str = "\
org.mozilla.firefox
com.spotify.Client
org.gimp.GIMP
org.freedesktop.Platform
";

    #[test]
    fn matches_exact_app_id() {
        assert_eq!(
            matching_app_ids(LIST_FIXTURE, "org.mozilla.firefox"),
            vec!["org.mozilla.firefox"]
        );
    }

    #[test]
    fn matches_short_name_case_insensitively() {
        assert_eq!(
            matching_app_ids(LIST_FIXTURE, "firefox"),
            vec!["org.mozilla.firefox"]
        );
        assert_eq!(matching_app_ids(LIST_FIXTURE, "CLIENT"), vec!["com.spotify.Client"]);
    }

    #[test]
    fn unknown_app_yields_no_matches() {
        assert!(matching_app_ids(LIST_FIXTURE, "nosuchapp").is_empty());
    }

    #[test]
    fn blank_lines_are_ignored() {
        assert!(matching_app_ids("\n\n", "firefox").is_empty());
    }

    #[test]
    fn duplicates_are_reported_once() {
        let fixture = "org.a.Firefox\norg.b.firefox\n";
        assert_eq!(
            matching_app_ids(fixture, "firefox"),
            vec!["org.a.Firefox", "org.b.firefox"]
        );
    }
}
