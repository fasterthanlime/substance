use camino::Utf8PathBuf;
use itertools::Itertools;
use owo_colors::OwoColorize;
use substance::NumberOfCopies;

fn main() -> Result<(), eyre::Error> {
    env_logger::init();

    let current_exe = Utf8PathBuf::from_path_buf(std::env::current_exe().unwrap()).unwrap();
    let exe_parent = current_exe.parent().unwrap();
    let git_root = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            .current_dir(exe_parent)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_owned();
    let git_root = Utf8PathBuf::from(git_root);
    let manifest_path = git_root.join("analysis-target").join("Cargo.toml");
    if !manifest_path.exists() {
        eprintln!(
            "{} {} {}",
            "❌".red(),
            "Manifest path does not exist:".bright_red(),
            manifest_path
        );
        return Err(eyre::eyre!(
            "{} Manifest path not found: {}",
            "❌",
            manifest_path
        ));
    }
    println!(
        "{} {} {}",
        "🚀".green(),
        "Using manifest path:".bright_green(),
        manifest_path
    );

    let context = substance::BuildRunner::for_manifest(&manifest_path).run()?;

    // Display the report
    println!("\n{}", "📊 BUILD REPORT".blue().bold());
    println!("{}", "═".repeat(50).blue());

    println!(
        "Build duration: {}, Binary size: {} (of which {} is .text)",
        format!("{:.2}s", context.wall_duration.as_secs_f64()).bright_yellow(),
        format_bytes(context.file_size.value()).bright_green(),
        format_bytes(context.text_size.value()).bright_blue()
    );

    println!("Number of crates in context: {}", context.crates.len());

    println!();
    println!(
        "{}",
        "🐉 Top 10 crates by number of generic LLVM functions"
            .purple()
            .bold()
    );
    for (i, krate) in context
        .crates
        .iter()
        .sorted_by_key(|c| -(c.llvm_functions.len() as isize))
        .enumerate()
        .take(10)
    {
        let total_copies: NumberOfCopies = krate.llvm_functions.values().map(|v| v.copies).sum();
        println!(
            "{}. {} ({} {}, {} {})",
            (i + 1).yellow(),
            krate.name.cyan().bold(),
            krate.llvm_functions.len().green(),
            "LLVM functions".bright_black(),
            total_copies.value().bright_magenta(),
            "copies".bright_magenta(),
        );
    }

    println!();
    println!(
        "{}",
        "💫 Top 10 crates by number of symbols".purple().bold()
    );
    for (i, krate) in context
        .crates
        .iter()
        .sorted_by_key(|c| -(c.symbols.len() as isize))
        .enumerate()
        .take(10)
    {
        println!(
            "{}. {} ({} {})",
            (i + 1).yellow(),
            krate.name.cyan().bold(),
            krate.symbols.len().blue(),
            "symbols".bright_black()
        );
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", size as u64, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}
