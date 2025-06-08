#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]

use std::{fmt, fs, path, str};

use binfarce::ar;
use binfarce::demangle::SymbolData;
use binfarce::elf32;
use binfarce::elf64;
use binfarce::macho;
use binfarce::pe;
use binfarce::ByteOrder;
use binfarce::Format;
use facet::Facet;
use multimap::MultiMap;

pub mod crate_name;
pub mod llvm_ir;

// Cargo JSON metadata structures
#[derive(Debug, Facet)]
struct CargoMessage {
    reason: Option<String>,
    #[facet(default)]
    target: Option<CargoTarget>,
    #[facet(default)]
    filenames: Option<Vec<String>>,
}

#[derive(Debug, Facet)]
struct CargoTarget {
    name: Option<String>,
    crate_types: Option<Vec<String>>,
}

// Re-export important types
pub use binfarce::demangle::SymbolData as BinarySymbol;

// Core library types that will be moved from main.rs
#[derive(Debug)]
pub struct BloatAnalyzer;

#[derive(Debug)]
pub struct BuildContext {
    pub target_triple: String,
    pub artifacts: Vec<Artifact>,
    pub std_crates: Vec<String>,
    pub dep_crates: Vec<String>,
    pub deps_symbols: MultiMap<String, String>,
}

#[derive(Debug)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub name: String,
    pub path: path::PathBuf,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ArtifactKind {
    Binary,
    Library,
    DynLib,
}

#[derive(Debug, Default)]
pub struct AnalysisConfig {
    pub symbols_section: Option<String>,
    pub split_std: bool,
    /// Whether to also analyze LLVM IR files (requires --emit=llvm-ir during build)
    pub analyze_llvm_ir: bool,
    /// Optional target directory to search for .ll files (defaults to "target")
    pub target_dir: Option<std::path::PathBuf>,
}

pub struct AnalysisResult {
    pub file_size: u64,
    pub text_size: u64,
    pub symbols: Vec<SymbolData>,
    pub section_name: Option<String>,
    /// LLVM IR analysis data (only present if LLVM IR files were analyzed)
    pub llvm_ir_data: Option<LlvmIrAnalysis>,
}

#[derive(Debug, Clone)]
pub struct LlvmIrAnalysis {
    /// Function instantiation data keyed by demangled function name
    pub instantiations: std::collections::HashMap<String, crate::llvm_ir::LlvmInstantiations>,
    /// Total LLVM IR lines across all functions
    pub total_lines: usize,
    /// Total number of function instantiations
    pub total_copies: usize,
    /// Paths to .ll files that were analyzed
    pub analyzed_files: Vec<std::path::PathBuf>,
}

#[derive(Debug)]
pub struct Method {
    pub name: String,
    pub crate_name: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct Crate {
    pub name: String,
    pub size: u64,
}

// Error types will be moved here
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum BloatError {
    StdDirNotFound(path::PathBuf),
    RustcFailed,
    CargoError(String),
    CargoMetadataFailed,
    CargoBuildFailed,
    UnsupportedCrateType,
    OpenFailed(path::PathBuf),
    InvalidCargoOutput,
    NoArtifacts,
    UnsupportedFileFormat(path::PathBuf),
    ParsingError(binfarce::ParseError),
    PdbError(pdb::Error),
    TargetDetectionFailed,
}

impl From<binfarce::ParseError> for BloatError {
    fn from(e: binfarce::ParseError) -> Self {
        BloatError::ParsingError(e)
    }
}

impl From<binfarce::UnexpectedEof> for BloatError {
    fn from(_: binfarce::UnexpectedEof) -> Self {
        BloatError::ParsingError(binfarce::ParseError::UnexpectedEof)
    }
}

impl From<pdb::Error> for BloatError {
    fn from(e: pdb::Error) -> Self {
        BloatError::PdbError(e)
    }
}

impl fmt::Display for BloatError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            BloatError::StdDirNotFound(ref path) => {
                write!(
                    f,
                    "failed to find a dir with std libraries. Expected location: {}",
                    path.display()
                )
            }
            BloatError::RustcFailed => {
                write!(f, "failed to execute 'rustc'. It should be in the PATH")
            }
            BloatError::CargoError(ref msg) => {
                write!(f, "{}", msg)
            }
            BloatError::CargoMetadataFailed => {
                write!(f, "failed to execute 'cargo'. It should be in the PATH")
            }
            BloatError::CargoBuildFailed => {
                write!(f, "failed to execute 'cargo build'. Probably a build error")
            }
            BloatError::UnsupportedCrateType => {
                write!(
                    f,
                    "only 'bin', 'dylib' and 'cdylib' crate types are supported"
                )
            }
            BloatError::OpenFailed(ref path) => {
                write!(f, "failed to open a file '{}'", path.display())
            }
            BloatError::InvalidCargoOutput => {
                write!(f, "failed to parse 'cargo' output")
            }
            BloatError::NoArtifacts => {
                write!(f, "'cargo' does not produce any build artifacts")
            }
            BloatError::UnsupportedFileFormat(ref path) => {
                write!(f, "'{}' has an unsupported file format", path.display())
            }
            BloatError::ParsingError(ref e) => {
                write!(f, "parsing failed cause '{}'", e)
            }
            BloatError::PdbError(ref e) => {
                write!(f, "error parsing pdb file cause '{}'", e)
            }
            BloatError::TargetDetectionFailed => {
                write!(f, "failed to detect target triple")
            }
        }
    }
}

