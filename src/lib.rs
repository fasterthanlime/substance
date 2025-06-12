#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]

use std::process::{Command, Stdio};
use std::{fmt, fs, path, str};

use binfarce::ar;
use binfarce::demangle::SymbolData;
use binfarce::elf32;
use binfarce::elf64;
use binfarce::macho;
use binfarce::pe;
use binfarce::ByteOrder;
use binfarce::Format;
use compact_str::CompactString;
use facet::Facet;
use log::{debug, error, info, warn};
use multimap::MultiMap;

use crate::types::{CrateName, DemangledSymbol, LlvmFilePath, LlvmIrLines, NumberOfCopies, ByteSize, UNDEMANGLED_CRATE};

pub mod crate_name;
pub mod llvm_ir;
pub mod formatting;
pub mod reporting;
pub mod analysis_ext;
pub mod types;

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

// Timing structures for build analysis
#[derive(Debug, Clone)]
pub struct TimingInfo {
    pub crate_name: String,
    pub duration: f64,
    pub rmeta_time: Option<f64>,
}

#[derive(Debug, Facet)]
struct TimingMessage {
    reason: String,
    package_id: String,
    target: TimingTarget,
    mode: String,
    duration: f64,
    rmeta_time: Option<f64>,
}

#[derive(Debug, Facet)]
struct TimingTarget {
    kind: Vec<String>,
    crate_types: Vec<String>,
    name: String,
    src_path: String,
    edition: String,
    doc: bool,
    doctest: bool,
    test: bool,
}

// Re-export important types
pub use binfarce::demangle::SymbolData as BinarySymbol;

impl TimingInfo {
    fn parse_from_json_line(line: &str) -> Option<Self> {
        // Only parse timing-info messages
        if !line.contains(r#""reason":"timing-info""#) {
            return None;
        }

        let timing_msg: TimingMessage = facet_json::from_str(line).ok()?;

        Some(TimingInfo {
            crate_name: timing_msg.target.name,
            duration: timing_msg.duration,
            rmeta_time: timing_msg.rmeta_time,
        })
    }
}

// Core library types that will be moved from main.rs
#[derive(Debug)]
pub struct BloatAnalyzer;

#[derive(Debug, Clone)]
pub struct BuildContext {
    pub target_triple: String,
    pub artifacts: Vec<Artifact>,
    pub std_crates: Vec<String>,
    pub dep_crates: Vec<String>,
    pub deps_symbols: MultiMap<String, String>,
}

#[derive(Debug, Clone)]
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

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BuildType {
    Debug,
    Release,
}

#[derive(Debug, Default)]
pub struct AnalysisConfig {
    pub symbols_section: Option<String>,
    pub split_std: bool,
    /// Whether to also analyze LLVM IR files (requires --emit=llvm-ir during build)
    pub analyze_llvm_ir: bool,
    /// Optional target directory to search for .ll files (defaults to "target")
    pub target_dir: Option<std::path::PathBuf>,
    /// Build type to determine where to look for LLVM IR files
    pub build_type: Option<BuildType>,
}

/// A symbol with its associated crate name
pub struct Symbol {
    pub data: SymbolData,
    pub crate_name: CompactString,
}

pub struct AnalysisResult {
    pub file_size: ByteSize,
    pub text_size: ByteSize,
    pub symbols: Vec<SymbolData>,
    pub enriched_symbols: Option<Vec<Symbol>>,
    pub section_name: Option<String>,
    /// LLVM IR analysis data (only present if LLVM IR files were analyzed)
    pub llvm_ir_data: Option<LlvmIrAnalysis>,
}

#[derive(Debug, Clone)]
pub struct LlvmIrAnalysis {
    /// Two-level map: crate_name -> (demangled_symbol -> stats)
    pub crates: std::collections::HashMap<CrateName, std::collections::HashMap<DemangledSymbol, crate::llvm_ir::LlvmInstantiations>>,
    /// Total LLVM IR lines across all functions
    pub total_lines: LlvmIrLines,
    /// Total number of function instantiations
    pub total_copies: NumberOfCopies,
    /// Paths to .ll files that were analyzed
    pub analyzed_files: Vec<LlvmFilePath>,
}

