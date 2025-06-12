//! Strongly-typed strings and quantities for better type safety

use std::{collections::HashMap, time::Duration};

use aliri_braid::braid;
use strong_type::StrongType;

/// Special crate name for symbols that couldn't be demangled
pub const UNDEMANGLED_CRATE: &str = "<undemangled>";

// Strongly-typed quantities
#[derive(StrongType)]
#[strong_type(auto_operators)]
pub struct LlvmIrLines(usize);

#[derive(StrongType)]
#[strong_type(auto_operators)]
pub struct NumberOfCopies(usize);

#[derive(StrongType)]
#[strong_type(auto_operators)]
pub struct ByteSize(u64);

#[derive(StrongType)]
#[strong_type(auto_operators)]
pub struct BuildTimeSeconds(f64);

/// A strongly-typed crate name
#[braid]
pub struct CrateName;

/// A mangled symbol name as it appears in the binary (e.g., "_ZN5serde3ser9Serialize9serialize17h...")
#[braid]
pub struct MangledSymbol;

/// A fully demangled symbol name including crate path (e.g., "serde::ser::Serialize::serialize")
#[braid]
pub struct DemangledSymbol;

/// The function/method name part of a symbol without the crate path (e.g., "serialize")
#[braid]
pub struct LlvmFunctionName;

/// A file path for .ll files
#[braid]
pub struct LlvmFilePath;

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
