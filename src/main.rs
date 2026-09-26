use std::process::ExitCode;

use clap::Parser;
use owo_colors::OwoColorize;

mod entry;
mod merge;
mod pathwalk;
mod resolver;

#[derive(Parser)]
#[command(name = "anyw", version, about)]
struct Args {
    /// Binary name to look up
    name: String,

    /// No color, scriptable output
    #[arg(long)]
    plain: bool,
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

    ExitCode::SUCCESS
}

fn source_label(entry: &entry::ResolvedEntry) -> Option<String> {
    let source = match entry.source {
        entry::Source::Path => return None,
        s => s,
    };
    let name = source_name(source);
    match (&entry.package_name, &entry.package_version) {
        (Some(pkg), Some(ver)) => Some(format!("{name}: {pkg} {ver}")),
        (Some(pkg), None) => Some(format!("{name}: {pkg}")),
        (None, _) => Some(name.to_string()),
    }
}

fn source_name(source: entry::Source) -> &'static str {
    match source {
        entry::Source::Path => "PATH",
        entry::Source::Pacman => "pacman",
        entry::Source::Apt => "apt",
        entry::Source::Dnf => "dnf",
        entry::Source::Flatpak => "flatpak",
        entry::Source::Nix => "nix",
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