impl LlvmIrAnalysis {
    /// Get top N functions by LLVM IR lines across all crates
    pub fn top_functions(&self, n: usize) -> Vec<(CrateName, DemangledSymbol, &crate::llvm_ir::LlvmInstantiations)> {
        let mut all_functions = Vec::new();
        
        for (crate_name, functions) in &self.crates {
            for (symbol_name, stats) in functions {
                all_functions.push((crate_name.clone(), symbol_name.clone(), stats));
            }
        }
        
        all_functions.sort_by_key(|(_, _, stats)| std::cmp::Reverse(stats.total_lines.value()));
        all_functions.truncate(n);
        all_functions
    }
    
    /// Get LLVM IR lines per crate, sorted by size
    pub fn lines_per_crate(&self) -> Vec<(CrateName, LlvmIrLines)> {
        let mut crate_sizes: std::collections::HashMap<CrateName, usize> = std::collections::HashMap::new();
        
        for (crate_name, functions) in &self.crates {
            let total_lines: usize = functions.values()
                .map(|stats| stats.total_lines.value())
                .sum();
            crate_sizes.insert(crate_name.clone(), total_lines);
        }
        
        let mut crate_list: Vec<(CrateName, LlvmIrLines)> = crate_sizes
            .into_iter()
            .map(|(name, lines)| (name, LlvmIrLines::new(lines)))
            .collect();
        crate_list.sort_by_key(|(_, lines)| std::cmp::Reverse(lines.value()));
        crate_list
    }
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

// Build options for controlling what gets built
#[derive(Default, Debug, Clone)]
pub struct BuildOptions {
    /// Build examples (--examples flag)
    pub build_examples: bool,
    /// Build all bins (--bins flag)
    pub build_bins: bool,
    /// Build specific bin (--bin NAME)
    pub build_bin: Option<String>,
    /// Build all targets (--all-targets)
    pub build_all_targets: bool,
}

// Build runner for cargo builds with all analysis features enabled
#[derive(Debug)]
pub struct BuildRunner {
    manifest_path: path::PathBuf,
    target_dir: path::PathBuf,
    build_type: BuildType,
    build_options: BuildOptions,
}

// Result of a build run with all parsed data
#[derive(Debug)]
pub struct BuildResult {
    pub context: BuildContext,
    pub timing_data: Vec<TimingInfo>,
    pub json_lines: Vec<String>,
}

// Analysis comparison types
#[derive(Debug, Clone)]
pub struct AnalysisComparison {
    pub file_size_diff: FileSizeDiff,
    pub symbol_changes: Vec<SymbolChange>,
    pub crate_changes: Vec<CrateChange>,
}

#[derive(Debug, Clone)]
pub struct FileSizeDiff {
    pub file_size_before: ByteSize,
    pub file_size_after: ByteSize,
    pub text_size_before: ByteSize,
    pub text_size_after: ByteSize,
}

#[derive(Debug, Clone)]
pub struct SymbolChange {
    pub name: String,
    pub demangled: String,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CrateChange {
    pub name: String,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
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
impl AnalysisResult {
    /// Enrich symbols with crate name information
    /// This consumes the original symbols vector to avoid cloning
    pub fn enrich_symbols(&mut self, context: &BuildContext, split_std: bool) {
        let symbols = std::mem::take(&mut self.symbols);
        let enriched: Vec<Symbol> = symbols.into_iter().map(|symbol| {
            let (crate_name, _is_exact) = crate_name::from_sym(context, split_std, &symbol.name);
            Symbol {
                data: symbol,
                crate_name: CompactString::new(&crate_name),
            }
        }).collect();
        
        self.enriched_symbols = Some(enriched);
    }
}

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

        info!("Parsing {} JSON messages from cargo", json_messages.len());

        for (i, line) in json_messages.iter().enumerate() {
            let build: CargoMessage = facet_json::from_str(line).map_err(|e| {
                error!("Failed to parse JSON line {}: {}", i, line);
                error!("Error: {:?}", e);
                BloatError::InvalidCargoOutput
            })?;

            // Only process compiler-artifact messages
            if build.reason.as_deref() != Some("compiler-artifact") {
                debug!("Skipping message {}: reason = {:?}", i, build.reason);
                continue;
            }

            debug!("Found compiler-artifact message at line {}", i);

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

                            let artifact = Artifact {
                                kind,
                                name: target_name.replace('-', "_"),
                                path: path::PathBuf::from(path),
                            };

                            debug!(
                                "Found artifact: {:?} - {} at {}",
                                artifact.kind, artifact.name, path
                            );
                            artifacts.push(artifact);
                        }
                    }
                }
            }
        }

        if artifacts.is_empty() {
            error!("No artifacts found in cargo build output");
            warn!("Make sure the project builds successfully and produces binaries or libraries");
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
        context: &BuildContext,
        config: &AnalysisConfig,
    ) -> Result<AnalysisResult, BloatError> {
        let section_name = config.symbols_section.as_deref().unwrap_or(".text");
        let mut result = collect_self_data(binary_path, section_name)?;

        // Enrich symbols with crate names
        result.enrich_symbols(context, config.split_std);

        // Optionally add LLVM IR analysis
        if config.analyze_llvm_ir {
            let target_dir = config
                .target_dir
                .as_deref()
                .unwrap_or(path::Path::new("target"));

            match Self::analyze_llvm_ir_from_target_dir(target_dir, config.build_type) {
                Ok(llvm_analysis) => {
                    result.llvm_ir_data = Some(llvm_analysis);
                }
                Err(e) => {
                    // Don't fail the entire analysis if LLVM IR is not available
                    // This allows graceful degradation
                    warn!("LLVM IR analysis failed: {}", e);
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
        build_type: Option<BuildType>,
    ) -> Result<LlvmIrAnalysis, BloatError> {
        let ll_files = find_llvm_ir_files(target_dir, build_type)?;

        if ll_files.is_empty() {
            return Err(BloatError::CargoError(
                "No LLVM IR files found. Make sure to build with RUSTFLAGS='--emit=llvm-ir'"
                    .to_string(),
            ));
        }

        let mut crates_map = std::collections::HashMap::new();
        let mut total_lines = 0usize;
        let mut total_copies = 0usize;

        for ll_file in &ll_files {
            let data =
                std::fs::read(ll_file).map_err(|_| BloatError::OpenFailed(ll_file.clone()))?;
            let instantiations = crate::llvm_ir::analyze_llvm_ir_data(&data);

            for (func_name, stats) in instantiations {
                // Try to extract crate name from the function name
                let mut crate_name_str = crate::crate_name::extract_crate_from_function(&func_name);
                
                // If we got "unknown", try to extract from the .ll filename
                if crate_name_str == "unknown" {
                    if let Some(file_stem) = ll_file.file_stem().and_then(|s| s.to_str()) {
                        // Parse crate name from filename like "serde-abc123"
                        if let Some((crate_part, _hash)) = file_stem.rsplit_once('-') {
                            crate_name_str = crate_part.to_string();
                        } else {
                            crate_name_str = UNDEMANGLED_CRATE.to_string();
                        }
                    } else {
                        crate_name_str = UNDEMANGLED_CRATE.to_string();
                    }
                }
                
                let crate_name = CrateName::from(crate_name_str);
                let demangled_symbol = DemangledSymbol::from(func_name);
                
                // Get or create the map for this crate
                let crate_functions = crates_map
                    .entry(crate_name)
                    .or_insert_with(std::collections::HashMap::new);
                
                // Add or update the stats for this function
                let entry = crate_functions
                    .entry(demangled_symbol)
                    .or_insert_with(crate::llvm_ir::LlvmInstantiations::default);
                entry.copies = NumberOfCopies::new(entry.copies.value() + stats.copies.value());
                entry.total_lines = LlvmIrLines::new(entry.total_lines.value() + stats.total_lines.value());
                total_lines += stats.total_lines.value();
                total_copies += stats.copies.value();
            }
        }

        let analyzed_files = ll_files.into_iter()
            .map(|p| LlvmFilePath::from(p.to_string_lossy().to_string()))
            .collect();

        Ok(LlvmIrAnalysis {
            crates: crates_map,
            total_lines: LlvmIrLines::new(total_lines),
            total_copies: NumberOfCopies::new(total_copies),
            analyzed_files,
        })
    }

    /// Analyze a single LLVM IR file
    pub fn analyze_llvm_ir_file(
        ll_file_path: &path::Path,
    ) -> Result<std::collections::HashMap<String, crate::llvm_ir::LlvmInstantiations>, BloatError>
    {
        let data = std::fs::read(ll_file_path)
            .map_err(|_| BloatError::OpenFailed(ll_file_path.to_owned()))?;
        Ok(crate::llvm_ir::analyze_llvm_ir_data(&data))
    }
}

impl BuildRunner {
    pub fn new(
        manifest_path: impl Into<path::PathBuf>,
        target_dir: impl Into<path::PathBuf>,
        build_type: BuildType,
    ) -> Self {
        Self {
            manifest_path: manifest_path.into(),
            target_dir: target_dir.into(),
            build_type,
            build_options: BuildOptions::default(),
        }
    }

    /// Set custom build options
    pub fn with_options(mut self, options: BuildOptions) -> Self {
        self.build_options = options;
        self
    }

    pub fn run(&self) -> Result<BuildResult, BloatError> {
        // Ensure manifest exists
        if !self.manifest_path.exists() {
            error!("Manifest file not found: {:?}", self.manifest_path);
            return Err(BloatError::OpenFailed(self.manifest_path.clone()));
        }

        info!("Building project from manifest: {:?}", self.manifest_path);
        info!("Target directory: {:?}", self.target_dir);
        info!("Build type: {:?}", self.build_type);
        info!("Build options: {:?}", self.build_options);

        // Build cargo command with all features enabled
        let mut cmd = Command::new("cargo");
        cmd.arg("build");

        // Add build target flags based on options
        if self.build_options.build_all_targets {
            cmd.arg("--all-targets");
        } else {
            if self.build_options.build_examples {
                cmd.arg("--examples");
            }
            if self.build_options.build_bins {
                cmd.arg("--bins");
            }
            if let Some(ref bin_name) = self.build_options.build_bin {
                cmd.arg("--bin");
                cmd.arg(bin_name);
            }
        }

        // Add build type flag
        match self.build_type {
            BuildType::Release => {
                cmd.arg("--release");
            }
            BuildType::Debug => {
                // Debug is default
            }
        }

        // Add required flags for analysis
        cmd.args([
            "--message-format=json",
            "-Z",
            "unstable-options",
            "--timings=json",
            "--manifest-path",
        ]);
        cmd.arg(&self.manifest_path);
        cmd.arg("--target-dir");
        cmd.arg(&self.target_dir);

        // Set environment variables for LLVM IR and timing
        let rustflags = "--emit=llvm-ir";
        cmd.env("RUSTFLAGS", rustflags);
        cmd.env("RUSTC_BOOTSTRAP", "1");

        // Log the environment variables
        info!("Setting RUSTFLAGS={}", rustflags);
        info!("Setting RUSTC_BOOTSTRAP=1 for timing information");

        // Log the full command
        info!("Executing cargo command: {:?}", cmd);
        info!("Command environment: RUSTFLAGS='{}' RUSTC_BOOTSTRAP='1'", rustflags);

        // Execute the build
        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                error!("Failed to execute cargo: {}", e);
                BloatError::CargoError(format!("Failed to execute cargo: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Cargo build failed with status: {:?}", output.status);
            error!("Stderr output:\n{}", stderr);
            return Err(BloatError::CargoBuildFailed);
        }

        info!("Cargo build completed successfully");

        // Check if LLVM IR files were generated
        let build_dir = match self.build_type {
            BuildType::Release => "release",
            BuildType::Debug => "debug",
        };
        let ll_check_dir = self.target_dir.join(build_dir).join("deps");
        if ll_check_dir.exists() {
            let ll_count = fs::read_dir(&ll_check_dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.path()
                                .extension()
                                .map(|ext| ext == "ll")
                                .unwrap_or(false)
                        })
                        .count()
                })
                .unwrap_or(0);
            
            if ll_count > 0 {
                info!("Found {} LLVM IR files in {}", ll_count, ll_check_dir.display());
            } else {
                warn!("No LLVM IR files found in {}. RUSTFLAGS may not have been applied correctly.", ll_check_dir.display());
            }
        }

        // Parse the JSON output
        let stdout = str::from_utf8(&output.stdout).map_err(|_| BloatError::InvalidCargoOutput)?;
        let json_lines: Vec<String> = stdout.lines().map(|s| s.to_string()).collect();
        let json_line_refs: Vec<&str> = json_lines.iter().map(|s| s.as_str()).collect();

        // Parse timing data
        let mut timing_data = Vec::new();
        for line in &json_lines {
            if let Some(timing) = TimingInfo::parse_from_json_line(line) {
                timing_data.push(timing);
            }
        }

        // Parse build context
        let context = BloatAnalyzer::from_cargo_metadata(
            &json_line_refs,
            &self.target_dir,
            None, // auto-detect target triple
        )?;

        Ok(BuildResult {
            context,
            timing_data,
            json_lines,
        })
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

impl SymbolChange {
    pub fn percent_change(&self) -> Option<f64> {
        match (self.size_before, self.size_after) {
            (Some(before), Some(after)) if before > 0 => {
                Some(((after as f64 - before as f64) / before as f64) * 100.0)
            }
            _ => None,
        }
    }

    pub fn absolute_change(&self) -> Option<i64> {
        match (self.size_before, self.size_after) {
            (Some(before), Some(after)) => Some(after as i64 - before as i64),
            _ => None,
        }
    }
}

impl CrateChange {
    pub fn percent_change(&self) -> Option<f64> {
        match (self.size_before, self.size_after) {
            (Some(before), Some(after)) if before > 0 => {
                Some(((after as f64 - before as f64) / before as f64) * 100.0)
            }
            _ => None,
        }
    }

    pub fn absolute_change(&self) -> Option<i64> {
        match (self.size_before, self.size_after) {
            (Some(before), Some(after)) => Some(after as i64 - before as i64),
            _ => None,
        }
    }
}

/// Extract crate name from a symbol name
/// This is a more sophisticated heuristic that handles various symbol patterns
fn extract_crate_from_symbol(symbol: &str) -> String {
    // Handle C/C++ symbols
    if symbol.starts_with("_") || symbol.contains("@") {
        return "C/C++".to_string();
    }

    // Skip generic implementations and trait bounds
    let cleaned = if symbol.starts_with("<") {
        // For symbols like "<T as alloc::vec::Vec>::method", extract the trait/type after "as"
        if let Some(as_pos) = symbol.find(" as ") {
            let after_as = &symbol[as_pos + 4..];
            if let Some(end) = after_as.find(">::") {
                after_as[..end].to_string()
            } else if let Some(end) = after_as.find(">") {
                after_as[..end].to_string()
            } else {
                after_as.to_string()
            }
        } else if let Some(space_pos) = symbol.find(" ") {
            // Handle other generic patterns
            symbol[space_pos + 1..].to_string()
        } else {
            symbol.to_string()
        }
    } else {
        symbol.to_string()
    };

    // Now extract the crate name from the cleaned symbol
    let parts: Vec<&str> = cleaned.split("::").collect();
    if parts.is_empty() {
        return "unknown".to_string();
    }

    let first_part = parts[0];
    
    // Common Rust standard library crates
    let std_crates = ["core", "alloc", "std", "proc_macro", "test"];
    if std_crates.contains(&first_part) {
        return first_part.to_string();
    }

    // If it's a known crate pattern, return it
    if !first_part.is_empty() 
        && !first_part.starts_with('<')
        && !first_part.starts_with('_')
        && !first_part.chars().all(|c| c.is_numeric())
        && first_part.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return first_part.to_string();
    }

    // For complex symbols, try to find a crate name in the path
    for part in parts {
        if !part.is_empty()
            && !part.starts_with('<')
            && !part.starts_with('_')
            && !part.chars().all(|c| c.is_numeric())
            && part.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            // Check if this looks like a crate name (not a type or function)
            if !part.chars().next().map_or(false, |c| c.is_uppercase()) {
                return part.to_string();
            }
        }
    }

    // Default to unknown
    "unknown".to_string()
}

