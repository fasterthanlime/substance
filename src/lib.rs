use owo_colors::OwoColorize;
pub use types::*;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::WalkBuilder;

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use binfarce::ar;
use log::{debug, error, info, trace, warn};

use crate::cargo::{CargoMessage, TimingInfo};
use crate::crate_name::StdHandling;
use crate::env::{collect_rlib_paths, stdlibs_dir};
use crate::errors::SubstanceError;
use crate::llvm_ir::analyze_llvm_ir_from_target_dir;
use crate::object::{collect_deps_symbols, collect_self_data};

pub mod cargo;
pub mod crate_name;
pub mod env;
pub mod errors;
pub mod formatting;
pub mod llvm_ir;
pub mod object;
pub mod reporting;
pub mod types;

pub struct BuildRunner {
    manifest_path: Utf8PathBuf,
    target_dir: Utf8PathBuf,

    /// Store the TempDir to ensure the temporary directory lives as long as the BuildRunner.
    _temp_dir: Option<tempfile::TempDir>,

    /// Flags like `--bin blah`, or `--example bleh` etc.
    additional_args: Vec<String>,
}

// Result of a build run with all parsed data
pub struct BuildResult {
    pub context: BuildContext,
    pub timing_data: Vec<TimingInfo>,
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

impl BuildRunner {
    /// Create a new BuildRunner instance.
    pub fn for_manifest(manifest_path: impl Into<Utf8PathBuf>) -> Self {
        use std::env;

        // Check if SUBSTANCE_TMP_DIR is set
        if let Ok(dir) = env::var("SUBSTANCE_TMP_DIR") {
            let manifest_path: Utf8PathBuf = manifest_path.into();

            // Mix in the hash of the manifest_path to the target_dir for uniqueness
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            manifest_path.hash(&mut hasher);
            let hash_val = hasher.finish();

            let base_target_dir = Utf8PathBuf::from(dir);
            let target_dir = base_target_dir.join(format!("{hash_val:016x}"));

            info!(
                "Using SUBSTANCE_TMP_DIR as target directory: {target_dir} (mixed with manifest hash)"
            );
            Self {
                manifest_path,
                target_dir,
                _temp_dir: None,
                additional_args: Vec::new(),
            }
        } else {
            // Generate a temporary directory for the target directory.
            let tmp_dir = tempfile::Builder::new()
                .prefix("substance-build-tmp")
                .tempdir()
                .expect("Failed to create temporary build directory");
            let target_dir = Utf8PathBuf::from_path_buf(tmp_dir.path().to_path_buf())
                .expect("Temporary target_dir is not valid UTF-8");

            // Store the TempDir so it is kept alive as long as BuildRunner lives.
            Self {
                manifest_path: manifest_path.into(),
                target_dir,
                _temp_dir: Some(tmp_dir),
                additional_args: Vec::new(),
            }
        }
    }

    /// Add an additional argument to the cargo build command.
    pub fn arg<T: Into<String>>(mut self, arg: T) -> Self {
        self.additional_args.push(arg.into());
        self
    }

    pub fn run(&self) -> Result<BuildContext, SubstanceError> {
        // Ensure manifest exists
        if !self.manifest_path.exists() {
            error!("Manifest file not found: {:?}", self.manifest_path);
            return Err(SubstanceError::OpenFailed(self.manifest_path.clone()));
        }

        info!("Building project from manifest: {:?}", self.manifest_path);
        info!("Target directory: {:?}", self.target_dir);

        let mut cmd = self.build_command();

        // Execute the build and forward stdout/stderr to the parent's stdout/stderr as it happens,
        // using two threads, but only collect JSON lines from stdout.

        let before_build = Instant::now();

        let mut cmd = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                error!("Failed to execute cargo: {e}");
                SubstanceError::CargoError(format!("Failed to execute cargo: {e}"))
            })?;

        use std::io::{BufRead, BufReader};
        use std::thread;

        let stdout = cmd.stdout.take().expect("Failed to take stdout");
        let stderr = cmd.stderr.take().expect("Failed to take stderr");

        struct StdoutResult {
            artifacts: Vec<Artifact>,
            timing_infos: Vec<TimingInfo>,
        }

