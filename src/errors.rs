//! Centralized error definitions for the `substance` crate.
//!
//! The previous implementation hand-rolled `Display` and `Error`
//! implementations.  This version leverages the `thiserror` crate to
//! remove boilerplate while preserving all error messages verbatim.

use camino::Utf8PathBuf;
use thiserror::Error;

/// All errors that can originate from `substance`.
///
/// Wherever possible, we derive `From` automatically via `thiserror`'s
/// `#[from]` attribute so that `?` can be used ergonomically throughout
/// the codebase.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error)]
pub enum SubstanceError {
    #[error("failed to find a dir with std libraries. Expected location: {0}")]
    StdDirNotFound(Utf8PathBuf),

    #[error("failed to execute 'rustc'. It should be in the PATH")]
    RustcFailed,

    #[error("{0}")]
    CargoError(String),

    #[error("failed to execute 'cargo'. It should be in the PATH")]
    CargoMetadataFailed,

    #[error("failed to execute 'cargo build'. Probably a build error")]
    CargoBuildFailed,

    #[error("only 'bin', 'dylib' and 'cdylib' crate types are supported")]
    UnsupportedCrateType,

    #[error("failed to open a file '{0}'")]
    OpenFailed(Utf8PathBuf),

    #[error("failed to parse 'cargo' output")]
    InvalidCargoOutput,

    #[error("'cargo' does not produce any build artifacts")]
    NoArtifacts,

    #[error("{0} has an unsupported file format")]
    UnsupportedFileFormat(Utf8PathBuf),

    #[error("parsing failed cause '{0}'")]
    ParsingError(#[from] binfarce::ParseError),

    #[error("error parsing pdb file cause '{0}'")]
    PdbError(#[from] pdb::Error),

    #[error("failed to detect target triple")]
    TargetDetectionFailed,
}

/// `binfarce::UnexpectedEof` does not implement `std::error::Error`, so
/// it cannot participate in the automatic `#[from]` conversions.
/// Instead, we convert it manually to the existing `ParsingError`
/// variant, mirroring the behaviour of the original implementation.
impl From<binfarce::UnexpectedEof> for SubstanceError {
    fn from(_: binfarce::UnexpectedEof) -> Self {
        SubstanceError::ParsingError(binfarce::ParseError::UnexpectedEof)
    }
}
