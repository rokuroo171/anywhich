use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// npm global installs (`npm -g`) view of one binary query.
///
/// One `npm ls -g --parseable --long` spawn lists every global package with
/// its install dir and version. Exports come from each package's
/// `package.json` `bin` field (a map or a bare string), which is what npm
/// itself uses; the entry name is the bin key, not the filename, so
/// `"codex": "bin/codex.js"` is found as `codex`.
///
/// A hit needs the package to export the name and npm to report a prefix:
/// the stub is then `prefix/bin/<bin-key>`, the path npm creates. Packages
/// without a prefix (npm broken) are reported without a path.
pub struct NpmResolver;

impl Resolver for NpmResolver {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let out = Command::new("npm")
            .args(["ls", "-g", "--parseable", "--long"])
            .output();
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
        let stdout = String::from_utf8_lossy(&out.stdout);
        let packages = parse_package_list(&stdout);
        let prefix = npm_prefix();

        for (dir, spec) in &packages {
            let short = short_name(dir);
            if package_bin_export(dir, &short, binary_name).is_none() {
                continue;
            };
            let (name, version) = split_spec(spec);
            let mut e = ResolvedEntry::new(Source::Npm);
            e.package_name = Some(name);
            if !version.is_empty() {
                e.package_version = Some(version);
            }
            if let Some(prefix) = &prefix {
                let stub = prefix.join("bin").join(binary_name);
                if is_file(&stub) {
                    e.path = Some(stub);
                }
            }
            return SourceResult::checked(vec![e]);
        }
        SourceResult::checked(Vec::new())
    }
}

/// (install dir, "name@version") pairs from `npm ls -g --parseable --long`.
///
/// The first line is the global root itself ("dir:name@version" where the
/// name@version is the project label, junk like "/usr/lib:lib@"); it is
/// skipped because the root is not a package.
pub fn parse_package_list(s_output: &str) -> Vec<(String, String)> {
    s_output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (dir, spec) = line.split_once(':')?;
            Some((dir.to_string(), spec.to_string()))
        })
        .collect()
}

/// npm's global prefix, from `npm prefix -g`. This is the one source of
/// truth: the `npm_config_prefix`/`PREFIX` env vars are often unset, and
/// guessing (/usr, /usr/local) would invent stubs that do not exist.
fn npm_prefix() -> Option<PathBuf> {
    let out = Command::new("npm").args(["prefix", "-g"]).output().ok()?;
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

/// Short package name from an install dir: the last path segment (for
/// `@scope/name` that is `name`, which is what npm uses for a string-form
/// `bin`).
fn short_name(dir: &str) -> String {
    dir.rsplit('/').find(|s| !s.is_empty()).unwrap_or("").to_string()
}

/// The `bin` field value exporting `binary_name` from this package's
/// `package.json`, if any. Map form exports by entry key; string form
/// exports under the package name, per npm's own rule.
fn package_bin_export(dir: &str, package_name: &str, binary_name: &str) -> Option<String> {
    let text = fs::read_to_string(PathBuf::from(dir).join("package.json")).ok()?;
    let manifest: Manifest = serde_json::from_str(&text).ok()?;
    match manifest.bin {
        Some(Bin::Map(map)) => map.into_iter().find(|(k, _)| *k == binary_name).map(|(_, v)| v),
        Some(Bin::Single(path)) if package_name == binary_name => Some(path),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Manifest {
    bin: Option<Bin>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Bin {
    Map(std::collections::HashMap<String, String>),
    Single(String),
}

/// "name@version" (scoped names may contain "@") to (name, version); a
/// missing version leaves it empty.
fn split_spec(spec: &str) -> (String, String) {
    match spec.rsplit_once('@') {
        Some((n, v)) if !n.is_empty() => (n.to_string(), v.to_string()),
        _ => (spec.to_string(), String::new()),
    }
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_FIXTURE: &str = "\
/usr/lib:lib@
/usr/lib/node_modules/@google/gemini-cli:@google/gemini-cli@0.49.0
/usr/lib/node_modules/@openai/codex:@openai/codex@0.154.0
/usr/lib/node_modules/corepack:corepack@0.35.0
";

    #[test]
    fn parses_dirs_and_specs_skipping_root() {
        let pkgs = parse_package_list(LIST_FIXTURE);
        assert_eq!(pkgs.len(), 3);
        assert_eq!(pkgs[0].0, "/usr/lib/node_modules/@google/gemini-cli");
        assert_eq!(pkgs[0].1, "@google/gemini-cli@0.49.0");
        assert_eq!(pkgs[2].1, "corepack@0.35.0");
    }

    #[test]
    fn empty_output_yields_nothing() {
        assert!(parse_package_list("").is_empty());
    }

    #[test]
    fn spec_splits_on_last_at() {
        let (name, version) = split_spec("@google/gemini-cli@0.49.0");
        assert_eq!(name, "@google/gemini-cli");
        assert_eq!(version, "0.49.0");
        let (name, version) = split_spec("corepack");
        assert_eq!(name, "corepack");
        assert_eq!(version, "");
    }

    #[test]
    fn bin_map_finds_entry_by_key() {
        let tmp = std::env::temp_dir().join("anywhich-npm-map");
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("package.json"),
            r#"{"name":"x","bin":{"codex":"bin/codex.js","other":"bin/o.js"}}"#,
        )
        .unwrap();
        let e = package_bin_export(pkg.to_str().unwrap(), "x", "codex");
        assert_eq!(e.as_deref(), Some("bin/codex.js"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn bin_string_exports_under_package_name() {
        let tmp = std::env::temp_dir().join("anywhich-npm-str");
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("package.json"), r#"{"name":"x","bin":"bin/gemini.js"}"#).unwrap();
        assert_eq!(
            package_bin_export(pkg.to_str().unwrap(), "x", "x").as_deref(),
            Some("bin/gemini.js")
        );
        assert!(package_bin_export(pkg.to_str().unwrap(), "x", "gemini").is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn short_name_takes_last_segment() {
        assert_eq!(short_name("/usr/lib/node_modules/corepack"), "corepack");
        assert_eq!(short_name("/usr/lib/node_modules/@openai/codex"), "codex");
    }

    #[test]
    fn missing_manifest_exports_nothing() {
        assert!(package_bin_export("/nonexistent-anywhich", "x", "tool").is_none());
    }
}
