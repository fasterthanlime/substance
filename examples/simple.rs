use camino::Utf8PathBuf;
use itertools::Itertools;
use owo_colors::OwoColorize;
use substance::{ByteSize, CrateName, DemangledSymbolWithoutHash, NumberOfCopies};

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

    let context = substance::BuildRunner::for_manifest(&manifest_path)
        .arg("--all-features")
        .run()?;

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
        "💫 Top 20 crates by number of symbols".purple().bold()
    );
    for (i, krate) in context
        .crates
        .iter()
        .sorted_by_key(|c| -(c.symbols.len() as isize))
        .enumerate()
        .take(20)
    {
        println!(
            "{}. {} ({} {})",
            (i + 1).yellow(),
            krate.name.cyan().bold(),
            krate.symbols.len().blue(),
            "symbols".bright_black()
        );
    }

    println!();
    println!(
        "{}",
        "📦 Top 20 crates by binary size (sum of symbol sizes)"
            .purple()
            .bold()
    );

    // Prepare vector of (crate, total symbol size) tuples (sum over symbols)
    let mut crate_sizes = context
        .crates
        .iter()
        .map(|krate| {
            let total_size: u64 = krate.symbols.values().map(|s| s.size.value()).sum();
            (krate, total_size)
        })
        .sorted_by_key(|&(_, size)| -(size as i64))
        .take(20)
        .collect::<Vec<_>>();

    for (i, (krate, size)) in crate_sizes.into_iter().enumerate() {
        println!(
            "{}. {} - {}",
            (i + 1).yellow(),
            krate.name.cyan().bold(),
            format_bytes(size).bright_green(),
        );
    }

    let std_crates: Vec<CrateName> = context.std_crates.iter().cloned().collect();

    println!();
    println!("{}", "⏰ Top 20 crates by build time".purple().bold());

    // Use the actual build timing information collected from Cargo's JSON output
    // (`TimingInfo::duration` is in seconds).
    let crate_times = context
        .crates
        .iter()
        .filter_map(|krate| krate.timing_info.as_ref().map(|ti| (krate, ti.duration)))
        .sorted_by(|a, b| {
            // Sort descending by duration
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        })
        .take(20)
        .collect::<Vec<_>>();

    for (i, (krate, seconds)) in crate_times.into_iter().enumerate() {
        println!(
            "{}. {} - {}",
            (i + 1).yellow(),
            krate.name.cyan().bold(),
            format!("{:.2}s", seconds).bright_blue(),
        );
    }

    println!();
    println!("{}", "🏋️  Top 20 largest symbols by size".purple().bold());

    struct AggregateSymbol {
        pub name: DemangledSymbolWithoutHash,
        pub size: ByteSize,
        pub copies: NumberOfCopies,
        pub crates: HashSet<CrateName>,
    }

    // Gather every symbol from every non-stdlib crate and aggregate by hashless demangled name
    let mut symbol_map: std::collections::HashMap<DemangledSymbolWithoutHash, AggregateSymbol> =
        std::collections::HashMap::new();
    use std::collections::HashSet;
    for krate in &context.crates {
        if std_crates.contains(&krate.name) {
            continue; // Skip standard library crates
        }
        for sym in krate.symbols.values() {
            let hashless = sym.name.strip_hash();
            symbol_map
                .entry(hashless.clone())
                .and_modify(|agg| {
                    agg.size = ByteSize::new(agg.size.value() + sym.size.value());
                    agg.copies = NumberOfCopies::new(agg.copies.value() + 1);
                    agg.crates.insert(krate.name.clone());
                })
                .or_insert_with(|| AggregateSymbol {
                    name: hashless.clone(),
                    size: sym.size,
                    copies: NumberOfCopies::new(1_usize),
                    crates: {
                        let mut hs = HashSet::new();
                        hs.insert(krate.name.clone());
                        hs
                    },
                });
        }
    }
    let all_symbols: Vec<_> = symbol_map
        .values()
        .map(|agg| {
            // Show all crate names that define the symbol
            (
                agg.crates.iter().cloned().collect::<Vec<_>>(),
                &agg.name,
                agg.size,
                agg.copies,
            )
        })
        .collect();

    for (i, (crate_names, symbol_name, size, copies)) in all_symbols
        .into_iter()
        .sorted_by_key(|(_, _, s, _)| -(s.value() as i64))
        .take(20)
        .enumerate()
    {
        let crates_str = crate_names
            .iter()
            .map(|c| c.cyan().bold().to_string())
            .collect::<Vec<_>>()
            .join(", ");

        let copies_info = if copies.value() > 1 {
            format!(
                " ({} {})",
                copies.value().bright_magenta(),
                "copies".bright_magenta()
            )
        } else {
            String::new()
        };

        println!(
            "{}. {} ({}) - {}{}",
            (i + 1).yellow(),
            symbol_name.blue(),
            crates_str,
            format_bytes(size.value()).bright_green(),
            copies_info
        );
    }

    println!();
    println!(
        "{}",
        "🦀 Top 20 largest LLVM functions by total lines"
            .purple()
            .bold()
    );

    // Collect every LLVM function from every non-stdlib crate with its line count
    let mut all_functions = Vec::new();
    for krate in &context.crates {
        if std_crates.contains(&krate.name) {
            continue; // Skip standard library crates
        }
        for func in krate.llvm_functions.values() {
            all_functions.push((&krate.name, &func.name, func.lines, func.copies));
        }
    }

    for (i, (crate_name, func_name, lines, copies)) in all_functions
        .into_iter()
        .sorted_by_key(|(_, _, l, _)| -(l.value() as isize))
        .take(20)
        .enumerate()
    {
        let copies_info = if copies.value() > 1 {
            format!(
                " ({} {})",
                copies.value().bright_magenta(),
                "copies".bright_magenta()
            )
        } else {
            String::new()
        };

        println!(
            "{}. {} ({}) - {} lines{}",
            (i + 1).yellow(),
            func_name.blue(),
            crate_name.cyan().bold(),
            lines.value().bright_blue(),
            copies_info
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
