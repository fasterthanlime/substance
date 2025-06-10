#!/usr/bin/env cargo

//! Example: Analyze a Rust binary using substance library
//!
//! This example shows how to:
//! 1. Run `cargo build --message-format=json`
//! 2. Parse the JSON output to get artifact information
//! 3. Use substance library to analyze the binary
//! 4. Display size information including biggest crates and symbols
//!
//! Usage: cargo run --example analyze_binary
//!
//! This will analyze the current project's binary.

use camino::Utf8PathBuf;
use std::collections::HashMap;
use std::fs;
use substance::{AnalysisConfig, ArtifactKind, BloatAnalyzer, BuildRunner, BuildType};

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;

    if bytes >= MIB {
        format!("{:.1}MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1}KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{}B", bytes)
    }
}

fn is_std_function(func_name: &str) -> bool {
    // Filter out standard library and common low-level functions
    let std_prefixes = [
        "std::",
        "core::",
        "alloc::",
        "hashbrown::",
        "gimli::",
        "addr2line::",
        "memchr::",
        "adler2::",
        "miniz_oxide::",
        "object::",
        "rustc_demangle::",
    ];

    std_prefixes
        .iter()
        .any(|prefix| func_name.starts_with(prefix))
}

struct CleanupGuard {
    temp_dir: Utf8PathBuf,
}

