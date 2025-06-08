#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]

use std::collections::HashMap;
use std::convert::TryInto;
use std::process::{self, Command};
use std::{path, str};

use json::object;

use cargo_bloat::{
    BloatAnalyzer, BuildContext, AnalysisConfig, AnalysisResult, 
    ArtifactKind, BloatError
};

#[cfg(feature = "cli")]
use pico_args;
#[cfg(feature = "cli")]
use term_size;

mod table;

use crate::table::Table;

// CLI-specific types
struct Methods {
    has_filter: bool,
    filter_out_size: u64,
    filter_out_len: usize,
    methods: Vec<Method>,
}

struct Method {
    name: String,
    crate_name: String,
    size: u64,
}

struct Crates {
    filter_out_size: u64,
    filter_out_len: usize,
    crates: Vec<Crate>,
}

struct Crate {
    name: String,
    size: u64,
}

fn main() {
    if let Ok(wrap) = std::env::var("RUSTC_WRAPPER") {
        if wrap.contains("cargo-bloat") {
            let args: Vec<_> = std::env::args().collect();
            match wrapper_mode(&args) {
                Ok(_) => return,
                Err(e) => {
                    eprintln!("Error: {}.", e);
                    process::exit(1);
                }
            }
        }
    }

    let mut args: Vec<_> = std::env::args_os().collect();
    args.remove(0); // file path
    if args.first().and_then(|s| s.to_str()) == Some("bloat") {
        args.remove(0);
    } else {
        eprintln!("Error: can be run only via `cargo bloat`.");
        process::exit(1);
    }

    let args = match parse_args(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {}.", e);
            process::exit(1);
        }
    };

    if args.help {
        println!("{}", HELP);
        return;
    }

    if args.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let (context, binary_path) = match process_crate(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {}.", e);
            process::exit(1);
        }
    };

    if let Some(ref path) = binary_path {
        eprintln!("    Analyzing {}", path.display());
        eprintln!();
    }

    let term_width = if !args.wide {
        #[cfg(feature = "cli")]
        {
            term_size::dimensions().map(|v| v.0)
        }
        #[cfg(not(feature = "cli"))]
        {
            None
        }
    } else {
        None
    };

    // Analyze the binary using the library
    let config = AnalysisConfig {
        symbols_section: args.symbols_section.clone(),
        split_std: args.split_std,
    };

    let analysis_result = match BloatAnalyzer::analyze_binary(&binary_path.unwrap(), &context, &config) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Error: {}.", e);
            process::exit(1);
        }
    };

    if args.crates {
        let crates = filter_crates_from_result(&analysis_result, &context, &args);
        match args.message_format {
            MessageFormat::Table => {
                if args.no_relative_size {
                    print_crates_table_no_relative(crates, &analysis_result, term_width);
                } else {
                    print_crates_table(crates, &analysis_result, term_width);
                }
            }
            MessageFormat::Json => {
                print_crates_json(
                    &crates.crates,
                    analysis_result.text_size,
                    analysis_result.file_size,
                );
            }
        }
    } else {
        let methods = filter_methods_from_result(&analysis_result, &context, &args);
        match args.message_format {
            MessageFormat::Table => {
                if args.no_relative_size {
                    print_methods_table_no_relative(methods, &analysis_result, term_width);
                } else {
                    print_methods_table(methods, &analysis_result, term_width);
                }
            }
            MessageFormat::Json => {
                print_methods_json(
                    &methods.methods,
                    analysis_result.text_size,
                    analysis_result.file_size,
                );
            }
        }
    }

    if args.message_format == MessageFormat::Table {
        if args.crates {
            println!();
            println!(
                "Note: numbers above are a result of guesswork. \
                      They are not 100% correct and never will be."
            );
        }

        if analysis_result.symbols.len() < 10 {
            println!();
            println!(
                "Warning: it seems like the `.text` section is nearly empty. \
                      Try removing `strip = true` from Cargo.toml"
            );
        }
    }
}

const HELP: &str = "\
Find out what takes most of the space in your executable

USAGE:
    cargo bloat [OPTIONS]

