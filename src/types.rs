//! Strongly-typed strings and quantities for better type safety

use std::collections::HashMap;

use aliri_braid::braid;
use strong_type::StrongType;

use crate::cargo::TimingInfo;

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

/// A fully demangled symbol name including crate path (e.g., "ariadne::write::<impl ariadne::Report<S>>::write_for_stream::h8f6ced0befa72529")
#[braid]
pub struct DemangledSymbol;

impl DemangledSymbol {
    /// Removes a Rust symbol hash suffix of the form `::h[0-9a-f]{16}` from the demangled symbol,
    /// if present, and returns a new DemangledSymbol with the hash removed.
    ///
    /// Example:
    ///     "foo::bar::h9e2b8a2a7a115765" -> "foo::bar"
    ///     "serde::ser::Serialize::serialize" (no hash) -> unchanged
    pub fn strip_hash(&self) -> DemangledSymbolWithoutHash {
        // Get the inner string representation
        let s = self.as_str();
        // Find the position of the hash suffix, if it matches ::h...
        if let Some(hash_pos) = s.rfind("::h") {
            // Check if the substring after ::h is exactly 16 lowercase hex digits
            let suffix = &s[(hash_pos + 3)..];
            if suffix.len() == 16
                && suffix
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && c.is_ascii_lowercase() || c.is_ascii_digit())
            {
                // Truncate the string up to hash_pos
                return DemangledSymbolWithoutHash::from(s[..hash_pos].to_string());
            }
        }
        // No hash matched; return DemangledSymbolWithoutHash constructed from self
        DemangledSymbolWithoutHash::from(s.to_string())
    }
}

/// A fully demangled symbol name excluding the hash (e.g., "ariadne::write::<impl ariadne::Report<S>>::write_for_stream::h8f6ced0befa72529")
#[braid]
pub struct DemangledSymbolWithoutHash;

/// The function/method name part of a symbol without the crate path (e.g., "serialize")
#[braid]
pub struct LlvmFunctionName;

/// A file path for .ll files
#[braid]
pub struct LlvmFilePath;

/// Info about a given crate
pub struct Crate {
    /// Something like `std`, `ks_facet`, etc.
    pub name: CrateName,

    /// Timing info
    pub timing_info: Option<TimingInfo>,

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
