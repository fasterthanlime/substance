use binfarce::demangle::SymbolName;
use std::collections::HashMap;
use crate::types::{LlvmIrLines, NumberOfCopies};

#[derive(Debug, Clone)]
pub struct LlvmInstantiations {
    pub copies: NumberOfCopies,
    pub total_lines: LlvmIrLines,
}

impl Default for LlvmInstantiations {
    fn default() -> Self {
        Self {
            copies: NumberOfCopies::new(0usize),
            total_lines: LlvmIrLines::new(0usize),
        }
    }
}

impl LlvmInstantiations {
    fn record_lines(&mut self, lines: usize) {
        self.copies = NumberOfCopies::new(self.copies.value() + 1);
        self.total_lines = LlvmIrLines::new(self.total_lines.value() + lines);
    }
}

pub fn analyze_llvm_ir_data(ir: &[u8]) -> HashMap<String, LlvmInstantiations> {
    let mut instantiations: HashMap<String, LlvmInstantiations> = HashMap::new();
    let mut current_function = None;
    let mut count = 0;

    for line in String::from_utf8_lossy(ir).lines() {
        if line.starts_with("define ") {
            current_function = parse_function_name(line);
        } else if line == "}" {
            if let Some(name) = current_function.take() {
                instantiations.entry(name).or_default().record_lines(count);
            }
            count = 0;
        } else if line.starts_with("  ") && !line.starts_with("   ") {
            count += 1;
        }
    }

    instantiations
}

fn parse_function_name(line: &str) -> Option<String> {
    let start = line.find('@')? + 1;
    let end = line[start..].find('(')?;
    let mangled = line[start..start + end].trim_matches('"');

    // Use binfarce's demangle instead of rustc-demangle
    let symbol_name = SymbolName::demangle(mangled);
    let mut name = symbol_name.trimmed.clone();

    // Remove hash suffix if present (same logic as cargo-llvm-lines)
    if has_hash(&name) {
        let len = name.len() - 19;
        name.truncate(len);
    }

    Some(name)
}

fn has_hash(name: &str) -> bool {
    let mut bytes = name.bytes().rev();
    for _ in 0..16 {
        if !bytes.next().is_some_and(is_ascii_hexdigit) {
            return false;
        }
    }
    bytes.next() == Some(b'h') && bytes.next() == Some(b':') && bytes.next() == Some(b':')
}

fn is_ascii_hexdigit(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name_demangling() {
        // Test cases extracted from actual LLVM IR output
        let test_cases = vec![
            (
                r#"define internal void @"_ZN4core3ptr42drop_in_place$LT$substance..BloatError$GT$17h70910838441ee278E"(ptr align 8 %_1) unnamed_addr #0 !dbg !123"#,
                "core::ptr::drop_in_place<substance::BloatError>",
            ),
            (
                r#"define internal void @"_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17haddeafc23f955172E"(ptr %self) unnamed_addr #0 !dbg !456"#,
                "<&T as core::fmt::Debug>::fmt",
            ),
            (
                r#"define internal void @"_ZN4core3fmt3num50_$LT$impl$u20$core..fmt..Debug$u20$for$u20$u32$GT$3fmt17h245219febfc19038E"(ptr %self) unnamed_addr #0 !dbg !789"#,
                "core::fmt::num::<impl core::fmt::Debug for u32>::fmt",
            ),
            (
                r#"define internal void @"_ZN3std2rt10lang_start28_$u7b$$u7b$closure$u7d$$u7d$17h0455556706c7eca7E"(ptr %self) unnamed_addr #0 !dbg !999"#,
                "std::rt::lang_start::{{closure}}",
            ),
        ];

        for (llvm_line, expected_demangled) in test_cases {
            println!("Testing line: {}", llvm_line);
            let result = parse_function_name(llvm_line);
            assert!(
                result.is_some(),
                "Failed to parse function name from: {}",
                llvm_line
            );
            let demangled = result.unwrap();
            println!("Got: {}", demangled);
            println!("Expected: {}", expected_demangled);
            assert_eq!(
                demangled, expected_demangled,
                "Demangling mismatch for {}\nExpected: {}\nGot: {}",
                llvm_line, expected_demangled, demangled
            );
        }
    }

    #[test]
    fn test_llvm_ir_analysis() {
        let sample_ir = r#"; ModuleID = 'test'
source_filename = "test"

define internal void @"_ZN4core3ptr42drop_in_place$LT$substance..BloatError$GT$17h70910838441ee278E"(ptr align 8 %_1) unnamed_addr #0 !dbg !123 {
start:
  %a = alloca [8 x i8], align 8
  %b = alloca [8 x i8], align 8
  call void @some_function()
  ret void
}

define internal void @"_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17haddeafc23f955172E"(ptr %self) unnamed_addr #0 !dbg !456 {
start:
  %temp = alloca [16 x i8], align 8
  call void @another_function()
  call void @yet_another_function()
  ret void
}

define internal void @"_ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17haddeafc23f955172E"(ptr %self) unnamed_addr #0 !dbg !789 {
start:
  %duplicate = alloca [8 x i8], align 8
  ret void
}
"#;

        let result = analyze_llvm_ir_data(sample_ir.as_bytes());

        println!("Found {} functions", result.len());
        for (name, stats) in &result {
            println!(
                "Function: {}, copies: {}, lines: {}",
                name, stats.copies, stats.total_lines
            );
        }

        // Should have 2 unique functions
        assert_eq!(result.len(), 2);

        // Check first function
        let drop_fn = result
            .get("core::ptr::drop_in_place<substance::BloatError>")
            .unwrap();
        assert_eq!(drop_fn.copies, 1);
        assert_eq!(drop_fn.total_lines, 4); // %a, %b, call, ret

        // Check second function (appears twice, should be merged)
        let debug_fn = result.get("<&T as core::fmt::Debug>::fmt").unwrap();
        assert_eq!(debug_fn.copies, 2); // Two instantiations
        assert_eq!(debug_fn.total_lines, 6); // 4 lines first + 2 lines second
    }

    #[test]
    fn test_hash_removal() {
        // Test hash detection and removal
        assert!(has_hash("some::function::name::h1234567890abcdef"));
        assert!(!has_hash("some::function::name"));
        assert!(!has_hash("some::function::name::h123")); // too short
        assert!(!has_hash("some::function::name::g1234567890abcdef")); // wrong prefix

        // Test with actual demangling that includes hash
        let mangled = "_ZN4test8function17h1234567890abcdefE";
        let symbol_name = SymbolName::demangle(mangled);
        let mut name = symbol_name.trimmed.clone();

        if has_hash(&name) {
            let len = name.len() - 19;
            name.truncate(len);
        }

        // The result should not contain the hash
        assert!(!name.contains("::h"));
    }
}
