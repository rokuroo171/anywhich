use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// pnpm global installs (`pnpm add -g`) view of one binary query.
///
/// Shims are script copies in `PNPM_HOME/bin`, one per package binary, and
/// the record for each `pnpm add -g` run is a hashed project dir under
/// `PNPM_HOME/global/<layout>/<hash>` holding a plain node_modules tree, so
/// `<pkg>/package.json` carries the name and version. Both levels below
/// `global/` are walked instead of hardcoded so a layout version bump does
/// not orphan the resolver. Shims without a record (pnpm's own `pn` family)
/// stay stub hits.
pub struct PnpmResolver {
    home: PathBuf,
}

impl Default for PnpmResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl PnpmResolver {
    pub fn new() -> Self {
        let home = std::env::var("PNPM_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".local/share/pnpm")
            });
        PnpmResolver { home }
    }

    #[cfg(test)]
    fn with_home(home: PathBuf) -> Self {
        PnpmResolver { home }
    }
}

impl Resolver for PnpmResolver {
    fn name(&self) -> &'static str {
        "pnpm"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let shim = self.home.join("bin").join(binary_name);
        if !is_file(&shim) {
            return SourceResult::checked(Vec::new());
        }
        let mut records: Vec<(String, Option<String>)> = package_records(&self.home, binary_name);
        records.dedup();
        if records.is_empty() {
            return SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Pnpm, &shim)]);
        }
        let entries = records
            .into_iter()
            .map(|(pkg, version)| {
                let mut e = ResolvedEntry::new(Source::Pnpm);
                e.package_name = Some(pkg);
                e.package_version = version;
                e.path = Some(shim.clone());
                e
            })
            .collect();
        SourceResult::checked(entries)
    }
}

/// (name, version) per `<pkg>/package.json` record for `binary_name`, from
/// every `global/<any>/<any>/node_modules/<name>/` tree. The version field
/// stays None when the manifest lacks one.
fn package_records(home: &Path, binary_name: &str) -> Vec<(String, Option<String>)> {
    let global = home.join("global");
    let mut found = Vec::new();
    let Ok(versions) = fs::read_dir(&global) else {
        return found;
    };
    for version in versions.filter_map(|e| e.ok()).map(|e| e.path()) {
        let Ok(projects) = fs::read_dir(&version) else {
            continue;
        };
        for project in projects.filter_map(|e| e.ok()).map(|e| e.path()) {
            let manifest = project
                .join("node_modules")
                .join(binary_name)
                .join("package.json");
            let Ok(text) = fs::read_to_string(&manifest) else {
                continue;
            };
            let name = manifest
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or(binary_name);
            found.push((name.to_string(), json_string_field(&text, "version")));
        }
    }
    found
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

    fn global_tree(tmp: &Path, packages: &[(&str, &str)]) -> PnpmResolver {
        let home = tmp.join("pnpm");
        let project = home.join("global/v11/5ce9-hash-0");
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::create_dir_all(project.join("node_modules")).unwrap();
        for (pkg, version) in packages {
            let dir = project.join("node_modules").join(pkg);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("package.json"),
                format!(r#"{{"name": "{pkg}", "version": "{version}"}}"#),
            )
            .unwrap();
        }
        PnpmResolver::with_home(home)
    }

    fn shim(tmp: &Path, name: &str) {
        fs::write(tmp.join("pnpm/bin").join(name), b"#!/bin/sh\n").unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn finds_package_version_and_shim() {
        let tmp = temp("anywhich-pnpm-hit");
        let r = global_tree(&tmp, &[("cowsay", "1.6.0")]);
        shim(&tmp, "cowsay");
        let result = r.resolve("cowsay");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].package_name.as_deref(), Some("cowsay"));
        assert_eq!(result.entries[0].package_version.as_deref(), Some("1.6.0"));
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(tmp.join("pnpm/bin/cowsay").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn shim_without_record_is_a_stub_hit() {
        let tmp = temp("anywhich-pnpm-stub");
        let r = global_tree(&tmp, &[("cowsay", "1.6.0")]);
        shim(&tmp, "pn");
        let result = r.resolve("pn");
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].package_name.is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn record_without_shim_is_not_a_hit() {
        let tmp = temp("anywhich-pnpm-stale");
        let r = global_tree(&tmp, &[("left-pad", "1.3.0")]);
        assert!(r.resolve("left-pad").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn record_dirs_at_other_layout_versions_are_found() {
        let tmp = temp("anywhich-pnpm-v12");
        let r = global_tree(&tmp, &[("cowsay", "1.6.0")]);
        let moved = tmp.join("pnpm/global/v12/abcd-hash-1");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(
            tmp.join("pnpm/global/v11/5ce9-hash-0"),
            &moved,
        )
        .unwrap();
        shim(&tmp, "cowsay");
        let result = r.resolve("cowsay");
        assert_eq!(result.entries[0].package_version.as_deref(), Some("1.6.0"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_shim_is_checked_empty() {
        let tmp = temp("anywhich-pnpm-miss");
        let r = global_tree(&tmp, &[("cowsay", "1.6.0")]);
        assert!(r.resolve("cowsay").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn version_field_helper_reads_the_manifest_shape() {
        assert_eq!(
            json_string_field(r#"{"name": "cowsay", "version": "1.6.0"}"#, "version").as_deref(),
            Some("1.6.0")
        );
        assert_eq!(json_string_field("{}", "version"), None);
    }
}
