use crate::model::{GpuInfo, RuntimeBackend};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::env;
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

pub fn runtime_root() -> Result<PathBuf> {
    let root = env::var_os("QBUTT_MEDIA_RUNTIME").context("QBUTT_MEDIA_RUNTIME is unavailable")?;
    Ok(PathBuf::from(root))
}

pub fn detect_gpu() -> Result<Option<GpuInfo>> {
    #[cfg(windows)]
    let nvidia = crate::cuda::detect_gpu()?;
    #[cfg(not(windows))]
    let nvidia: Option<GpuInfo> = None;
    if nvidia.as_ref().is_some_and(|gpu| gpu.supported) {
        return Ok(nvidia);
    }
    let intel = detect_intel_gpu()?;
    Ok(intel.or(nvidia))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsVideoController {
    name: String,
    #[serde(rename = "PNPDeviceID")]
    pnp_device_id: String,
    driver_version: String,
    #[serde(rename = "AdapterRAM")]
    adapter_ram: Option<u64>,
}

#[derive(Debug)]
struct DxgiAdapter {
    index: u32,
    name: String,
    vendor_id: u32,
    device_id: u32,
    dedicated_memory: u64,
    luid: [u8; 8],
}

fn detect_intel_gpu() -> Result<Option<GpuInfo>> {
    let mut command = Command::new("powershell.exe");
    hide_console(&mut command);
    let script = "$ProgressPreference='SilentlyContinue'; @(Get-CimInstance Win32_VideoController | Select-Object Name,PNPDeviceID,DriverVersion,AdapterRAM) | ConvertTo-Json -Compress";
    let output = match command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to inspect Windows display adapters"),
    };
    if !output.status.success() {
        return Ok(None);
    }
    parse_intel_gpu_json(&output.stdout, &dxgi_adapters()?)
}

fn parse_intel_gpu_json(bytes: &[u8], adapters: &[DxgiAdapter]) -> Result<Option<GpuInfo>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .context("PowerShell returned invalid display-adapter JSON")?;
    let rows = if value.is_array() {
        serde_json::from_value::<Vec<WindowsVideoController>>(value)?
    } else if value.is_object() {
        vec![serde_json::from_value::<WindowsVideoController>(value)?]
    } else {
        vec![]
    };
    let intel = rows
        .into_iter()
        .filter(|gpu| gpu.pnp_device_id.to_ascii_uppercase().contains("VEN_8086"))
        .collect::<Vec<_>>();
    if intel.is_empty() {
        return Ok(None);
    }
    let dxgi_intel = adapters
        .iter()
        .filter(|adapter| adapter.vendor_id == 0x8086)
        .collect::<Vec<_>>();
    let device_count = u32::try_from(dxgi_intel.len()).context("too many Intel GPUs")?;
    let mut candidates = intel
        .into_iter()
        .filter_map(|controller| {
            let device_id = pnp_device_id(&controller.pnp_device_id)?;
            let matching = dxgi_intel
                .iter()
                .copied()
                .filter(|adapter| adapter.device_id == device_id)
                .collect::<Vec<_>>();
            let adapter = match matching.as_slice() {
                [adapter] => Some(*adapter),
                _ => {
                    let expected_name = normalized_gpu_name(&controller.name);
                    let named = matching
                        .into_iter()
                        .filter(|adapter| normalized_gpu_name(&adapter.name) == expected_name)
                        .collect::<Vec<_>>();
                    match named.as_slice() {
                        [adapter] => Some(*adapter),
                        _ => None,
                    }
                }
            }?;
            Some((controller, adapter))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(gpu, adapter)| {
        (
            gpu.adapter_ram.unwrap_or_default(),
            adapter.dedicated_memory,
        )
    });
    let (gpu, adapter) = candidates
        .pop()
        .context("Intel GPU was reported by WMI but no exact DXGI adapter match was found")?;
    Ok(Some(GpuInfo {
        name: gpu.name,
        vendor: "intel".to_owned(),
        backend: RuntimeBackend::Openvino,
        discovery_index: adapter.index,
        device_count,
        gpu_id: gpu.pnp_device_id.trim().to_ascii_uppercase(),
        adapter_luid: Some(canonical_luid(&adapter.luid)?),
        memory_mib: gpu.adapter_ram.unwrap_or_default() / (1024 * 1024),
        capability: "intel-gpu".to_owned(),
        driver_version: gpu.driver_version,
        supported: true,
    }))
}

fn pnp_device_id(value: &str) -> Option<u32> {
    value
        .to_ascii_uppercase()
        .split(['\\', '&'])
        .find_map(|part| u32::from_str_radix(part.strip_prefix("DEV_")?, 16).ok())
}

fn normalized_gpu_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_luid(bytes: &[u8; 8]) -> Result<String> {
    if bytes.iter().all(|byte| *byte == 0) {
        bail!("DXGI returned a zero adapter LUID")
    }
    Ok(format!("luid:{}", hex::encode(bytes)))
}

#[cfg(windows)]
fn dxgi_adapters() -> Result<Vec<DxgiAdapter>> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIFactory1};

    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.context("failed to create a DXGI factory")?;
    let mut adapters = Vec::new();
    let mut index = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(error).context("failed to enumerate DXGI adapters"),
        };
        let description =
            unsafe { adapter.GetDesc1() }.context("failed to read a DXGI adapter description")?;
        let name_length = description
            .Description
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(description.Description.len());
        let mut luid = [0_u8; 8];
        luid[..4].copy_from_slice(&description.AdapterLuid.LowPart.to_le_bytes());
        luid[4..].copy_from_slice(&description.AdapterLuid.HighPart.to_le_bytes());
        adapters.push(DxgiAdapter {
            index,
            name: String::from_utf16_lossy(&description.Description[..name_length]),
            vendor_id: description.VendorId,
            device_id: description.DeviceId,
            dedicated_memory: description.DedicatedVideoMemory as u64,
            luid,
        });
        index += 1;
    }
    Ok(adapters)
}

#[cfg(not(windows))]
fn dxgi_adapters() -> Result<Vec<DxgiAdapter>> {
    Ok(Vec::new())
}
