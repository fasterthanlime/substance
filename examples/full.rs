use camino::Utf8PathBuf;
use itertools::Itertools;
use owo_colors::OwoColorize;
use substance::{BuildContext, ByteSize, CrateName, DemangledSymbolWithoutHash, NumberOfCopies};

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

    let a = substance::BuildRunner::for_manifest(&manifest_path)
        .arg("--features")
        .arg("facet-json")
        .run()?;

    let b = substance::BuildRunner::for_manifest(&manifest_path)
        .arg("--features")
        .arg("facet-toml")
        .run()?;

    show_report(&a)?;
    show_report(&b)?;
    show_diff(&a, &b)?;

    Ok(())
}

fn show_report(context: &BuildContext) -> eyre::Result<()> {
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
    let crate_sizes = context
        .crates
        .iter()
        .map(|krate| {
            let total_size: u64 = krate.symbols.values().map(|s| s.size.value()).sum();
            (krate, total_size)
        })
        .sorted_by_key(|&(_, size)| std::cmp::Reverse(size))
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

    let std_crates: Vec<CrateName> = context.std_crates.to_vec();

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

fn show_diff(baseline: &BuildContext, current: &BuildContext) -> eyre::Result<()> {
    use itertools::Itertools;
    use std::collections::{HashMap, HashSet};

    println!();
    println!("{}", "🆚 DIFF REPORT".yellow().bold());
    println!("{}", "═".repeat(50).yellow());

    // Helper closures --------------------------------------------------------
    let pct = |old: f64, new: f64| -> f64 {
        if old == 0.0 {
            0.0
        } else {
            (new - old) / old * 100.0
        }
    };

    let format_change = |delta: f64| -> String {
        if delta.abs() < 0.01 {
            "≈0%".to_string()
        } else if delta > 0.0 {
            format!("🔺 +{:.2}%", delta).bright_red().to_string()
        } else {
            format!("🔻 {:.2}%", delta).bright_green().to_string()
        }
    };

    // Build maps of crates ----------------------------------------------------
    let base_map: HashMap<_, _> = baseline.crates.iter().map(|c| (&c.name, c)).collect();
    let curr_map: HashMap<_, _> = current.crates.iter().map(|c| (&c.name, c)).collect();

    // Added / removed crates --------------------------------------------------
    let added: Vec<_> = curr_map
        .keys()
        .filter(|k| !base_map.contains_key(*k))
        .sorted()
        .collect();
    let removed: Vec<_> = base_map
        .keys()
        .filter(|k| !curr_map.contains_key(*k))
        .sorted()
        .collect();

    if !added.is_empty() {
        println!("{}", "➕ Crates added:".purple().bold());
        for name in &added {
            println!("  {}", name.cyan().bold());
        }
        println!();
    }

    if !removed.is_empty() {
        println!("{}", "➖ Crates removed:".purple().bold());
        for name in &removed {
            println!("  {}", name.cyan().bold());
        }
        println!();
    }

    // Crate size & build-time changes ----------------------------------------
    println!("{}", "🏗️  Notable crate changes (>5%)".purple().bold());

    let mut interesting = Vec::new();

    for (name, base_crate) in &base_map {
        if let Some(curr_crate) = curr_map.get(name) {
            // Size diff
            let base_size: u64 = base_crate.symbols.values().map(|s| s.size.value()).sum();
            let curr_size: u64 = curr_crate.symbols.values().map(|s| s.size.value()).sum();

            // Build-time diff (optional)
            let base_time = base_crate
                .timing_info
                .as_ref()
                .map(|ti| ti.duration)
                .unwrap_or(0.0);
            let curr_time = curr_crate
                .timing_info
                .as_ref()
                .map(|ti| ti.duration)
                .unwrap_or(0.0);

            let size_pct = pct(base_size as f64, curr_size as f64);
            let time_pct = pct(base_time, curr_time);

            let size_changed = size_pct.abs() > 5.0;
            let time_changed = time_pct.abs() > 10.0 && curr_time > 0.33;

            if size_changed || time_changed {
                interesting.push((
                    *name, base_size, curr_size, size_pct, base_time, curr_time, time_pct,
                ));
            }
        }
    }

    if interesting.is_empty() {
        println!(
            "{}",
            "No significant crate-level changes found.".bright_black()
        );
    } else {
        for (name, b_sz, c_sz, sz_pct, b_t, c_t, t_pct) in
            interesting.into_iter().sorted_by_key(|t| -t.3.abs() as i64)
        {
            let size_line = format!(
                "{} → {} ({})",
                format_bytes(b_sz).bright_blue(),
                format_bytes(c_sz).bright_blue(),
                format_change(sz_pct)
            );

            let time_line = if b_t == 0.0 && c_t == 0.0 {
                "no timing info".to_string()
            } else {
                format!("{:.2}s → {:.2}s ({})", b_t, c_t, format_change(t_pct))
            };

            println!(
                "  {}  |  {}  |  {}",
                name.cyan().bold(),
                size_line,
                time_line
            );
        }
    }

    // Function-level (symbol size) and LLVM IR line differences --------------
    // We handle symbol-size changes and LLVM-line changes separately because
    // they come from different data sources.

    // -----------------------------------------------------------------------
    // 1)  Symbol-level size changes
    // -----------------------------------------------------------------------
    println!();
    println!("{}", "📐 Notable symbol size changes (>5%)".purple().bold());

    // Build a map of (crate, symbol_without_hash) -> total_size
    let collect_symbol_sizes =
        |ctx: &BuildContext| -> HashMap<(CrateName, DemangledSymbolWithoutHash), u64> {
            let mut map = HashMap::new();
            for krate in &ctx.crates {
                for sym in krate.symbols.values() {
                    let key = (krate.name.clone(), sym.name.strip_hash());
                    map.entry(key)
                        .and_modify(|v| *v += sym.size.value())
                        .or_insert(sym.size.value());
                }
            }
            map
        };

    type SymMap = HashMap<(CrateName, DemangledSymbolWithoutHash), u64>;
    let base_sym_map: SymMap = collect_symbol_sizes(baseline);
    let curr_sym_map: SymMap = collect_symbol_sizes(current);

    // Determine “interesting” symbols: top-20 by size in either build
    let mut candidate_syms: HashSet<(CrateName, DemangledSymbolWithoutHash)> = HashSet::new();
    let add_top = |map: &SymMap, set: &mut HashSet<(CrateName, DemangledSymbolWithoutHash)>| {
        let mut v: Vec<_> = map.iter().collect();
        v.sort_by_key(|(_, size)| std::cmp::Reverse(**size));
        for (k, _) in v.into_iter().take(20) {
            set.insert(k.clone());
        }
    };
    add_top(&base_sym_map, &mut candidate_syms);
    add_top(&curr_sym_map, &mut candidate_syms);

    // Compare sizes
    let mut symbol_changes = Vec::new();
    for key in candidate_syms {
        let base_size = *base_sym_map.get(&key).unwrap_or(&0);
        let curr_size = *curr_sym_map.get(&key).unwrap_or(&0);
        if base_size == curr_size {
            continue;
        }
        let delta_pct = pct(base_size as f64, curr_size as f64);
        if delta_pct.abs() > 5.0 || base_size == 0 || curr_size == 0 {
            symbol_changes.push((key, base_size, curr_size, delta_pct));
        }
    }

    if symbol_changes.is_empty() {
        println!(
            "{}",
            "No significant symbol-level changes found.".bright_black()
        );
    } else {
        symbol_changes.sort_by(|a, b| {
            b.3.abs()
                .partial_cmp(&a.3.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for ((crate_name, symbol_name), old_sz, new_sz, pct_chg) in symbol_changes {
            let line = if old_sz == 0 && new_sz > 0 {
                format!("added ({})", format_bytes(new_sz).bright_green())
            } else if old_sz > 0 && new_sz == 0 {
                "removed".bright_red().to_string()
            } else {
                format!(
                    "{} → {} ({})",
                    format_bytes(old_sz).bright_blue(),
                    format_bytes(new_sz).bright_blue(),
                    format_change(pct_chg)
                )
            };

            println!("  {}::{}", crate_name.cyan().bold(), symbol_name.blue());
            println!("      size : {}", line);
        }
    }

    // -----------------------------------------------------------------------
    // 2)  LLVM-IR line-count changes
    // -----------------------------------------------------------------------
    println!();
    println!(
        "{}",
        "🧬 Notable LLVM IR function changes (>5%)".purple().bold()
    );

    // Map of (crate, function_name) -> total_lines
    let collect_fn_lines =
        |ctx: &BuildContext| -> HashMap<(CrateName, substance::LlvmFunctionName), usize> {
            let mut map = HashMap::new();
            for krate in &ctx.crates {
                for func in krate.llvm_functions.values() {
                    let key = (krate.name.clone(), func.name.clone());
                    map.entry(key)
                        .and_modify(|v| *v += func.lines.value())
                        .or_insert(func.lines.value());
                }
            }
            map
        };

    type FnMap = HashMap<(CrateName, substance::LlvmFunctionName), usize>;
    let base_fn_map: FnMap = collect_fn_lines(baseline);
    let curr_fn_map: FnMap = collect_fn_lines(current);

    // Determine interesting functions: top-20 by line count in either build
    let mut candidate_fns: HashSet<(CrateName, substance::LlvmFunctionName)> = HashSet::new();
    let add_top_fn = |map: &FnMap, set: &mut HashSet<(CrateName, substance::LlvmFunctionName)>| {
        let mut v: Vec<_> = map.iter().collect();
        v.sort_by_key(|&(_, &lines)| std::cmp::Reverse(lines));
        for (k, _) in v.into_iter().take(20) {
            set.insert(k.clone());
        }
    };
    add_top_fn(&base_fn_map, &mut candidate_fns);
    add_top_fn(&curr_fn_map, &mut candidate_fns);

    let mut fn_changes = Vec::new();
    for key in candidate_fns {
        let base_lines = *base_fn_map.get(&key).unwrap_or(&0);
        let curr_lines = *curr_fn_map.get(&key).unwrap_or(&0);
        if base_lines == curr_lines {
            continue;
        }
        let delta_pct = pct(base_lines as f64, curr_lines as f64);
        if delta_pct.abs() > 5.0 || base_lines == 0 || curr_lines == 0 {
            fn_changes.push((key, base_lines, curr_lines, delta_pct));
        }
    }

    if fn_changes.is_empty() {
        println!(
            "{}",
            "No significant LLVM-IR function changes found.".bright_black()
        );
    } else {
        fn_changes.sort_by(|a, b| {
            b.3.abs()
                .partial_cmp(&a.3.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for ((crate_name, fn_name), old_ln, new_ln, pct_chg) in fn_changes {
            let line = if old_ln == 0 && new_ln > 0 {
                format!("added ({} lines)", new_ln)
            } else if old_ln > 0 && new_ln == 0 {
                "removed".to_string()
            } else {
                format!("{} → {} ({})", old_ln, new_ln, format_change(pct_chg))
            };

            println!("  {}::{}", crate_name.cyan().bold(), fn_name.blue());
            println!("      lines: {}", line);
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
