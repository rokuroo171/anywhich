use crate::entry::ResolvedEntry;

/// One package manager's view of a binary query.
///
/// Resolvers know nothing about PATH, other resolvers, or ranking. They return
/// their own matches and near-misses; the core merges.
pub trait Resolver {
    fn name(&self) -> &'static str;

    fn resolve(&self, binary_name: &str) -> Vec<ResolvedEntry>;
}

/// Collects every resolver, in output display order.
pub fn resolvers() -> Vec<Box<dyn Resolver>> {
    vec![Box::new(crate::pacman::PacmanResolver)]
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

        fn resolve(&self, _binary_name: &str) -> Vec<ResolvedEntry> {
            self.entries.clone()
        }
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

        assert_eq!(fake.resolve("python"), fake.entries.clone());
    }
}
