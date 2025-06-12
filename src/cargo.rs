use std::convert::TryFrom;

use camino::Utf8PathBuf;
use facet::Facet;

use crate::types::CrateName;

// Cargo JSON metadata structures
#[derive(Debug, Facet)]
struct RawCargoMessage {
    /// "compiler-artifact", "timing-info", etc.
    reason: String,

    /// which target the message is for
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
pub(crate) struct CargoTarget {
    /// The name of the build target, something like: "static_assertions", "proc_macro2", etc.
    pub(crate) name: Option<String>,

    /// kind: ["lib", "bin", etc.]
    pub(crate) kind: Option<Vec<String>>,

    /// crate_types: ["lib", "bin", etc.]
    pub(crate) crate_types: Option<Vec<String>>,
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

impl TryFrom<RawCargoMessage> for CargoMessage {
    type Error = &'static str;

    fn try_from(msg: RawCargoMessage) -> Result<Self, Self::Error> {
        match msg.reason.as_str() {
            "timing-info" => {
                let target = msg.target.ok_or("Missing target for timing-info")?;
                let duration = msg.duration.ok_or("Missing duration for timing-info")?;
                Ok(CargoMessage::TimingInfo(TimingInfo {
                    target,
                    duration,
                    rmeta_time: msg.rmeta_time,
                }))
            }
            "compiler-artifact" => {
                let target = msg.target.ok_or("Missing target for compiler-artifact")?;
                let crate_name = target
                    .name
                    .clone()
                    .ok_or("Missing crate name for compiler-artifact")?
                    .into();
                let filenames = msg
                    .filenames
                    .ok_or("Missing filenames for compiler-artifact")?
                    .into_iter()
                    .map(|s| Utf8PathBuf::from(s))
                    .collect();
                Ok(CargoMessage::CompilerArtifact(CompilerArtifact {
                    crate_name,
                    filenames,
                }))
            }
            _ => Err("Unknown cargo message reason"),
        }
    }
}

impl CargoMessage {
    /// Parse a Cargo JSON message line into `CargoMessage`.
    ///
    /// This function expects a JSON line as produced by cargo with `-Zunstable-options --message-format=json`.
    /// It uses `facet_json` to parse the line into a `RawCargoMessage`, then converts it using `TryFrom`.
    pub fn parse(json_line: &str) -> Result<Self, &'static str> {
        let raw: RawCargoMessage =
            facet_json::from_str(json_line).map_err(|_| "Failed to parse cargo message JSON")?;
        CargoMessage::try_from(raw)
    }
}
