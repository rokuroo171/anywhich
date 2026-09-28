use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// uv tool installs (`uv tool install`) view of one binary query.
///
/// Each tool lives in a venv under `~/.local/share/uv/tools/<tool>` with its
/// shims symlinked into `~/.local/bin`. The per-tool `uv-receipt.toml` maps
/// each entrypoint (shim name) to the package it comes from, and the version
/// sits in the site-packages dist-info directory name, so the resolver walks
/// files only, no subprocess.
pub struct UvResolver {
    tools: PathBuf,
}

impl Default for UvResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl UvResolver {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let tools = std::env::var("UV_TOOL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(&home).join(".local/share/uv/tools"));
        UvResolver { tools }
    }

    #[cfg(test)]
    fn with_tools(tools: PathBuf) -> Self {
        UvResolver { tools }
    }
}

impl Resolver for UvResolver {
    fn name(&self) -> &'static str {
        "uv"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let dirs = match fs::read_dir(&self.tools) {
            Ok(d) => d,
            Err(_) => return SourceResult::checked(Vec::new()),
        };
        let mut tools: Vec<PathBuf> = dirs.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        tools.sort();
        let mut entries = Vec::new();
        for tool in tools {
            if !tool.is_dir() {
                continue;
            }
            let Ok(receipt) = fs::read_to_string(tool.join("uv-receipt.toml")) else {
                continue;
            };
            for line in entrypoint_lines(&receipt, binary_name) {
                let Some(pkg) = toml_string_field(line, "from") else {
                    continue;
                };
                let shim = match toml_string_field(line, "install-path") {
                    Some(p) if is_file(Path::new(&p)) => PathBuf::from(p),
                    _ => continue,
                };
                let version = dist_info_version(&tool, &pkg);
                let mut e = ResolvedEntry::new(Source::Uv);
                e.package_name = Some(pkg);
                e.package_version = version;
                e.path = Some(shim);
                entries.push(e);
            }
        }
        SourceResult::checked(entries)
    }
}

/// Entry point lines of a `uv-receipt.toml` whose `name` is the queried
/// binary, from blocks like:
/// ```toml
/// entrypoints = [
///     { name = "pycowsay", install-path = "/home/u/.local/bin/pycowsay", from = "pycowsay" },
/// ]
/// ```
fn entrypoint_lines<'a>(receipt: &'a str, binary_name: &str) -> Vec<&'a str> {
    let wanted = format!("name = \"{binary_name}\"");
    receipt
        .lines()
        .filter(|l| l.contains(&wanted) && l.contains("from = "))
        .collect()
}

/// Value of a `key = "value"` occurrence in one TOML line.
fn toml_string_field(line: &str, key: &str) -> Option<String> {
    let marker = format!("{key} = \"");
    let start = line.find(&marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    let value = &rest[..end];
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Version of `pkg` from `<tool>/lib/python*/site-packages/<pkg>-<ver>.
/// dist-info`, the directory name being the installed record.
fn dist_info_version(tool: &Path, pkg: &str) -> Option<String> {
    let lib = fs::read_dir(tool.join("lib")).ok()?;
    for py in lib.filter_map(|e| e.ok()).map(|e| e.path()) {
        let Ok(sites) = fs::read_dir(py.join("site-packages")) else {
            continue;
        };
        let prefix = format!("{pkg}-");
        for entry in sites.filter_map(|e| e.ok()).map(|e| e.path()) {
            let Some(name) = entry.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(version) = name
                .strip_prefix(&prefix)
                .and_then(|v| v.strip_suffix(".dist-info"))
            else {
                continue;
            };
            if version.is_empty() {
                continue;
            }
            return Some(version.to_string());
        }
    }
    None
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECEIPT: &str = "\
[tool]
requirements = [{ name = \"pycowsay\" }]
entrypoints = [
    { name = \"pycowsay\", install-path = \"/tools-root/.local/bin/pycowsay\", from = \"pycowsay\" },
]
";

    fn write_receipt(tool: &Path, name: &str, shim: &Path, from: &str) {
        fs::write(
            tool.join("uv-receipt.toml"),
            format!(
                "[tool]\nrequirements = [{{ name = \"{from}\" }}]\nentrypoints = [\n    {{ name = \"{name}\", install-path = \"{}\", from = \"{from}\" }},\n]\n",
                shim.display()
            ),
        )
        .unwrap();
    }

    fn tool_tree(tmp: &Path, version: Option<&str>) -> (UvResolver, PathBuf) {
        let tools = tmp.join("tools");
        let bin = tmp.join(".local/bin");
        let tool = tools.join("pycowsay");
        fs::create_dir_all(tool.join("lib/python3.13/site-packages")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        if let Some(v) = version {
            fs::create_dir_all(
                tool.join("lib/python3.13/site-packages")
                    .join(format!("pycowsay-{v}.dist-info")),
            )
            .unwrap();
        }
        let shim = bin.join("pycowsay");
        fs::write(&shim, b"#!/usr/bin/env python\n").unwrap();
        write_receipt(&tool, "pycowsay", &shim, "pycowsay");
        (UvResolver::with_tools(tools), shim)
    }

    fn temp(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn finds_package_version_and_shim_through_the_receipt() {
        let tmp = temp("anywhich-uv-hit");
        let (r, shim) = tool_tree(&tmp, Some("0.0.0.2"));
        let result = r.resolve("pycowsay");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].package_name.as_deref(), Some("pycowsay"));
        assert_eq!(result.entries[0].package_version.as_deref(), Some("0.0.0.2"));
        assert_eq!(result.entries[0].path.as_deref(), Some(shim.as_path()));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn entrypoint_name_may_differ_from_the_package() {
        let tmp = temp("anywhich-uv-renamed");
        let (r, shim) = tool_tree(&tmp, None);
        write_receipt(&tmp.join("tools/pycowsay"), "say", &shim, "pycowsay");
        let result = r.resolve("say");
        assert_eq!(result.entries[0].package_name.as_deref(), Some("pycowsay"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_dist_info_keeps_the_name_only() {
        let tmp = temp("anywhich-uv-noversion");
        let (r, _) = tool_tree(&tmp, None);
        let result = r.resolve("pycowsay");
        assert_eq!(result.entries[0].package_version, None);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_shim_path_is_not_a_hit() {
        let tmp = temp("anywhich-uv-stale");
        let (r, shim) = tool_tree(&tmp, Some("0.0.0.2"));
        fs::remove_file(&shim).unwrap();
        assert!(r.resolve("pycowsay").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn other_binaries_yield_nothing() {
        let tmp = temp("anywhich-uv-miss");
        let (r, _) = tool_tree(&tmp, Some("0.0.0.2"));
        assert!(r.resolve("pydoc").entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn field_and_entrypoint_helpers_read_the_receipt_shape() {
        let line = "    { name = \"pycowsay\", install-path = \"/u/.local/bin/pycowsay\", from = \"pycowsay\" },";
        assert_eq!(
            toml_string_field(line, "install-path").as_deref(),
            Some("/u/.local/bin/pycowsay")
        );
        assert_eq!(entrypoint_lines(RECEIPT, "pycowsay").len(), 1);
        assert!(entrypoint_lines(RECEIPT, "pydoc").is_empty());
    }
}
