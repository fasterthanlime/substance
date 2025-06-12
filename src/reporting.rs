//! Report generation for binary analysis results
//!
//! This module provides structures and functions for generating comprehensive
//! reports from binary analysis results. Reports can be generated in multiple
//! formats including Markdown, JSON, and plain text.
//!
//! # Report Sections
//!
//! When comparing two versions, the report includes:
//!
//! 1. **Header & Summary** - Overview of changes
//! 2. **Size Comparison** - File and text size changes
//! 3. **Crate Size Changes** - Per-crate size differences
//! 4. **Build Time Changes** - Compilation time analysis
//! 5. **Symbol Changes** - Individual symbol size changes
//! 6. **Current State Analysis** - Top crates and symbols
//! 7. **LLVM IR Analysis** - Monomorphization and instantiation data

use crate::types::{
    ByteSize, CrateName, DemangledSymbol, LlvmFunctionName, LlvmIrLines, NumberOfCopies,
};
use std::{collections::HashMap, time::Duration};

/// Available output formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// For CI
    Markdown,
    /// With colors, for CLI
    PlainText,
}

/// Complete analysis report for a single version
pub struct BuildReport {
    /// For how long `cargo build` ran
    pub build_duration: Duration,

    /// The resulting binary size
    pub file_size: ByteSize,

    /// The size of the .text section
    pub text_size: ByteSize,

    /// All crates with their sizes (for comparison)
    /// HashMap<crate_name, size_bytes>
    pub crates: Vec<Crate>,
}

/// Info about a given crate
pub struct Crate {
    /// Something like `std`, `ks_facet`, etc.
    pub name: CrateName,

    /// Symbols found in the binary
    pub symbols: HashMap<DemangledSymbol, Symbol>,

    /// LLVM functions found in .ll files
    pub llvm_functions: HashMap<LlvmFunctionName, LlvmFunction>,
}

/// Info about a symbol
pub struct Symbol {
    /// A fully demangled symbol name including crate path (e.g., "serde::ser::Serialize::serialize")
    pub name: DemangledSymbol,

    /// The size of this symbol in the .text section
    pub size: ByteSize,
}

/// Info about an LLVM function
pub struct LlvmFunction {
    /// An LLVM function name
    pub name: LlvmFunctionName,

    /// How many lines of LLVM IR this function has
    pub lines: LlvmIrLines,

    /// How many copies of this function exist in the binary
    pub copies: NumberOfCopies,
}
