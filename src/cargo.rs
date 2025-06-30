use camino::Utf8PathBuf;
use facet::Facet;

use crate::types::CrateName;

// Cargo JSON metadata structures
#[derive(Debug, Facet)]
struct RawCargoMessage {
    /// "compiler-artifact", "timing-info", etc.
    reason: String,

    /// which target the message is for
    #[facet(default)]
    target: Option<CargoTarget>,

    /// compiler-artifact only
    #[facet(default)]
    filenames: Option<Vec<String>>,

    /// timing-info only
    #[facet(default)]
    duration: Option<f64>,

    /// timing-info only
    #[facet(default)]
    rmeta_time: Option<f64>,
}

#[derive(Debug, Facet)]
pub struct CargoTarget {
    /// The name of the build target, something like: "static_assertions", "proc_macro2", etc.
    pub name: Option<String>,

    /// kind: ["lib", "bin", etc.]
    pub kind: Option<Vec<String>>,

    /// crate_types: ["lib", "bin", etc.]
    pub crate_types: Option<Vec<String>>,
}

// Timing structures for build analysis
#[derive(Debug)]
pub struct TimingInfo {
    // cf. [`CargoMessage`]
    pub target: CargoTarget,

    // cf. [`CargoMessage`]
    pub duration: f64,

    // cf. [`CargoMessage`]
    pub rmeta_time: Option<f64>,
}

pub struct CompilerArtifact {
    // cf. [`CargoMessage`]
    pub crate_name: CrateName,

    // cf. [`CargoMessage`]
    pub filenames: Vec<Utf8PathBuf>,
}

pub(crate) enum CargoMessage {
    TimingInfo(TimingInfo),
    CompilerArtifact(CompilerArtifact),
}

use std::fmt;

#[derive(Debug)]
pub enum CargoMessageError {
    UnknownReason(String),
    MissingTarget { reason: String },
    MissingDuration,
    MissingCrateName,
    MissingFilenames,
    InvalidJson(String),
}

impl fmt::Display for CargoMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use CargoMessageError::*;
        match self {
            UnknownReason(reason) => write!(f, "Unknown cargo message reason: {}", reason),
            MissingTarget { reason } => write!(f, "Missing target for {}", reason),
            MissingDuration => write!(f, "Missing duration for timing-info"),
            MissingCrateName => write!(f, "Missing crate name for compiler-artifact"),
            MissingFilenames => write!(f, "Missing filenames for compiler-artifact"),
            InvalidJson(err) => write!(f, "Failed to parse cargo message JSON: {}", err),
        }
    }
}

impl std::error::Error for CargoMessageError {}

impl CargoMessage {
    /// Parse a Cargo JSON message line into `CargoMessage`.
    ///
    /// This function expects a JSON line as produced by cargo with `-Zunstable-options --message-format=json`.
    /// It uses `facet_json` to parse the line into a `RawCargoMessage`, then converts it using `TryFrom`.
    pub fn parse(json_line: &str) -> Result<Option<Self>, CargoMessageError> {
        let raw: RawCargoMessage = facet_json::from_str(json_line)
            .map_err(|e| CargoMessageError::InvalidJson(e.to_string()))?;
        match raw.reason.as_str() {
            "timing-info" => {
                let target = raw.target.ok_or_else(|| CargoMessageError::MissingTarget {
                    reason: "timing-info".to_string(),
                })?;
                let duration = raw.duration.ok_or(CargoMessageError::MissingDuration)?;
                Ok(Some(CargoMessage::TimingInfo(TimingInfo {
                    target,
                    duration,
                    rmeta_time: raw.rmeta_time,
                })))
            }
            "compiler-artifact" => {
                let target = raw.target.ok_or_else(|| CargoMessageError::MissingTarget {
                    reason: "compiler-artifact".to_string(),
                })?;
                let crate_name = target
                    .name
                    .clone()
                    .ok_or(CargoMessageError::MissingCrateName)?
                    .into();
                let filenames = raw
                    .filenames
                    .ok_or(CargoMessageError::MissingFilenames)?
                    .into_iter()
                    .map(Utf8PathBuf::from)
                    .collect();
                Ok(Some(CargoMessage::CompilerArtifact(CompilerArtifact {
                    crate_name,
                    filenames,
                })))
            }
            "build-script-executed" => {
                // ignore
                Ok(None)
            }
            "compiler-message" => {
                // ignore
                Ok(None)
            }
            "build-finished" => {
                // ignore
                Ok(None)
            }
            other => Err(CargoMessageError::UnknownReason(other.to_string())),
        }
    }
}
