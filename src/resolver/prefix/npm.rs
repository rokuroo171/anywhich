use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// npm global installs (`npm -g`) view of one binary query.
///
/// One `npm root -g` spawn locates the global tree; the packages under it
/// are read directly (top level, one level into `@scope`). Exports come from
/// each package's `package.json` `bin` field (a map or a bare string), which
/// is what npm itself uses; the entry name is the bin key, not the filename,
/// so `"tool": "bin/tool.js"` is found as `tool`. Name and version come
/// from the same manifest. The stub is `<root>/../bin/<name>`, npm's own
/// global install location, derived from the root instead of a second
/// `npm prefix -g` spawn; `npm ls` as a data source cost seconds per query.
pub struct NpmResolver;

impl Resolver for NpmResolver {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("npm").args(["root", "-g"]).output();
        let out = match out {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SourceResult::unavailable("npm not found");
            }
            Err(e) => {
                return SourceResult::unavailable(format!("npm could not be run: {e}"));
            }
        };
        if !out.status.success() {
            return SourceResult::unavailable(format!("npm exited with {}", out.status));
        }
        let root_text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if root_text.is_empty() {
            return SourceResult::checked(Vec::new());
        }
        let root = PathBuf::from(root_text);

        let Some((_, name, version)) = find_export(&root, binary_name) else {
            return SourceResult::checked(Vec::new());
        };
        let mut e = ResolvedEntry::new(Source::Npm);
        e.package_name = Some(name);
        e.package_version = version;
        if let Some(prefix) = root.parent() {
            let stub = prefix.join("bin").join(binary_name);
            if is_file(&stub) {
                e.path = Some(stub);
            }
        }
        SourceResult::checked(vec![e])
    }
}

/// First package (sorted, so first-match is deterministic) under the global
/// root whose `package.json` exports `binary_name`, as (dir, name, version).
fn find_export(root: &Path, binary_name: &str) -> Option<(PathBuf, String, Option<String>)> {
    let mut dirs = package_dirs(root);
    dirs.sort();
    for dir in dirs {
        let Some(manifest) = read_manifest(&dir) else {
            continue;
        };
        if bin_exports(&manifest, &dir, binary_name) {
            let short = short_name(&dir.to_string_lossy());
            let name = manifest.name.clone().unwrap_or(short);
            return Some((dir, name, manifest.version.clone().filter(|v| !v.is_empty())));
        }
    }
    None
}

/// Package directories directly under a global `node_modules` root:
/// top-level entries, descending one level into `@scope` directories.
fn package_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return dirs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('@') {
            if let Ok(scope) = fs::read_dir(&path) {
                dirs.extend(scope.flatten().map(|e| e.path()));
            }
        } else {
            dirs.push(path);
        }
    }
    dirs
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether this manifest exports `binary_name`: map form by entry key,
/// string form under the package name, per npm's own rule.
fn bin_exports(manifest: &Manifest, dir: &Path, binary_name: &str) -> bool {
    let package_name = manifest
        .name
        .clone()
        .unwrap_or_else(|| short_name(&dir.to_string_lossy()));
    match &manifest.bin {
        Some(Bin::Map(map)) => map.contains_key(binary_name),
        Some(Bin::Single(_)) => package_name == binary_name,
        None => false,
    }
}

#[derive(Deserialize)]
struct Manifest {
    name: Option<String>,
    version: Option<String>,
    bin: Option<Bin>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Bin {
    Map(std::collections::HashMap<String, String>),
    // The target path is parsed for shape but not consumed: the reported
    // path is npm's stub under prefix/bin, not the manifest's relative file.
    #[allow(dead_code)]
    Single(String),
}

/// Short package name from an install dir: the last path segment (for
/// `@scope/name` that is `name`, which is what npm uses for a string-form
/// `bin`).
fn short_name(dir: &str) -> String {
    dir.rsplit('/').find(|s| !s.is_empty()).unwrap_or("").to_string()
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_pkg(root: &Path, rel: &str, json: &str) -> PathBuf {
        let dir = root.join(rel);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("package.json"), json).unwrap();
        dir
    }

    #[test]
    fn finds_map_and_string_exports() {
        let tmp = std::env::temp_dir().join("anywhich-npm-tree");
        let _ = fs::remove_dir_all(&tmp);
        let root = tmp.join("node_modules");
        write_pkg(
            &root,
            "@scope/tool",
            r#"{"name":"@scope/tool","version":"1.2.3","bin":{"tool":"bin/tool.js"}}"#,
        );
        write_pkg(
            &root,
            "other",
            r#"{"name":"other","version":"1.0.0","bin":"bin/other.js"}"#,
        );

        let found = find_export(&root, "tool").unwrap();
        assert_eq!(found.1, "@scope/tool");
        assert_eq!(found.2.as_deref(), Some("1.2.3"));
        let found = find_export(&root, "other").unwrap();
        assert_eq!(found.1, "other");
        assert!(find_export(&root, "otherX").is_none());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn string_bin_does_not_export_other_names() {
        let tmp = std::env::temp_dir().join("anywhich-npm-str");
        let _ = fs::remove_dir_all(&tmp);
        let root = tmp.join("node_modules");
        write_pkg(&root, "x", r#"{"name":"x","bin":"bin/cli.js"}"#);
        assert!(find_export(&root, "x").is_some());
        assert!(find_export(&root, "cli").is_none());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn first_match_is_sorted_order_not_read_order() {
        let tmp = std::env::temp_dir().join("anywhich-npm-sorted");
        let _ = fs::remove_dir_all(&tmp);
        let root = tmp.join("node_modules");
        write_pkg(&root, "zzz", r#"{"name":"zzz","bin":{"tool":"t.js"}}"#);
        write_pkg(&root, "aaa", r#"{"name":"aaa","bin":{"tool":"t.js"}}"#);
        let found = find_export(&root, "tool").unwrap();
        assert_eq!(found.1, "aaa");

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_manifest_is_skipped() {
        let tmp = std::env::temp_dir().join("anywhich-npm-nomani");
        let _ = fs::remove_dir_all(&tmp);
        let root = tmp.join("node_modules");
        fs::create_dir_all(root.join("broken")).unwrap();
        write_pkg(&root, "good", r#"{"name":"good","bin":{"tool":"t.js"}}"#);
        assert_eq!(find_export(&root, "tool").unwrap().1, "good");

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn short_name_takes_last_segment() {
        assert_eq!(short_name("/usr/lib/node_modules/corepack"), "corepack");
        assert_eq!(short_name("/usr/lib/node_modules/@scope/tool"), "tool");
    }
}