impl CleanupGuard {
    fn new(temp_dir: Utf8PathBuf) -> Self {
        Self { temp_dir }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if self.temp_dir.exists() {
            if let Err(e) = fs::remove_dir_all(&self.temp_dir) {
                eprintln!(
                    "⚠️ Failed to cleanup temporary directory {}: {}",
                    self.temp_dir,
                    e
                );
            } else {
                println!(
                    "🧹 Cleaned up temporary directory: {}",
                    self.temp_dir
                );
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a temporary target directory for fresh timing measurements
    let temp_target_dir = Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .expect("temp dir is not UTF-8")
        .join(format!("substance_timing_{}", std::process::id()));
    println!("🔨 Building project with analysis features enabled...");
    println!("📁 Using target dir: {}", temp_target_dir);

    // Ensure cleanup happens even on early return
    let _cleanup_guard = CleanupGuard::new(temp_target_dir.clone());

    // Use the new BuildRunner API
    let build_result = BuildRunner::new(
        "Cargo.toml",
        temp_target_dir.as_std_path(),
        BuildType::Debug,
    )
    .run()?;

    println!("✅ Build completed successfully");

    let context = build_result.context;
    let mut timing_data = build_result.timing_data;

    println!("Found {} crates with timing data", timing_data.len());

    println!("Target triple: {}", context.target_triple);
    println!("Found {} artifacts", context.artifacts.len());

    // Step 3: Find the analyze_binary example to analyze
    let binary_artifact = context
        .artifacts
        .iter()
        .find(|a| a.kind == ArtifactKind::Binary && a.name == "analyze_binary")
        .ok_or("analyze_binary example not found - make sure it built successfully")?;

    println!(
        "📈 Analyzing binary: {} ({})",
        binary_artifact.name,
        binary_artifact.path.display()
    );

    // Step 4: Analyze the binary
    let config = AnalysisConfig {
        symbols_section: None,                                  // Use default .text section
        split_std: false,                                       // Group std crates together
        analyze_llvm_ir: true,                                  // Also analyze LLVM IR files
        target_dir: Some(temp_target_dir.as_std_path().to_owned()), // Use our temporary target directory
    };

    let result = BloatAnalyzer::analyze_binary(&binary_artifact.path, &context, &config)?;

    // Step 5: Display results
    println!("\n📊 Analysis Results:");
    println!("─────────────────────");
    println!(
        "File size:    {} bytes ({})",
        result.file_size,
        format_bytes(result.file_size)
    );
    println!(
        "Text section: {} bytes ({})",
        result.text_size,
        format_bytes(result.text_size)
    );
    println!(
        "Text/File:    {:.1}%",
        result.text_size as f64 / result.file_size as f64 * 100.0
    );
    println!("Symbol count: {}", result.symbols.len());

    if let Some(section_name) = &result.section_name {
        println!("Section:      {}", section_name);
    }

    // Show top 10 largest symbols
    println!("\n🔍 Top 10 Largest Symbols:");
    println!("─────────────────────────");

    // Create indices to sort by size without cloning symbols
    let mut symbol_indices: Vec<usize> = (0..result.symbols.len()).collect();
    symbol_indices.sort_by_key(|&i| std::cmp::Reverse(result.symbols[i].size));

    for (rank, &i) in symbol_indices.iter().take(10).enumerate() {
        let symbol = &result.symbols[i];
        let percent = symbol.size as f64 / result.text_size as f64 * 100.0;
        println!(
            "{:2}. {:>8} ({:>5.1}%) {}",
            rank + 1,
            format_bytes(symbol.size),
            percent,
            symbol.name.trimmed
        );
    }

    // Calculate crate sizes by analyzing all symbols
    println!("\n📦 Top 10 Biggest Crates:");
    println!("─────────────────────────");

    let mut crate_sizes: HashMap<String, u64> = HashMap::new();

    for symbol in &result.symbols {
        let (crate_name, _is_exact) =
            substance::crate_name::from_sym(&context, config.split_std, &symbol.name);
        *crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
    }

    // Sort crates by size
    let mut crate_list: Vec<(&String, &u64)> = crate_sizes.iter().collect();
    crate_list.sort_by_key(|(_name, &size)| std::cmp::Reverse(size));

    for (rank, (crate_name, &size)) in crate_list.iter().take(10).enumerate() {
        let file_percent = size as f64 / result.file_size as f64 * 100.0;
        let text_percent = size as f64 / result.text_size as f64 * 100.0;
        println!(
            "{:2}. {:>8} bytes ({:>5.1}% file, {:>5.1}% text) {}",
            rank + 1,
            format_bytes(size),
            file_percent,
            text_percent,
            crate_name
        );
    }

    let remaining_crates = crate_list.len().saturating_sub(10);
    if remaining_crates > 0 {
        let remaining_size: u64 = crate_list.iter().skip(10).map(|(_, &size)| size).sum();
        println!(
            "    ... and {} more crates ({} total)",
            remaining_crates,
            format_bytes(remaining_size)
        );
    }

    // Show dependency crates found
    if !context.dep_crates.is_empty() {
        println!(
            "\n📋 Dependency Crates Found ({}):",
            context.dep_crates.len()
        );
        println!("───────────────────────────────");
        for (i, crate_name) in context.dep_crates.iter().take(15).enumerate() {
            println!("{:2}. {}", i + 1, crate_name);
        }
        if context.dep_crates.len() > 15 {
            println!("    ... and {} more", context.dep_crates.len() - 15);
        }
    }

    // Show std crates found
    if !context.std_crates.is_empty() {
        println!(
            "\n🦀 Standard Library Crates ({}):",
            context.std_crates.len()
        );
        println!("──────────────────────────────");
        for (i, crate_name) in context.std_crates.iter().take(10).enumerate() {
            println!("{:2}. {}", i + 1, crate_name);
        }
        if context.std_crates.len() > 10 {
            println!("    ... and {} more", context.std_crates.len() - 10);
        }
    }

    // Show LLVM IR analysis if available
    if let Some(llvm_analysis) = &result.llvm_ir_data {
        println!("\n🔥 LLVM IR Analysis:");
        println!("───────────────────");
        println!("Total LLVM IR lines: {}", llvm_analysis.total_lines);
        println!("Total instantiations: {}", llvm_analysis.total_copies);
        println!("Analyzed {} .ll files", llvm_analysis.analyzed_files.len());

        // Show top 10 most complex non-std functions by LLVM IR lines
        let mut functions: Vec<(&String, &substance::llvm_ir::LlvmInstantiations)> = llvm_analysis
            .instantiations
            .iter()
            .filter(|(func_name, _)| !is_std_function(func_name))
            .collect();
        functions.sort_by_key(|(_, stats)| std::cmp::Reverse(stats.total_lines));

        println!("\n🔍 Top 10 Most Complex User/Dependency Functions (LLVM IR):");
        println!("─────────────────────────────────────────────────────────");
        for (rank, (func_name, stats)) in functions.iter().take(10).enumerate() {
            let percent = stats.total_lines as f64 / llvm_analysis.total_lines as f64 * 100.0;
            println!(
                "{:2}. {:>6} lines ({:>5.1}%) {} instantiations: {}",
                rank + 1,
                stats.total_lines,
                percent,
                stats.copies,
                func_name
            );
        }

        if functions.len() < 10 {
            println!(
                "    (Filtered out {} std library functions)",
                llvm_analysis.instantiations.len() - functions.len()
            );
        }
    } else {
        println!("\n💡 Tip: Add RUSTFLAGS='--emit=llvm-ir' when building to get LLVM IR analysis");
    }

    // Show timing data analysis
    if !timing_data.is_empty() {
        println!("\n⏱️  Crate Build Timing Analysis:");
        println!("─────────────────────────────");

        // Sort by duration (longest first)
        timing_data.sort_by(|a, b| b.duration.partial_cmp(&a.duration).unwrap());

        let total_time: f64 = timing_data.iter().map(|t| t.duration).sum();
        println!("Total build time: {:.3}s", total_time);

        println!("\n🐌 Top 10 Slowest Crates to Build:");
        println!("──────────────────────────────────");
        for (rank, timing) in timing_data.iter().take(10).enumerate() {
            let percent = timing.duration / total_time * 100.0;
            let rmeta_info = if let Some(rmeta_time) = timing.rmeta_time {
                format!(" (rmeta: {:.3}s)", rmeta_time)
            } else {
                String::new()
            };
            println!(
                "{:2}. {:>6.3}s ({:>5.1}%) {}{}",
                rank + 1,
                timing.duration,
                percent,
                timing.crate_name,
                rmeta_info
            );
        }

        if timing_data.len() > 10 {
            let remaining_time: f64 = timing_data.iter().skip(10).map(|t| t.duration).sum();
            println!(
                "    ... and {} more crates ({:.3}s total)",
                timing_data.len() - 10,
                remaining_time
            );
        }
    } else {
        println!("\n💡 Tip: Use RUSTC_BOOTSTRAP=1 cargo build -Z unstable-options --timings=json to get timing data");
    }

    println!("\n✨ Analysis complete!");

    Ok(())
}
