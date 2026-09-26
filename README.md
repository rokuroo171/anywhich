# anywhich

A `which` that finds what is installed on your system, even when it is not on `$PATH`. The command you type is `anyw`.

`which` stops at `$PATH` and only shows the first hit. On a machine with more than one package manager, a binary can be installed and still come back empty. `anyw` walks all of `$PATH`, shows every match in order, and tells you which one runs.

Linux works today, with resolvers for pacman, apt, dnf, Flatpak, Nix, npm -g, cargo install, go install, and pipx. Windows lookups follow PATHEXT (`.exe`, `.bat`, `.cmd`); the Scoop and Chocolatey resolvers are fixture-tested only and no Windows binary ships yet. macOS is untested.

## Usage

```
$ anyw python

PATH:
  1. /usr/bin/python (active)
  2. /usr/local/bin/python

Currently resolves to: /usr/bin/python
```

Not found:

```
$ anyw foobar

No matches in PATH.
foobar is not installed via any known source.
```

Exit code is 1 when nothing matches, so it works in scripts.

## Flags

- `--plain`: no color, scriptable output.
- `--why`: one reason line under each PATH entry explaining why it ranked there.

## Building

```
cargo build --release
```

The binary lands at `target/release/anyw`. Run the tests with `cargo test`.

## License

Dual-licensed under MIT or Apache-2.0.