impl AnalysisComparison {
    pub fn compare(
        before: &AnalysisResult,
        after: &AnalysisResult,
    ) -> Result<Self, BloatError> {
        // Create file size diff
        let file_size_diff = FileSizeDiff {
            file_size_before: before.file_size,
            file_size_after: after.file_size,
            text_size_before: before.text_size,
            text_size_after: after.text_size,
        };

        // Check if we have enriched symbols, use them if available
        let use_enriched = before.enriched_symbols.is_some() && after.enriched_symbols.is_some();

        // Compare symbols using demangled names as keys to avoid duplicates
        let mut symbol_changes = Vec::new();
        
        if use_enriched {
            // Use enriched symbols with proper crate names
            let before_symbols = before.enriched_symbols.as_ref().unwrap();
            let after_symbols = after.enriched_symbols.as_ref().unwrap();
            
            let mut before_by_demangled = std::collections::HashMap::new();
            let mut after_by_demangled = std::collections::HashMap::new();

            // Index symbols by demangled name
            for symbol in before_symbols {
                let key = symbol.data.name.trimmed.clone();
                let entry = before_by_demangled.entry(key).or_insert((symbol, 0u64));
                entry.1 += symbol.data.size;
            }
            for symbol in after_symbols {
                let key = symbol.data.name.trimmed.clone();
                let entry = after_by_demangled.entry(key).or_insert((symbol, 0u64));
                entry.1 += symbol.data.size;
            }

            // Find all unique demangled names
            let mut all_demangled = std::collections::HashSet::new();
            all_demangled.extend(before_by_demangled.keys().cloned());
            all_demangled.extend(after_by_demangled.keys().cloned());

            // Create symbol changes based on demangled names
            for demangled in all_demangled {
                let before_info = before_by_demangled.get(&demangled);
                let after_info = after_by_demangled.get(&demangled);
                
                // Use the mangled name from whichever version has it
                let mangled_name = match (before_info, after_info) {
                    (Some((sym, _)), _) => sym.data.name.complete.clone(),
                    (None, Some((sym, _))) => sym.data.name.complete.clone(),
                    _ => demangled.clone(), // Shouldn't happen
                };
                
                symbol_changes.push(SymbolChange {
                    name: mangled_name,
                    demangled: demangled,
                    size_before: before_info.map(|(_, size)| *size),
                    size_after: after_info.map(|(_, size)| *size),
                });
            }
        } else {
            // Fallback to old behavior
            let mut before_by_demangled = std::collections::HashMap::new();
            let mut after_by_demangled = std::collections::HashMap::new();

            for symbol in &before.symbols {
                let key = symbol.name.trimmed.clone();
                let entry = before_by_demangled.entry(key).or_insert((symbol, 0u64));
                entry.1 += symbol.size;
            }
            for symbol in &after.symbols {
                let key = symbol.name.trimmed.clone();
                let entry = after_by_demangled.entry(key).or_insert((symbol, 0u64));
                entry.1 += symbol.size;
            }

            let mut all_demangled = std::collections::HashSet::new();
            all_demangled.extend(before_by_demangled.keys().cloned());
            all_demangled.extend(after_by_demangled.keys().cloned());

            for demangled in all_demangled {
                let before_info = before_by_demangled.get(&demangled);
                let after_info = after_by_demangled.get(&demangled);
                
                let mangled_name = match (before_info, after_info) {
                    (Some((sym, _)), _) => sym.name.complete.clone(),
                    (None, Some((sym, _))) => sym.name.complete.clone(),
                    _ => demangled.clone(),
                };
                
                symbol_changes.push(SymbolChange {
                    name: mangled_name,
                    demangled: demangled,
                    size_before: before_info.map(|(_, size)| *size),
                    size_after: after_info.map(|(_, size)| *size),
                });
            }
        }

        // Compare crates by grouping symbols by crate
        let mut before_crate_sizes: std::collections::HashMap<CompactString, u64> =
            std::collections::HashMap::new();
        let mut after_crate_sizes: std::collections::HashMap<CompactString, u64> =
            std::collections::HashMap::new();

        if use_enriched {
            // Use proper crate names from enriched symbols
            let before_symbols = before.enriched_symbols.as_ref().unwrap();
            let after_symbols = after.enriched_symbols.as_ref().unwrap();
            
            for symbol in before_symbols {
                *before_crate_sizes.entry(symbol.crate_name.clone()).or_insert(0) += symbol.data.size;
            }
            
            for symbol in after_symbols {
                *after_crate_sizes.entry(symbol.crate_name.clone()).or_insert(0) += symbol.data.size;
            }
        } else {
            // Fallback to extracting from symbol names
            for symbol in &before.symbols {
                let crate_name = extract_crate_from_symbol(&symbol.name.trimmed);
                *before_crate_sizes.entry(CompactString::new(&crate_name)).or_insert(0) += symbol.size;
            }

            for symbol in &after.symbols {
                let crate_name = extract_crate_from_symbol(&symbol.name.trimmed);
                *after_crate_sizes.entry(CompactString::new(&crate_name)).or_insert(0) += symbol.size;
            }
        }

        // Build crate changes
        let mut all_crates = std::collections::HashSet::new();
        all_crates.extend(before_crate_sizes.keys().cloned());
        all_crates.extend(after_crate_sizes.keys().cloned());

        let mut crate_changes = Vec::new();
        for crate_name in all_crates {
            let size_before = before_crate_sizes.get(&crate_name).copied();
            let size_after = after_crate_sizes.get(&crate_name).copied();

            crate_changes.push(CrateChange {
                name: crate_name.to_string(),
                size_before,
                size_after,
            });
        }

        Ok(AnalysisComparison {
            file_size_diff,
            symbol_changes,
            crate_changes,
        })
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

    d.file_size = ByteSize::new(fs::metadata(path).unwrap().len());

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
        file_size: ByteSize::new(0u64),
        text_size: ByteSize::new(text_size),
        enriched_symbols: None,
        section_name: Some(section_name.to_owned()),
        llvm_ir_data: None,
    };

