use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// deno-installed binaries (`deno install -g`) view of one binary query.
///
/// Each install is a generated sh shim at `~/.deno/bin/<name>` plus a hidden
/// sibling directory `~/.deno/bin/.<name>/` holding a node_modules tree for
/// the backing package. The record is that tree's package.json, so the
/// resolver walks files only, no subprocess. The dependency key, not the
/// shim name, is the package name: a renamed install (`deno install -g -n
/// serve npm:serve-handler`) keeps the real name in the record.
pub struct DenoResolver {
    home: PathBuf,
}

impl Default for DenoResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DenoResolver {
    pub fn new() -> Self {
        let home = std::env::var("DENO_INSTALL_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".deno")
            });
        DenoResolver { home }
    }

    #[cfg(test)]
    fn with_home(home: PathBuf) -> Self {
        DenoResolver { home }
    }
}

impl Resolver for DenoResolver {
    fn name(&self) -> &'static str {
        "deno"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let bin_dir = self.home.join("bin");
        let bin_path = bin_dir.join(binary_name);
        if !is_file(&bin_path) {
            return SourceResult::checked(Vec::new());
        }
        let record_dir = bin_dir.join(format!(".{binary_name}"));
        if let Some((pkg, version)) = package_record(&record_dir) {
            let mut e = ResolvedEntry::new(Source::Deno);
            e.package_name = Some(pkg);
            e.package_version = version;
            e.path = Some(bin_path);
            return SourceResult::checked(vec![e]);
        }
        SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Deno, &bin_path)])
    }
}

/// (package name, version) from a shim's record directory, via the first
/// dependency of `.package.json` and the version in the matching
/// node_modules package.json. Version stays None when only the name is
/// readable.
fn package_record(record_dir: &Path) -> Option<(String, Option<String>)> {
    let deps = fs::read_to_string(record_dir.join("package.json")).ok()?;
    let pkg = json_first_string_field(&deps, "dependencies")?;
    let manifest = fs::read_to_string(
        record_dir
            .join("node_modules")
            .join(&pkg)
            .join("package.json"),
    )
    .ok();
    let version = manifest
        .as_deref()
        .and_then(|m| find_string_field(m, "version"));
    Some((pkg, version))
}

/// Value of the first string entry of a JSON object field, one level deep.
/// Enough for package.json dependency maps; no JSON crate needed for this.
fn json_first_string_field(json: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
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

/// Value of a flat string field like "version": "1.6.0".
fn find_string_field(json: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
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

    fn shim_tree(tmp: &Path, shim: &str, dep: Option<(&str, &str)>) -> DenoResolver {
        let home = tmp.join(".deno");
        let bin = home.join("bin");
        fs::create_dir_all(bin.join(format!(".{shim}"))).unwrap();
        fs::write(bin.join(shim), b"#!/bin/sh\n").unwrap();
        if let Some((pkg, version)) = dep {
            let record = bin.join(format!(".{shim}"));
            fs::write(
                record.join("package.json"),
                format!(r#"{{"dependencies": {{"{pkg}": "*"}}}}"#),
            )
            .unwrap();
            fs::create_dir_all(record.join("node_modules").join(pkg)).unwrap();
            fs::write(
                record.join("node_modules").join(pkg).join("package.json"),
                format!(r#"{{"name": "{pkg}", "version": "{version}"}}"#),
            )
            .unwrap();
        }
        DenoResolver::with_home(home)
    }

    fn temp(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn finds_package_and_version_through_the_hidden_record() {
        let tmp = temp("anywhich-deno-hit");
        let r = shim_tree(&tmp, "cowsay", Some(("cowsay", "1.6.0")));
        let result = r.resolve("cowsay");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].package_name.as_deref(), Some("cowsay"));
        assert_eq!(result.entries[0].package_version.as_deref(), Some("1.6.0"));
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(tmp.join(".deno/bin/cowsay").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn renamed_shims_keep_the_real_package_name() {
        let tmp = temp("anywhich-deno-renamed");
        let r = shim_tree(&tmp, "serve", Some(("serve-handler", "6.1.5")));
        let result = r.resolve("serve");
        assert_eq!(result.entries[0].package_name.as_deref(), Some("serve-handler"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn shim_without_record_dir_is_a_stub_hit() {
        let tmp = temp("anywhich-deno-stub");
        let r = shim_tree(&tmp, "orphan", None);
        let result = r.resolve("orphan");
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].package_name.is_none());
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(tmp.join(".deno/bin/orphan").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_shim_is_checked_empty() {
        let tmp = temp("anywhich-deno-miss");
        let r = shim_tree(&tmp, "other", Some(("other", "1.0.0")));
        assert!(r.resolve("cowsay").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn version_record_with_missing_node_modules_keeps_the_name() {
        let tmp = temp("anywhich-deno-noversion");
        let r = shim_tree(&tmp, "cowsay", Some(("cowsay", "1.6.0")));
        fs::remove_dir_all(tmp.join(".deno/bin/.cowsay/node_modules")).unwrap();
        let result = r.resolve("cowsay");
        assert_eq!(result.entries[0].package_name.as_deref(), Some("cowsay"));
        assert_eq!(result.entries[0].package_version, None);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn json_field_helpers_read_the_fixture_shape() {
        assert_eq!(
            json_first_string_field(r#"{"dependencies": {"cowsay": "*"}}"#, "dependencies"),
            Some("cowsay".to_string())
        );
        assert_eq!(
            find_string_field(r#"{"name": "cowsay", "version": "1.6.0"}"#, "version"),
            Some("1.6.0".to_string())
        );
        assert_eq!(json_first_string_field("{}", "dependencies"), None);
    }
}
