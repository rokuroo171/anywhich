use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// cargo-installed binaries (`cargo install`) view of one binary query.
///
/// cargo keeps an install record at `~/.cargo/.crates.toml` mapping each
/// installed binary back to its crate and version. Reading that record is
/// cheaper and more precise than spawning cargo, so this resolver does no
/// process spawn at all. A hit needs the record entry and the actual binary
/// in `~/.cargo/bin`; a stale record without the binary is not a hit.
pub struct CargoResolver {
    home: PathBuf,
}

impl Default for CargoResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CargoResolver {
    pub fn new() -> Self {
        let home = std::env::var("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".cargo")
            });
        CargoResolver { home }
    }

    #[cfg(test)]
    fn with_home(home: PathBuf) -> Self {
        CargoResolver { home }
    }
}

impl Resolver for CargoResolver {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let bin_dir = self.home.join("bin");
        let Some(bin_path) = existing_binary(&bin_dir, binary_name) else {
            return SourceResult::checked(Vec::new());
        };
        if let Ok(text) = fs::read_to_string(self.home.join(".crates.toml")) {
            if let Some((crate_name, version)) = crate_for_binary(&text, binary_name) {
                let mut e = ResolvedEntry::new(Source::Cargo);
                e.package_name = Some(crate_name);
                e.package_version = Some(version);
                e.path = Some(bin_path);
                return SourceResult::checked(vec![e]);
            }
        }
        SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Cargo, &bin_path)])
    }
}

/// (crate name, version) owning `binary_name` from `.crates.toml` content.
///
/// The record stores arrays of binary paths keyed by "crate version (source)":
/// ```toml
/// [ripgrep 14.1.0 registry+https://github.com/rust-lang/crates.io-index]
/// "rg" = "14.1.0"
/// ```
fn crate_for_binary(text: &str, binary_name: &str) -> Option<(String, String)> {
    let wanted = format!("\"{binary_name}\"");
    for section in text.split('[') {
        let Some((header, body)) = section.split_once(']') else {
            continue;
        };
        if !body.lines().any(|l| l.trim_start().starts_with(&wanted)) {
            continue;
        }
        let header = header.trim_end();
        let mut parts = header.split_whitespace();
        let name = parts.next()?.to_string();
        let version = parts.next()?.to_string();
        return Some((name, version));
    }
    None
}

/// The binary in `bin_dir` under the name as typed, else the exe spelling,
/// because cargo writes `name.exe` on Windows and the extra stat is free
/// where it never matches.
fn existing_binary(bin_dir: &Path, binary_name: &str) -> Option<PathBuf> {
    let typed = bin_dir.join(binary_name);
    if is_file(&typed) {
        return Some(typed);
    }
    let exe = bin_dir.join(format!("{binary_name}.exe"));
    if is_file(&exe) {
        return Some(exe);
    }
    None
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD_FIXTURE: &str = "\
[ripgrep 14.1.0 registry+https://github.com/rust-lang/crates.io-index]
\"rg\" = \"14.1.0\"

[fd-find 10.0.0 registry+https://github.com/rust-lang/crates.io-index]
\"fd\" = \"10.0.0\"

[bottom 0.11.0 registry+https://github.com/rust-lang/crates.io-index]
\"btm\" = \"0.11.0\"
";

    fn resolver_with_record(tmp: &Path, text: Option<&str>) -> CargoResolver {
        let home = tmp.join(".cargo");
        fs::create_dir_all(home.join("bin")).unwrap();
        if let Some(t) = text {
            fs::write(home.join(".crates.toml"), t).unwrap();
        }
        CargoResolver::with_home(home)
    }

    fn write_bin(home: &Path, name: &str) {
        let bin = home.join(".cargo/bin").join(name);
        fs::write(&bin, b"").unwrap();
    }

    #[test]
    fn finds_crate_and_version_for_recorded_binary() {
        let tmp = std::env::temp_dir().join("anywhich-cargo-hit");
        let _ = fs::remove_dir_all(&tmp);
        let r = resolver_with_record(&tmp, Some(RECORD_FIXTURE));
        write_bin(&tmp, "rg");
        let result = r.resolve("rg");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].package_name.as_deref(), Some("ripgrep"));
        assert_eq!(result.entries[0].package_version.as_deref(), Some("14.1.0"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    // Windows: cargo writes name.exe, so the probe must try the exe
    // spelling. Typed name still wins when both exist.
    #[test]
    fn exe_spelling_is_probed_after_the_typed_name() {
        let tmp = std::env::temp_dir().join("anywhich-cargo-exe");
        let _ = fs::remove_dir_all(&tmp);
        let r = resolver_with_record(&tmp, Some(RECORD_FIXTURE));
        fs::write(tmp.join(".cargo/bin").join("rg.exe"), b"").unwrap();
        let result = r.resolve("rg");
        assert_eq!(result.entries[0].package_name.as_deref(), Some("ripgrep"));
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(tmp.join(".cargo/bin/rg.exe").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn binary_without_record_entry_is_a_stub_hit() {
        let tmp = std::env::temp_dir().join("anywhich-cargo-stub");
        let _ = fs::remove_dir_all(&tmp);
        let r = resolver_with_record(&tmp, Some(RECORD_FIXTURE));
        write_bin(&tmp, "rg");
        fs::write(tmp.join(".cargo/bin").join("orphan"), b"").unwrap();
        let result = r.resolve("orphan");
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].package_name.is_none());
        assert_eq!(
            result.entries[0].path.as_deref(),
            Some(tmp.join(".cargo/bin/orphan").as_path())
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn record_without_binary_is_not_a_hit() {
        let tmp = std::env::temp_dir().join("anywhich-cargo-stale");
        let _ = fs::remove_dir_all(&tmp);
        let r = resolver_with_record(&tmp, Some(RECORD_FIXTURE));
        let result = r.resolve("rg");
        assert!(result.entries.is_empty());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn missing_record_is_checked_empty() {
        let tmp = std::env::temp_dir().join("anywhich-cargo-norecord");
        let _ = fs::remove_dir_all(&tmp);
        let r = resolver_with_record(&tmp, None);
        write_bin(&tmp, "rg");
        let result = r.resolve("rg");
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].package_name.is_none());
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn parse_finds_owner_across_sections() {
        let (name, version) = crate_for_binary(RECORD_FIXTURE, "fd").unwrap();
        assert_eq!(name, "fd-find");
        assert_eq!(version, "10.0.0");
    }

    #[test]
    fn parse_returns_none_for_unknown_binary() {
        assert!(crate_for_binary(RECORD_FIXTURE, "nope").is_none());
    }
}