    Ok(d)
}

fn collect_macho_data(data: &[u8]) -> Result<AnalysisResult, BloatError> {
    let (symbols, text_size) = macho::parse(data)?.symbols()?;
    let d = AnalysisResult {
        symbols,
        file_size: ByteSize::new(0u64),
        text_size: ByteSize::new(text_size),
        enriched_symbols: None,
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
            file_size: ByteSize::new(0u64),
            text_size: ByteSize::new(text_size),
            enriched_symbols: None,
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
        file_size: ByteSize::new(0u64),
        text_size: ByteSize::new(text_size),
        enriched_symbols: None,
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
fn find_llvm_ir_files(
    target_dir: &path::Path,
    build_type: Option<BuildType>,
) -> Result<Vec<path::PathBuf>, BloatError> {
    let mut ll_files = Vec::new();

    // Determine the build directory based on build type
    let build_dir = match build_type {
        Some(BuildType::Release) => "release",
        Some(BuildType::Debug) | None => "debug",
    };

    info!("Searching for LLVM IR files in {} build directory", build_dir);

    // Search in multiple potential locations within target directory
    let search_dirs = vec![
        target_dir.join(build_dir),
        target_dir.join(build_dir).join("deps"),
        target_dir.join(build_dir).join("examples"),
        target_dir.join(build_dir).join("incremental"),
    ];

    for search_dir in &search_dirs {
        if search_dir.exists() {
            debug!("Searching directory: {}", search_dir.display());
            let initial_count = ll_files.len();
            find_ll_files_in_dir(&search_dir, &mut ll_files)?;
            let found_count = ll_files.len() - initial_count;
            if found_count > 0 {
                info!("Found {} .ll files in {}", found_count, search_dir.display());
            }
        } else {
            debug!("Directory does not exist: {}", search_dir.display());
        }
    }

    info!("Total LLVM IR files found: {}", ll_files.len());

    Ok(ll_files)
}

fn find_ll_files_in_dir(
    dir: &path::Path,
    ll_files: &mut Vec<path::PathBuf>,
) -> Result<(), BloatError> {
    let entries = fs::read_dir(dir).map_err(|_| BloatError::OpenFailed(dir.to_owned()))?;

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            // Recursively search subdirectories (like incremental compilation dirs)
            find_ll_files_in_dir(&path, ll_files)?;
        } else if let Some(extension) = path.extension() {
            if extension == "ll" {
                // Exclude build scripts
                if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                    if file_name.starts_with("build_script_") || file_name.starts_with("build-script-") {
                        debug!("Skipping build script .ll file: {}", path.display());
                        continue;
                    }
                }
                // Include all non-build-script .ll files
                debug!("Found .ll file: {}", path.display());
                ll_files.push(path);
            }
        }
    }

    Ok(())
}
