use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// bun global installs (`bun add -g`) view of one binary query.
///
/// Each package binary is a symlink in `~/.bun/bin` into the single global
/// project at `~/.bun/install/global/node_modules`, where every package's
/// package.json carries name, version, and its bin entries. The resolver
/// matches the queried binary against those bin entries, so renamed or
/// differently named bins still attribute to their package. Files only, no
/// subprocess.
pub struct BunResolver {
    home: PathBuf,
}

impl Default for BunResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl BunResolver {
    pub fn new() -> Self {
        let home = std::env::var("BUN_INSTALL")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".bun")
            });
        BunResolver { home }
    }

    #[cfg(test)]
    fn with_home(home: PathBuf) -> Self {
        BunResolver { home }
    }
}

impl Resolver for BunResolver {
    fn name(&self) -> &'static str {
        "bun"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let shim = self.home.join("bin").join(binary_name);
        if !is_file(&shim) {
            return SourceResult::checked(Vec::new());
        }
        let modules = self.home.join("install/global/node_modules");
        let mut records: Vec<(String, Option<String>)> = owning_packages(&modules, binary_name);
        records.dedup();
        if records.is_empty() {
            return SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Bun, &shim)]);
        }
        let entries = records
            .into_iter()
            .map(|(pkg, version)| {
                let mut e = ResolvedEntry::new(Source::Bun);
                e.package_name = Some(pkg);
                e.package_version = version;
                e.path = Some(shim.clone());
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// (name, version) per package under the global node_modules whose bin
/// entries contain `binary_name`, either as a string bin (`"bin": "cli.js"`,
/// package name is the bin) or an object (`"bin": {"name": "cli.js"}`).
fn owning_packages(modules: &Path, binary_name: &str) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let Ok(packages) = fs::read_dir(modules) else {
        return found;
    };
    for dir in packages.filter_map(|e| e.ok()).map(|e| e.path()) {
        let Ok(text) = fs::read_to_string(dir.join("package.json")) else {
            continue;
        };
        let Some(name) = json_string_field(&text, "name") else {
            continue;
        };
        if !bins_contain(&text, &name, binary_name) {
            continue;
        }
        found.push((name, json_string_field(&text, "version")));
    }
    found
}

/// Whether `binary_name` is a bin of the package described by the manifest
/// text: a string bin is the package name itself, an object bin lists bin
/// names as keys.
fn bins_contain(manifest: &str, package_name: &str, binary_name: &str) -> bool {
    let marker = "\"bin\"";
    let Some(start) = manifest.find(marker) else {
        return false;
    };
    let rest = &manifest[start + marker.len()..];
    let Some(colon) = rest.find(':') else {
        return false;
    };
    let rest = &rest[colon + 1..];
    if rest.trim_start().starts_with('{') {
        let key = format!("\"{binary_name}\"");
        rest.contains(&key)
    } else {
        binary_name == package_name
    }
}

/// Value of a flat `"key": "value"` string field in a package.json.
fn json_string_field(json: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let start = json.find(&marker)? + marker.len();
    let rest = &json[start..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote = rest.find('"')? + 1;
    let rest = &rest[quote..];
    let end = rest.find('"')?;
    let value = &rest[..end];
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global_tree(tmp: &Path, manifests: &[&str]) -> BunResolver {
        let home = tmp.join(".bun");
        let modules = home.join("install/global/node_modules");
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::create_dir_all(&modules).unwrap();
        for (i, manifest) in manifests.iter().enumerate() {
            let dir = modules.join(format!("pkg{i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("package.json"), manifest).unwrap();
        }
        BunResolver::with_home(home)
    }

    fn shim(tmp: &Path, name: &str) {
        fs::write(tmp.join(".bun/bin").join(name), b"#!/usr/bin/env bun\n").unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn finds_package_version_and_shim() {
        let tmp = temp("anywhich-bun-hit");
        let r = global_tree(&tmp, &[r#"{"name": "cowsay", "version": "1.6.0", "bin": {"cowsay": "cli.js", "cowthink": "cli.js"}}"#]);
        shim(&tmp, "cowsay");
        let result = r.resolve("cowsay");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].package_name.as_deref(), Some("cowsay"));
        assert_eq!(result.entries[0].package_version.as_deref(), Some("1.6.0"));
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(tmp.join(".bun/bin/cowsay").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn string_bin_means_the_package_name_itself() {
        let tmp = temp("anywhich-bun-stringbin");
        let r = global_tree(&tmp, &[r#"{"name": "serve", "version": "1.0.0", "bin": "cli.js"}"#]);
        shim(&tmp, "serve");
        let result = r.resolve("serve");
        assert_eq!(result.entries[0].package_name.as_deref(), Some("serve"));
        assert!(r.resolve("other").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn shim_without_matching_record_is_a_stub_hit() {
        let tmp = temp("anywhich-bun-stub");
        let r = global_tree(&tmp, &[r#"{"name": "cowsay", "version": "1.6.0", "bin": {"cowsay": "cli.js"}}"#]);
        shim(&tmp, "orphan");
        let result = r.resolve("orphan");
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].package_name.is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn record_without_shim_is_not_a_hit() {
        let tmp = temp("anywhich-bun-stale");
        let r = global_tree(&tmp, &[r#"{"name": "cowsay", "version": "1.6.0", "bin": {"cowsay": "cli.js"}}"#]);
        assert!(r.resolve("cowsay").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_shim_is_checked_empty() {
        let tmp = temp("anywhich-bun-miss");
        let r = global_tree(&tmp, &[r#"{"name": "cowsay", "version": "1.6.0"}"#]);
        assert!(r.resolve("cowsay").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn bin_detection_reads_both_manifest_shapes() {
        assert!(bins_contain(
            r#"{"name": "cowsay", "bin": {"cowsay": "cli.js"}}"#,
            "cowsay",
            "cowsay"
        ));
        assert!(bins_contain(
            r#"{"name": "cowsay", "bin": {"cowthink": "cli.js"}}"#,
            "cowsay",
            "cowthink"
        ));
        assert!(bins_contain(r#"{"name": "serve", "bin": "cli.js"}"#, "serve", "serve"));
        assert!(!bins_contain(r#"{"name": "serve", "bin": "cli.js"}"#, "serve", "other"));
        assert!(!bins_contain(r#"{"name": "lib"}"#, "lib", "lib"));
    }
}