OPTIONS:
    -h, --help                      Prints help information
    -V, --version                   Prints version information
        --lib                       Build only this package's library
        --bin <NAME>                Build only the specified binary
        --example <NAME>            Build only the specified example
        --test <NAME>               Build only the specified test target
    -p, --package <SPEC>            Package to build
        --release                   Build artifacts in release mode, with optimizations
    -j, --jobs <N>                  Number of parallel jobs, defaults to # of CPUs
        --features <FEATURES>       Space-separated list of features to activate
        --all-features              Activate all available features
        --no-default-features       Do not activate the `default` feature
        --profile <PROFILE>         Build with the given profile.
        --config <CONFIG>           Build with the given cargo config
        --target <TARGET>           Build for the target triple
        --target-dir <DIRECTORY>    Directory for all generated artifacts
        --frozen                    Require Cargo.lock and cache are up to date
        --locked                    Require Cargo.lock is up to date
    -Z <FLAG>...                    Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for details
        --crates                    Per crate bloatedness
        --filter <CRATE|REGEXP>     Filter functions by crate
        --split-std                 Split the 'std' crate to original crates like core, alloc, etc.
        --symbols-section <NAME>    Use custom symbols section (ELF-only) [default: .text]
        --no-relative-size          Hide 'File' and '.text' columns
        --full-fn                   Print full function name with hash values
    -n <NUM>                        Number of lines to show, 0 to show all [default: 20]
    -w, --wide                      Do not trim long function names
        --message-format <FMT>      Output format [default: table] [possible values: table, json]
";

#[derive(Clone, Copy, PartialEq)]
enum MessageFormat {
    Table,
    Json,
}

fn parse_message_format(s: &str) -> Result<MessageFormat, &'static str> {
    match s {
        "table" => Ok(MessageFormat::Table),
        "json" => Ok(MessageFormat::Json),
        _ => Err("invalid message format"),
    }
}

pub struct Args {
    help: bool,
    version: bool,
    lib: bool,
    bin: Option<String>,
    example: Option<String>,
    test: Option<String>,
    package: Option<String>,
    release: bool,
    jobs: Option<u32>,
    features: Option<String>,
    all_features: bool,
    no_default_features: bool,
    profile: Option<String>,
    config: Option<String>,
    target: Option<String>,
    target_dir: Option<String>,
    frozen: bool,
    locked: bool,
    unstable: Vec<String>,
    crates: bool,
    filter: Option<String>,
    split_std: bool,
    symbols_section: Option<String>,
    no_relative_size: bool,
    full_fn: bool,
    n: usize,
    wide: bool,
    verbose: bool,
    manifest_path: Option<String>,
    message_format: MessageFormat,
}

fn parse_args(raw_args: Vec<std::ffi::OsString>) -> Result<Args, pico_args::Error> {
    let mut input = pico_args::Arguments::from_vec(raw_args);
    let args = Args {
        help: input.contains(["-h", "--help"]),
        version: input.contains(["-V", "--version"]),
        lib: input.contains("--lib"),
        bin: input.opt_value_from_str("--bin")?,
        example: input.opt_value_from_str("--example")?,
        test: input.opt_value_from_str("--test")?,
        package: input.opt_value_from_str(["-p", "--package"])?,
        release: input.contains("--release"),
        jobs: input.opt_value_from_str(["-j", "--jobs"])?,
        features: input.opt_value_from_str("--features")?,
        all_features: input.contains("--all-features"),
        no_default_features: input.contains("--no-default-features"),
        profile: input.opt_value_from_str("--profile")?,
        config: input.opt_value_from_str("--config")?,
        target: input.opt_value_from_str("--target")?,
        target_dir: input.opt_value_from_str("--target-dir")?,
        frozen: input.contains("--frozen"),
        locked: input.contains("--locked"),
        unstable: input.values_from_str("-Z")?,
        crates: input.contains("--crates"),
        filter: input.opt_value_from_str("--filter")?,
        split_std: input.contains("--split-std"),
        symbols_section: input.opt_value_from_str("--symbols-section")?,
        no_relative_size: input.contains("--no-relative-size"),
        full_fn: input.contains("--full-fn"),
        n: input.opt_value_from_str("-n")?.unwrap_or(20),
        wide: input.contains(["-w", "--wide"]),
        verbose: input.contains(["-v", "--verbose"]),
        manifest_path: input.opt_value_from_str("--manifest-path")?,
        message_format: input
            .opt_value_from_fn("--message-format", parse_message_format)?
            .unwrap_or(MessageFormat::Table),
    };

    let remaining = input.finish();
    if !remaining.is_empty() {
        eprintln!("Warning: unused arguments left: {:?}.", remaining);
    }

    Ok(args)
}

