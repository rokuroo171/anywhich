use std::fs;
use std::path::Path;

use crate::entry::ResolvedEntry;

/// PATH hits annotated with resolver provenance, plus resolver entries no
/// PATH hit claimed (installed off PATH, or sources without a path such as
/// Flatpak app ids).
pub struct Merged {
    pub path_hits: Vec<ResolvedEntry>,
    pub others: Vec<ResolvedEntry>,
}

/// Identity of a file across symlinked directories (usrmerge, nix profiles):
/// the canonical path when resolvable, the literal path otherwise.
pub(crate) fn key(path: &Path) -> String {
    match fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

pub fn merge(mut path_hits: Vec<ResolvedEntry>, results: &[(String, crate::resolver::SourceResult)]) -> Merged {
    let mut others = Vec::new();
    for (_, result) in results {
        for entry in &result.entries {
            let Some(ref entry_path) = entry.path else {
                others.push(entry.clone());
                continue;
            };
            let entry_key = key(entry_path);
            let mut claimed = false;
            for hit in path_hits.iter_mut() {
                let Some(ref hit_path) = hit.path else {
                    continue;
                };
                if key(hit_path) == entry_key {
                    hit.source = entry.source;
                    hit.package_name = entry.package_name.clone();
                    hit.package_version = entry.package_version.clone();
                    claimed = true;
                    break;
                }
            }
            if !claimed {
                others.push(entry.clone());
            }
        }
    }
    Merged { path_hits, others }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Source;
    use crate::resolver::SourceResult;
    use std::path::PathBuf;

    fn path_entry(source: Source, path: &Path) -> ResolvedEntry {
        let mut e = ResolvedEntry::new(source);
        e.path = Some(path.to_path_buf());
        e
    }

    // Unix-only: the claim path resolves profile symlinks, and Windows
    // symlink creation needs privileges the test must not assume. The
    // unclaimed and pathless cases below run everywhere.
    #[cfg(unix)]
    #[test]
    fn claims_path_hits_by_canonical_path() {
        use std::env;
        use std::fs;

        let tmp = env::temp_dir().join("anywhich-merge-claim");
        let store = tmp.join("store").join("0123456789abcdefghijklmnopqrstuv-tool-1.0");
        let profile = tmp.join("profile");
        fs::create_dir_all(store.join("bin")).unwrap();
        fs::create_dir_all(profile.join("bin")).unwrap();
        fs::write(store.join("bin").join("tool"), b"").unwrap();
        std::os::unix::fs::symlink(store.join("bin").join("tool"), profile.join("bin").join("tool"))
            .unwrap();

        let hits = vec![path_entry(Source::Path, &profile.join("bin").join("tool"))];
        let mut owned = ResolvedEntry::new(Source::Nix);
        owned.path = Some(store.join("bin").join("tool"));
        owned.package_name = Some("tool".to_string());
        owned.package_version = Some("1.0".to_string());
        let results = vec![("nix".to_string(), SourceResult::checked(vec![owned]))];

        let merged = merge(hits, &results);
        assert_eq!(merged.path_hits.len(), 1);
        assert_eq!(merged.path_hits[0].source, Source::Nix);
        assert_eq!(merged.path_hits[0].package_name.as_deref(), Some("tool"));
        assert!(merged.others.is_empty());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn unclaimed_pathful_entries_become_others() {
        let hits = vec![path_entry(Source::Path, Path::new("/nonexistent-a/bin/tool"))];
        let mut off_path = ResolvedEntry::new(Source::Pacman);
        off_path.path = Some(PathBuf::from("/nonexistent-b/bin/tool"));
        off_path.package_name = Some("tool".to_string());
        let results = vec![("pacman".to_string(), SourceResult::checked(vec![off_path]))];

        let merged = merge(hits, &results);
        assert_eq!(merged.path_hits[0].source, Source::Path);
        assert_eq!(merged.others.len(), 1);
        assert_eq!(merged.others[0].source, Source::Pacman);
    }

    #[test]
    fn pathless_entries_become_others() {
        let mut app = ResolvedEntry::new(Source::Flatpak);
        app.package_name = Some("com.spotify.Client".to_string());
        let results = vec![("flatpak".to_string(), SourceResult::checked(vec![app]))];

        let merged = merge(Vec::new(), &results);
        assert_eq!(merged.others.len(), 1);
        assert_eq!(merged.others[0].package_name.as_deref(), Some("com.spotify.Client"));
    }
}
