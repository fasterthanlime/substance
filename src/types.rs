//! Strongly-typed strings and quantities for better type safety

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