impl Args {
    fn get_profile(&self) -> &str {
        if let Some(profile) = &self.profile {
            profile
        } else if self.release {
            "release"
        } else {
            "dev"
        }
    }
}

fn wrapper_mode(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();

    Command::new(&args[1])
        .args(&args[2..])
        .status()
        .map_err(|_| BloatError::CargoBuildFailed)?;

    let time_ns: u64 = start.elapsed().as_nanos().try_into()?;

    let mut crate_name = String::new();
    for (i, arg) in args.iter().enumerate() {
        if arg == "--crate-name" {
            crate_name = args[i + 1].clone();
            break;
        }
    }

    let mut build_script = false;

    if crate_name == "build_script_build" {
        build_script = true;

        let mut out_dir = String::new();
        let mut extra_filename = String::new();

        for (i, arg) in args.iter().enumerate() {
            if arg == "--out-dir" {
                out_dir = args[i + 1].clone();
            }

            if arg.starts_with("extra-filename") {
                extra_filename = arg[15..].to_string();
            }
        }

        if !out_dir.is_empty() {
            let path = std::path::Path::new(&out_dir);
            if let Some(name) = path.file_name() {
                let name = name.to_str().unwrap().to_string();
                let name = name.replace(&extra_filename, "");
                let name = name.replace('-', "_");
                crate_name = name;
            }
        }
    }

    // Still not resolved?
    if crate_name == "build_script_build" {
        crate_name = "?".to_string();
    }

    // TODO: the same crates but with different versions?

    // `cargo` will ignore raw JSON, so we have to use a prefix
    eprintln!(
        "json-time {}",
        object! {
            "crate_name" => crate_name,
            "time" => time_ns,
            "build_script" => build_script
        }
        .dump()
    );

    Ok(())
}




fn process_crate(args: &Args) -> Result<(BuildContext, Option<path::PathBuf>), BloatError> {
    // Run `cargo build` without json output first, so we could print build errors.
    {
        let cmd = &mut Command::new("cargo");
        cmd.args(get_cargo_args(args, false));
        cmd.envs(get_cargo_envs(args));

        cmd.spawn()
            .map_err(|_| BloatError::CargoBuildFailed)?
            .wait()
            .map_err(|_| BloatError::CargoBuildFailed)?;
    }

    // Run `cargo build` with json output and collect it.
    let cmd = &mut Command::new("cargo");
    cmd.args(get_cargo_args(args, true));
    cmd.envs(get_cargo_envs(args));
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    let child = cmd.spawn().map_err(|_| BloatError::CargoBuildFailed)?;

    let output = child
        .wait_with_output()
        .map_err(|_| BloatError::CargoBuildFailed)?;
    if !output.status.success() {
        return Err(BloatError::CargoBuildFailed);
    }

    let stdout = str::from_utf8(&output.stdout).unwrap();
    let json_lines: Vec<&str> = stdout.lines().collect();

    // Use the library to parse cargo metadata
    let context = BloatAnalyzer::from_cargo_metadata(
        &json_lines,
        &path::PathBuf::from("target"),
        args.target.as_deref(),
    )?;

    // Find the binary artifact to analyze
    let binary_path = context.artifacts.iter()
        .find(|a| a.kind == ArtifactKind::Binary)
        .map(|a| a.path.clone())
        .ok_or(BloatError::UnsupportedCrateType)?;

    Ok((context, Some(binary_path)))
}

