//! Extensions to analysis types for enhanced functionality
//!
//! This module provides additional methods for analysis types to extract
//! commonly needed information like top crates, top symbols, and
//! comparison metrics.

use crate::{
    AnalysisResult, AnalysisComparison, BuildContext, TimingInfo,
    CrateChange, SymbolChange,
};
use std::collections::HashMap;

/// Extensions to AnalysisResult for extracting summary data
impl AnalysisResult {
    /// Get the top N crates by size with percentage of total
    ///
    /// Returns a vector of (crate_name, size_bytes, percentage)
    ///
    /// # Arguments
    /// * `n` - Maximum number of crates to return
    /// * `build_context` - Build context for crate name resolution
    /// * `split_std` - Whether to split standard library into components
    pub fn top_crates(&self, n: usize, build_context: &BuildContext, split_std: bool) -> Vec<(String, u64, f64)> {
        let crate_sizes = self.crate_sizes(build_context, split_std);
        
        // Sort by size descending
        let mut crate_list: Vec<(String, u64)> = crate_sizes.into_iter().collect();
        crate_list.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
        
        // Calculate percentages and take top N
        crate_list
            .into_iter()
            .take(n)
            .map(|(name, size)| {
                let percentage = if self.text_size.value() > 0 {
                    size as f64 / self.text_size.value() as f64 * 100.0
                } else {
                    0.0
                };
                (name, size, percentage)
            })
            .collect()
    }
    
    /// Get the top N symbols by size
    ///
    /// Returns a vector of (symbol_name, size_bytes)
    pub fn top_symbols(&self, n: usize) -> Vec<(String, u64)> {
        let mut symbol_list: Vec<(String, u64)> = self.symbols
            .iter()
            .map(|s| (s.name.trimmed.clone(), s.size))
            .collect();
        
        symbol_list.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
        symbol_list.into_iter().take(n).collect()
    }
    
    /// Get size breakdown by crate
    ///
    /// Returns a map from crate name to total size in bytes
    pub fn crate_sizes(&self, build_context: &BuildContext, split_std: bool) -> HashMap<String, u64> {
        let mut crate_sizes = HashMap::new();
        
        for symbol in &self.symbols {
            let (crate_name, _) = crate::crate_name::from_sym(
                build_context,
                split_std,
                &symbol.name,
            );
            *crate_sizes.entry(crate_name).or_insert(0) += symbol.size;
        }
        
        crate_sizes
    }
    
    /// Get total size of symbols from a specific crate
    pub fn crate_size(&self, crate_name: &str, build_context: &BuildContext, split_std: bool) -> u64 {
        self.symbols
            .iter()
            .filter(|symbol| {
                let (symbol_crate, _) = crate::crate_name::from_sym(
                    build_context,
                    split_std,
                    &symbol.name,
                );
                symbol_crate == crate_name
            })
            .map(|symbol| symbol.size)
            .sum()
    }
}

/// Timing change information for build time comparisons
#[derive(Debug, Clone)]
pub struct TimingChange {
    pub crate_name: String,
    pub baseline_time: Option<f64>,
    pub current_time: Option<f64>,
}

impl TimingChange {
    /// Calculate the absolute time difference
    pub fn absolute_diff(&self) -> f64 {
        match (self.baseline_time, self.current_time) {
            (Some(before), Some(after)) => after - before,
            (None, Some(after)) => after,
            (Some(before), None) => -before,
            _ => 0.0,
        }
    }
    
    /// Calculate the percentage change
    pub fn percent_change(&self) -> Option<f64> {
        match (self.baseline_time, self.current_time) {
            (Some(before), Some(after)) if before > 0.0 => {
                Some(((after - before) / before) * 100.0)
            }
            _ => None,
        }
    }
}

/// Extensions to AnalysisComparison for filtering and analysis
impl AnalysisComparison {
    /// Get crate changes that exceed a size threshold
    ///
    /// # Arguments
    /// * `threshold` - Minimum absolute size change in bytes to include
    pub fn significant_changes(&self, threshold: u64) -> Vec<&CrateChange> {
        self.crate_changes
            .iter()
            .filter(|change| {
                change.absolute_change()
                    .map(|c| c.abs() as u64 >= threshold)
                    .unwrap_or(true) // Include new/removed crates regardless of threshold
            })
            .collect()
    }
    
