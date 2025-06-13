use crate::{
    types::{CrateName, LlvmFunctionNameRef, MangledSymbolRef},
    BuildContext,
};
use binfarce::demangle::{self, SymbolName};

pub const UNKNOWN: &str = "[Unknown]";

pub enum StdHandling {
    // Show core, addr2line, etc. separately
    Split,
    // Show "std" for all crates in libstd
    Merged,
}

pub fn from_sym(
    context: &BuildContext,
    std_handling: StdHandling,
    sym: &SymbolName,
) -> (CrateName, bool) {
    let (mut name, is_exact) = from_sym_impl(context, sym);

    match std_handling {
        StdHandling::Merged => {
            if context.std_crates.contains(&name) {
                name = CrateName::from("std");
            }
        }
        StdHandling::Split => {}
    }

    (name, is_exact)
}

fn from_sym_impl(context: &BuildContext, sym: &SymbolName) -> (CrateName, bool) {
    if let Some(name) = context
        .deps_symbols
        .get(MangledSymbolRef::from_str(&sym.complete))
    {
        return (name.clone(), true);
    }

    match sym.kind {
        demangle::Kind::Legacy => {
            let (name, is_exact) = parse_sym(context, &sym.complete);
            (CrateName::from(name), is_exact)
        }
        demangle::Kind::V0 => match sym.crate_name {
            Some(ref name) => (CrateName::from(name.to_string()), true),
            None => {
                let (name, is_exact) = parse_sym_v0(context, &sym.trimmed);
                (CrateName::from(name), is_exact)
            }
        },
        demangle::Kind::Unknown => (CrateName::from(UNKNOWN.to_string()), true),
    }
}

// A simple stupid symbol parser.
// Should be replaced by something better later.
fn parse_sym(d: &BuildContext, sym: &str) -> (String, bool) {
    // TODO: ` for `

    let mut is_exact = true;
    let name = if sym.contains(" as ") {
        let parts: Vec<_> = sym.split(" as ").collect();
        let crate_name1 = parse_crate_from_sym(parts[0]);
        let crate_name2 = parse_crate_from_sym(parts[1]);

        // <crate_name1::Type as crate_name2::Trait>::fn

        // `crate_name1` can be empty in cases when it's just a type parameter, like:
        // <T as core::fmt::Display>::fmt::h92003a61120a7e1a
        if crate_name1.is_empty() {
            crate_name2
        } else {
            if crate_name1 == crate_name2 {
                crate_name1
            } else {
                // This is an uncertain case.
                //
                // Example:
                // <euclid::rect::TypedRect<f64> as resvg::geom::RectExt>::x
                //
                // Here we defined and instanced the `RectExt` trait
                // in the `resvg` crate, but the first crate is `euclid`.
                // Usually, those traits will be present in `deps_symbols`
                // so they will be resolved automatically, in other cases it's an UB.

                if let Some(names) = d.deps_symbols.get_vec(sym) {
                    if names.contains(&CrateName::from(crate_name1.clone())) {
                        crate_name1
                    } else if names.contains(&CrateName::from(crate_name2.clone())) {
                        crate_name2
                    } else {
                        // Example:
                        // <std::collections::hash::map::DefaultHasher as core::hash::Hasher>::finish
                        // ["cc", "cc", "fern", "fern", "svgdom", "svgdom"]

                        is_exact = false;
                        crate_name1
                    }
                } else {
                    // If the symbol is not in `deps_symbols` then it probably
                    // was imported/inlined to the crate bin itself.

                    is_exact = false;
                    crate_name1
                }
            }
        }
    } else {
        parse_crate_from_sym(sym)
    };

    (name, is_exact)
}

fn parse_crate_from_sym(sym: &str) -> String {
    if !sym.contains("::") {
        return String::new();
    }

    let mut crate_name = if let Some(s) = sym.split("::").next() {
        s.to_string()
    } else {
        sym.to_string()
    };

    if crate_name.starts_with('<') {
        while crate_name.starts_with('<') {
            crate_name.remove(0);
        }

        while crate_name.starts_with('&') {
            crate_name.remove(0);
        }

        crate_name = crate_name.split_whitespace().last().unwrap().to_owned();
    }

    crate_name
}

fn parse_sym_v0(d: &BuildContext, sym: &str) -> (String, bool) {
    let name = parse_crate_from_sym(sym);

    // Check that such crate name is an actual dependency
    // and not some random string.
    if d.std_crates.contains(&CrateName::from(name.clone()))
        || d.dep_crates.contains(&CrateName::from(name.clone()))
    {
        (name, false)
    } else {
        (UNKNOWN.to_string(), true)
    }
}

/// Extract crate name from an LLVM IR function name
///
/// This is used for analyzing LLVM IR output where function names
/// have a different format than regular symbol names.
///
/// # Examples
/// - `<T as alloc::vec::Vec>::method` -> `alloc`
/// - `core::ptr::drop_in_place` -> `core`
/// - `_ZN4core3ptr13drop_in_place17h1234567890abcdefE` -> `core`
pub fn extract_crate_from_function(func_name: &LlvmFunctionNameRef) -> String {
    let func_name = func_name.as_str();

    // Handle generic implementations and trait bounds
    let cleaned = if func_name.starts_with('<') {
        // For functions like "<T as alloc::vec::Vec>::method", extract after "as"
        if let Some(as_pos) = func_name.find(" as ") {
            let after_as = &func_name[as_pos + 4..];
            if let Some(end) = after_as.find(">::") {
                after_as[..end].to_string()
            } else if let Some(end) = after_as.find('>') {
                after_as[..end].to_string()
            } else {
                after_as.to_string()
            }
        } else if let Some(space_pos) = func_name.find(' ') {
            // Handle other generic patterns
            func_name[space_pos + 1..].to_string()
        } else {
            func_name.to_string()
        }
    } else {
        func_name.to_string()
    };

    // Extract the crate name from the cleaned function name
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

    // For complex functions, try to find a crate name in the path
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
