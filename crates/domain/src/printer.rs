use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    Online,
    Offline,
    Paused,
    Busy,
    PaperOut,
    Error,
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors independent driver capability flags"
)]
pub struct PrinterCapabilities {
    pub bins: Vec<String>,
    pub collate: bool,
    pub color: bool,
    pub copies: u32,
    pub dpis: Vec<String>,
    pub duplex: bool,
    pub extent: Vec<[u32; 2]>,
    pub medias: Vec<String>,
    pub nup: Vec<u16>,
    pub papers: BTreeMap<String, [Option<u32>; 2]>,
    pub printrate: Option<PrintRate>,
    pub supports_custom_paper_size: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PrintRate {
    pub unit: PrintRateUnit,
    pub rate: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrintRateUnit {
    Ppm,
    Ipm,
    Lmp,
    Cpm,
}
