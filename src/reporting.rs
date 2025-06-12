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

use crate::{
    AnalysisResult, BuildContext, TimingInfo,
    formatting::*,
    CrateChange as SubstanceCrateChange,
    SymbolChange as SubstanceSymbolChange,
    types::{CrateName, DemangledSymbol},
};
use std::collections::HashMap;
use std::time::Duration;
use std::fmt::Write;

/// Configuration for report generation
#[derive(Debug, Clone)]
pub struct ReportConfig {
    /// Maximum number of items to show in each section
    pub limits: SectionLimits,
    /// Minimum threshold for showing size changes (in bytes)
    pub size_threshold: u64,
    /// Minimum threshold for showing percentage changes
    pub percent_threshold: f64,
    /// Which sections to include in the report
    pub sections: ReportSections,
    /// Output format for the report
    pub format: ReportFormat,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            limits: SectionLimits::default(),
            size_threshold: 0,
            percent_threshold: 0.0,
            sections: ReportSections::default(),
            format: ReportFormat::Markdown,
        }
    }
}

/// Limits for various report sections
#[derive(Debug, Clone)]
pub struct SectionLimits {
    pub top_crates: usize,
    pub top_symbols: usize,
    pub symbol_changes: usize,
    pub build_time_changes: usize,
    pub llvm_functions: usize,
    pub llvm_function_changes: usize,
    pub llvm_crate_changes: usize,
}

impl Default for SectionLimits {
    fn default() -> Self {
        Self {
            top_crates: 15,
            top_symbols: 30,
            symbol_changes: 50,
            build_time_changes: 15,
            llvm_functions: 30,
            llvm_function_changes: 50,
            llvm_crate_changes: 20,
        }
    }
}

/// Which sections to include in the report
#[derive(Debug, Clone)]
pub struct ReportSections {
    pub summary: bool,
    pub crate_size_changes: bool,
    pub build_time_changes: bool,
    pub symbol_changes: bool,
    pub current_top_crates: bool,
    pub current_top_symbols: bool,
    pub llvm_analysis: bool,
    pub llvm_differential: bool,
}

impl Default for ReportSections {
    fn default() -> Self {
        Self {
            summary: true,
            crate_size_changes: true,
            build_time_changes: true,
            symbol_changes: true,
            current_top_crates: true,
            current_top_symbols: true,
            llvm_analysis: true,
            llvm_differential: true,
        }
    }
}

/// Available output formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Markdown,
    Json,
    PlainText,
}

/// Complete analysis report for a single version
#[derive(Debug, Clone)]
pub struct SingleVersionReport {
    /// Git commit hash or version identifier
    pub version: String,
    
    /// Basic size metrics
    pub metrics: SizeMetrics,
    
    /// Build timing information
    pub build_time: BuildTime,
    
    /// Top crates by size with percentage of total
    /// Vec<(crate_name, size_bytes, percentage)>
    pub top_crates: Vec<(String, u64, f64)>,
    
    /// Top symbols by size
    /// Vec<(symbol_name, size_bytes)>
    pub top_symbols: Vec<(String, u64)>,
    
    /// All crates with their sizes (for comparison)
    /// HashMap<crate_name, size_bytes>
    pub all_crates: HashMap<String, u64>,
    
    /// All symbols with their sizes (for comparison)
    /// HashMap<symbol_name, size_bytes>
    pub all_symbols: HashMap<String, u64>,
    
    /// LLVM IR analysis if available
    pub llvm_analysis: Option<LlvmSummary>,
    
    /// Raw build context for advanced analysis
    pub build_context: BuildContext,
}

