use std::fs;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use owo_colors::OwoColorize;

mod entry;
mod merge;
mod pathext;
mod pathwalk;
mod resolver;
mod why;

#[derive(Parser)]
#[command(name = "anyw", version, about)]
struct Args {
    /// Binary name to look up
    name: String,

    /// No color, scriptable output
    #[arg(long)]
    plain: bool,

    /// Explain why each PATH entry ranked where it did
    #[arg(long)]
    why: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let (dirs, skipped) = pathwalk::path_dirs();
    let path_hits = pathwalk::walk(&args.name, &dirs);
    let results: Vec<(String, resolver::SourceResult)> = resolver::resolvers()
        .iter()
        .map(|r| (r.name().to_string(), r.resolve(&args.name)))
        .collect();
    let merged = merge::merge(path_hits, &results);

    if merged.path_hits.is_empty() {
        println!("No matches in PATH.");
        for dir in &skipped {
            println!("  skipped {dir}: not a readable directory");
        }
        print_other_sources(&merged, &results);
        if !merged.others.is_empty() {
            println!();
            println!("{} is installed but not on PATH.", args.name);
            return ExitCode::SUCCESS;
        }
        println!();
        if results.iter().any(|(_, r)| matches!(r.status, resolver::SourceStatus::Checked)) {
            println!("{} is not installed via any known source.", args.name);
        } else {
            println!(
                "{} could not be checked via any known source.",
                args.name
            );
        }
        return ExitCode::from(1);
    }

    println!("PATH:");
    let reason_lines = if args.why {
        why::reasons(&merged.path_hits)
    } else {
        Vec::new()
    };
    for (idx, hit) in merged.path_hits.iter().enumerate() {
        let path = hit
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let label = source_label(hit);
        let line = match label {
            Some(label) => format!("  {}. {} -> {}", idx + 1, path, label),
            None => format!("  {}. {}", idx + 1, path),
        };
        let suffix = if hit.active { " (active)" } else { "" };
        if hit.active && !args.plain {
            println!("{}{}", line.green(), suffix);
        } else {
            println!("{}{}", line, suffix);
        }
        if let Some(reason) = reason_lines.get(idx) {
            println!("       {reason}");
        }
    }

    if args.why && !skipped.is_empty() {
        println!();
        println!("Skipped PATH directories:");
        for dir in &skipped {
            println!("  {dir}");
        }
    }

    let winner = merged.path_hits[0].clone();
    let provider = source_label(&winner);
    let winner_path = winner.path.as_deref().map(|p| p.display().to_string()).unwrap_or_default();
    let line = match provider {
        Some(p) => format!("Currently resolves to: {} ({})", winner_path, p),
        None => format!("Currently resolves to: {}", winner_path),
    };
    if args.plain {
        println!("{line}");
    } else {
        println!("{}", line.green());
    }

    if !args.plain {
        if let Some(note) = self_note(&merged.path_hits, dirs.len(), results.len()) {
            println!("{note}");
        }
    }

    ExitCode::SUCCESS
}

/// The self-reference note from DESIGN.md: when the lookup just ran finds
/// the binary that is currently running, say so. Trigger is the file
/// identity, not the typed name, so renamed or symlinked installs still
/// qualify. Resolution itself is never special-cased; if anyw cannot find
/// itself through the normal walk, that is a real not-found.
fn self_note(path_hits: &[entry::ResolvedEntry], dir_count: usize, source_count: usize) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    note_for(&exe, path_hits, dir_count, source_count)
}

fn note_for(exe: &Path, path_hits: &[entry::ResolvedEntry], dir_count: usize, source_count: usize) -> Option<String> {
    let is_me = |p: &Path| same_file(p, exe);
    let winner_is_me = path_hits
        .first()
        .and_then(|e| e.path.as_deref())
        .map(is_me)
        .unwrap_or(false);
    if winner_is_me {
        let dirs = if dir_count == 1 { "directory" } else { "directories" };
        let sources = if source_count == 1 { "source" } else { "sources" };
        return Some(format!(
            "That's me. {} PATH {} and {} {} to confirm it.",
            counted(dir_count),
            dirs,
            counted(source_count).to_lowercase(),
            sources,
        ));
    }
    let shadowed = path_hits
        .iter()
        .any(|h| !h.active && h.path.as_deref().map(is_me).unwrap_or(false));
    if shadowed {
        return Some("That's me in second place. The other one is an impostor.".to_string());
    }
    None
}

