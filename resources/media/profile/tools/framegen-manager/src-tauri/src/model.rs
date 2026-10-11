use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModelId {
    #[serde(rename = "rife-4.26")]
    Rife426,
}

impl ModelId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rife426 => "rife-4.26",
        }
    }

    pub fn model_filename(&self) -> &'static str {
        match self {
            Self::Rife426 => "rife_v4.26.onnx",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub backend: RuntimeBackend,
    pub discovery_index: u32,
    pub device_count: u32,
    pub gpu_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_luid: Option<String>,
    pub memory_mib: u64,
    pub capability: String,
    pub driver_version: String,
    pub supported: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeBackend {
    Trt,
    Openvino,
}

impl RuntimeBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trt => "trt",
            Self::Openvino => "openvino",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeState {
    NotInstalled,
    Ready,
    Damaged,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeStatus {
    pub state: RuntimeState,
    pub runtime_root: PathBuf,
    pub version_dir: Option<PathBuf>,
    pub selected_models: Vec<ModelId>,
    pub problems: Vec<String>,
    pub orphaned_bytes: u64,
    pub gpu: Option<GpuInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanAction {
    Install,
    Repair,
    Remove,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanComponent {
    pub label: String,
    pub download_bytes: u64,
    pub installed_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallPlan {
    pub action: PlanAction,
    pub selected_models: Vec<ModelId>,
    pub components: Vec<PlanComponent>,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub runtime_root: PathBuf,
    pub gpu: Option<GpuInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OperationResult {
    pub status: RuntimeStatus,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub label: String,
    pub current: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeManifest {
    pub schema_version: u32,
    pub version: u32,
    pub runtime_id: String,
    pub ready: bool,
    pub backend: RuntimeBackend,
    pub runtime_version: String,
    pub version_dir: PathBuf,
    pub vsscript_dir: PathBuf,
    pub python_home: PathBuf,
    pub dll_dirs: Vec<PathBuf>,
    pub plugin_dir: PathBuf,
    pub model_dir: PathBuf,
    pub engine_dir: PathBuf,
    pub engine_namespace: String,
    pub selected_models: Vec<ModelId>,
    pub hardware_resource: String,
    pub device_binding: DeviceBinding,
    pub files: BTreeMap<String, ManifestFile>,
    pub checksums: ChecksumContract,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceBinding {
    pub vendor: String,
    pub discovery_index: u32,
    pub device_ordinal: u32,
    pub gpu_id: String,
    pub capability: String,
    pub driver_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestFile {
    pub size: u64,
    pub sha256: String,
    pub component: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChecksumContract {
    pub algorithm: String,
}
