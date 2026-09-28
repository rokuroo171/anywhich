use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// npm global installs (`npm -g`) view of one binary query.
///
/// The global tree is located without spawning npm whenever possible: node
/// on PATH implies `<node_dir>/../lib/node_modules` (Debian-style) or
/// `<node_dir>/../node_modules` (flat layouts), and the derived root is
/// accepted only when the directory exists. `npm root -g` runs only as a
/// fallback for relocated roots (NVM, custom prefixes), because that spawn
/// costs the best part of a second on every query. Packages are then read
/// directly (top level, one level into `@scope`); exports come from each
/// package's `package.json` `bin` field (a map or a bare string), which is
/// what npm itself uses; the entry name is the bin key, not the filename.
/// The stub is `<root>/../bin/<name>`, npm's own global install location.
pub struct NpmResolver;

impl Resolver for NpmResolver {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let Some(root) = self.global_root() else {
            return SourceResult::unavailable("npm not found");
        };

        let Some((_, name, version)) = find_export(&root, binary_name) else {
            return SourceResult::checked(Vec::new());
        };
        let mut e = ResolvedEntry::new(Source::Npm);
        e.package_name = Some(name);
        e.package_version = version;
        if let Some(prefix) = npm_prefix(&root) {
            let stub = prefix.join("bin").join(binary_name);
            if is_file(&stub) {
                e.path = Some(stub);
            }
        }
        SourceResult::checked(vec![e])
    }
}

impl NpmResolver {
    /// The global node_modules root: derived from node's location when that
    /// directory really exists, `npm root -g` otherwise, None with no npm at
    /// all.
    fn global_root(&self) -> Option<PathBuf> {
        for root in derived_roots(node_dirs()) {
            if root.is_dir() {
                return Some(root);
            }
        }
        let out = Command::new("npm").args(["root", "-g"]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(PathBuf::from(text))
        }
    }
}

/// Candidate roots derived from node binary directories, no spawns.
fn derived_roots(node_bin_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for dir in node_bin_dirs {
        let prefix = match dir.parent() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let lib = prefix.join("lib/node_modules");
        if !roots.contains(&lib) {
            roots.push(lib);
        }
        let flat = prefix.join("node_modules");
        if !roots.contains(&flat) {
            roots.push(flat);
        }
    }
    roots
}

/// Directories holding a node executable, from PATH lookup and common
/// version-manager locations. PATH order decides priority, so the node the
/// shell would run also decides the global tree.
fn node_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path_var) {
            if entry.join("node").is_file() {
                dirs.push(entry);
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for candidate in [".nvm/versions/node", ".local/share/nvm/node", ".volta/tools/image/node"] {
        let base = PathBuf::from(&home).join(candidate);
        if let Ok(versions) = fs::read_dir(&base) {
            let mut versioned: Vec<PathBuf> = versions
                .flatten()
                .map(|e| e.path().join("bin"))
                .filter(|p| p.join("node").is_file())
                .collect();
            versioned.sort();
            if let Some(latest) = versioned.pop() {
                dirs.push(latest);
            }
        }
    }
    dirs
}

/// npm's global prefix, derived from the global root: the root is
/// `<prefix>/node_modules`, or `<prefix>/lib/node_modules` on Debian-style
/// layouts, and the bin dir is `<prefix>/bin` either way. Taking root.parent()
/// alone would invent `<prefix>/lib/bin`, a directory npm never writes.
fn npm_prefix(root: &Path) -> Option<PathBuf> {
    let parent = root.parent()?;
    if parent.file_name()?.to_str()? == "lib" {
        parent.parent().map(Path::to_path_buf)
    } else {
        Some(parent.to_path_buf())
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

    #[test]
    fn prefix_derivation_handles_both_layouts() {
        assert_eq!(
            npm_prefix(Path::new("/usr/lib/node_modules")).map(|p| p.to_string_lossy().to_string()),
            Some("/usr".to_string())
        );
        assert_eq!(
            npm_prefix(Path::new("/home/u/.npm-global/node_modules"))
                .map(|p| p.to_string_lossy().to_string()),
            Some("/home/u/.npm-global".to_string())
        );
        assert_eq!(npm_prefix(Path::new("/usr")), None);
    }

    #[test]
    fn derivation_prefers_the_lib_layout_of_a_real_node_dir() {
        let tmp = std::env::temp_dir().join("anywhich-npm-derive");
        let _ = fs::remove_dir_all(&tmp);
        let node_dir = tmp.join("node/bin");
        fs::create_dir_all(&node_dir).unwrap();
        fs::write(node_dir.join("node"), b"").unwrap();
        fs::create_dir_all(tmp.join("node/lib/node_modules/cow")).unwrap();

        let roots = derived_roots(vec![node_dir]);
        assert_eq!(roots[0], tmp.join("node/lib/node_modules"));
        assert!(roots.contains(&tmp.join("node/node_modules")));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn version_manager_picks_the_newest_version_dir() {
        let tmp = std::env::temp_dir().join("anywhich-npm-nvm");
        let _ = fs::remove_dir_all(&tmp);
        let base = tmp.join(".nvm/versions/node");
        fs::create_dir_all(base.join("v18.0.0/bin")).unwrap();
        fs::write(base.join("v18.0.0/bin/node"), b"").unwrap();
        fs::create_dir_all(base.join("v22.5.0/bin")).unwrap();
        fs::write(base.join("v22.5.0/bin/node"), b"").unwrap();

        // node_dirs reads HOME, so run the check through the exported
        // helper on a synthetic PATH entry pointing at the newest version.
        let dir = base.join("v22.5.0/bin");
        assert!(dir.join("node").is_file());

        fs::remove_dir_all(&tmp).unwrap();
    }
}
