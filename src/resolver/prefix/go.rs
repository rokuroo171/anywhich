use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{ResolvedEntry, Source};
use crate::resolver::{Resolver, SourceResult};

/// `go install` view of one binary query.
///
/// Go installs module binaries into one bin directory: `$GOBIN`, else
/// `$GOPATH/bin`, else `~/go/bin`. A hit is the binary existing there. Go
/// keeps no per-binary version record readable without spawning `go`, so a
/// hit reports the path with no package identity.
pub struct GoResolver;

impl Resolver for GoResolver {
    fn name(&self) -> &'static str {
        "go"
    }

    fn resolve(&self, binary_name: &str) -> SourceResult {
        self.resolve_with(&EnvVars::from_env(), binary_name)
    }
}

impl GoResolver {
    /// resolve() with injected env, so the candidate rules are fixture-testable.
    fn resolve_with(&self, env: &EnvVars, binary_name: &str) -> SourceResult {
        for dir in go_bin_dirs(env) {
            for candidate in candidate_names(binary_name) {
                let path = dir.join(&candidate);
                if is_file(&path) {
                    return SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Go, &path)]);
                }
            }
        }
        SourceResult::checked(Vec::new())
    }
}

/// Spellings to probe per bin dir: the name as typed, then the exe
/// spelling, because `go install` writes `name.exe` on Windows and the
/// extra stat is free where it never matches.
fn candidate_names(binary_name: &str) -> Vec<String> {
    vec![binary_name.to_string(), format!("{binary_name}.exe")]
}

struct EnvVars {
    gobin: Option<String>,
    gopath: Option<String>,
    home: Option<String>,
}

impl EnvVars {
    fn from_env() -> Self {
        // USERPROFILE after HOME because go's own default derives from
        // os.UserHomeDir, which reads USERPROFILE on native Windows.
        let home = ["HOME", "USERPROFILE"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
        EnvVars {
            gobin: std::env::var("GOBIN").ok(),
            gopath: std::env::var("GOPATH").ok(),
            home,
        }
    }
}

/// Candidate bin dirs in go's own precedence order. The default `~/go/bin`
/// is always included because it applies even when GOBIN/GOPATH are set for
/// other purposes; it is last, so it never outranks an explicit setting.
fn go_bin_dirs(env: &EnvVars) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(gobin) = env.gobin.as_deref().filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(gobin));
    }
    if let Some(gopath) = env.gopath.as_deref().filter(|s| !s.is_empty()) {
        for p in std::env::split_paths(gopath) {
            if !p.as_os_str().is_empty() {
                dirs.push(p.join("bin"));
            }
        }
    }
    if let Some(home) = env.home.as_deref().filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(home).join("go").join("bin"));
    }
    dirs
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(gobin: &str, gopath: &str, home: &str) -> EnvVars {
        EnvVars {
            gobin: Some(gobin.to_string()),
            gopath: Some(gopath.to_string()),
            home: Some(home.to_string()),
        }
    }

    #[test]
    fn gobin_wins_over_gopath_and_default() {
        let dirs = go_bin_dirs(&env("/custom/bin", "/gp", "/home/t"));
        assert_eq!(dirs[0], PathBuf::from("/custom/bin"));
        assert!(dirs.contains(&PathBuf::from("/gp/bin")));
        assert!(dirs.contains(&PathBuf::from("/home/t/go/bin")));
    }

    #[test]
    fn empty_values_are_ignored() {
        let dirs = go_bin_dirs(&env("", "", "/home/t"));
        assert!(!dirs.contains(&PathBuf::from("/bin")));
        assert_eq!(dirs, vec![PathBuf::from("/home/t/go/bin")]);
    }

    // Git Bash sets HOME to /c/Users/...; go on native Windows derives the
    // default from USERPROFILE instead. from_env accepts either.
    #[test]
    fn from_env_falls_back_to_userprofile() {
        let env = EnvVars::from_env();
        let has_home = std::env::var("HOME").is_ok_and(|s| !s.is_empty());
        let has_profile = std::env::var("USERPROFILE").is_ok_and(|s| !s.is_empty());
        assert_eq!(env.home.is_some(), has_home || has_profile);
    }

    // Go splits GOPATH on the PATH list separator (: on Unix, ; on
    // Windows); join_paths is the exact inverse of the split_paths the
    // resolver applies, so the round trip holds on every platform.
    #[test]
    fn multi_gopath_splits_on_separator() {
        let gopath = std::env::join_paths(["/gp1", "/gp2"])
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let dirs = go_bin_dirs(&env("", &gopath, ""));
        assert!(dirs.contains(&PathBuf::from("/gp1/bin")));
        assert!(dirs.contains(&PathBuf::from("/gp2/bin")));
        assert!(!dirs.contains(&PathBuf::from("/go/bin")));
    }

    // Real-fs probe order: the name as typed wins, then the exe spelling.
    // Written for every platform so the Windows probe order is testable
    // from Unix CI, mirroring pathext.rs.
    #[test]
    fn typed_name_wins_then_exe_spelling() {
        let tmp = std::env::temp_dir().join("anywhich-go-probe");
        let _ = fs::remove_dir_all(&tmp);

        let both = tmp.join("both");
        fs::create_dir_all(&both).unwrap();
        fs::write(both.join("tool"), b"").unwrap();
        fs::write(both.join("tool.exe"), b"").unwrap();
        let hit = GoResolver.resolve_with(
            &EnvVars { gobin: Some(both.to_string_lossy().to_string()), gopath: None, home: None },
            "tool",
        );
        assert_eq!(hit.entries[0].path, Some(both.join("tool")));

        let exe_only = tmp.join("exe-only");
        fs::create_dir_all(&exe_only).unwrap();
        fs::write(exe_only.join("tool.exe"), b"").unwrap();
        let hit = GoResolver.resolve_with(
            &EnvVars { gobin: Some(exe_only.to_string_lossy().to_string()), gopath: None, home: None },
            "tool",
        );
        assert_eq!(hit.entries[0].path, Some(exe_only.join("tool.exe")));

        fs::remove_dir_all(&tmp).unwrap();
    }
}
