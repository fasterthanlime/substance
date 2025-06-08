//! Simple example showing substance library usage without cargo integration
//! 
//! This example demonstrates analyzing a pre-built binary directly.

use substance::{BloatAnalyzer, AnalysisConfig, BuildContext};
use std::collections::HashMap;
use std::path::PathBuf;
use std::env;

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
    // Get binary path from command line or use a default
    let binary_path = env::args().nth(1).unwrap_or_else(|| {
        // Try to find the example binary itself
        "target/debug/examples/analyze_binary".to_string()
    });
    
    let path = PathBuf::from(&binary_path);
    
    if !path.exists() {
        eprintln!("Binary not found: {}", binary_path);
        eprintln!("Usage: cargo run --example simple_analysis [path/to/binary]");
        std::process::exit(1);
    }
    
    println!("📈 Analyzing binary: {}", path.display());
    
    // Create a minimal context for standalone analysis
    let context = BuildContext {
        target_triple: "aarch64-apple-darwin".to_string(),
        artifacts: vec![],
        std_crates: vec![
            "std".to_string(), 
            "core".to_string(), 
            "alloc".to_string(),
            "proc_macro".to_string(),
        ],
        dep_crates: vec![
            "substance".to_string(),
        ],
        deps_symbols: Default::default(),
    };
    
    let config = AnalysisConfig {
        symbols_section: None, // Use default .text section
        split_std: false,      // Group std crates together
        analyze_llvm_ir: false, // Don't analyze LLVM IR in this simple example
        target_dir: None,      // Use default target directory
    };
    
    let result = BloatAnalyzer::analyze_binary(&path, &context, &config)?;
    
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
    
    println!("\n✨ Analysis complete!");
    
    Ok(())
}