fn filter_methods_from_result(result: &AnalysisResult, context: &BuildContext, args: &Args) -> Methods {
    use cargo_bloat::crate_name;
    
    // Create indices to sort by size without cloning symbols
    let mut symbol_indices: Vec<usize> = (0..result.symbols.len()).collect();
    symbol_indices.sort_by_key(|&i| result.symbols[i].size);

    let n = if args.n == 0 {
        result.symbols.len()
    } else {
        args.n
    };

    let mut methods = Vec::with_capacity(n);

    enum FilterBy {
        None,
        Crate(String),
        #[cfg(feature = "regex-filter")]
        Regex(regex::Regex),
        #[cfg(not(feature = "regex-filter"))]
        Substring(String),
    }

    let filter = if let Some(ref text) = args.filter {
        if context.std_crates.contains(text) || context.dep_crates.contains(text) {
            FilterBy::Crate(text.clone())
        } else {
            #[cfg(feature = "regex-filter")]
            {
                match regex::Regex::new(text) {
                    Ok(re) => FilterBy::Regex(re),
                    Err(_) => {
                        eprintln!(
                            "Warning: the filter value contains an unknown crate \
                                   or an invalid regexp. Ignored."
                        );
                        FilterBy::None
                    }
                }
            }

            #[cfg(not(feature = "regex-filter"))]
            {
                FilterBy::Substring(text.clone())
            }
        }
    } else {
        FilterBy::None
    };

    let has_filter = !matches!(filter, FilterBy::None);

    let mut filter_out_size = 0;
    let mut filter_out_len = 0;

    for &i in symbol_indices.iter().rev() {
        let sym = &result.symbols[i];
        let (mut crate_name, is_exact) = crate_name::from_sym(context, args.split_std, &sym.name);

        if !is_exact {
            crate_name.push('?');
        }

        let name = if args.full_fn {
            sym.name.complete.clone()
        } else {
            sym.name.trimmed.clone()
        };

        match filter {
            FilterBy::None => {}
            FilterBy::Crate(ref crate_name_f) => {
                if crate_name_f != &crate_name {
                    continue;
                }
            }
            #[cfg(feature = "regex-filter")]
            FilterBy::Regex(ref re) => {
                if !re.is_match(&name) {
                    continue;
                }
            }
            #[cfg(not(feature = "regex-filter"))]
            FilterBy::Substring(ref s) => {
                if !name.contains(s) {
                    continue;
                }
            }
        }

        filter_out_len += 1;

        if n == 0 || methods.len() < n {
            methods.push(Method {
                name,
                crate_name,
                size: sym.size,
            })
        } else {
            filter_out_size += sym.size;
        }
    }

    Methods {
        has_filter,
        filter_out_size,
        filter_out_len,
        methods,
    }
}

fn filter_crates_from_result(result: &AnalysisResult, context: &BuildContext, args: &Args) -> Crates {
    use cargo_bloat::crate_name;
    
    let mut crates = Vec::new();
    let mut sizes = HashMap::new();

    for sym in result.symbols.iter() {
        let (crate_name, _) = crate_name::from_sym(context, args.split_std, &sym.name);

        if let Some(v) = sizes.get(&crate_name).cloned() {
            sizes.insert(crate_name.to_string(), v + sym.size);
        } else {
            sizes.insert(crate_name.to_string(), sym.size);
        }
    }

    let mut list: Vec<(&String, &u64)> = sizes.iter().collect();
    list.sort_by_key(|v| v.1);

    let n = if args.n == 0 { list.len() } else { args.n };
    for &(k, v) in list.iter().rev().take(n) {
        crates.push(Crate {
            name: k.clone(),
            size: *v,
        });
    }

    let mut filter_out_size = 0;
    if n < list.len() {
        for &(_, v) in list.iter().rev().skip(n) {
            filter_out_size += *v;
        }
    }

    Crates {
        filter_out_size,
        filter_out_len: list.len() - crates.len(),
        crates,
    }
}

fn get_cargo_envs(args: &Args) -> Vec<(String, String)> {
    let mut list = Vec::new();

    let profile = args.get_profile()
        .to_ascii_uppercase()
        .replace('-', "_");

    // No matter which profile we are building for, never strip the binary
    // because we need the symbols.
    list.push((format!("CARGO_PROFILE_{}_STRIP", profile), "false".to_string()));

    // For MSVC targets, force debug info. We can't easily check target here,
    // so we'll always set debug info to be safe.
    list.push((format!("CARGO_PROFILE_{}_DEBUG", profile), "true".to_string()));

    list
}

