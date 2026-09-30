## v0.2.0

Windows ships. The release workflow cross-compiles `anyw.exe` (x64 windows-gnu) and attaches it to every GitHub release, alongside the four Linux targets. On Windows, WinGet portable installs are now attributed, and Scoop machine-wide installs are found.

- Windows: new WinGet resolver attributes portable packages through their Links shim (`winget: Fastfetch-cli`).
- Windows: the Scoop resolver honors `SCOOP_GLOBAL` / `%ProgramData%\scoop`, so `scoop install -g` apps are found; user shims outrank global ones.
- Windows: cargo, go, and npm resolvers probe the `.exe`/`.cmd` shim spellings; go reads `USERPROFILE` when `HOME` is unset.
- Windows binary attached to GitHub releases on every `v*` tag, with sha256 checksums.
- A showcase GIF, recorded with VHS, in the README.
- Resolvers for pacman, apt, dnf, apk, Flatpak, Snap, Nix, npm -g, cargo install, go install, and pipx (unchanged from 0.1.0).

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
