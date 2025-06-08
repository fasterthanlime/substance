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

use substance::{BloatAnalyzer, AnalysisConfig, ArtifactKind};
use std::collections::HashMap;
use std::process::Command;
use std::path::PathBuf;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔨 Building project with JSON output...");
    
    // Step 1: Run cargo build with JSON output for examples
    let output = Command::new("cargo")
        .args(["build", "--examples", "--message-format=json"])
        .output()?;

    if !output.status.success() {
        eprintln!("❌ Cargo build failed");
        eprintln!("stdout: {}", std::str::from_utf8(&output.stdout).unwrap_or("<invalid utf8>"));
        eprintln!("stderr: {}", std::str::from_utf8(&output.stderr).unwrap_or("<invalid utf8>"));
        std::process::exit(1);
    }

    let stdout = std::str::from_utf8(&output.stdout)?;
    let json_lines: Vec<&str> = stdout.lines().collect();

    println!("✅ Build completed successfully");
    
    // Step 2: Parse cargo metadata using the library
    println!("📊 Parsing cargo metadata...");
    let context = BloatAnalyzer::from_cargo_metadata(
        &json_lines,
        &PathBuf::from("target"),
        None // auto-detect target triple
    )?;

    println!("Target triple: {}", context.target_triple);
    println!("Found {} artifacts", context.artifacts.len());
    
    // Step 3: Find the analyze_binary example to analyze
    let binary_artifact = context.artifacts.iter()
        .find(|a| a.kind == ArtifactKind::Binary && a.name == "analyze_binary")
        .ok_or("analyze_binary example not found - make sure it built successfully")?;

    println!("📈 Analyzing binary: {} ({})", 
             binary_artifact.name, 
             binary_artifact.path.display());

    // Step 4: Analyze the binary
    let config = AnalysisConfig {
        symbols_section: None, // Use default .text section
        split_std: false,      // Group std crates together
    };

    let result = BloatAnalyzer::analyze_binary(
        &binary_artifact.path,
        &context,
        &config,
    )?;

    // Step 5: Display results
    println!("\n📊 Analysis Results:");
    println!("─────────────────────");
    println!("File size:    {} bytes ({})", 
             result.file_size, 
             format_bytes(result.file_size));
    println!("Text section: {} bytes ({})", 
             result.text_size, 
             format_bytes(result.text_size));
    println!("Text/File:    {:.1}%", 
             result.text_size as f64 / result.file_size as f64 * 100.0);
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
        println!("{:2}. {:>8} ({:>5.1}%) {}", 
                 rank + 1,
                 format_bytes(symbol.size),
                 percent,
                 symbol.name.trimmed);
    }

    // Calculate crate sizes by analyzing all symbols
    println!("\n📦 Top 10 Biggest Crates:");
    println!("─────────────────────────");
    
    let mut crate_sizes: HashMap<String, u64> = HashMap::new();
    
    for symbol in &result.symbols {
        let (crate_name, _is_exact) = substance::crate_name::from_sym(&context, config.split_std, &symbol.name);
        *crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
    }
    
    // Sort crates by size
    let mut crate_list: Vec<(&String, &u64)> = crate_sizes.iter().collect();
    crate_list.sort_by_key(|(_name, &size)| std::cmp::Reverse(size));
    
    for (rank, (crate_name, &size)) in crate_list.iter().take(10).enumerate() {
        let file_percent = size as f64 / result.file_size as f64 * 100.0;
        let text_percent = size as f64 / result.text_size as f64 * 100.0;
        println!("{:2}. {:>8} bytes ({:>5.1}% file, {:>5.1}% text) {}", 
                 rank + 1,
                 format_bytes(size),
                 file_percent,
                 text_percent,
                 crate_name);
    }
    
    let remaining_crates = crate_list.len().saturating_sub(10);
    if remaining_crates > 0 {
        let remaining_size: u64 = crate_list.iter().skip(10).map(|(_, &size)| size).sum();
        println!("    ... and {} more crates ({} total)", remaining_crates, format_bytes(remaining_size));
    }

    // Show dependency crates found
    if !context.dep_crates.is_empty() {
        println!("\n📋 Dependency Crates Found ({}):", context.dep_crates.len());
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
        println!("\n🦀 Standard Library Crates ({}):", context.std_crates.len());
        println!("──────────────────────────────");
        for (i, crate_name) in context.std_crates.iter().take(10).enumerate() {
            println!("{:2}. {}", i + 1, crate_name);
        }
        if context.std_crates.len() > 10 {
            println!("    ... and {} more", context.std_crates.len() - 10);
        }
    }

    println!("\n✨ Analysis complete!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analysis_workflow() {
        // This is more of an integration test that would run in CI
        // For now, just ensure the main function compiles
        assert!(true);
    }
}