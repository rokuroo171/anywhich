# anywhich

A `which` that finds what is installed on your system, even when it is not on `$PATH`. The command you type is `anyw`.

`which` stops at `$PATH` and only shows the first hit. On a machine with more than one package manager, a binary can be installed and still come back empty. `anyw` walks all of `$PATH`, shows every match in order, and tells you which one runs.

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
foobar is not on PATH.
```

Exit code is 1 when nothing matches, so it works in scripts.

## Flags

- `--plain`: no color, scriptable output.

## Building

```
cargo build --release
```

The binary lands at `target/release/anyw`. Run the tests with `cargo test`.

## Status

Early, 0.1.0-track.

Linux is the platform that works today. Resolvers ship for pacman, apt, dnf, Flatpak, Nix, npm -g, cargo install, go install, and pipx.

Windows does not work yet. The PATH walk searches for the bare name only and ignores PATHEXT (.exe, .bat, .cmd), so `anyw python` misses `python.exe`, and any extensionless file in a PATH directory counts as a hit. The Scoop and Chocolatey resolvers exist but were written and fixture-tested on Linux, and until the walk itself follows Windows lookup rules, their output cannot line up with the PATH chain shown next to it. Fixing the Windows walk comes first, then WinGet.

macOS is untested. Homebrew is unwritten.

## License

Dual-licensed under MIT or Apache-2.0.
