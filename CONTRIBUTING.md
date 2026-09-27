# Contributing to anywhich

Thanks for wanting to help. anywhich is a small tool with a narrow promise: tell the truth about where a binary comes from, every source, every PATH hit, no guessing. Contributions that keep that promise are welcome.

## Getting started

```
cargo build
cargo test
```

That is the whole loop. `cargo test` runs everywhere because every test uses fixture strings or fixture directory trees. Never write a test that shells out to a real package manager; CI machines have none installed, and the fixtures are the reason the suite runs in seconds.

## Reporting bugs

anywhich diagnoses environments, so your output is the bug report. Please include:

1. The verbatim output of `anyw <name> --why --plain`
2. Your OS and distribution
3. Which package managers you have installed (`pacman`, `apt`, `dnf`, `apk`, `flatpak`, `snap`, `nix`, `npm`, `cargo`, `go`, `pipx`, and so on)
4. What you expected to see, if it is not obvious

Wrap pasted output in code blocks. If the output is long, trim unrelated resolvers but keep the Checked block intact; it is usually the interesting part.

## Proposing changes

Open an issue first for anything that changes output format, exit codes, or the resolver trait. Small fixes and new resolvers can go straight to a pull request.

Keep pull requests focused. One resolver, one fix, one feature. If you find yourself writing "also" in the description, split it.

## Using AI assistance

AI tools are fine to use while contributing, with one requirement: you submit it, you understand it, you can defend it in review. Bring your own judgment to what the tool produced, verify the claims (run the tests, read the code), and write pull request descriptions and review replies in your own words. Submissions that read as unreviewed tool output will be asked to go back and earn the diff.

## Adding a package manager

The resolver set is designed to grow without touching the core. To add one:

1. New file in `src/resolver/pkgdb/` (distro file databases) or `src/resolver/prefix/` (tools with their own install stores).
2. One `Source` variant in `src/entry.rs` and its display name in `Source::name`.
3. One registry line in `src/resolver/mod.rs`.
4. Fixture tests for the parser. Distinguish "checked, nothing found" from "could not check"; resolvers return `SourceResult::checked` with empty entries for a clean miss, and `unavailable(reason)` only when the check itself could not run.
5. A resolver note in the PR describing the mechanism you read from (which command, which file), so the fixtures can be checked against reality.

Platform rule: a platform ships binaries only after it works end to end there. If your target cannot be verified from your machine, say so in the PR rather than claiming it.

## Wanted right now

- pkgdb resolvers: xbps (Void), zypper (openSUSE)
- Windows verification from a real Windows box (Scoop and Chocolatey resolvers exist, fixture-tested only)
- macOS reports (the walk should work; nobody has confirmed it)

## License

By contributing, you agree that your contributions are dual-licensed under MIT or Apache-2.0, the same as the rest of the project.
