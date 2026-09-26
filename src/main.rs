use std::process::ExitCode;

use clap::Parser;
use owo_colors::OwoColorize;

mod entry;
mod pacman;
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
    let hits = pathwalk::walk(&args.name, &dirs);

    if hits.is_empty() {
        println!("No matches in PATH.");
        for dir in &skipped {
            println!("  skipped {dir}: not a readable directory");
        }
        let regs = resolver::resolvers();
        println!();
        println!("Checked:");
        for r in &regs {
            let found = r.resolve(&args.name);
            if found.is_empty() {
                println!("  - {}: not found", r.name());
            } else {
                let pkgs: Vec<String> = found
                    .iter()
                    .filter_map(|e| e.package_name.clone())
                    .collect();
                println!("  - {}: {}", r.name(), pkgs.join(", "));
            }
        }
        println!();
        println!("{} is not installed via any known source.", args.name);
        return ExitCode::from(1);
    }

    println!("PATH:");
    for (idx, hit) in hits.iter().enumerate() {
        let path = hit
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if hit.active && !args.plain {
            println!("  {}. {} (active)", idx + 1, path.green());
        } else {
            println!("  {}. {}", idx + 1, path);
        }
    }

    if let Some(winner) = hits[0].path.as_deref() {
        let line = format!("Currently resolves to: {}", winner.display());
        if args.plain {
            println!("{line}");
        } else {
            println!("{}", line.green());
        }
    }

    ExitCode::SUCCESS
}