    /// Get symbol changes that exceed a size threshold
    ///
    /// # Arguments
    /// * `threshold` - Minimum absolute size change in bytes to include
    pub fn significant_symbol_changes(&self, threshold: u64) -> Vec<&SymbolChange> {
        self.symbol_changes
            .iter()
            .filter(|change| {
                match (change.size_before, change.size_after) {
                    (Some(before), Some(after)) => 
                        (after as i64 - before as i64).abs() as u64 >= threshold,
                    (None, Some(after)) => after >= threshold,
                    (Some(before), None) => before >= threshold,
                    _ => false,
                }
            })
            .collect()
    }
    
    /// Calculate build time changes between two sets of timing data
    ///
    /// Returns a vector of timing changes sorted by absolute difference
    pub fn build_time_changes(
        baseline_timings: &[TimingInfo], 
        current_timings: &[TimingInfo]
    ) -> Vec<TimingChange> {
        let mut baseline_map: HashMap<String, f64> = HashMap::new();
        let mut current_map: HashMap<String, f64> = HashMap::new();
        
        for timing in baseline_timings {
            baseline_map.insert(timing.crate_name.clone(), timing.duration);
        }
        for timing in current_timings {
            current_map.insert(timing.crate_name.clone(), timing.duration);
        }
        
        // Combine all crate names
        let mut all_crates = std::collections::HashSet::new();
        all_crates.extend(baseline_map.keys().cloned());
        all_crates.extend(current_map.keys().cloned());
        
        let mut changes: Vec<TimingChange> = all_crates
            .into_iter()
            .map(|name| TimingChange {
                crate_name: name.clone(),
                baseline_time: baseline_map.get(&name).copied(),
                current_time: current_map.get(&name).copied(),
            })
            .collect();
        
        // Sort by absolute difference
        changes.sort_by(|a, b| {
            b.absolute_diff().abs()
                .partial_cmp(&a.absolute_diff().abs())
                .unwrap()
        });
        
        changes
    }
    
    /// Get the total size change across all crates
    pub fn total_size_change(&self) -> i64 {
        self.crate_changes
            .iter()
            .filter_map(|c| c.absolute_change())
            .sum()
    }
    
    /// Get crates sorted by percentage change
    ///
    /// Only includes crates that existed in both versions
    pub fn crates_by_percent_change(&self) -> Vec<&CrateChange> {
        let mut changes: Vec<&CrateChange> = self.crate_changes
            .iter()
            .filter(|c| c.percent_change().is_some())
            .collect();
        
        changes.sort_by(|a, b| {
            let a_pct = a.percent_change().unwrap_or(0.0).abs();
            let b_pct = b.percent_change().unwrap_or(0.0).abs();
            b_pct.partial_cmp(&a_pct).unwrap()
        });
        
        changes
    }
    
    /// Get new crates (didn't exist in baseline)
    pub fn new_crates(&self) -> Vec<&CrateChange> {
        self.crate_changes
            .iter()
            .filter(|c| c.size_before.is_none() && c.size_after.is_some())
            .collect()
    }
    
    /// Get removed crates (existed in baseline but not current)
    pub fn removed_crates(&self) -> Vec<&CrateChange> {
        self.crate_changes
            .iter()
            .filter(|c| c.size_before.is_some() && c.size_after.is_none())
            .collect()
    }
    
    /// Get new symbols (didn't exist in baseline)
    pub fn new_symbols(&self) -> Vec<&SymbolChange> {
        self.symbol_changes
            .iter()
            .filter(|s| s.size_before.is_none() && s.size_after.is_some())
            .collect()
    }
    
    /// Get removed symbols (existed in baseline but not current)
    pub fn removed_symbols(&self) -> Vec<&SymbolChange> {
        self.symbol_changes
            .iter()
            .filter(|s| s.size_before.is_some() && s.size_after.is_none())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_timing_change_calculations() {
        let change = TimingChange {
            crate_name: "test".to_string(),
            baseline_time: Some(10.0),
            current_time: Some(15.0),
        };
        
        assert_eq!(change.absolute_diff(), 5.0);
        assert_eq!(change.percent_change(), Some(50.0));
        
        let removed = TimingChange {
            crate_name: "removed".to_string(),
            baseline_time: Some(10.0),
            current_time: None,
        };
        
        assert_eq!(removed.absolute_diff(), -10.0);
        assert_eq!(removed.percent_change(), None);
    }
}