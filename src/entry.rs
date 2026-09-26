use std::path::PathBuf;

/// Where a resolved entry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Path,
    Pacman,
    Apt,
    Flatpak,
    Nix,
}

/// One place a queried binary was found, or one candidate a resolver rejected.
///
/// For `Source::Path`, entries are ranked by PATH order: rank 0 is the one the
/// shell would run. Other resolvers leave `rank` at 0; the core ranks across
/// sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEntry {
    /// Filesystem path if the binary itself was located, None when the source
    /// knows about it without a path (a Flatpak app id, a Nix store candidate).
    pub path: Option<PathBuf>,
    pub source: Source,
    /// Would this entry be the one that runs right now.
    pub active: bool,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    /// Position within the source's own ordering (PATH index for Source::Path).
    pub rank: usize,
}

impl ResolvedEntry {
    pub fn new(source: Source) -> Self {
        ResolvedEntry {
            path: None,
            source,
            active: false,
            package_name: None,
            package_version: None,
            rank: 0,
        }
    }
}