impl SingleVersionReport {
    /// Create a report from analysis results
    pub fn from_analysis(
        analysis: &AnalysisResult,
        version: String,
        build_context: BuildContext,
        timing_data: Vec<TimingInfo>,
        wall_time: Duration,
    ) -> Self {
        // Calculate crate sizes
        let mut crate_sizes = HashMap::new();
        for symbol in &analysis.symbols {
            let (crate_name, _) = crate::crate_name::from_sym(
                &build_context,
                true, // split_std
                &symbol.name,
            );
            *crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
        }
        
        // Store all crates for comparison
        let all_crates = crate_sizes.clone();
        
        // Sort crates by size and calculate percentages
        let mut crate_list: Vec<(String, u64)> = crate_sizes.into_iter().collect();
        crate_list.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
        
        let top_crates: Vec<(String, u64, f64)> = crate_list
            .into_iter()
            .take(15)
            .map(|(name, size)| {
                let percentage = size as f64 / analysis.text_size.value() as f64 * 100.0;
                (name, size, percentage)
            })
            .collect();
        
        // Get all symbols
        let all_symbols: HashMap<String, u64> = analysis.symbols
            .iter()
            .map(|s| (s.name.trimmed.clone(), s.size))
            .collect();
        
        // Get top symbols
        let mut symbol_list: Vec<(String, u64)> = all_symbols.iter()
            .map(|(name, size)| (name.clone(), *size))
            .collect();
        symbol_list.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
        let top_symbols = symbol_list.into_iter().take(30).collect();
        
        // Calculate total CPU time
        let total_cpu_time: f64 = timing_data.iter().map(|t| t.duration).sum();
        
        // Create build time info
        let build_time = BuildTime {
            wall_time,
            total_cpu_time,
            crate_timings: timing_data.iter()
                .map(|t| CrateTiming {
                    crate_name: t.crate_name.clone(),
                    duration: t.duration,
                })
                .collect(),
        };
        
        // Create LLVM summary if available
        let llvm_analysis = analysis.llvm_ir_data.as_ref().map(|llvm| {
            // Get top functions using helper method
            let top_functions = llvm.top_functions(30)
                .into_iter()
                .map(|(_crate_name, symbol_name, stats)| LlvmFunctionStats {
                    function_name: <DemangledSymbol as AsRef<str>>::as_ref(&symbol_name).to_string(),
                    total_lines: stats.total_lines.value(),
                    copies: stats.copies.value(),
                    percentage: stats.total_lines.value() as f64 / llvm.total_lines.value() as f64 * 100.0,
                })
                .collect();
            
            // Get crate IR sizes using helper method
            let crate_ir_sizes = llvm.lines_per_crate()
                .into_iter()
                .map(|(crate_name, lines)| (<CrateName as AsRef<str>>::as_ref(&crate_name).to_string(), lines.value()))
                .collect();
            
            LlvmSummary {
                total_lines: llvm.total_lines.value(),
                total_instantiations: llvm.total_copies.value(),
                analyzed_files: llvm.analyzed_files.len(),
                top_functions,
                crate_ir_sizes,
            }
        });
        
        Self {
            version,
            metrics: SizeMetrics {
                file_size: analysis.file_size,
                text_size: analysis.text_size,
                data_size: None, // TODO: Extract from binary sections
                bss_size: None,  // TODO: Extract from binary sections
            },
            build_time,
            top_crates,
            top_symbols,
            all_crates,
            all_symbols,
            llvm_analysis,
            build_context,
        }
    }
}

/// Basic size metrics for a build
#[derive(Debug, Clone, Copy)]
pub struct SizeMetrics {
    pub file_size: crate::types::ByteSize,
    pub text_size: crate::types::ByteSize,
    pub data_size: Option<crate::types::ByteSize>,
    pub bss_size: Option<crate::types::ByteSize>,
}

/// Build timing information
#[derive(Debug, Clone)]
pub struct BuildTime {
    /// Wall clock time for the build
    pub wall_time: Duration,
    /// Total CPU time across all crates
    pub total_cpu_time: f64,
    /// Per-crate timing information
    pub crate_timings: Vec<CrateTiming>,
}

/// Timing information for a single crate
#[derive(Debug, Clone)]
pub struct CrateTiming {
    pub crate_name: String,
    pub duration: f64,
}

/// LLVM IR analysis summary
#[derive(Debug, Clone)]
pub struct LlvmSummary {
    pub total_lines: usize,
    pub total_instantiations: usize,
    pub analyzed_files: usize,
    /// Top functions by LLVM IR lines
    pub top_functions: Vec<LlvmFunctionStats>,
    /// LLVM IR lines per crate
    pub crate_ir_sizes: Vec<(String, usize)>,
}

/// LLVM function statistics
#[derive(Debug, Clone)]
pub struct LlvmFunctionStats {
    pub function_name: String,
    pub total_lines: usize,
    pub copies: usize,
    pub percentage: f64,
}

/// Main report enum that can represent either a single analysis or comparison
#[derive(Debug, Clone)]
pub enum Report {
    /// Report for a single version analysis
    Single(SingleVersionReport),
    
    /// Comparison report between two versions
    Comparison {
        /// The baseline version (e.g., "main" branch)
        baseline: SingleVersionReport,
        
        /// The current version being analyzed
        current: SingleVersionReport,
        
        /// Pre-computed comparison data
        comparison: ComparisonData,
    },
}

/// Pre-computed comparison data between two versions
#[derive(Debug, Clone)]
pub struct ComparisonData {
    /// Size changes summary
    pub size_changes: SizeChanges,
    
    /// Crate-level changes sorted by absolute change
    pub crate_changes: Vec<CrateChange>,
    
    /// Build time changes per crate
    /// Vec<(crate_name, baseline_time, current_time)>
    pub build_time_changes: Vec<(String, Option<f64>, Option<f64>)>,
    
    /// Symbol-level changes sorted by absolute change
    pub symbol_changes: Vec<SymbolChange>,
    
