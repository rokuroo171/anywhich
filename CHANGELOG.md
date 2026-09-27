## v0.1.0

Initial version.

- Walks all of `$PATH` and shows every match in order, marking the one that runs.
- Resolvers for pacman, apt, dnf, apk, Flatpak, Snap, Nix, npm -g, cargo install, go install, and pipx.
- Resolvers are checked in parallel; the npm lookup reads the global tree directly instead of spawning `npm ls`.
- Reports binaries that are installed but not on `$PATH` instead of coming back empty.
- `--why` prints one reason line under each PATH entry.
- `--plain` prints colorless output for scripts.
- Windows: lookups follow PATHEXT (`.exe`, `.bat`, `.cmd`), plus Scoop and Chocolatey resolvers, fixture-tested only.
- Exit code is 1 when nothing is found anywhere, so the tool works in scripts.