impl std::error::Error for BloatError {}

// Placeholder implementations - will be filled in subsequent steps
impl BloatAnalyzer {
    pub fn from_cargo_metadata(
        json_messages: &[&str],
        _target_dir: &path::Path,
        target_triple: Option<&str>,
    ) -> Result<BuildContext, BloatError> {
        // Get target triple
        let target_triple = if let Some(triple) = target_triple {
            triple.to_string()
        } else {
            get_default_target()?
        };

        // Parse cargo JSON messages to extract artifacts
        let mut artifacts = Vec::new();
        for line in json_messages {
            let build: CargoMessage = facet_json::from_str(line).map_err(|e| {
                eprintln!("Failed to parse JSON line: {}", line);
                eprintln!("Error: {:?}", e);
                BloatError::InvalidCargoOutput
            })?;

            // Only process compiler-artifact messages
            if build.reason.as_deref() != Some("compiler-artifact") {
                continue;
            }

            if let Some(target) = &build.target {
                if let Some(target_name) = &target.name {
                    if let (Some(filenames), Some(crate_types)) =
                        (&build.filenames, &target.crate_types)
                    {
                        for (path, crate_type) in filenames.iter().zip(crate_types.iter()) {
                            let kind = match crate_type.as_str() {
                                "bin" => ArtifactKind::Binary,
                                "lib" | "rlib" => ArtifactKind::Library,
                                "dylib" | "cdylib" => ArtifactKind::DynLib,
                                _ => continue, // Simply ignore.
                            };

                            artifacts.push(Artifact {
                                kind,
                                name: target_name.replace('-', "_"),
                                path: path::PathBuf::from(path),
                            });
                        }
                    }
                }
            }
        }

        if artifacts.is_empty() {
            return Err(BloatError::NoArtifacts);
        }

        // Collect rlib paths from artifacts
        let mut rlib_paths = Vec::new();
        let mut dep_crates = Vec::new();
        for artifact in &artifacts {
            dep_crates.push(artifact.name.clone());

            if artifact.kind == ArtifactKind::Library {
                rlib_paths.push((artifact.name.clone(), artifact.path.clone()));
            }
        }

        dep_crates.dedup();
        dep_crates.sort();

        // Get std crates - always collect them since we can't tell if build-std was used from JSON
        let target_dylib_path = stdlibs_dir(&target_triple)?;
        let std_paths = collect_rlib_paths(&target_dylib_path);
        let mut std_crates: Vec<String> = std_paths.iter().map(|v| v.0.clone()).collect();
        rlib_paths.extend_from_slice(&std_paths);
        std_crates.sort();

        // Remove std crates that were explicitly added as dependencies.
        for c in &dep_crates {
            if let Some(idx) = std_crates.iter().position(|v| v == c) {
                std_crates.remove(idx);
            }
        }

        // Build symbol mapping
        let deps_symbols = collect_deps_symbols(rlib_paths)?;

        Ok(BuildContext {
            target_triple,
            artifacts,
            std_crates,
            dep_crates,
            deps_symbols,
        })
    }

