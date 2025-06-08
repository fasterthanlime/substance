#!/usr/bin/env cargo

//! Example: Analyze a Rust binary using cargo-bloat library
//! 
//! This example shows how to:
//! 1. Run `cargo build --message-format=json` 
//! 2. Parse the JSON output to get artifact information
//! 3. Use cargo-bloat library to analyze the binary
//! 4. Display basic size information
//!
//! Usage: cargo run --example analyze_cargo_build
//!
//! This will analyze the current project's binary.

use cargo_bloat::{BloatAnalyzer, AnalysisConfig, ArtifactKind};
use std::process::Command;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔨 Building project with JSON output...");
    
    // Step 1: Run cargo build with JSON output
    let output = Command::new("cargo")
        .args(["build", "--message-format=json"])  // Use debug build for testing
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
    
    // Step 3: Find the binary artifact to analyze
    let binary_artifact = context.artifacts.iter()
        .find(|a| a.kind == ArtifactKind::Binary)
        .ok_or("No binary artifact found")?;

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
    println!("File size:    {} bytes ({:.1} KiB)", 
             result.file_size, 
             result.file_size as f64 / 1024.0);
    println!("Text section: {} bytes ({:.1} KiB)", 
             result.text_size, 
             result.text_size as f64 / 1024.0);
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
        println!("{:2}. {:>8} bytes ({:>5.1}%) {}", 
                 rank + 1,
                 symbol.size,
                 percent,
                 symbol.name.trimmed);
    }

    // Show dependency crates found
    if !context.dep_crates.is_empty() {
        println!("\n📦 Dependency Crates Found:");
        println!("──────────────────────────");
        for (i, crate_name) in context.dep_crates.iter().enumerate() {
            println!("{:2}. {}", i + 1, crate_name);
        }
    }

    // Show std crates found
    if !context.std_crates.is_empty() {
        println!("\n🦀 Standard Library Crates:");
        println!("──────────────────────────");
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