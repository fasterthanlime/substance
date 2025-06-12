use crate::{errors::SubstanceError, types::CrateName};

pub(crate) fn stdlibs_dir() -> Result<camino::Utf8PathBuf, SubstanceError> {
    use camino::Utf8PathBuf;
    use std::process::Command;

    log::debug!("Finding stdlib directory");

    let output = Command::new("rustc")
        .arg("--print")
        .arg("target-libdir")
        .output()
        .map_err(|e| {
            log::error!("Failed to execute rustc: {}", e);
            SubstanceError::RustcFailed
        })?;

    // Handle potential non-UTF8 output gracefully, rather than panicking.
    // If it's not valid UTF-8, it's likely an issue with rustc's output itself.
    let stdout = std::str::from_utf8(&output.stdout).map_err(|e| {
        log::error!("rustc output is not valid UTF-8: {}", e);
        SubstanceError::RustcFailed // Use an existing error variant if `InvalidUtf8` isn't available
    })?;

    // Use camino::Utf8PathBuf directly, as expected by SubstanceError::StdDirNotFound
    let rustlib = Utf8PathBuf::from(stdout.trim());

    log::debug!("Found rustlib path: {:?}", rustlib);

    if !rustlib.exists() {
        log::error!("Stdlib directory not found: {:?}", rustlib);
        // This line caused the type mismatch, now `rustlib` is Utf8PathBuf
        return Err(SubstanceError::StdDirNotFound(rustlib));
    }

    log::info!("Successfully located stdlib directory: {:?}", rustlib);
    Ok(rustlib)
}

pub(crate) fn collect_rlib_paths(
    deps_dir: &camino::Utf8Path,
) -> Vec<(CrateName, camino::Utf8PathBuf)> {
    let mut rlib_paths: Vec<(CrateName, camino::Utf8PathBuf)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(deps_dir) {
        for entry in entries.flatten() {
            let path = camino::Utf8PathBuf::from_path_buf(entry.path())
                .ok()
                .unwrap();
            if let Some("rlib") = path.extension() {
                let crate_name = rlib_path_to_cratename(&path);
                rlib_paths.push((crate_name, path));
            }
        }
    }

    rlib_paths.sort_by(|a, b| a.0.cmp(&b.0));

    rlib_paths
}

fn rlib_path_to_cratename(path: &camino::Utf8PathBuf) -> CrateName {
    let mut stem = path.file_stem().unwrap().to_string();
    if let Some(idx) = stem.bytes().position(|b| b == b'-') {
        stem.drain(idx..);
    }
    stem.drain(0..3); // trim 'lib'
    CrateName::from(stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    // Helper for creating crate names in tests, since CrateName::from is used in implementation.
    fn crate_name<S: Into<String>>(s: S) -> CrateName {
        CrateName::from(s.into())
    }

    #[test]
    fn test_rlib_path_to_cratename_examples() {
        let test_cases = [
            ("libaliri_braid-e8f9425717316f27.rlib", "aliri_braid"),
            ("libariadne-4461764e96a6aad7.rlib", "ariadne"),
            ("libariadne-fa988516bead4084.rlib", "ariadne"),
            ("libbinfarce-54616b3acfd03db2.rlib", "binfarce"),
            ("libbinfarce-c8780ecc9f2812a9.rlib", "binfarce"),
            ("libbitflags-0ada03d8b14b6bb7.rlib", "bitflags"),
            ("libbyteorder-681a16c1411db6d4.rlib", "byteorder"),
            ("libcompact_str-0145c52390060b57.rlib", "compact_str"),
            (
                "libfacet_macros_parse-663a66863aa3d445.rlib",
                "facet_macros_parse",
            ),
            ("libunicode_ident-c4cd5a2669b29311.rlib", "unicode_ident"),
            ("libsubstance-f1b89b45b144f585.rlib", "substance"),
        ];

        for (filename, expected_crate) in &test_cases {
            let path = Utf8PathBuf::from(filename);
            let got = rlib_path_to_cratename(&path);
            let want = crate_name(*expected_crate);
            assert_eq!(got, want, "failed on filename: {filename}");
        }
    }

    #[test]
    fn test_rlib_path_to_cratename_minimal() {
        let path = Utf8PathBuf::from("libfoo-123456abcdefabcd.rlib");
        let cratename = rlib_path_to_cratename(&path);
        assert_eq!(cratename, crate_name("foo"));
    }

    #[test]
    fn test_rlib_path_to_cratename_underscore() {
        let path = Utf8PathBuf::from("libbar_baz-abcd1234abcdabcd.rlib");
        let cratename = rlib_path_to_cratename(&path);
        assert_eq!(cratename, crate_name("bar_baz"));
    }
}