    pub fn analyze_binary(
        binary_path: &path::Path,
        _context: &BuildContext,
        config: &AnalysisConfig,
    ) -> Result<AnalysisResult, BloatError> {
        let section_name = config.symbols_section.as_deref().unwrap_or(".text");
        let mut result = collect_self_data(binary_path, section_name)?;
        
        // Optionally add LLVM IR analysis
        if config.analyze_llvm_ir {
            let target_dir = config.target_dir.as_deref().unwrap_or(path::Path::new("target"));
            let crate_name = binary_path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.split('-').next().unwrap_or(s));
            
            match Self::analyze_llvm_ir_from_target_dir(target_dir, crate_name) {
                Ok(llvm_analysis) => {
                    result.llvm_ir_data = Some(llvm_analysis);
                }
                Err(_) => {
                    // Don't fail the entire analysis if LLVM IR is not available
                    // This allows graceful degradation
                }
            }
        }
        
        Ok(result)
    }

    pub fn analyze_binary_simple(
        _binary_path: &path::Path,
        _config: &AnalysisConfig,
    ) -> Result<AnalysisResult, BloatError> {
        todo!("Will be implemented later")
    }

    /// Analyze LLVM IR files in the target directory
    pub fn analyze_llvm_ir_from_target_dir(
        target_dir: &path::Path,
        crate_name: Option<&str>,
    ) -> Result<LlvmIrAnalysis, BloatError> {
        let ll_files = find_llvm_ir_files(target_dir, crate_name)?;
        
        if ll_files.is_empty() {
            return Err(BloatError::CargoError(
                "No LLVM IR files found. Make sure to build with RUSTFLAGS='--emit=llvm-ir'".to_string()
            ));
        }

        let mut combined_instantiations = std::collections::HashMap::new();
        let mut total_lines = 0;
        let mut total_copies = 0;

        for ll_file in &ll_files {
            let data = std::fs::read(ll_file).map_err(|_| BloatError::OpenFailed(ll_file.clone()))?;
            let instantiations = crate::llvm_ir::analyze_llvm_ir_data(&data);
            
            for (func_name, stats) in instantiations {
                let entry = combined_instantiations
                    .entry(func_name)
                    .or_insert_with(crate::llvm_ir::LlvmInstantiations::default);
                entry.copies += stats.copies;
                entry.total_lines += stats.total_lines;
                total_lines += stats.total_lines;
                total_copies += stats.copies;
            }
        }

        Ok(LlvmIrAnalysis {
            instantiations: combined_instantiations,
            total_lines,
            total_copies,
            analyzed_files: ll_files,
        })
    }

    /// Analyze a single LLVM IR file
    pub fn analyze_llvm_ir_file(
        ll_file_path: &path::Path,
    ) -> Result<std::collections::HashMap<String, crate::llvm_ir::LlvmInstantiations>, BloatError> {
        let data = std::fs::read(ll_file_path).map_err(|_| BloatError::OpenFailed(ll_file_path.to_owned()))?;
        Ok(crate::llvm_ir::analyze_llvm_ir_data(&data))
    }
}

impl BuildContext {
    pub fn from_target_dir(
        _target_dir: &path::Path,
        _target_triple: &str,
    ) -> Result<Self, BloatError> {
        todo!("Will be implemented in step 9")
    }

    pub fn from_rlib_paths(
        _binary_path: &path::Path,
        _rlib_paths: &[(String, path::PathBuf)],
        _target_triple: &str,
    ) -> Result<Self, BloatError> {
        todo!("Will be implemented in step 9")
    }

    pub fn minimal(_target_triple: &str) -> Result<Self, BloatError> {
        todo!("Will be implemented in step 9")
    }
}

// Binary parsing utility functions
fn map_file(path: &path::Path) -> Result<memmap2::Mmap, BloatError> {
    let file = fs::File::open(path).map_err(|_| BloatError::OpenFailed(path.to_owned()))?;
    let file =
        unsafe { memmap2::Mmap::map(&file).map_err(|_| BloatError::OpenFailed(path.to_owned()))? };
    Ok(file)
}