#[allow(clippy::vec_init_then_push)]
fn get_cargo_args(args: &Args, json_output: bool) -> Vec<String> {
    let mut list = Vec::new();
    list.push("build".to_string());

    if json_output {
        list.push("--message-format=json".to_string());
    }

    if args.release {
        list.push("--release".to_string());
    }

    if args.lib {
        list.push("--lib".to_string());
    } else if let Some(ref bin) = args.bin {
        list.push(format!("--bin={}", bin));
    } else if let Some(ref example) = args.example {
        list.push(format!("--example={}", example));
    } else if let Some(ref test) = args.test {
        list.push(format!("--test={}", test));
    }

    if let Some(ref package) = args.package {
        list.push(format!("--package={}", package));
    }

    if args.all_features {
        list.push("--all-features".to_string());
    } else {
        if args.no_default_features {
            list.push("--no-default-features".to_string());
        }

        if let Some(ref features) = args.features {
            list.push(format!("--features={}", features));
        }
    }

    if let Some(ref path) = args.manifest_path {
        list.push(format!("--manifest-path={}", path))
    }

    if args.verbose {
        list.push("-v".into());
    }

    if let Some(ref profile) = args.profile {
        list.push(format!("--profile={}", profile));
    }

    if let Some(ref config) = args.config {
        list.push(format!("--config={}", config));
    }

    if let Some(ref target) = args.target {
        list.push(format!("--target={}", target));
    }

    if let Some(ref target_dir) = args.target_dir {
        list.push(format!("--target-dir={}", target_dir));
    }

    if args.frozen {
        list.push("--frozen".to_string());
    }

    if args.locked {
        list.push("--locked".to_string());
    }

    for arg in &args.unstable {
        list.push(format!("-Z={}", arg));
    }

    if let Some(jobs) = args.jobs {
        list.push(format!("-j{}", jobs));
    }

    list
}









fn print_methods_table(methods: Methods, data: &AnalysisResult, term_width: Option<usize>) {
    let section_name = data.section_name.as_deref().unwrap_or(".text");
    let mut table = Table::new(&["File", section_name, "Size", "Crate", "Name"]);
    table.set_width(term_width);

    for method in &methods.methods {
        table.push(&[
            format_percent(method.size as f64 / data.file_size as f64 * 100.0),
            format_percent(method.size as f64 / data.text_size as f64 * 100.0),
            format_size(method.size),
            method.crate_name.clone(),
            method.name.clone(),
        ]);
    }

    {
        let others_count = if methods.has_filter {
            methods.filter_out_len - methods.methods.len()
        } else {
            data.symbols.len() - methods.methods.len()
        };

        if others_count != 0 {
            table.push(&[
                format_percent(methods.filter_out_size as f64 / data.file_size as f64 * 100.0),
                format_percent(methods.filter_out_size as f64 / data.text_size as f64 * 100.0),
                format_size(methods.filter_out_size),
                String::new(),
                format!(
                    "And {} smaller methods. Use -n N to show more.",
                    others_count
                ),
            ]);
        }
    }

    if methods.has_filter {
        let total = methods.methods.iter().fold(0u64, |s, m| s + m.size) + methods.filter_out_size;

        table.push(&[
            format_percent(total as f64 / data.file_size as f64 * 100.0),
            format_percent(total as f64 / data.text_size as f64 * 100.0),
            format_size(total),
            String::new(),
            format!(
                "filtered data size, the file size is {}",
                format_size(data.file_size)
            ),
        ]);
    } else {
        table.push(&[
            format_percent(data.text_size as f64 / data.file_size as f64 * 100.0),
            format_percent(100.0),
            format_size(data.text_size),
            String::new(),
            format!(
                "{} section size, the file size is {}",
                section_name,
                format_size(data.file_size)
            ),
        ]);
    }

    print!("{}", table);
}

