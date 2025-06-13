use binfarce::Format;
use camino::Utf8Path;

use crate::{
    errors::SubstanceError,
    types::{CrateName, MangledSymbol},
};

/// Contains raw symbols read by binfarce
pub(crate) struct RawObjectAnalysis {
    pub(crate) symbols: Vec<binfarce::demangle::SymbolData>,
    pub(crate) text_size: u64,
}

pub(crate) fn collect_self_data(
    path: &Utf8Path,
    section_name: &str,
) -> Result<RawObjectAnalysis, SubstanceError> {
    let data = &map_file(path)?;

    let mut d = match binfarce::detect_format(data) {
        Format::Elf32 { byte_order: _ } => collect_elf_data(path, data, section_name)?,
        Format::Elf64 { byte_order: _ } => collect_elf_data(path, data, section_name)?,
        Format::Macho => collect_macho_data(data)?,
        Format::PE => collect_pe_data(path, data)?,
        Format::Unknown => return Err(SubstanceError::UnsupportedFileFormat(path.to_owned())),
    };

    // Multiple symbols may point to the same address.
    // Remove duplicates.
    d.symbols.sort_by_key(|v| v.address);
    d.symbols.dedup_by_key(|v| v.address);

    Ok(d)
}

fn collect_elf_data(
    path: &Utf8Path,
    data: &[u8],
    section_name: &str,
) -> Result<RawObjectAnalysis, SubstanceError> {
    let is_64_bit = match data[4] {
        1 => false,
        2 => true,
        _ => return Err(SubstanceError::UnsupportedFileFormat(path.to_owned())),
    };

    let byte_order = match data[5] {
        1 => binfarce::ByteOrder::LittleEndian,
        2 => binfarce::ByteOrder::BigEndian,
        _ => return Err(SubstanceError::UnsupportedFileFormat(path.to_owned())),
    };

    let (symbols, text_size) = if is_64_bit {
        binfarce::elf64::parse(data, byte_order)?.symbols(section_name)?
    } else {
        binfarce::elf32::parse(data, byte_order)?.symbols(section_name)?
    };

    let d = RawObjectAnalysis { symbols, text_size };

    Ok(d)
}

fn collect_macho_data(data: &[u8]) -> Result<RawObjectAnalysis, SubstanceError> {
    let (symbols, text_size) = binfarce::macho::parse(data)?.symbols()?;
    let d = RawObjectAnalysis { symbols, text_size };

    Ok(d)
}

fn collect_pe_data(path: &Utf8Path, data: &[u8]) -> Result<RawObjectAnalysis, SubstanceError> {
    let (symbols, text_size) = binfarce::pe::parse(data)?.symbols()?;

    // `pe::parse` will return zero symbols for an executable built with MSVC.
    if symbols.is_empty() {
        let pdb_path = {
            let file_name = if let Some(file_name) = path.file_name() {
                file_name.replace('-', "_")
            } else {
                return Err(SubstanceError::OpenFailed(path.to_owned()));
            };
            path.with_file_name(file_name).with_extension("pdb")
        };

        collect_pdb_data(&pdb_path, text_size)
    } else {
        Ok(RawObjectAnalysis { symbols, text_size })
    }
}

fn collect_pdb_data(
    pdb_path: &Utf8Path,
    text_size: u64,
) -> Result<RawObjectAnalysis, SubstanceError> {
    use pdb::FallibleIterator;

    let file = std::fs::File::open(pdb_path)
        .map_err(|_| SubstanceError::OpenFailed(pdb_path.to_owned()))?;
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
            address.map(|address| binfarce::demangle::SymbolData {
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

    let d = RawObjectAnalysis { symbols, text_size };

    Ok(d)
}

// Binary parsing utility functions
fn map_file(path: &camino::Utf8Path) -> Result<memmap2::Mmap, SubstanceError> {
    let file =
        std::fs::File::open(path).map_err(|_| SubstanceError::OpenFailed(path.to_owned()))?;
    let file = unsafe {
        memmap2::Mmap::map(&file).map_err(|_| SubstanceError::OpenFailed(path.to_owned()))?
    };
    Ok(file)
}

/// Constructs a map of mangled symbol names to library (crate) names
pub(crate) fn collect_deps_symbols(
    libs: Vec<(CrateName, camino::Utf8PathBuf)>,
) -> Result<multimap::MultiMap<MangledSymbol, CrateName>, SubstanceError> {
    let mut map = multimap::MultiMap::new();

    for (name, path) in libs {
        let file = map_file(&path)?;
        let raw_syms = crate::ar::parse(&file)?;
        for sym in raw_syms.into_iter().map(MangledSymbol::new) {
            map.insert(sym, name.clone());
        }
    }

    for (_, v) in map.iter_all_mut() {
        v.dedup();
    }

    Ok(map)
}