fn collect_self_data(path: &path::Path, section_name: &str) -> Result<AnalysisResult, BloatError> {
    let data = &map_file(path)?;

    let mut d = match binfarce::detect_format(data) {
        Format::Elf32 { byte_order: _ } => collect_elf_data(path, data, section_name)?,
        Format::Elf64 { byte_order: _ } => collect_elf_data(path, data, section_name)?,
        Format::Macho => collect_macho_data(data)?,
        Format::PE => collect_pe_data(path, data)?,
        Format::Unknown => return Err(BloatError::UnsupportedFileFormat(path.to_owned())),
    };

    // Multiple symbols may point to the same address.
    // Remove duplicates.
    d.symbols.sort_by_key(|v| v.address);
    d.symbols.dedup_by_key(|v| v.address);

    d.file_size = fs::metadata(path).unwrap().len();

    Ok(d)
}

fn collect_elf_data(
    path: &path::Path,
    data: &[u8],
    section_name: &str,
) -> Result<AnalysisResult, BloatError> {
    let is_64_bit = match data[4] {
        1 => false,
        2 => true,
        _ => return Err(BloatError::UnsupportedFileFormat(path.to_owned())),
    };

    let byte_order = match data[5] {
        1 => ByteOrder::LittleEndian,
        2 => ByteOrder::BigEndian,
        _ => return Err(BloatError::UnsupportedFileFormat(path.to_owned())),
    };

    let (symbols, text_size) = if is_64_bit {
        elf64::parse(data, byte_order)?.symbols(section_name)?
    } else {
        elf32::parse(data, byte_order)?.symbols(section_name)?
    };

    let d = AnalysisResult {
        symbols,
        file_size: 0,
        text_size,
        section_name: Some(section_name.to_owned()),
        llvm_ir_data: None,
    };

    Ok(d)
}

fn collect_macho_data(data: &[u8]) -> Result<AnalysisResult, BloatError> {
    let (symbols, text_size) = macho::parse(data)?.symbols()?;
    let d = AnalysisResult {
        symbols,
        file_size: 0,
        text_size,
        section_name: None,
        llvm_ir_data: None,
    };

    Ok(d)
}

fn collect_pe_data(path: &path::Path, data: &[u8]) -> Result<AnalysisResult, BloatError> {
    let (symbols, text_size) = pe::parse(data)?.symbols()?;

    // `pe::parse` will return zero symbols for an executable built with MSVC.
    if symbols.is_empty() {
        let pdb_path = {
            let file_name = if let Some(file_name) = path.file_name() {
                if let Some(file_name) = file_name.to_str() {
                    file_name.replace('-', "_")
                } else {
                    return Err(BloatError::OpenFailed(path.to_owned()));
                }
            } else {
                return Err(BloatError::OpenFailed(path.to_owned()));
            };
            path.with_file_name(file_name).with_extension("pdb")
        };

        collect_pdb_data(&pdb_path, text_size)
    } else {
        Ok(AnalysisResult {
            symbols,
            file_size: 0,
            text_size,
            section_name: None,
            llvm_ir_data: None,
        })
    }
}