fn print_methods_table_no_relative(methods: Methods, data: &AnalysisResult, term_width: Option<usize>) {
    let mut table = Table::new(&["Size", "Crate", "Name"]);
    table.set_width(term_width);

    for method in &methods.methods {
        table.push(&[
            format_size(method.size),
            method.crate_name.clone(),
            method.name.clone(),
        ]);
    }

    {
        let others_count = if methods.has_filter {
            methods.filter_out_len - methods.methods.len()
        } else {
            data.symbols.len() - methods.methods.len()
        };

        if others_count != 0 {
            table.push(&[
                format_size(methods.filter_out_size),
                String::new(),
                format!(
                    "And {} smaller methods. Use -n N to show more.",
                    others_count
                ),
            ]);
        }
    }

    if methods.has_filter {
        let total = methods.methods.iter().fold(0u64, |s, m| s + m.size) + methods.filter_out_size;

        table.push(&[
            format_size(total),
            String::new(),
            format!(
                "filtered data size, the file size is {}",
                format_size(data.file_size)
            ),
        ]);
    } else {
        table.push(&[
            format_size(data.text_size),
            String::new(),
            format!(
                ".text section size, the file size is {}",
                format_size(data.file_size)
            ),
        ]);
    }

    print!("{}", table);
}

fn print_methods_json(methods: &[Method], text_size: u64, file_size: u64) {
    let mut items = json::JsonValue::new_array();
    for method in methods {
        let mut map = json::JsonValue::new_object();
        map["crate"] = method.crate_name.clone().into();
        map["name"] = method.name.clone().into();
        map["size"] = method.size.into();

        items.push(map).unwrap();
    }

    let mut root = json::JsonValue::new_object();
    root["file-size"] = file_size.into();
    root["text-section-size"] = text_size.into();
    root["functions"] = items;

    println!("{}", root.dump());
}

fn print_crates_table(crates: Crates, data: &AnalysisResult, term_width: Option<usize>) {
    let section_name = data.section_name.as_deref().unwrap_or(".text");
    let mut table = Table::new(&["File", section_name, "Size", "Crate"]);
    table.set_width(term_width);

    for item in &crates.crates {
        table.push(&[
            format_percent(item.size as f64 / data.file_size as f64 * 100.0),
            format_percent(item.size as f64 / data.text_size as f64 * 100.0),
            format_size(item.size),
            item.name.clone(),
        ]);
    }

    if crates.filter_out_len != 0 {
        table.push(&[
            format_percent(crates.filter_out_size as f64 / data.file_size as f64 * 100.0),
            format_percent(crates.filter_out_size as f64 / data.text_size as f64 * 100.0),
            format_size(crates.filter_out_size),
            format!(
                "And {} more crates. Use -n N to show more.",
                crates.filter_out_len
            ),
        ]);
    }

    table.push(&[
        format_percent(data.text_size as f64 / data.file_size as f64 * 100.0),
        format_percent(100.0),
        format_size(data.text_size),
        format!(
            "{} section size, the file size is {}",
            section_name,
            format_size(data.file_size)
        ),
    ]);

    print!("{}", table);
}

fn print_crates_table_no_relative(crates: Crates, data: &AnalysisResult, term_width: Option<usize>) {
    let mut table = Table::new(&["Size", "Crate"]);
    table.set_width(term_width);

    for item in &crates.crates {
        table.push(&[format_size(item.size), item.name.clone()]);
    }

    if crates.filter_out_len != 0 {
        table.push(&[
            format_size(crates.filter_out_size),
            format!(
                "And {} more crates. Use -n N to show more.",
                crates.filter_out_len
            ),
        ]);
    }

    let section_name = data.section_name.as_deref().unwrap_or(".text");
    table.push(&[
        format_size(data.text_size),
        format!(
            "{} section size, the file size is {}",
            section_name,
            format_size(data.file_size)
        ),
    ]);

    print!("{}", table);
}

fn print_crates_json(crates: &[Crate], text_size: u64, file_size: u64) {
    let mut items = json::JsonValue::new_array();
    for item in crates {
        let mut map = json::JsonValue::new_object();
        map["name"] = item.name.clone().into();
        map["size"] = item.size.into();

        items.push(map).unwrap();
    }

    let mut root = json::JsonValue::new_object();
    root["file-size"] = file_size.into();
    root["text-section-size"] = text_size.into();
    root["crates"] = items;

    println!("{}", root.dump());
}

fn format_percent(n: f64) -> String {
    format!("{:.1}%", n)
}

fn format_size(bytes: u64) -> String {
    let kib = 1024;
    let mib = 1024 * kib;

    if bytes >= mib {
        format!("{:.1}MiB", bytes as f64 / mib as f64)
    } else if bytes >= kib {
        format!("{:.1}KiB", bytes as f64 / kib as f64)
    } else {
        format!("{}B", bytes)
    }
}

