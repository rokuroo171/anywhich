use crate::entry::{ResolvedEntry, Source};
use crate::merge;

/// One reason line per PATH hit, same order, explaining why it ranked there.
///
/// Rules, first match wins, at most one contrast clause per entry:
/// - active: "first match in PATH", plus the empty-directory count when rank > 0
/// - canonical-equal to the winner: "same file as 1" (beats any version clause)
/// - both versions known and differing: "version differs: A (src) vs B (src)"
/// - both versions known and equal: "same version, different install"
/// - otherwise, source still Path after merge: "no known source claims this file"
/// - otherwise: "shadowed by 1"
pub fn reasons(path_hits: &[ResolvedEntry]) -> Vec<String> {
    let winner = path_hits.first();
    let winner_key = winner.and_then(|e| e.path.as_deref()).map(merge::key);
    let winner_version = winner.and_then(|e| e.package_version.as_deref());
    let winner_source = winner.map(|e| e.source);

    path_hits
        .iter()
        .map(|hit| {
            if hit.active {
                return match hit.rank {
                    0 => "first match in PATH".to_string(),
                    n => format!("first match in PATH; {n} earlier PATH directories had no match"),
                };
            }
            let shadowed_by = "shadowed by 1".to_string();
            let hit_key = hit.path.as_deref().map(merge::key);
            if let (Some(wk), Some(hk)) = (winner_key.as_deref(), hit_key.as_deref()) {
                if wk == hk {
                    return format!("{shadowed_by}; same file as 1");
                }
            }
            match (winner_version, hit.package_version.as_deref()) {
                (Some(wv), Some(hv)) if wv != hv => format!(
                    "{shadowed_by}; version differs: {wv} ({}) vs {hv} ({})",
                    winner_source.map(Source::name).unwrap_or("?"),
                    hit.source.name(),
                ),
                (Some(_), Some(_)) => format!("{shadowed_by}; same version, different install"),
                _ => match hit.source {
                    Source::Path => format!("{shadowed_by}; no known source claims this file"),
                    _ => shadowed_by,
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn entry(source: Source, path: Option<&Path>, version: Option<&str>) -> ResolvedEntry {
        let mut e = ResolvedEntry::new(source);
        e.path = path.map(Path::to_path_buf);
        e.package_version = version.map(|v| v.to_string());
        e
    }

    #[test]
    fn active_first_is_first_match() {
        let mut e = entry(Source::Nix, Some(Path::new("/p")), Some("3.12.6"));
        e.active = true;
        assert_eq!(reasons(&[e]), vec!["first match in PATH"]);
    }

    #[test]
    fn active_after_empty_dirs_mentions_count() {
        let mut e = entry(Source::Apt, Some(Path::new("/usr/bin/t")), Some("1"));
        e.active = true;
        e.rank = 3;
        assert_eq!(
            reasons(&[e]),
            vec!["first match in PATH; 3 earlier PATH directories had no match"]
        );
    }

    // Unix-only: the same-file clause depends on resolving a symlink to the
    // winner's target, and Windows symlink creation needs privileges the
    // test must not assume. The pure version-clause tests below run everywhere.
    #[cfg(unix)]
    #[test]
    fn same_canonical_file_beats_version_clause() {
        use std::env;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let tmp = env::temp_dir().join("anywhich-why-samefile");
        let real_dir = tmp.join("real");
        let link_dir = tmp.join("link");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&link_dir).unwrap();

        let bin = real_dir.join("t");
        fs::write(&bin, b"").unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        let link = link_dir.join("t");
        std::os::unix::fs::symlink(&bin, &link).unwrap();

        let mut winner = entry(Source::Apt, Some(&bin), Some("1.0"));
        winner.active = true;
        let dupe = entry(Source::Path, Some(&link), None);
        let reasons = reasons(&[winner, dupe]);
        assert_eq!(reasons[1], "shadowed by 1; same file as 1");

        fs::remove_file(&link).unwrap();
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn differing_versions_are_stated_not_ranked() {
        let mut winner = entry(Source::Nix, Some(Path::new("/a/t")), Some("3.12.6"));
        winner.active = true;
        let shadowed = entry(Source::Pacman, Some(Path::new("/b/t")), Some("3.12.6-1"));
        let reasons = reasons(&[winner, shadowed]);
        assert_eq!(
            reasons[1],
            "shadowed by 1; version differs: 3.12.6 (nix) vs 3.12.6-1 (pacman)"
        );
    }

    #[test]
    fn equal_versions_get_install_contrast() {
        let mut winner = entry(Source::Nix, Some(Path::new("/a/t")), Some("2.0"));
        winner.active = true;
        let shadowed = entry(Source::Pacman, Some(Path::new("/b/t")), Some("2.0"));
        assert_eq!(
            reasons(&[winner, shadowed])[1],
            "shadowed by 1; same version, different install"
        );
    }

    #[test]
    fn unclaimed_file_says_so() {
        let mut winner = entry(Source::Path, Some(Path::new("/a/t")), None);
        winner.active = true;
        let shadowed = entry(Source::Path, Some(Path::new("/b/t")), None);
        assert_eq!(
            reasons(&[winner, shadowed])[1],
            "shadowed by 1; no known source claims this file"
        );
    }

    #[test]
    fn claimed_without_version_just_names_the_shadow() {
        let mut winner = entry(Source::Path, Some(Path::new("/a/t")), None);
        winner.active = true;
        let shadowed = entry(Source::Flatpak, None, None);
        assert_eq!(reasons(&[winner, shadowed])[1], "shadowed by 1");
    }
}