fn collect_pdb_data(pdb_path: &path::Path, text_size: u64) -> Result<AnalysisResult, BloatError> {
    use pdb::FallibleIterator;

    let file = fs::File::open(pdb_path).map_err(|_| BloatError::OpenFailed(pdb_path.to_owned()))?;
    let mut pdb = pdb::PDB::open(file)?;

    let dbi = pdb.debug_information()?;
    let symbol_table = pdb.global_symbols()?;
    let address_map = pdb.address_map()?;

    let mut out_symbols = Vec::new();

    // Collect the PublicSymbols.
    let mut public_symbols = Vec::new();

    let mut symbols = symbol_table.iter();
    while let Ok(Some(symbol)) = symbols.next() {
        if let Ok(pdb::SymbolData::Public(data)) = symbol.parse() {
            if data.code || data.function {
                public_symbols.push((data.offset, data.name.to_string().into_owned()));
            }
        }
    }

    let mut modules = dbi.modules()?;
    while let Some(module) = modules.next()? {
        let info = match pdb.module_info(&module)? {
            Some(info) => info,
            None => continue,
        };
        let mut symbols = info.symbols()?;
        while let Some(symbol) = symbols.next()? {
            if let Ok(pdb::SymbolData::Public(data)) = symbol.parse() {
                if data.code || data.function {
                    public_symbols.push((data.offset, data.name.to_string().into_owned()));
                }
            }
        }
    }

    let cmp_offsets = |a: &pdb::PdbInternalSectionOffset, b: &pdb::PdbInternalSectionOffset| {
        a.section.cmp(&b.section).then(a.offset.cmp(&b.offset))
    };
    public_symbols.sort_unstable_by(|a, b| cmp_offsets(&a.0, &b.0));

    // Now find the Procedure symbols in all modules
    // and if possible the matching PublicSymbol record with the mangled name.
    let mut handle_proc = |proc: pdb::ProcedureSymbol| {
        let mangled_symbol = public_symbols
            .binary_search_by(|probe| {
                let low = cmp_offsets(&probe.0, &proc.offset);
                let high = cmp_offsets(&probe.0, &(proc.offset + proc.len));

                use std::cmp::Ordering::*;
                match (low, high) {
                    // Less than the low bound -> less.
                    (Less, _) => Less,
                    // More than the high bound -> greater.
                    (_, Greater) => Greater,
                    _ => Equal,
                }
            })
            .ok()
            .map(|x| &public_symbols[x]);

        let demangled_name = proc.name.to_string().into_owned();
        out_symbols.push((
            proc.offset.to_rva(&address_map),
            proc.len as u64,
            demangled_name,
            mangled_symbol,
        ));
    };

    let mut symbols = symbol_table.iter();
    while let Ok(Some(symbol)) = symbols.next() {
        if let Ok(pdb::SymbolData::Procedure(proc)) = symbol.parse() {
            handle_proc(proc);
        }
    }

    let mut modules = dbi.modules()?;
    while let Some(module) = modules.next()? {
        let info = match pdb.module_info(&module)? {
            Some(info) => info,
            None => continue,
        };

        let mut symbols = info.symbols()?;

        while let Some(symbol) = symbols.next()? {
            if let Ok(pdb::SymbolData::Procedure(proc)) = symbol.parse() {
                handle_proc(proc);
            }
        }
    }

    let symbols = out_symbols
        .into_iter()
        .filter_map(|(address, size, unmangled_name, mangled_name)| {
            address.map(|address| SymbolData {
                name: mangled_name
                    .map(|(_, mangled_name)| binfarce::demangle::SymbolName::demangle(mangled_name))
                    // Assume the Symbol record name is unmangled if we didn't find one.
                    // Note that unmangled names stored in PDB have a different format from
                    // one stored in binaries itself. Specifically they do not include hash
                    // and can have a bit different formatting.
                    // We also assume that a Legacy mangling scheme were used.
                    .unwrap_or_else(|| binfarce::demangle::SymbolName {
                        complete: unmangled_name.clone(),
                        trimmed: unmangled_name.clone(),
                        crate_name: None,
                        kind: binfarce::demangle::Kind::Legacy,
                    }),
                address: address.0 as u64,
                size,
            })
        })
        .collect();

    let d = AnalysisResult {
        symbols,
        file_size: 0,
        text_size,
        section_name: None,
        llvm_ir_data: None,
    };

    Ok(d)
}

fn collect_deps_symbols(
    libs: Vec<(String, path::PathBuf)>,
) -> Result<MultiMap<String, String>, BloatError> {
    let mut map = MultiMap::new();

    for (name, path) in libs {
        let file = map_file(&path)?;
        for sym in ar::parse(&file)? {
            map.insert(sym, name.clone());
        }
    }

    for (_, v) in map.iter_all_mut() {
        v.dedup();
    }

    Ok(map)
}

