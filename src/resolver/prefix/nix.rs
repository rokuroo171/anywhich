use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// Nix store and profile view of one binary query.
///
/// Lookup order: active profiles (from $NIX_PROFILES, falling back to the
/// user's default profile), then older profile generations, then a scan of
/// the store itself. Entries dedupe by resolved store path, so a binary in
/// the current profile is not re-reported from the generation that had it
/// or from a raw store scan. Provenance comes from the store path name
/// (hash-name-version), so no manifest parsing is needed.
pub struct NixResolver;

impl Resolver for NixResolver {
    fn name(&self) -> &'static str {
        "nix"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        let profiles = active_profiles();
        let generations_root = PathBuf::from("/nix/var/nix/profiles");
        let store_root = PathBuf::from("/nix/store");

        let store_missing = !store_root.is_dir();
        let gens_missing = !generations_root.is_dir();
        if store_missing && gens_missing && profiles.is_empty() {
            return SourceResult::unavailable("nix not found");
        }

        let mut entries = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        scan_profiles(&profiles, binary_name, &mut entries, &mut seen);
        if generations_root.is_dir() {
            scan_generations(&generations_root, binary_name, &mut entries, &mut seen);
        }
        if store_root.is_dir() {
            scan_store(&store_root, binary_name, &mut entries, &mut seen);
        }
        SourceResult::checked(entries)
    }
}