/// True file identity, not path equality: cargo hardlinks target/debug/anyw
/// into deps/, and /proc/self/exe reports the other link, so strings miss.
/// Falls back to path equality when either side cannot be statted (tests).
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => (ma.dev(), ma.ino()) == (mb.dev(), mb.ino()),
        _ => a == b,
    }
}

#[cfg(windows)]
fn same_file(a: &Path, b: &Path) -> bool {
    merge::key(a) == merge::key(b)
}

/// Counts spelled out for the small numbers a PATH realistically has;
/// digits past that, because inventing a word list to thirty is bloat.
fn counted(n: usize) -> String {
    const WORDS: [&str; 12] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve",
    ];
    match WORDS.get(n.wrapping_sub(1)) {
        Some(w) => {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
        None => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Source;
    use std::path::Path;

    fn hit(path: &str, active: bool) -> entry::ResolvedEntry {
        let mut e = entry::ResolvedEntry::new(Source::Path);
        e.path = Some(Path::new(path).to_path_buf());
        e.active = active;
        e
    }

    fn exe(path: &str) -> &Path {
        Path::new(path)
    }

    #[test]
    fn winner_self_gets_the_work_count_line() {
        let hits = vec![hit("/usr/bin/anyw", true)];
        let note = note_for(exe("/usr/bin/anyw"), &hits, 7, 11);
        assert_eq!(
            note.as_deref(),
            Some("That's me. Seven PATH directories and eleven sources to confirm it.")
        );
    }

    #[test]
    fn singular_forms_read_correctly() {
        let hits = vec![hit("/usr/bin/anyw", true)];
        let note = note_for(exe("/usr/bin/anyw"), &hits, 1, 1);
        assert_eq!(
            note.as_deref(),
            Some("That's me. One PATH directory and one source to confirm it.")
        );
    }

    #[test]
    fn shadowed_self_gets_the_impostor_line() {
        let hits = vec![hit("/usr/local/bin/anyw", true), hit("/usr/bin/anyw", false)];
        let note = note_for(exe("/usr/bin/anyw"), &hits, 3, 11);
        assert_eq!(
            note.as_deref(),
            Some("That's me in second place. The other one is an impostor.")
        );
    }

    #[test]
    fn unrelated_binary_gets_nothing() {
        let hits = vec![hit("/usr/bin/other", true)];
        assert!(note_for(exe("/usr/bin/anyw"), &hits, 3, 11).is_none());
    }

    #[test]
    fn counts_past_twelve_are_digits() {
        assert_eq!(counted(13), "13");
        assert_eq!(counted(0), "0");
    }
}

fn source_label(entry: &entry::ResolvedEntry) -> Option<String> {
    let source = match entry.source {
        entry::Source::Path => return None,
        s => s,
    };
    let name = source.name();
    match (&entry.package_name, &entry.package_version) {
        (Some(pkg), Some(ver)) => Some(format!("{name}: {pkg} {ver}")),
        (Some(pkg), None) => Some(format!("{name}: {pkg}")),
        (None, _) => Some(name.to_string()),
    }
}

fn print_other_sources(merged: &merge::Merged, results: &[(String, resolver::SourceResult)]) {
    println!();
    println!("Checked:");
    for (rname, result) in results {
        match &result.status {
            resolver::SourceStatus::Checked => {
                if result.entries.is_empty() {
                    println!("  - {rname}: not found");
                } else if !merged.others.is_empty() {
                    println!("  - {rname}: found");
                } else {
                    let names: Vec<String> = result
                        .entries
                        .iter()
                        .filter_map(|e| e.package_name.clone())
                        .collect();
                    println!("  - {rname}: {}", names.join(", "));
                }
            }
            resolver::SourceStatus::Unavailable(reason) => {
                println!("  - {rname}: unavailable ({reason})");
            }
        }
    }
    for other in &merged.others {
        let path = other
            .path
            .as_deref()
            .map(|p| format!(" ({})", p.display()))
            .unwrap_or_default();
        let label = source_label(other).unwrap_or_default();
        println!("  - {label}{path}");
    }
}