fn collect_rlib_paths(deps_dir: &path::Path) -> Vec<(String, path::PathBuf)> {
    let mut rlib_paths: Vec<(String, path::PathBuf)> = Vec::new();
    if let Ok(entries) = fs::read_dir(deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(Some("rlib")) = path.extension().map(|s| s.to_str()) {
                let mut stem = path.file_stem().unwrap().to_str().unwrap().to_string();
                if let Some(idx) = stem.bytes().position(|b| b == b'-') {
                    stem.drain(idx..);
                }

                stem.drain(0..3); // trim 'lib'

                rlib_paths.push((stem, path));
            }
        }
    }

    rlib_paths.sort_by(|a, b| a.0.cmp(&b.0));

    rlib_paths
}

fn stdlibs_dir(target_triple: &str) -> Result<path::PathBuf, BloatError> {
    use std::process::Command;

    // Support xargo by applying the rustflags
    // This is meant to match how cargo handles the RUSTFLAG environment
    // variable.
    // See https://github.com/rust-lang/cargo/blob/69aea5b6f69add7c51cca939a79644080c0b0ba0
    // /src/cargo/core/compiler/build_context/target_info.rs#L434-L441
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_else(|_| String::new());

    let rustflags = rustflags
        .split(' ')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(AsRef::<std::ffi::OsStr>::as_ref);

    let output = Command::new("rustc")
        .args(rustflags)
        .arg("--print=sysroot")
        .output()
        .map_err(|_| BloatError::RustcFailed)?;

    let stdout = str::from_utf8(&output.stdout).unwrap();

    // From the `cargo` itself (this is a one long link):
    // https://github.com/rust-lang/cargo/blob/065e3ef98d3edbce5c9e66d927d9ac9944cc6639
    // /src/cargo/core/compiler/build_context/target_info.rs#L130..L133
    let mut rustlib = path::PathBuf::from(stdout.trim());
    rustlib.push("lib");
    rustlib.push("rustlib");
    rustlib.push(target_triple);
    rustlib.push("lib");

    if !rustlib.exists() {
        return Err(BloatError::StdDirNotFound(rustlib));
    }

    Ok(rustlib)
}

fn get_default_target() -> Result<String, BloatError> {
    use std::process::Command;

    let output = Command::new("rustc")
        .arg("-Vv")
        .output()
        .map_err(|_| BloatError::RustcFailed)?;

    let stdout = str::from_utf8(&output.stdout).unwrap();
    for line in stdout.lines() {
        if line.starts_with("host:") {
            return Ok(line[6..].to_owned());
        }
    }

    Err(BloatError::RustcFailed)
}

/// Find LLVM IR (.ll) files in the target directory
fn find_llvm_ir_files(target_dir: &path::Path, crate_name: Option<&str>) -> Result<Vec<path::PathBuf>, BloatError> {
    let mut ll_files = Vec::new();
    
    // Search in multiple potential locations within target directory
    let search_dirs = vec![
        target_dir.join("debug"),
        target_dir.join("debug").join("deps"),
        target_dir.join("debug").join("examples"),
        target_dir.join("debug").join("incremental"),
    ];
    
    for search_dir in search_dirs {
        if search_dir.exists() {
            find_ll_files_in_dir(&search_dir, crate_name, &mut ll_files)?;
        }
    }
    
    Ok(ll_files)
}

fn find_ll_files_in_dir(
    dir: &path::Path, 
    crate_name: Option<&str>, 
    ll_files: &mut Vec<path::PathBuf>
) -> Result<(), BloatError> {
    let entries = fs::read_dir(dir).map_err(|_| BloatError::OpenFailed(dir.to_owned()))?;
    
    for entry in entries.flatten() {
        let path = entry.path();
        
        if path.is_dir() {
            // Recursively search subdirectories (like incremental compilation dirs)
            find_ll_files_in_dir(&path, crate_name, ll_files)?;
        } else if let Some(extension) = path.extension() {
            if extension == "ll" {
                // If crate_name is specified, filter by it
                if let Some(name) = crate_name {
                    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if file_stem.starts_with(name) || file_stem.contains(&format!("-{}", name)) {
                            ll_files.push(path);
                        }
                    }
                } else {
                    // Include all .ll files if no crate name specified
                    ll_files.push(path);
                }
            }
        }
    }
    
    Ok(())
}