    /// LLVM IR comparison if available
    pub llvm_comparison: Option<LlvmComparison>,
}

/// Summary of size changes
#[derive(Debug, Clone, Copy)]
pub struct SizeChanges {
    pub file_size_diff: i64,
    pub text_size_diff: i64,
    pub file_size_percent: f64,
    pub text_size_percent: f64,
}

/// Crate-level change information
#[derive(Debug, Clone)]
pub struct CrateChange {
    pub name: String,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
}

impl CrateChange {
    pub fn absolute_change(&self) -> Option<i64> {
        match (self.size_before, self.size_after) {
            (Some(before), Some(after)) => Some(after as i64 - before as i64),
            (None, Some(after)) => Some(after as i64),
            (Some(before), None) => Some(-(before as i64)),
            _ => None,
        }
    }
    
    pub fn percent_change(&self) -> Option<f64> {
        match (self.size_before, self.size_after) {
            (Some(before), Some(after)) if before > 0 => {
                Some(((after as f64 - before as f64) / before as f64) * 100.0)
            }
            _ => None,
        }
    }
}

/// Symbol-level change information
#[derive(Debug, Clone)]
pub struct SymbolChange {
    pub name: String,
    pub demangled: String,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
}

/// LLVM IR comparison between versions
#[derive(Debug, Clone)]
pub struct LlvmComparison {
    pub total_lines_diff: i64,
    pub total_instantiations_diff: i64,
    /// Function-level changes sorted by absolute line change
    pub function_changes: Vec<LlvmFunctionChange>,
    /// Crate-level IR changes sorted by absolute change
    pub crate_ir_changes: Vec<(String, i64, usize, usize)>,
}

/// Individual function LLVM IR change
#[derive(Debug, Clone)]
pub struct LlvmFunctionChange {
    pub function_name: String,
    pub baseline_lines: usize,
    pub current_lines: usize,
    pub baseline_copies: usize,
    pub current_copies: usize,
}

