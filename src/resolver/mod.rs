use crate::entry::ResolvedEntry;

pub mod pkgdb;
pub mod prefix;

/// Outcome of one source's check.
///
/// `Checked` means the resolver ran to completion; `entries` may still be
/// empty. `Unavailable` means the check itself could not run and the source
/// says nothing about whether the binary is installed.
pub enum SourceStatus {
    Checked,
    Unavailable(String),
}

pub struct SourceResult {
    pub entries: Vec<ResolvedEntry>,
    pub status: SourceStatus,
}

impl SourceResult {
    pub fn checked(entries: Vec<ResolvedEntry>) -> Self {
        SourceResult { entries, status: SourceStatus::Checked }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        SourceResult { entries: Vec::new(), status: SourceStatus::Unavailable(reason.into()) }
    }
}

/// One package manager's view of a binary query.
///
/// Resolvers know nothing about PATH, other resolvers, or ranking. They return
/// their own matches and near-misses; the core merges.
pub trait Resolver: Send + Sync {
    fn name(&self) -> &'static str;

    fn resolve(&self, binary_name: &str) -> SourceResult;
}

/// Runs every resolver concurrently, one thread each, and returns results in
/// registry order. A slow resolver holds up the answer only as long as its
/// own runtime instead of adding to every other source's; npm alone costs
/// seconds, and sequential spawning made every query pay the sum.
pub fn resolve_all(binary_name: &str) -> Vec<(String, SourceResult)> {
    let handles: Vec<_> = resolvers()
        .into_iter()
        .map(|r| {
            let name = r.name().to_string();
            let binary_name = binary_name.to_string();
            std::thread::spawn(move || (name, r.resolve(&binary_name)))
        })
        .collect();
    handles
        .into_iter()
        .map(|h| h.join().expect("resolver thread panicked"))
        .collect()
}

/// Collects every resolver, in output display order.
pub fn resolvers() -> Vec<Box<dyn Resolver>> {
    vec![
        Box::new(pkgdb::pacman::PacmanResolver),
        Box::new(pkgdb::apt::AptResolver),
        Box::new(pkgdb::dnf::DnfResolver),
        Box::new(pkgdb::apk::ApkResolver),
        Box::new(prefix::flatpak::FlatpakResolver),
        Box::new(prefix::nix::NixResolver),
        Box::new(prefix::npm::NpmResolver),
        Box::new(prefix::cargo::CargoResolver::new()),
        Box::new(prefix::go::GoResolver),
        Box::new(prefix::pipx::PipxResolver),
        Box::new(prefix::scoop::ScoopResolver),
        Box::new(prefix::chocolatey::ChocolateyResolver),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Source;

    struct Fake {
        entries: Vec<ResolvedEntry>,
    }

    impl Resolver for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn resolve(&self, _binary_name: &str) -> SourceResult {
            SourceResult::checked(self.entries.clone())
        }
    }

    #[test]
    fn unavailable_status_carries_reason() {
        let r = SourceResult::unavailable("pacman not found");
        assert!(matches!(r.status, SourceStatus::Unavailable(_)));
        assert!(r.entries.is_empty());
    }

    #[test]
    fn checked_status_may_have_empty_entries() {
        let r = SourceResult::checked(Vec::new());
        assert!(matches!(r.status, SourceStatus::Checked));
        assert!(r.entries.is_empty());
    }

    #[test]
    fn registry_starts_empty_with_well_formed_resolvers() {
        let regs = resolvers();
        let names: Vec<&str> = regs.iter().map(|r| r.name()).collect();
        for name in &names {
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn resolver_returns_its_own_entries_untouched() {
        let mut entry = ResolvedEntry::new(Source::Pacman);
        entry.package_name = Some("python".to_string());
        let fake = Fake { entries: vec![entry.clone()] };

        assert_eq!(fake.resolve("python").entries, fake.entries.clone());
    }
}
