#!/usr/bin/env cargo

//! Example: Compare debug and release builds
//!
//! This example shows how to:
//! 1. Build both debug and release versions using BuildRunner
//! 2. Analyze both binaries
//! 3. Compare the analysis results
//! 4. Display size differences sorted by relative change
//!
//! Usage: cargo run --example compare_builds

use camino::Utf8PathBuf;
use std::collections::HashMap;
use std::fs;
use substance::{AnalysisComparison, AnalysisConfig, ArtifactKind, BloatAnalyzer, BuildRunner, BuildType};

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
                println!("🧹 Cleaned up temporary directory: {}", self.temp_dir);
            }
        }
    }
}

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

fn format_bytes_signed(bytes: i64) -> String {
    let abs_bytes = bytes.unsigned_abs();
    let formatted = format_bytes(abs_bytes);
    if bytes < 0 {
        format!("-{}", formatted)
    } else {
        format!("+{}", formatted)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a temporary target directory
    let temp_target_dir = Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .expect("temp dir is not UTF-8")
        .join(format!("substance_compare_{}", std::process::id()));
    
    // Ensure cleanup happens even on early return
    let _cleanup_guard = CleanupGuard::new(temp_target_dir.clone());

    println!("🔨 Building debug and release versions...");
    println!("📁 Using target dir: {}", temp_target_dir);

    // Build debug version
    println!("\n🐛 Building debug version...");
    let debug_build = BuildRunner::new(
        "Cargo.toml",
        temp_target_dir.as_std_path(),
        BuildType::Debug,
    )
    .run()?;
    println!("✅ Debug build completed");

    // Build release version
    println!("\n🚀 Building release version...");
    let release_build = BuildRunner::new(
        "Cargo.toml",
        temp_target_dir.as_std_path(),
        BuildType::Release,
    )
    .run()?;
    println!("✅ Release build completed");

    // Find the compare_builds example binary in both builds
    let debug_binary = debug_build
        .context
        .artifacts
        .iter()
        .find(|a| a.kind == ArtifactKind::Binary && a.name == "compare_builds")
        .ok_or("compare_builds example not found in debug build")?;

    let release_binary = release_build
        .context
        .artifacts
        .iter()
        .find(|a| a.kind == ArtifactKind::Binary && a.name == "compare_builds")
        .ok_or("compare_builds example not found in release build")?;

    println!("\n📊 Analyzing binaries...");
    
    // Analyze both binaries
    let config = AnalysisConfig::default();
    let debug_analysis = BloatAnalyzer::analyze_binary(&debug_binary.path, &debug_build.context, &config)?;
    let release_analysis = BloatAnalyzer::analyze_binary(&release_binary.path, &release_build.context, &config)?;

    // Compare analyses
    println!("\n🔍 Comparing debug vs release builds...");
    let comparison = AnalysisComparison::compare(&debug_analysis, &release_analysis)?;

    // Display file size comparison
    println!("\n📏 File Size Comparison:");
    println!("─────────────────────────");
    println!(
        "Debug:   {} ({} bytes)",
        format_bytes(comparison.file_size_diff.file_size_before),
        comparison.file_size_diff.file_size_before
    );
    println!(
        "Release: {} ({} bytes)",
        format_bytes(comparison.file_size_diff.file_size_after),
        comparison.file_size_diff.file_size_after
    );
    let file_size_change = comparison.file_size_diff.file_size_after as i64 - 
                           comparison.file_size_diff.file_size_before as i64;
    let file_size_pct = (file_size_change as f64 / comparison.file_size_diff.file_size_before as f64) * 100.0;
    println!(
        "Change:  {} ({:+.1}%)",
        format_bytes_signed(file_size_change),
        file_size_pct
    );

    println!("\n📊 Text Section Comparison:");
    println!("───────────────────────────");
    println!(
        "Debug:   {} ({} bytes)",
        format_bytes(comparison.file_size_diff.text_size_before),
        comparison.file_size_diff.text_size_before
    );
    println!(
        "Release: {} ({} bytes)",
        format_bytes(comparison.file_size_diff.text_size_after),
        comparison.file_size_diff.text_size_after
    );
    let text_size_change = comparison.file_size_diff.text_size_after as i64 - 
                           comparison.file_size_diff.text_size_before as i64;
    let text_size_pct = (text_size_change as f64 / comparison.file_size_diff.text_size_before as f64) * 100.0;
    println!(
        "Change:  {} ({:+.1}%)",
        format_bytes_signed(text_size_change),
        text_size_pct
    );

    // Analyze symbols by crate (since crate_changes is not implemented yet)
    println!("\n📦 Analyzing crate size changes...");
    
    // Group symbols by crate for debug build
    let mut debug_crate_sizes: HashMap<String, u64> = HashMap::new();
    for symbol in &debug_analysis.symbols {
        let (crate_name, _) = substance::crate_name::from_sym(&debug_build.context, false, &symbol.name);
        *debug_crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
    }
    
    // Group symbols by crate for release build
    let mut release_crate_sizes: HashMap<String, u64> = HashMap::new();
    for symbol in &release_analysis.symbols {
        let (crate_name, _) = substance::crate_name::from_sym(&release_build.context, false, &symbol.name);
        *release_crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
    }
    
    // Create crate changes
    let mut crate_changes = Vec::new();
    let mut all_crates = std::collections::HashSet::new();
    all_crates.extend(debug_crate_sizes.keys().cloned());
    all_crates.extend(release_crate_sizes.keys().cloned());
    
    for crate_name in all_crates {
        let size_before = debug_crate_sizes.get(&crate_name).copied();
        let size_after = release_crate_sizes.get(&crate_name).copied();
        
        let change = substance::CrateChange {
            name: crate_name,
            size_before,
            size_after,
        };
        crate_changes.push(change);
    }
    
    // Sort crates by absolute percent change
    crate_changes.sort_by(|a, b| {
        let a_pct = a.percent_change().map(|p| p.abs()).unwrap_or(0.0);
        let b_pct = b.percent_change().map(|p| p.abs()).unwrap_or(0.0);
        b_pct.partial_cmp(&a_pct).unwrap()
    });
    
    println!("\n📊 Top 20 Crate Size Changes (by relative change):");
    println!("─────────────────────────────────────────────────");
    for (i, change) in crate_changes.iter().take(20).enumerate() {
        match (change.size_before, change.size_after) {
            (Some(before), Some(after)) => {
                let pct = change.percent_change().unwrap();
                let abs_change = change.absolute_change().unwrap();
                println!(
                    "{:2}. {:+6.1}% {} ({} → {}) [{}]",
                    i + 1,
                    pct,
                    change.name,
                    format_bytes(before),
                    format_bytes(after),
                    format_bytes_signed(abs_change)
                );
            }
            (None, Some(after)) => {
                println!(
                    "{:2}.   NEW   {} ({})",
                    i + 1,
                    change.name,
                    format_bytes(after)
                );
            }
            (Some(before), None) => {
                println!(
                    "{:2}. REMOVED {} (was {})",
                    i + 1,
                    change.name,
                    format_bytes(before)
                );
            }
            _ => {}
        }
    }
    
    // Show top symbol changes
    let mut symbol_changes = comparison.symbol_changes.clone();
    symbol_changes.sort_by(|a, b| {
        let a_pct = a.percent_change().map(|p| p.abs()).unwrap_or(0.0);
        let b_pct = b.percent_change().map(|p| p.abs()).unwrap_or(0.0);
        b_pct.partial_cmp(&a_pct).unwrap()
    });
    
    // Filter to only show symbols that changed significantly
    let significant_changes: Vec<_> = symbol_changes
        .into_iter()
        .filter(|s| {
            match (s.size_before, s.size_after) {
                (Some(before), Some(after)) => before != after,
                _ => true, // Include new or removed symbols
            }
        })
        .collect();
    
    println!("\n🔍 Top 20 Symbol Size Changes (by relative change):");
    println!("──────────────────────────────────────────────────");
    for (i, change) in significant_changes.iter().take(20).enumerate() {
        match (change.size_before, change.size_after) {
            (Some(before), Some(after)) => {
                let pct = change.percent_change().unwrap();
                let abs_change = change.absolute_change().unwrap();
                println!(
                    "{:2}. {:+6.1}% {} ({} → {}) [{}]",
                    i + 1,
                    pct,
                    change.demangled,
                    format_bytes(before),
                    format_bytes(after),
                    format_bytes_signed(abs_change)
                );
            }
            (None, Some(after)) => {
                println!(
                    "{:2}.   NEW   {} ({})",
                    i + 1,
                    change.demangled,
                    format_bytes(after)
                );
            }
            (Some(before), None) => {
                println!(
                    "{:2}. REMOVED {} (was {})",
                    i + 1,
                    change.demangled,
                    format_bytes(before)
                );
            }
            _ => {}
        }
    }
    
    println!("\n✨ Comparison complete!");
    
    Ok(())
}