impl ComparisonData {
    /// Create comparison data from two single version reports
    pub fn from_reports(baseline: &SingleVersionReport, current: &SingleVersionReport) -> Self {
        
        // Calculate size changes
        let size_changes = SizeChanges {
            file_size_diff: current.metrics.file_size.value() as i64 - baseline.metrics.file_size.value() as i64,
            text_size_diff: current.metrics.text_size.value() as i64 - baseline.metrics.text_size.value() as i64,
            file_size_percent: if baseline.metrics.file_size.value() > 0 {
                ((current.metrics.file_size.value() as f64 - baseline.metrics.file_size.value() as f64) 
                    / baseline.metrics.file_size.value() as f64) * 100.0
            } else {
                0.0
            },
            text_size_percent: if baseline.metrics.text_size.value() > 0 {
                ((current.metrics.text_size.value() as f64 - baseline.metrics.text_size.value() as f64) 
                    / baseline.metrics.text_size.value() as f64) * 100.0
            } else {
                0.0
            },
        };
        
        // Compare all crates
        let baseline_crates = &baseline.all_crates;
        let current_crates = &current.all_crates;
        
        // Get all unique crate names
        let mut all_crates = std::collections::HashSet::new();
        all_crates.extend(baseline_crates.keys().cloned());
        all_crates.extend(current_crates.keys().cloned());
        
        let mut crate_changes: Vec<CrateChange> = all_crates.into_iter()
            .map(|name| CrateChange {
                name: name.clone(),
                size_before: baseline_crates.get(&name).copied(),
                size_after: current_crates.get(&name).copied(),
            })
            .collect();
        
        // Sort by absolute change
        crate_changes.sort_by_key(|c| c.absolute_change().map(|v| -v.abs()).unwrap_or(0));
        
        // Compare build times
        let mut baseline_times: HashMap<String, f64> = HashMap::new();
        let mut current_times: HashMap<String, f64> = HashMap::new();
        
        for timing in &baseline.build_time.crate_timings {
            baseline_times.insert(timing.crate_name.clone(), timing.duration);
        }
        for timing in &current.build_time.crate_timings {
            current_times.insert(timing.crate_name.clone(), timing.duration);
        }
        
        let mut all_crates = std::collections::HashSet::new();
        all_crates.extend(baseline_times.keys().cloned());
        all_crates.extend(current_times.keys().cloned());
        
        let mut build_time_changes: Vec<(String, Option<f64>, Option<f64>)> = all_crates
            .into_iter()
            .map(|name| {
                (
                    name.clone(),
                    baseline_times.get(&name).copied(),
                    current_times.get(&name).copied(),
                )
            })
            .collect();
        
        // Sort by absolute time difference
        build_time_changes.sort_by(|a, b| {
            let a_diff = match (a.1, a.2) {
                (Some(before), Some(after)) => (after - before).abs(),
                (None, Some(after)) => after,
                (Some(before), None) => before,
                _ => 0.0,
            };
            let b_diff = match (b.1, b.2) {
                (Some(before), Some(after)) => (after - before).abs(),
                (None, Some(after)) => after,
                (Some(before), None) => before,
                _ => 0.0,
            };
            b_diff.partial_cmp(&a_diff).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        // Compare all symbols
        let baseline_symbols = &baseline.all_symbols;
        let current_symbols = &current.all_symbols;
        
        // Get all unique symbol names
        let mut all_symbols = std::collections::HashSet::new();
        all_symbols.extend(baseline_symbols.keys().cloned());
        all_symbols.extend(current_symbols.keys().cloned());
        
        let mut symbol_changes: Vec<SymbolChange> = all_symbols.into_iter()
            .filter_map(|name| {
                let size_before = baseline_symbols.get(&name).copied();
                let size_after = current_symbols.get(&name).copied();
                
                // Only include symbols that changed
                match (size_before, size_after) {
                    (Some(before), Some(after)) if before != after => Some(SymbolChange {
                        name: name.clone(),
                        demangled: name.clone(), // Already demangled
                        size_before: Some(before),
                        size_after: Some(after),
                    }),
                    (None, Some(after)) => Some(SymbolChange {
                        name: format!("{}::new", name), // Mark as new
                        demangled: name.clone(),
                        size_before: None,
                        size_after: Some(after),
                    }),
                    (Some(before), None) => Some(SymbolChange {
                        name: format!("{}::removed", name), // Mark as removed
                        demangled: name.clone(),
                        size_before: Some(before),
                        size_after: None,
                    }),
                    _ => None,
                }
            })
            .collect();
        
        // Sort by absolute change
        symbol_changes.sort_by_key(|s| {
            match (s.size_before, s.size_after) {
                (Some(before), Some(after)) => -(after as i64 - before as i64).abs(),
                (None, Some(after)) => -(after as i64),
                (Some(before), None) => -(before as i64),
                _ => 0,
            }
        });
        
        // LLVM comparison if available
        let llvm_comparison = match (&baseline.llvm_analysis, &current.llvm_analysis) {
            (Some(baseline_llvm), Some(current_llvm)) => {
                Some(LlvmComparison::from_summaries(baseline_llvm, current_llvm))
            }
            _ => None,
        };
        
        Self {
            size_changes,
            crate_changes,
            build_time_changes,
            symbol_changes,
            llvm_comparison,
        }
    }
}

impl Report {
    /// Create a comparison report from two single version reports
    pub fn comparison(
        baseline: SingleVersionReport,
        current: SingleVersionReport,
        crate_changes: Vec<SubstanceCrateChange>,
        symbol_changes: Vec<SubstanceSymbolChange>,
    ) -> Self {
        // Calculate size changes
        let size_changes = SizeChanges {
            file_size_diff: current.metrics.file_size.value() as i64 - baseline.metrics.file_size.value() as i64,
            text_size_diff: current.metrics.text_size.value() as i64 - baseline.metrics.text_size.value() as i64,
            file_size_percent: if baseline.metrics.file_size.value() > 0 {
                ((current.metrics.file_size.value() as f64 - baseline.metrics.file_size.value() as f64) 
                    / baseline.metrics.file_size.value() as f64) * 100.0
            } else {
                0.0
            },
            text_size_percent: if baseline.metrics.text_size.value() > 0 {
                ((current.metrics.text_size.value() as f64 - baseline.metrics.text_size.value() as f64) 
                    / baseline.metrics.text_size.value() as f64) * 100.0
            } else {
                0.0
            },
        };
        
        // Convert crate changes
        let crate_changes = crate_changes.into_iter()
            .map(|c| CrateChange {
                name: c.name,
                size_before: c.size_before,
                size_after: c.size_after,
            })
            .collect();
        
        // Convert symbol changes
        let symbol_changes = symbol_changes.into_iter()
            .map(|s| SymbolChange {
                name: s.name,
                demangled: s.demangled,
                size_before: s.size_before,
                size_after: s.size_after,
            })
            .collect();
        
        // Calculate build time changes
        let mut baseline_times: HashMap<String, f64> = HashMap::new();
        let mut current_times: HashMap<String, f64> = HashMap::new();
        
        for timing in &baseline.build_time.crate_timings {
            baseline_times.insert(timing.crate_name.clone(), timing.duration);
        }
        for timing in &current.build_time.crate_timings {
            current_times.insert(timing.crate_name.clone(), timing.duration);
        }
        
        let mut all_crates = std::collections::HashSet::new();
        all_crates.extend(baseline_times.keys().cloned());
        all_crates.extend(current_times.keys().cloned());
        
        let mut build_time_changes: Vec<(String, Option<f64>, Option<f64>)> = all_crates
            .into_iter()
            .map(|name| {
                (
                    name.clone(),
                    baseline_times.get(&name).copied(),
                    current_times.get(&name).copied(),
                )
            })
            .collect();
        
        // Sort by absolute time difference
        build_time_changes.sort_by(|a, b| {
            let a_diff = match (a.1, a.2) {
                (Some(before), Some(after)) => (after - before).abs(),
                (None, Some(after)) => after,
                (Some(before), None) => before,
                _ => 0.0,
            };
            let b_diff = match (b.1, b.2) {
                (Some(before), Some(after)) => (after - before).abs(),
                (None, Some(after)) => after,
                (Some(before), None) => before,
                _ => 0.0,
            };
            b_diff.partial_cmp(&a_diff).unwrap()
        });
        
        // Calculate LLVM comparison if available
        let llvm_comparison = match (&baseline.llvm_analysis, &current.llvm_analysis) {
            (Some(baseline_llvm), Some(current_llvm)) => {
                Some(LlvmComparison::from_summaries(baseline_llvm, current_llvm))
            }
            _ => None,
        };
        
        Self::Comparison {
            baseline,
            current,
            comparison: ComparisonData {
                size_changes,
                crate_changes,
                build_time_changes,
                symbol_changes,
                llvm_comparison,
            },
        }
    }
    
    /// Generate a report with the given configuration
    pub fn generate(&self, config: &ReportConfig) -> String {
        match config.format {
            ReportFormat::Markdown => self.to_markdown(config),
            ReportFormat::Json => self.to_json(config),
            ReportFormat::PlainText => self.to_plain_text(config),
        }
    }
    
    /// Generate markdown report
    pub fn to_markdown(&self, config: &ReportConfig) -> String {
        let mut md = String::new();
        
        match self {
            Report::Single(report) => {
                self.write_single_markdown(&mut md, report, config);
            }
            Report::Comparison { baseline, current, comparison } => {
                self.write_comparison_markdown(&mut md, baseline, current, comparison, config);
            }
        }
        
        md
    }
    
    /// Generate JSON report
    fn to_json(&self, _config: &ReportConfig) -> String {
        // TODO: Implement JSON serialization
        "{\"error\": \"JSON output not yet implemented\"}".to_string()
    }
    
    /// Generate plain text report
    fn to_plain_text(&self, _config: &ReportConfig) -> String {
        // TODO: Implement plain text output
        "Plain text output not yet implemented".to_string()
    }
    
    /// Write single version markdown report
    fn write_single_markdown(&self, md: &mut String, report: &SingleVersionReport, config: &ReportConfig) {
        writeln!(md, "# 🌊 Binary Size Analysis Report").unwrap();
        writeln!(md).unwrap();
        writeln!(md, "Analyzing commit `{}`", report.version).unwrap();
        writeln!(md).unwrap();
        
        if config.sections.summary {
            writeln!(md, "## 📊 Size Metrics").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Metric | Value |").unwrap();
            writeln!(md, "|--------|-------|").unwrap();
            writeln!(md, "| File size | {} |", format_bytes(report.metrics.file_size.value())).unwrap();
            writeln!(md, "| Text size | {} |", format_bytes(report.metrics.text_size.value())).unwrap();
            writeln!(md, "| Build time | {:.2}s |", report.build_time.wall_time.as_secs_f64()).unwrap();
            writeln!(md).unwrap();
        }
        
        if config.sections.current_top_crates && !report.top_crates.is_empty() {
            writeln!(md, "## 📦 Top Crates by Size").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Crate | Size | % of Total |").unwrap();
            writeln!(md, "|-------|------|------------|").unwrap();
            for (crate_name, size, percent) in report.top_crates.iter().take(config.limits.top_crates) {
                writeln!(md, "| {} | {} | {:.1}% |", crate_name, format_bytes(*size), percent).unwrap();
            }
            writeln!(md).unwrap();
        }
        
        if config.sections.llvm_analysis {
            if let Some(llvm) = &report.llvm_analysis {
                writeln!(md, "## 🔥 LLVM IR Analysis").unwrap();
                writeln!(md).unwrap();
                writeln!(md, "| Metric | Value |").unwrap();
                writeln!(md, "|--------|-------|").unwrap();
                writeln!(md, "| Total LLVM IR lines | {} |", llvm.total_lines).unwrap();
                writeln!(md, "| Total instantiations | {} |", llvm.total_instantiations).unwrap();
                writeln!(md, "| Analyzed .ll files | {} |", llvm.analyzed_files).unwrap();
                writeln!(md).unwrap();
                
                if !llvm.top_functions.is_empty() {
                    writeln!(md, "### 🔍 Top Functions by LLVM IR Lines").unwrap();
                    writeln!(md).unwrap();
                    writeln!(md, "<details>").unwrap();
                    writeln!(md, "<summary>Top {} most complex functions (click to expand)</summary>", 
                        config.limits.llvm_functions).unwrap();
                    writeln!(md).unwrap();
                    writeln!(md, "| Lines | % | Copies | Function |").unwrap();
                    writeln!(md, "|-------|---|--------|----------|").unwrap();
                    
                    for func in llvm.top_functions.iter().take(config.limits.llvm_functions) {
                        writeln!(md, "| {} | {:.1}% | {} | `{}` |", 
                            func.total_lines, func.percentage, func.copies, func.function_name).unwrap();
                    }
                    
                    writeln!(md).unwrap();
                    writeln!(md, "</details>").unwrap();
                    writeln!(md).unwrap();
                }
            } else {
                writeln!(md, "_💡 Tip: LLVM IR analysis data not available. Make sure to build with RUSTFLAGS='--emit=llvm-ir'._").unwrap();
                writeln!(md).unwrap();
            }
        }
        
        writeln!(md, "---").unwrap();
        writeln!(md, "_Generated by [Substance](https://github.com/fasterthanlime/substance)_").unwrap();
    }
    
    /// Write comparison markdown report
    fn write_comparison_markdown(
        &self, 
        md: &mut String, 
        baseline: &SingleVersionReport, 
        current: &SingleVersionReport,
        comparison: &ComparisonData,
        config: &ReportConfig
    ) {
        writeln!(md, "# 🌊 Binary Size Analysis Report").unwrap();
        writeln!(md).unwrap();
        writeln!(md, "Comparing `{}` with `{}`", baseline.version, current.version).unwrap();
        writeln!(md).unwrap();
        
        if config.sections.summary {
            writeln!(md, "## 📊 Size Comparison").unwrap();
            writeln!(md).unwrap();
            
            let file_emoji = if comparison.size_changes.file_size_diff > 0 { "📈" } 
                else if comparison.size_changes.file_size_diff < 0 { "📉" } 
                else { "➖" };
            let text_emoji = if comparison.size_changes.text_size_diff > 0 { "📈" } 
                else if comparison.size_changes.text_size_diff < 0 { "📉" } 
                else { "➖" };
            let time_diff = current.build_time.wall_time.as_secs_f64() - baseline.build_time.wall_time.as_secs_f64();
            let time_emoji = if time_diff < 0.0 { "⚡" } 
                else if time_diff > 0.0 { "🐌" } 
                else { "➖" };
            
            writeln!(md, "| Metric | Baseline | Current | Change |").unwrap();
            writeln!(md, "|--------|----------|---------|--------|").unwrap();
            writeln!(md, "| File size | {} | {} | {} {} |", 
                format_bytes(baseline.metrics.file_size.value()),
                format_bytes(current.metrics.file_size.value()),
                file_emoji,
                format_size_diff(comparison.size_changes.file_size_diff)
            ).unwrap();
            writeln!(md, "| Text size | {} | {} | {} {} |", 
                format_bytes(baseline.metrics.text_size.value()),
                format_bytes(current.metrics.text_size.value()),
                text_emoji,
                format_size_diff(comparison.size_changes.text_size_diff)
            ).unwrap();
            writeln!(md, "| Build time | {:.2}s | {:.2}s | {} {:+.2}s |", 
                baseline.build_time.wall_time.as_secs_f64(),
                current.build_time.wall_time.as_secs_f64(),
                time_emoji,
                time_diff
            ).unwrap();
            writeln!(md).unwrap();
        }
        
        if config.sections.crate_size_changes && !comparison.crate_changes.is_empty() {
            writeln!(md, "## 📦 Top Crate Size Changes").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Crate | Baseline | Current | Change | % |").unwrap();
            writeln!(md, "|-------|----------|---------|--------|---|").unwrap();
            
            let mut sorted_changes = comparison.crate_changes.clone();
            sorted_changes.sort_by_key(|c| -c.absolute_change().unwrap_or(0).abs());
            
            for change in sorted_changes.iter()
                .filter(|c| c.absolute_change().map(|v| v.abs() as u64 >= config.size_threshold).unwrap_or(true))
                .take(config.limits.top_crates) 
            {
                match (change.size_before, change.size_after) {
                    (Some(before), Some(after)) => {
                        let abs_change = change.absolute_change().unwrap();
                        let pct = change.percent_change().unwrap();
                        let emoji = if abs_change > 0 { "📈" } 
                            else if abs_change < 0 { "📉" } 
                            else { "➖" };
                        writeln!(md, "| {} | {} | {} | {} {} | {:+.1}% |",
                            change.name,
                            format_bytes(before),
                            format_bytes(after),
                            emoji,
                            format_size_diff(abs_change),
                            pct
                        ).unwrap();
                    }
                    (None, Some(after)) => {
                        writeln!(md, "| {} | - | {} | 🆕 {} | NEW |",
                            change.name,
                            format_bytes(after),
                            format!("+{}", format_bytes(after))
                        ).unwrap();
                    }
                    (Some(before), None) => {
                        writeln!(md, "| {} | {} | - | 🗑️ {} | REMOVED |",
                            change.name,
                            format_bytes(before),
                            format!("-{}", format_bytes(before))
                        ).unwrap();
                    }
                    _ => {}
                }
            }
            writeln!(md).unwrap();
        }
        
        if config.sections.build_time_changes && !comparison.build_time_changes.is_empty() {
            writeln!(md, "## ⏱️ Top Crate Build Time Changes").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Crate | Baseline | Current | Change | % |").unwrap();
            writeln!(md, "|-------|----------|---------|--------|---|").unwrap();
            
            for (crate_name, before, after) in comparison.build_time_changes.iter()
                .take(config.limits.build_time_changes) 
            {
                match (before, after) {
                    (Some(before), Some(after)) => {
                        let diff = after - before;
                        let pct = (diff / before) * 100.0;
                        let emoji = if diff < 0.0 { "⚡" }
                            else if diff > 0.0 { "🐌" }
                            else { "➖" };
                        writeln!(md, "| {} | {:.2}s | {:.2}s | {} {:+.2}s | {:+.1}% |",
                            crate_name, before, after, emoji, diff, pct
                        ).unwrap();
                    }
                    (None, Some(after)) => {
                        writeln!(md, "| {} | - | {:.2}s | 🆕 +{:.2}s | NEW |",
                            crate_name, after, after
                        ).unwrap();
                    }
                    (Some(before), None) => {
                        writeln!(md, "| {} | {:.2}s | - | 🗑️ -{:.2}s | REMOVED |",
                            crate_name, before, before
                        ).unwrap();
                    }
                    _ => {}
                }
            }
            writeln!(md).unwrap();
        }
        
        if config.sections.symbol_changes && !comparison.symbol_changes.is_empty() {
            writeln!(md, "## 🔍 Biggest Symbol Changes").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "<details>").unwrap();
            writeln!(md, "<summary>Top {} symbol size changes (click to expand)</summary>", 
                config.limits.symbol_changes).unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Change | Before | After | Symbol |").unwrap();
            writeln!(md, "|--------|--------|-------|--------|").unwrap();
            
            let mut sorted_symbols = comparison.symbol_changes.clone();
            sorted_symbols.sort_by_key(|s| {
                match (s.size_before, s.size_after) {
                    (Some(before), Some(after)) => -(after as i64 - before as i64).abs(),
                    (None, Some(after)) => -(after as i64),
                    (Some(before), None) => -(before as i64),
                    _ => 0,
                }
            });
            
            for symbol in sorted_symbols.iter()
                .filter(|s| {
                    match (s.size_before, s.size_after) {
                        (Some(before), Some(after)) => 
                            (after as i64 - before as i64).abs() as u64 >= config.size_threshold,
                        (None, Some(after)) => after >= config.size_threshold,
                        (Some(before), None) => before >= config.size_threshold,
                        _ => false,
                    }
                })
                .take(config.limits.symbol_changes) 
            {
                match (symbol.size_before, symbol.size_after) {
                    (Some(before), Some(after)) => {
                        let change = after as i64 - before as i64;
                        let emoji = if change > 0 { "📈" } else { "📉" };
                        writeln!(md, "| {} {} | {} | {} | `{}` |",
                            emoji,
                            format_size_diff(change),
                            format_bytes(before),
                            format_bytes(after),
                            symbol.demangled
                        ).unwrap();
                    }
                    (None, Some(after)) => {
                        writeln!(md, "| 🆕 +{} | NEW | {} | `{}` |",
                            format_bytes(after),
                            format_bytes(after),
                            symbol.demangled
                        ).unwrap();
                    }
                    (Some(before), None) => {
                        writeln!(md, "| 🗑️ -{} | {} | REMOVED | `{}` |",
                            format_bytes(before),
                            format_bytes(before),
                            symbol.demangled
                        ).unwrap();
                    }
                    _ => {}
                }
            }
            
            writeln!(md).unwrap();
            writeln!(md, "</details>").unwrap();
            writeln!(md).unwrap();
        }
        
        if config.sections.current_top_crates && !current.top_crates.is_empty() {
            writeln!(md, "## 📦 Top Crates by Size (Current Version)").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Crate | Size | % of Total |").unwrap();
            writeln!(md, "|-------|------|------------|").unwrap();
            
            for (crate_name, size, percent) in current.top_crates.iter().take(config.limits.top_crates) {
                writeln!(md, "| {} | {} | {:.1}% |", crate_name, format_bytes(*size), percent).unwrap();
            }
            writeln!(md).unwrap();
        }
        
        if config.sections.current_top_symbols && !current.top_symbols.is_empty() {
            writeln!(md, "## 🔍 Top Symbols by Size (Current Version)").unwrap();
            writeln!(md).unwrap();
            writeln!(md, "<details>").unwrap();
            writeln!(md, "<summary>Top {} largest symbols (click to expand)</summary>", 
                config.limits.top_symbols).unwrap();
            writeln!(md).unwrap();
            writeln!(md, "| Size | Symbol |").unwrap();
            writeln!(md, "|------|--------|").unwrap();
            
            for (symbol_name, size) in current.top_symbols.iter().take(config.limits.top_symbols) {
                writeln!(md, "| {} | `{}` |", format_bytes(*size), symbol_name).unwrap();
            }
            
            writeln!(md).unwrap();
            writeln!(md, "</details>").unwrap();
            writeln!(md).unwrap();
        }
        
        // TODO: Add LLVM IR differential analysis sections
        
        writeln!(md, "---").unwrap();
        writeln!(md, "_Generated by [Substance](https://github.com/fasterthanlime/substance)_").unwrap();
    }
}

impl LlvmComparison {
    /// Create comparison from two LLVM summaries
    fn from_summaries(baseline: &LlvmSummary, current: &LlvmSummary) -> Self {
        let total_lines_diff = current.total_lines as i64 - baseline.total_lines as i64;
        let total_instantiations_diff = current.total_instantiations as i64 - baseline.total_instantiations as i64;
        
        // TODO: Calculate function-level and crate-level changes
        
        Self {
            total_lines_diff,
            total_instantiations_diff,
            function_changes: Vec::new(),
            crate_ir_changes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    
    fn create_test_report(version: &str, symbols: Vec<(&str, u64)>, crates: Vec<(&str, u64)>) -> SingleVersionReport {
        let mut all_symbols = HashMap::new();
        for (name, size) in &symbols {
            all_symbols.insert(name.to_string(), *size);
        }
        
        let mut all_crates = HashMap::new();
        for (name, size) in &crates {
            all_crates.insert(name.to_string(), *size);
        }
        
        SingleVersionReport {
            version: version.to_string(),
            metrics: SizeMetrics {
                file_size: 1000,
                text_size: 800,
                data_size: None,
                bss_size: None,
            },
            build_time: BuildTime {
                wall_time: Duration::from_secs(10),
                total_cpu_time: 10.0,
                crate_timings: vec![],
            },
            top_crates: vec![],
            top_symbols: vec![],
            all_crates,
            all_symbols,
            llvm_analysis: None,
            build_context: BuildContext {
                target_triple: "x86_64-unknown-linux-gnu".to_string(),
                artifacts: vec![],
                std_crates: vec![],
                dep_crates: vec![],
                deps_symbols: Default::default(),
            },
        }
    }
    
    #[test]
    fn test_symbol_comparison() {
        let baseline = create_test_report(
            "baseline",
            vec![
                ("foo::bar", 100),
                ("baz::qux", 200),
                ("removed::symbol", 50),
            ],
            vec![
                ("crate1", 300),
                ("crate2", 250),
            ],
        );
        
        let current = create_test_report(
            "current",
            vec![
                ("foo::bar", 150), // Changed
                ("baz::qux", 200), // Same
                ("new::symbol", 75), // New
            ],
            vec![
                ("crate1", 350),
                ("crate2", 275),
            ],
        );
        
        let comparison = ComparisonData::from_reports(&baseline, &current);
        
        // Check symbol changes
        assert_eq!(comparison.symbol_changes.len(), 3); // changed, new, removed
        
        // Find specific changes
        let foo_change = comparison.symbol_changes.iter()
            .find(|s| s.demangled == "foo::bar")
            .expect("foo::bar change not found");
        assert_eq!(foo_change.size_before, Some(100));
        assert_eq!(foo_change.size_after, Some(150));
        
        let new_symbol = comparison.symbol_changes.iter()
            .find(|s| s.demangled == "new::symbol")
            .expect("new::symbol not found");
        assert_eq!(new_symbol.size_before, None);
        assert_eq!(new_symbol.size_after, Some(75));
        
        let removed_symbol = comparison.symbol_changes.iter()
            .find(|s| s.demangled == "removed::symbol")
            .expect("removed::symbol not found");
        assert_eq!(removed_symbol.size_before, Some(50));
        assert_eq!(removed_symbol.size_after, None);
        
        // Check crate changes
        assert_eq!(comparison.crate_changes.len(), 2);
        
        let crate1_change = comparison.crate_changes.iter()
            .find(|c| c.name == "crate1")
            .expect("crate1 change not found");
        assert_eq!(crate1_change.size_before, Some(300));
        assert_eq!(crate1_change.size_after, Some(350));
    }
}