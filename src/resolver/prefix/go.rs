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
        let env = EnvVars::from_env();
        let dirs = go_bin_dirs(&env);
        if let Some(path) = dirs
            .iter()
            .map(|dir| dir.join(binary_name))
            .find(|p| is_file(p))
        {
            return SourceResult::checked(vec![ResolvedEntry::stub_only(Source::Go, &path)]);
        }
        SourceResult::checked(Vec::new())
    }
}

struct EnvVars {
    gobin: Option<String>,
    gopath: Option<String>,
    home: Option<String>,
}

impl EnvVars {
    fn from_env() -> Self {
        EnvVars {
            gobin: std::env::var("GOBIN").ok(),
            gopath: std::env::var("GOPATH").ok(),
            home: std::env::var("HOME").ok(),
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

    #[test]
    fn multi_gopath_splits_on_separator() {
        let dirs = go_bin_dirs(&env("", "/gp1:/gp2", ""));
        assert!(dirs.contains(&PathBuf::from("/gp1/bin")));
        assert!(dirs.contains(&PathBuf::from("/gp2/bin")));
        assert!(!dirs.contains(&PathBuf::from("/go/bin")));
    }
}
