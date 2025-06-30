//! Strongly-typed strings and quantities for better type safety

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use aliri_braid::braid;
use camino::Utf8PathBuf;
use multimap::MultiMap;
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

pub struct BuildContext {
    /// Crate names of libraries found under the libstd `target-libdir`,
    /// something like: `$RUSTUP_HOME/toolchains/stable-$TRIPLE/lib/rustlib/$TRIPLE/lib`
    pub std_crates: Vec<CrateName>,

    /// Crate names of dependencies, found under the unique target for this build
    pub dep_crates: Vec<CrateName>,

    /// Maps mangled symbols to the crate names they belong to
    pub deps_symbols: MultiMap<MangledSymbol, CrateName>,

    /// Optional global timing information (e.g. total build time)
    pub wall_duration: Duration,

    /// Total file size of the binary
    pub file_size: ByteSize,

    /// Size of the .text section
    pub text_size: ByteSize,

    pub crates: Vec<Crate>,
}

/// Symbol, aggregated per crate
#[derive(Clone)]
pub struct AggregateSymbol {
    pub name: DemangledSymbolWithoutHash,

    /// Total size of the symbol across all crates
    pub total_size: ByteSize,

    /// Number of copies we found
    pub copies: NumberOfCopies,

    /// All the crates this symbol was found in. It can be found in
    /// several crates because it might be monomorphized
    pub crates: HashSet<CrateName>,
}

/// LLVM function, aggregated per crate
#[derive(Clone)]
pub struct AggregateLlvmFunction {
    pub name: LlvmFunctionName,

    /// Total number of LLVM IR lines across all crates
    pub total_llvm_lines: LlvmIrLines,

    /// Number of copies we found
    pub copies: NumberOfCopies,

    /// All the crates this function was found in.
    pub crates: HashSet<CrateName>,
}

impl BuildContext {
    /// Returns the total number of LLVM IR lines across all crates in the build context.
    pub fn num_llvm_lines(&self) -> usize {
        self.crates.iter().map(|krate| krate.num_llvm_lines()).sum()
    }

    pub fn all_symbols(&self) -> HashMap<DemangledSymbolWithoutHash, AggregateSymbol> {
        // Aggregate every non-stdlib symbol by its hash-stripped demangled name
        let mut symbol_map: HashMap<DemangledSymbolWithoutHash, AggregateSymbol> = HashMap::new();

        for krate in &self.crates {
            for sym in krate.symbols.values() {
                let hashless = sym.name.strip_hash();

                symbol_map
                    .entry(hashless.clone())
                    .and_modify(|agg| {
                        // Accumulate size
                        agg.total_size += sym.size;
                        // Count another copy of the symbol
                        agg.copies += NumberOfCopies(1);
                        // Track which crate this copy came from
                        agg.crates.insert(krate.name.clone());
                    })
                    .or_insert_with(|| {
                        // First sighting of this symbol
                        let mut crates_set: HashSet<CrateName> = HashSet::new();
                        crates_set.insert(krate.name.clone());

                        AggregateSymbol {
                            name: hashless.clone(),
                            total_size: sym.size,
                            copies: NumberOfCopies(1),
                            crates: crates_set,
                        }
                    });
            }
        }

        symbol_map
    }

    /// Returns a map from LLVM function name (LlvmFunctionName) to its aggregate information,
    /// combining across all crates in the build context, keyed by function name.
    pub fn all_llvm_functions(&self) -> HashMap<LlvmFunctionName, AggregateLlvmFunction> {
        let mut llvm_map: HashMap<LlvmFunctionName, AggregateLlvmFunction> = HashMap::new();

        for krate in &self.crates {
            for func in krate.llvm_functions.values() {
                let fname = func.name.clone();

                llvm_map
                    .entry(fname.clone())
                    .and_modify(|agg| {
                        // Accumulate LLVM IR line count
                        agg.total_llvm_lines += func.lines;
                        // Count another copy
                        agg.copies += func.copies;
                        // Track which crate
                        agg.crates.insert(krate.name.clone());
                    })
                    .or_insert_with(|| {
                        let mut crates_set: HashSet<CrateName> = HashSet::new();
                        crates_set.insert(krate.name.clone());
                        AggregateLlvmFunction {
                            name: fname.clone(),
                            total_llvm_lines: func.lines,
                            copies: func.copies,
                            crates: crates_set,
                        }
                    });
            }
        }

        llvm_map
    }
}

/// An artifact generated by the build — a single `.rlib` file, etc.
pub struct Artifact {
    /// binary, library, or dynlib
    pub kind: ArtifactKind,

    /// crate name, e.g. `facet` or `core`
    pub name: CrateName,

    /// absolute path to the artifact
    pub path: Utf8PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub enum ArtifactKind {
    Binary,
    Library,
    DynLib,
}

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

impl Crate {
    pub fn num_llvm_lines(&self) -> usize {
        self.llvm_functions
            .values()
            .map(|f| f.lines.value())
            .sum::<usize>()
    }
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