/// Active profile directories, in precedence order: $NIX_PROFILES when set,
/// otherwise the user's default profile.
fn active_profiles() -> Vec<PathBuf> {
    if let Ok(list) = env::var("NIX_PROFILES") {
        return list
            .split_whitespace()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect();
    }
    match env::var("HOME") {
        Ok(home) => {
            let p = PathBuf::from(home).join(".nix-profile");
            if p.is_dir() {
                vec![p]
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

/// Parse "hash-name" or "hash-name-version" from a nix store directory name.
/// The hash is 32 lowercase base-32 characters. The version, when present,
/// is the trailing hyphen-separated segment that starts with a digit.
pub fn parse_store_entry(component: &str) -> Option<(String, Option<String>)> {
    let (hash, rest) = component.split_once('-')?;
    if hash.len() != 32
        || !hash
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
    {
        return None;
    }
    let parts: Vec<&str> = rest.split('-').collect();
    let split = parts
        .iter()
        .enumerate()
        .filter(|(i, part)| *i > 0 && part.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|(i, _)| i)
        .next_back();
    match split {
        Some(i) => Some((parts[..i].join("-"), Some(parts[i..].join("-")))),
        None => Some((rest.to_string(), None)),
    }
}

/// The store directory component of a resolved path (the part after "store"),
/// so provenance survives both real /nix/store paths and fixture roots.
fn store_component(resolved: &Path) -> Option<String> {
    let mut comps = resolved.components();
    while let Some(c) = comps.next() {
        if c.as_os_str() == "store" {
            return comps.next().map(|c| c.as_os_str().to_string_lossy().to_string());
        }
    }
    None
}

/// Provenance for a profile/generation/store binary, resolved through
/// symlinks to its store path.
fn provenance(bin_path: &Path) -> (Option<String>, Option<String>) {
    let resolved = fs::canonicalize(bin_path).ok();
    let comp = resolved
        .as_deref()
        .and_then(store_component)
        .or_else(|| {
            bin_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
        });
    match comp.and_then(|c| parse_store_entry(&c)) {
        Some((name, version)) => (Some(name), version),
        None => (None, None),
    }
}

fn dedupe_key(bin_path: &Path) -> Option<String> {
    let resolved = fs::canonicalize(bin_path).ok()?;
    store_component(&resolved).or_else(|| Some(resolved.to_string_lossy().to_string()))
}

fn scan_profiles(
    profiles: &[PathBuf],
    binary_name: &str,
    entries: &mut Vec<ResolvedEntry>,
    seen: &mut Vec<String>,
) {
    for (idx, profile) in profiles.iter().enumerate() {
        let bin = profile.join("bin").join(binary_name);
        if !bin.exists() {
            continue;
        }
        if let Some(key) = dedupe_key(&bin) {
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
        }
        let (name, version) = provenance(&bin);
        let mut e = ResolvedEntry::new(Source::Nix);
        e.path = Some(bin);
        e.package_name = name;
        e.package_version = version;
        e.rank = idx;
        entries.push(e);
    }
}

fn scan_generations(
    gens_root: &Path,
    binary_name: &str,
    entries: &mut Vec<ResolvedEntry>,
    seen: &mut Vec<String>,
) {
    let Ok(dirs) = fs::read_dir(gens_root) else {
        return;
    };
    let mut gens: Vec<PathBuf> = dirs
        .flatten()
        .map(|d| d.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("profile") && n.ends_with("-link")
                })
                .unwrap_or(false)
        })
        .collect();
    gens.sort();
    for gen in gens {
        scan_profiles(&[gen], binary_name, entries, seen);
    }
}

fn scan_store(
    store_root: &Path,
    binary_name: &str,
    entries: &mut Vec<ResolvedEntry>,
    seen: &mut Vec<String>,
) {
    let Ok(dirs) = fs::read_dir(store_root) else {
        return;
    };
    let versioned = format!("{binary_name}-");
    let mut hits: Vec<PathBuf> = dirs
        .flatten()
        .map(|d| d.path())
        .filter(|p| {
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
                return false;
            };
            let Some((_, rest)) = name.split_once('-') else {
                return false;
            };
            (rest == binary_name || rest.starts_with(&versioned))
                && parse_store_entry(&name).is_some()
                && p.join("bin").join(binary_name).exists()
        })
        .collect();
    hits.sort();
    for dir in hits {
        let bin = dir.join("bin").join(binary_name);
        if let Some(key) = dedupe_key(&bin) {
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
        }
        let comp = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (name, version) = parse_store_entry(&comp).unwrap_or((binary_name.to_string(), None));
        let mut e = ResolvedEntry::new(Source::Nix);
        e.path = Some(bin);
        e.package_name = Some(name);
        e.package_version = version;
        entries.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdefghijklmnopqrstuv";

    #[test]
    fn parses_name_and_version() {
        let comp = format!("{HASH}-python3-3.12.6");
        assert_eq!(
            parse_store_entry(&comp),
            Some(("python3".to_string(), Some("3.12.6".to_string())))
        );
    }

    #[test]
    fn parses_name_without_version() {
        let comp = format!("{HASH}-libfoo2");
        assert_eq!(
            parse_store_entry(&comp),
            Some(("libfoo2".to_string(), None))
        );
    }

    #[test]
    fn rejects_bad_components() {
        assert!(parse_store_entry("short-hash-python").is_none());
        assert!(parse_store_entry("0123456789ABCDEFGHIJKLMNOPQRSTUV-python").is_none());
        assert!(parse_store_entry("nodash").is_none());
    }

    #[test]
    fn profiles_yield_entries_with_provenance_and_rank() {
        let tmp = env::temp_dir().join("anywhich-nix-profiles");
        let store = tmp.join("store").join(format!("{HASH}-python3-3.12.6"));
        let profile = tmp.join("profile");
        fs::create_dir_all(store.join("bin")).unwrap();
        fs::create_dir_all(profile.join("bin")).unwrap();
        fs::write(store.join("bin").join("python3"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&store, profile.join("bin").join("python3")).unwrap();

        let mut entries = Vec::new();
        let mut seen = Vec::new();
        scan_profiles(&[profile.clone()], "python3", &mut entries, &mut seen);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].package_name.as_deref(), Some("python3"));
        assert_eq!(entries[0].package_version.as_deref(), Some("3.12.6"));
        assert!(!entries[0].active);
        assert_eq!(entries[0].rank, 0);
        assert_eq!(entries[0].path, Some(profile.join("bin").join("python3")));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn same_store_path_in_two_profiles_is_reported_once() {
        let tmp = env::temp_dir().join("anywhich-nix-dedupe");
        let store = tmp.join("store").join(format!("{HASH}-tool-1.0"));
        let p1 = tmp.join("p1");
        let p2 = tmp.join("p2");
        fs::create_dir_all(store.join("bin")).unwrap();
        for p in [&p1, &p2] {
            fs::create_dir_all(p.join("bin")).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&store, p.join("bin").join("tool")).unwrap();
        }
        fs::write(store.join("bin").join("tool"), b"#!/bin/sh\n").unwrap();

        let mut entries = Vec::new();
        let mut seen = Vec::new();
        scan_profiles(&[p1, p2], "tool", &mut entries, &mut seen);
        assert_eq!(entries.len(), 1);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn generations_are_scanned_and_deduped_by_store_path() {
        let tmp = env::temp_dir().join("anywhich-nix-gens");
        let gens = tmp.join("profiles");
        let s1 = tmp.join("store").join(format!("{HASH}-tool-1.0"));
        let s2 = tmp.join("store").join(format!("{HASH}-tool-2.0"));
        fs::create_dir_all(s1.join("bin")).unwrap();
        fs::create_dir_all(s2.join("bin")).unwrap();
        for (root, s) in [(&gens.join("profile-7-link"), &s1), (&gens.join("profile-8-link"), &s2)] {
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::write(s.join("bin").join("tool"), b"#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(s, root.join("bin").join("tool")).unwrap();
        }

        let mut entries = Vec::new();
        let mut seen = Vec::new();
        scan_generations(&gens, "tool", &mut entries, &mut seen);
        assert_eq!(entries.len(), 2);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn store_scan_matches_name_and_versioned_dirs_only() {
        let tmp = env::temp_dir().join("anywhich-nix-store");
        let store = tmp.join("store");
        for dir in [
            format!("{HASH}-tool-1.0"),
            format!("{HASH}-tool-2.1"),
            format!("{HASH}-tooling-9.9"),
            format!("{HASH}-other-1.0"),
        ] {
            fs::create_dir_all(store.join(&dir).join("bin")).unwrap();
        }
        fs::write(store.join(format!("{HASH}-tool-1.0")).join("bin").join("tool"), b"").unwrap();
        fs::write(store.join(format!("{HASH}-tool-2.1")).join("bin").join("tool"), b"").unwrap();
        fs::write(store.join(format!("{HASH}-other-1.0")).join("bin").join("other"), b"").unwrap();

        let mut entries = Vec::new();
        let mut seen = Vec::new();
        scan_store(&store, "tool", &mut entries, &mut seen);

        assert_eq!(entries.len(), 2);
        let versions: Vec<Option<&String>> = entries.iter().map(|e| e.package_version.as_ref()).collect();
        assert_eq!(
            versions,
            vec![Some(&"1.0".to_string()), Some(&"2.1".to_string())]
        );

        fs::remove_dir_all(&tmp).unwrap();
    }
}
