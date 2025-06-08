# substance

A Rust library for analyzing the size composition of binaries by examining their symbols and mapping them back to their originating crates.

Supports ELF (Linux, BSD), Mach-O (macOS) and PE (Windows) binaries. Originally derived from cargo-bloat but redesigned as a library.

See the original cargo-bloat: <https://github.com/RazrFalcon/cargo-bloat>

## Features

- **Binary format support**: ELF, Mach-O, PE, and PDB debug symbols
- **Crate mapping**: Maps symbols back to their originating Rust crates
- **Cargo integration**: Designed to work with `cargo build --message-format=json`
- **Symbol analysis**: Identifies the largest functions and their sizes
- **LLVM IR analysis**: Analyzes compilation complexity by counting LLVM instruction lines (inspired by [cargo-llvm-lines](https://github.com/dtolnay/cargo-llvm-lines))
- **Flexible configuration**: Customizable symbol sections and std library handling

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
substance = "0.12.1"
```

## Quick Start

```rust
use substance::{BloatAnalyzer, AnalysisConfig, ArtifactKind};
use std::process::Command;
use std::path::PathBuf;

// Run cargo build with JSON output
let output = Command::new("cargo")
    .args(["build", "--bin", "my-binary", "--message-format=json"])
    .output()?;

let stdout = std::str::from_utf8(&output.stdout)?;
let json_lines: Vec<&str> = stdout.lines().collect();

// Parse cargo metadata
let context = BloatAnalyzer::from_cargo_metadata(
    &json_lines,
    &PathBuf::from("target"),
    None // auto-detect target triple
)?;

// Find binary artifact
let binary_artifact = context.artifacts.iter()
    .find(|a| a.kind == ArtifactKind::Binary)
    .unwrap();

// Analyze the binary
let config = AnalysisConfig {
    symbols_section: None, // Use default .text section
    split_std: false,      // Group std crates together
    analyze_llvm_ir: true, // Also analyze LLVM IR (requires --emit=llvm-ir)
    target_dir: None,      // Use default "target" directory
};

let result = BloatAnalyzer::analyze_binary(
    &binary_artifact.path,
    &context,
    &config,
)?;

// Access results
println!("File size: {} bytes", result.file_size);
println!("Text section: {} bytes", result.text_size);
println!("Symbol count: {}", result.symbols.len());

// Access LLVM IR analysis if available
if let Some(llvm_analysis) = &result.llvm_ir_data {
    println!("Total LLVM IR lines: {}", llvm_analysis.total_lines);
    println!("Function instantiations: {}", llvm_analysis.total_copies);
}

// Analyze by crate
use std::collections::HashMap;
let mut crate_sizes: HashMap<String, u64> = HashMap::new();

for symbol in &result.symbols {
    let (crate_name, _is_exact) = substance::crate_name::from_sym(
        &context,
        config.split_std,
        &symbol.name
    );
    *crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
}

// Sort and display biggest crates
let mut crate_list: Vec<(&String, &u64)> = crate_sizes.iter().collect();
crate_list.sort_by_key(|(_name, &size)| std::cmp::Reverse(size));

for (crate_name, &size) in crate_list.iter().take(10) {
    println!("{}: {} bytes", crate_name, size);
}
```

## Core API

### Main Types

- **`BloatAnalyzer`** - Main entry point with static analysis methods
- **`BuildContext`** - Contains crate mappings and target information
- **`AnalysisResult`** - Analysis results with symbols and size information
- **`AnalysisConfig`** - Configuration for analysis behavior

### Key Methods

- **`BloatAnalyzer::from_cargo_metadata()`** - Create build context from cargo JSON
- **`BloatAnalyzer::analyze_binary()`** - Analyze a binary file for symbols
- **`crate_name::from_sym()`** - Map symbol to originating crate

## Example Usage

The repository includes comprehensive examples:

```bash
# Simple binary analysis without cargo integration
cargo run --example simple_analysis

# Full cargo integration workflow
cargo run --example analyze_binary
```

These examples demonstrate:
- Simple binary analysis without cargo integration (`simple_analysis`)
- Full cargo integration workflow (`analyze_binary`)
- Parsing cargo metadata from JSON output
- Analyzing binaries for symbol information
- Displaying largest symbols and crates
- Formatting size information

Example output from `simple_analysis`:
```
📈 Analyzing binary: target/debug/examples/analyze_binary

📊 Analysis Results:
─────────────────────
File size:    2265072 bytes (2.2MiB)
Text section: 943024 bytes (920.9KiB)
Text/File:    41.6%
Symbol count: 3725

🔍 Top 10 Largest Symbols:
─────────────────────────
 1.  27.7KiB (  3.0%) json::parser::Parser::parse
 2.   9.7KiB (  1.1%) std::backtrace_rs::symbolize::gimli::resolve
 3.   9.2KiB (  1.0%) std::backtrace_rs::symbolize::gimli::Context::new
 4.   8.6KiB (  0.9%) analyze_binary::main
 5.   7.1KiB (  0.8%) gimli::read::dwarf::Unit<R>::new

📦 Top 10 Biggest Crates:
─────────────────────────
 1. 560.8KiB bytes ( 25.4% file,  60.9% text) std
 2.  76.7KiB bytes (  3.5% file,   8.3% text) pdb
 3.  76.6KiB bytes (  3.5% file,   8.3% text) binfarce
 4.  45.3KiB bytes (  2.0% file,   4.9% text) json
 5.  25.5KiB bytes (  1.2% file,   2.8% text) substance
```

## Advanced Usage

### Custom Binary Analysis

For analyzing binaries without cargo integration:

```rust
use substance::{BloatAnalyzer, AnalysisConfig, BuildContext};

// Create minimal context for standalone analysis
let context = BuildContext {
    target_triple: "x86_64-unknown-linux-gnu".to_string(),
    artifacts: vec![],
    std_crates: vec!["std".to_string(), "core".to_string(), "alloc".to_string()],
    dep_crates: vec![],
    deps_symbols: Default::default(),
};

let config = AnalysisConfig::default();
let result = BloatAnalyzer::analyze_binary(&binary_path, &context, &config)?;
```

### Configuration Options

```rust
let config = AnalysisConfig {
    symbols_section: Some(".custom_section".to_string()), // Custom symbol section
    split_std: true,           // Split std into core/alloc/etc instead of grouping
    analyze_llvm_ir: true,     // Enable LLVM IR analysis for compilation complexity
    target_dir: Some(PathBuf::from("custom_target")), // Custom target directory
};
```

### LLVM IR Analysis

To enable LLVM IR analysis for understanding compilation complexity:

```bash
# Build with LLVM IR emission
RUSTFLAGS="--emit=llvm-ir" cargo build

# Then analyze with LLVM IR enabled
let config = AnalysisConfig {
    analyze_llvm_ir: true,
    ..Default::default()
};
```

This provides additional insights into:
- Generic function instantiation costs
- Compilation complexity per function
- LLVM IR instruction counts
- Monomorphization impact

## Error Handling

The library provides comprehensive error handling through `BloatError`:

```rust
use substance::BloatError;

match BloatAnalyzer::analyze_binary(&path, &context, &config) {
    Ok(result) => { /* process result */ },
    Err(BloatError::OpenFailed(path)) => {
        eprintln!("Could not open binary: {}", path.display());
    },
    Err(BloatError::UnsupportedFileFormat(path)) => {
        eprintln!("Unsupported binary format: {}", path.display());
    },
    Err(e) => eprintln!("Analysis failed: {}", e),
}
```

## Platform Support

- **Linux**: Full ELF support (32/64-bit)
- **macOS**: Full Mach-O support
- **Windows**: PE support with PDB debug symbols
- **Other Unix**: Basic ELF support

## Performance Notes

- Uses memory mapping for efficient large file access
- Deduplicates symbols to avoid double-counting
- Index-based sorting to minimize memory allocation
- Optimized for binaries up to several hundred MB

## Dependencies

- `binfarce` - Binary format parsing
- `pdb` - Windows debug symbol support
- `memmap2` - Memory-mapped file access
- `json` - Cargo output parsing
- `multimap` - Symbol to crate mapping

## Contributing

This library focuses on accurate binary analysis and clean API design. Contributions should maintain:

- Zero-copy parsing where possible
- Comprehensive error handling
- Cross-platform compatibility
- Clean separation between parsing and analysis

## Attribution

- **Binary analysis**: Originally derived from [cargo-bloat](https://github.com/RazrFalcon/cargo-bloat) by RazrFalcon
- **LLVM IR analysis**: Inspired by [cargo-llvm-lines](https://github.com/dtolnay/cargo-llvm-lines) by dtolnay, which was originally suggested by @eddyb for debugging compiler memory usage and compile times

## License

Licensed under the MIT license.
