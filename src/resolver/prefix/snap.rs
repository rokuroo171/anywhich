use std::fs;
use std::path::PathBuf;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

const SNAP_DIR: &str = "/snap";
const SNAP_BIN: &str = "/snap/bin";

/// Snap packages view of one binary query.
///
/// snapd lays out one directory per snap under /snap with a `current`
/// symlink, and machine-writes `meta/snap.yaml` for each. The `apps:` keys
/// are the command names snapd exposes as `/snap/bin/<key>` shims, so a hit
/// is an exact app-key match, with identity and version from the same
/// manifest. No spawn; a missing /snap is the only unavailable case.
pub struct SnapResolver;

impl Resolver for SnapResolver {
    fn name(&self) -> &'static str {
        "snap"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let Ok(entries) = fs::read_dir(SNAP_DIR) else {
            return SourceResult::unavailable(format!("no snap directory at {SNAP_DIR}"));
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("current").exists())
            .collect();
        dirs.sort();
        let mut found = Vec::new();
        for dir in dirs {
            let manifest_path = dir.join("current").join("meta").join("snap.yaml");
            let Ok(text) = fs::read_to_string(&manifest_path) else {
                continue;
            };
            if let Some((name, version)) = manifest_hit(&text, binary_name) {
                let mut e = ResolvedEntry::new(Source::Snap);
                e.package_name = Some(name);
                e.package_version = if version.is_empty() { None } else { Some(version) };
                e.path = Some(PathBuf::from(SNAP_BIN).join(binary_name));
                found.push(e);
            }
        }
        SourceResult::checked(found)
    }
}

/// (snap name, version) when the manifest's `apps:` section exposes a
/// command named `query`. Only 2-space-indented keys count as app names;
/// deeper lines are app details (command, desktop), and leaving the section
/// on the next top-level key stops the scan. Values may contain spaces
/// (snapd versions look like "126.0-2 (12324)"), so they are taken whole.
pub fn manifest_hit(manifest: &str, query: &str) -> Option<(String, String)> {
    let mut snap_name = String::new();
    let mut version = String::new();
    let mut in_apps = false;
    let mut hit = false;

    for line in manifest.lines() {
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if in_apps {
                let indent = line.len() - line.trim_start().len();
                if indent == 2 {
                    if let Some(key) = line.trim().strip_suffix(':') {
                        if key == query {
                            hit = true;
                        }
                    }
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("name:") {
            snap_name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("version:") {
            version = rest.trim().to_string();
        } else if line == "apps:" {
            in_apps = true;
        } else {
            in_apps = false;
        }
    }
    if hit {
        Some((snap_name, version))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
name: firefox
version: 126.0-2 (12324)
summary: Mozilla Firefox web browser
architectures:
  - build-on: [amd64]
    run-on: [amd64]
apps:
  firefox:
    command: usr/lib/firefox/firefox
  geckodriver:
    command: bin/geckodriver
hooks:
  configure:
    command-chain: []
confinement: strict
";

    #[test]
    fn finds_app_key_with_name_and_version() {
        assert_eq!(
            manifest_hit(FIREFOX, "firefox"),
            Some(("firefox".to_string(), "126.0-2 (12324)".to_string()))
        );
    }

    #[test]
    fn finds_secondary_app() {
        assert_eq!(
            manifest_hit(FIREFOX, "geckodriver"),
            Some(("firefox".to_string(), "126.0-2 (12324)".to_string()))
        );
    }

    #[test]
    fn non_apps_sections_do_not_match() {
        assert_eq!(manifest_hit(FIREFOX, "configure"), None);
        assert_eq!(manifest_hit(FIREFOX, "build-on"), None);
        assert_eq!(manifest_hit(FIREFOX, "amd64"), None);
    }

    #[test]
    fn app_detail_lines_do_not_match() {
        assert_eq!(manifest_hit(FIREFOX, "command"), None);
        assert_eq!(manifest_hit(FIREFOX, "command-chain"), None);
    }

    #[test]
    fn unknown_query_yields_nothing() {
        assert_eq!(manifest_hit(FIREFOX, "chromium"), None);
    }

    #[test]
    fn version_may_be_absent_or_spaced() {
        let manifest = "name: core\nbase: core20\napps:\n  core:\n    command: bin/core\n";
        assert_eq!(
            manifest_hit(manifest, "core"),
            Some(("core".to_string(), String::new()))
        );
    }

    #[test]
    fn apps_section_ends_at_next_top_level_key() {
        let manifest = "name: a\napps:\n  tool:\n    command: x\nlint:\n  tool: ignored\n";
        assert_eq!(manifest_hit(manifest, "tool").map(|(n, _)| n), Some("a".to_string()));
        assert_eq!(manifest_hit(manifest, "ignored"), None);
    }
}
