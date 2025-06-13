use camino::Utf8PathBuf;
use owo_colors::OwoColorize;
use substance::BuildReport;

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
    let manifest_path = git_root.join("Cargo.toml");
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

    let build_context = substance::BuildRunner::for_manifest(&manifest_path)
        .arg("--example")
        .arg("analysis_target")
        .run()?;

    // Generate a complete BuildReport
    let report = BuildReport {
        build_duration: build_context.wall_duration,
        file_size: build_context.file_size,
        text_size: build_context.text_size,
        crates: build_context.crates,
    };

    // Display the report
    println!("\n{}", "📊 BUILD REPORT".blue().bold());
    println!("{}", "═".repeat(50).blue());

    println!("\n{}", "Build Metrics:".green().bold());
    println!(
        "  Build duration: {}",
        format!("{:.2}s", report.build_duration.as_secs_f64()).bright_green()
    );
    println!(
        "  Binary size: {}",
        format_bytes(report.file_size.value()).bright_green()
    );
    println!(
        "  Text section: {}",
        format_bytes(report.text_size.value()).bright_green()
    );

    // Show crate breakdown by symbols and size
    println!("\n{}", "Crate Analysis:".green().bold());

    // Sort crates by total symbol size
    let mut crates_by_size: Vec<_> = report
        .crates
        .iter()
        .map(|c| {
            let total_size: u64 = c.symbols.values().map(|s| s.size.value()).sum();
            (c, total_size)
        })
        .collect();
    crates_by_size.sort_by(|a, b| b.1.cmp(&a.1));

    println!("\n{}", "Top 20 crates by code size:".blue());
    for (crate_info, total_size) in crates_by_size.iter().take(20) {
        let symbol_count = crate_info.symbols.len();
        let llvm_count = crate_info.llvm_functions.len();

        println!(
            "  {:>10} {:>6} symbols {:>6} functions   {}",
            format_bytes(*total_size).bright_yellow(),
            symbol_count.to_string().bright_yellow(),
            llvm_count.to_string().bright_yellow(),
            crate_info.name.to_string().white()
        );
    }

    // Show largest symbols
    println!("\n{}", "Top 10 largest symbols:".blue());
    let mut all_symbols: Vec<_> = report
        .crates
        .iter()
        .flat_map(|c| c.symbols.values().map(move |s| (c.name.clone(), s)))
        .collect();
    all_symbols.sort_by(|a, b| b.1.size.value().cmp(&a.1.size.value()));

    for (crate_name, symbol) in all_symbols.iter().take(10) {
        println!(
            "  {:>10} {} ({})",
            format_bytes(symbol.size.value()).bright_yellow(),
            symbol.name.to_string().cyan(),
            crate_name.to_string().bright_blue()
        );
    }

    // Show LLVM IR statistics
    println!("\n{}", "LLVM IR Analysis:".blue());
    let total_llvm_functions: usize = report.crates.iter().map(|c| c.llvm_functions.len()).sum();
    let total_llvm_lines: usize = report
        .crates
        .iter()
        .flat_map(|c| c.llvm_functions.values())
        .map(|f| f.lines.value())
        .sum();
    let total_copies: usize = report
        .crates
        .iter()
        .flat_map(|c| c.llvm_functions.values())
        .map(|f| f.copies.value())
        .sum();

    println!(
        "  Total LLVM functions: {}",
        total_llvm_functions.to_string().bright_green()
    );
    println!(
        "  Total LLVM IR lines: {}",
        total_llvm_lines.to_string().bright_green()
    );
    println!(
        "  Total instantiations: {}",
        total_copies.to_string().bright_green()
    );

    // Show functions with most copies
    println!("\n{}", "Functions with most instantiations:".blue());
    let mut all_llvm_fns: Vec<_> = report
        .crates
        .iter()
        .flat_map(|c| c.llvm_functions.values())
        .collect();
    all_llvm_fns.sort_by(|a, b| b.copies.value().cmp(&a.copies.value()));

    for func in all_llvm_fns.iter().take(10) {
        if func.copies.value() > 1 {
            println!(
                "  {:>3} copies, {:>6} lines   {}",
                func.copies.value().to_string().bright_yellow(),
                func.lines.value().to_string().bright_yellow(),
                func.name.to_string().white()
            );
        }
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