        // Thread for stdout: collect JSON lines, print them, and return Vec<CargoMessage>.
        let stdout_handle = thread::spawn(move || {
            // Parse cargo messages to extract artifacts
            let mut artifacts = Vec::new();
            let mut timing_infos = Vec::new();

            let reader = BufReader::new(stdout);

            for line_result in reader.lines() {
                let Ok(line) = line_result else { continue };
                let msg = match CargoMessage::parse(&line) {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("Failed to parse cargo message: {err}.\nLine: {line}");
                        continue;
                    }
                };
                let Some(msg) = msg else {
                    eprintln!("Received cargo JSON message: {line}");
                    continue;
                };

                match msg {
                    CargoMessage::TimingInfo(timing_info) => {
                        timing_infos.push(timing_info);
                    }
                    CargoMessage::CompilerArtifact(artifact) => {
                        let kind = {
                            // Try to guess artifact kind from its file extension (best effort).
                            let path = &artifact
                                .filenames
                                .first()
                                .expect("No filename in CompilerArtifact");
                            if let Some(ext) = path.extension() {
                                match ext {
                                    "rlib" | "lib" => ArtifactKind::Library,
                                    "dylib" | "so" | "dll" => ArtifactKind::DynLib,
                                    "exe" | "bin" => ArtifactKind::Binary,
                                    _ => ArtifactKind::Binary, // fallback
                                }
                            } else {
                                ArtifactKind::Binary
                            }
                        };

                        for filename in &artifact.filenames {
                            let artifact_struct = Artifact {
                                kind,
                                name: artifact.crate_name.clone(),
                                path: filename.clone(),
                            };
                            trace!(
                                "Found artifact: {:?} - {} at {}",
                                artifact_struct.kind,
                                artifact_struct.name,
                                filename
                            );
                            artifacts.push(artifact_struct);
                        }
                    }
                }
            }

            StdoutResult {
                artifacts,
                timing_infos,
            }
        });

        // Thread for stderr: print to parent's stderr, but DO NOT collect lines.
        let stderr_handle = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                eprintln!("{line}");
            }
        });

        // Wait for the command to finish
        let status = cmd.wait().map_err(|e| {
            error!("Failed to wait for cargo: {e}");
            SubstanceError::CargoError(format!("Failed to wait for cargo: {e}"))
        })?;

        let wall_duration = before_build.elapsed();

        // Wait for both threads to finish reading
        let stdout_result = stdout_handle.join().unwrap();
        let _ = stderr_handle.join();

        if !status.success() {
            error!("Cargo build failed with status: {status:?}");
            // Stderr was already streamed, so we don't print it again here
            return Err(SubstanceError::CargoBuildFailed);
        }

        info!("Cargo build completed successfully");

        // Collect rlib paths from artifacts
        let mut rlib_paths: Vec<(CrateName, Utf8PathBuf)> = Vec::new();
        let mut dep_crates = Vec::new();
        for artifact in &stdout_result.artifacts {
            dep_crates.push(artifact.name.clone());

            if matches!(artifact.kind, ArtifactKind::Library) {
                rlib_paths.push((artifact.name.clone(), artifact.path.clone()));
            }
        }

        dep_crates.dedup();
        dep_crates.sort();

        // Get std crates - always collect them since we can't tell if build-std was used from JSON
        let target_dylib_path = stdlibs_dir()?;
        let std_paths = collect_rlib_paths(&target_dylib_path);

        let mut std_crates: Vec<CrateName> = std_paths.iter().map(|v| v.0.clone()).collect();
        rlib_paths.extend_from_slice(&std_paths);
        std_crates.sort();

        // Remove std crates that were explicitly added as dependencies.
        for c in &dep_crates {
            if let Some(idx) = std_crates.iter().position(|v| v == c) {
                std_crates.remove(idx);
            }
        }

        // Build symbol mapping
        info!("Building dependency symbol mapping...");
        let deps_symbols = collect_deps_symbols(rlib_paths)?;
        debug!("Collected symbols for {} dependencies.", deps_symbols.len());

        // Find the binary artifact first, filtering out build scripts
        info!("Locating binary artifact for analysis (excluding build-script-build)...");
        let binary_artifact = stdout_result
            .artifacts
            .into_iter()
            .find(|a| {
                matches!(a.kind, ArtifactKind::Binary) && a.name.as_str() != "build-script-build"
            })
            .ok_or(SubstanceError::CargoError(
                "No binary artifact found (all were build-script-build or missing).".to_string(),
            ))?;
        info!(
            "Binary artifact found: {} (path: {})",
            binary_artifact.name, binary_artifact.path
        );

        // Get file size of the binary
        let file_metadata = std::fs::metadata(&binary_artifact.path)
            .map_err(|_| SubstanceError::OpenFailed(binary_artifact.path.clone()))?;
        let file_size = ByteSize::new(file_metadata.len());
        info!("Binary file size: {} bytes", file_size.value().yellow());

        info!(
            "Collecting self data (.text section) from binary artifact: {}",
            binary_artifact.path.blue()
        );
        let raw_data = collect_self_data(&binary_artifact.path, ".text")?;
        let text_size = ByteSize::new(raw_data.text_size);
        debug!(
            "Collected self data for binary artifact (.text section size: {} bytes).",
            text_size.value().green()
        );

        let mut context = BuildContext {
            std_crates,
            dep_crates,
            deps_symbols,
            wall_duration,
            file_size,
            text_size,
            crates: Default::default(),
        };

        // Analyze LLVM IR (if any) for this crate from the target dir
        info!(
            "Analyzing LLVM IR files (if present) in target dir: {}",
            self.target_dir.blue()
        );
        let llvm_functions =
            analyze_llvm_ir_from_target_dir(&self.target_dir).unwrap_or_else(|err| {
                warn!(
                    "Failed to analyze LLVM IR files: {}. Continuing without LLVM IR data.",
                    err.red()
                );
                HashMap::new()
            });

        info!(
            "LLVM IR analysis: found {} LLVM functions.",
            llvm_functions.len().bright_purple()
        );

        // Compute build times per crate.
        let mut crate_build_times: HashMap<CrateName, Duration> = HashMap::new();
        for timing in &stdout_result.timing_infos {
            let crate_name = timing
                .target
                .name
                .clone()
                .map(CrateName::from)
                .unwrap_or_else(|| CrateName::from("unknown"));
            crate_build_times
                .entry(crate_name)
                .or_insert_with(|| Duration::from_secs_f64(timing.duration));
        }

        // Build crate information from the collected data
        let mut crates_map: HashMap<CrateName, Crate> = HashMap::new();

        // Process binary symbols and group by crate
        for symbol in raw_data.symbols {
            let (crate_name, _exact) =
                crate_name::from_sym(&context, StdHandling::Merged, &symbol.name);
            let demangled_symbol = DemangledSymbol::from(symbol.name.complete);
            let symbol_obj = Symbol {
                name: demangled_symbol.clone(),
                size: ByteSize::new(symbol.size),
            };

            crates_map
                .entry(crate_name)
                .or_insert_with(|| Crate {
                    name: CrateName::from(""),
                    symbols: HashMap::new(),
                    llvm_functions: HashMap::new(),
                    timing_info: None,
                })
                .symbols
                .insert(demangled_symbol, symbol_obj);
        }

        // Process LLVM functions and group by crate
        for (llvm_fn_name, llvm_fn) in llvm_functions {
            // Extract crate name from the function path using robust logic
            let crate_name = {
                let crate_string = crate_name::extract_crate_from_function(&llvm_fn_name);
                if crate_string == "unknown" {
                    // Fallback to binary artifact name as main crate
                    binary_artifact.name.clone()
                } else {
                    CrateName::from(crate_string)
                }
            };

            // Update the LlvmFunction with its proper name
            let mut llvm_fn_with_name = llvm_fn;
            llvm_fn_with_name.name = llvm_fn_name.clone();

            crates_map
                .entry(crate_name)
                .or_insert_with(|| Crate {
                    name: CrateName::from(""),
                    symbols: HashMap::new(),
                    llvm_functions: HashMap::new(),
                    timing_info: None,
                })
                .llvm_functions
                .insert(llvm_fn_name, llvm_fn_with_name);
        }
        // Set the proper crate names, populate timing information, and collect into a Vec
        let mut crates: Vec<Crate> = crates_map
            .into_iter()
            .map(|(name, mut crate_obj)| {
                // Assign the crate name
                crate_obj.name = name.clone();

                // If we have recorded build timing for this crate, attach it
                if let Some(dur) = crate_build_times.get(&name) {
                    crate_obj.timing_info = Some(TimingInfo {
                        target: crate::cargo::CargoTarget {
                            name: Some(name.as_str().to_string()),
                            kind: None,
                            crate_types: None,
                        },
                        duration: dur.as_secs_f64(),
                        rmeta_time: None,
                    });
                }

                crate_obj
            })
            .collect();

        // Sort crates by name for consistent output
        crates.sort_by(|a, b| a.name.cmp(&b.name));

        context.crates = crates;

        Ok(context)
    }

    fn build_command(&self) -> Command {
        let mut cmd = Command::new("cargo");
        cmd.arg("build");

        // Just pass additional args
        cmd.args(&self.additional_args);

        // Add required flags for analysis
        cmd.args([
            "--message-format=json",
            "-Z",
            "unstable-options",
            "-Z",
            "binary-dep-depinfo",
            "-Z",
            "checksum-freshness",
            "--timings=json",
            "--manifest-path",
        ]);
        cmd.arg(&self.manifest_path);
        cmd.arg("--target-dir");
        cmd.arg(&self.target_dir);
        let rustflags = "--emit=llvm-ir -Cdebuginfo=line-tables-only -Cstrip=none";

        // Set environment variables for LLVM IR, timing, and Cstrip
        cmd.env("RUSTFLAGS", rustflags);
        cmd.env("RUSTC_BOOTSTRAP", "1");
        // Force colored output in cargo/rustc even if not a tty
        cmd.env("CLICOLOR_FORCE", "1");

        cmd
    }
}

/// Finds all `.ll` files within a given directory, ignoring `build` directories.
pub fn find_llvm_ir_files(root_dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, SubstanceError> {
    let mut ll_files = Vec::new();

    let walker = WalkBuilder::new(root_dir).build();

    for entry in walker {
        let entry = entry.map_err(|e| {
            SubstanceError::CargoError(format!(
                "Error iterating directory during search for .ll files: {e}"
            ))
        })?;
        let path = entry.path();
        let path = match Utf8Path::from_path(path) {
            Some(path) => path,
            None => {
                eprintln!("Failed to convert path to Utf8Path: non-UTF8 path encountered");
                continue;
            }
        };

        // Check if the path is a file and ends with .ll
        if path.is_file() && path.extension() == Some("ll") {
            ll_files.push(path.to_path_buf());
        }
    }

    Ok(ll_files)
}
