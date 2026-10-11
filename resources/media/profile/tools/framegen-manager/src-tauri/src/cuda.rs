use crate::archive::sha256;
use crate::model::{GpuInfo, RuntimeBackend};
use anyhow::{Context, Result, bail};
use std::ffi::{CStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};

struct CudaLibrary(HMODULE);

impl CudaLibrary {
    fn symbol(&self, name: &CStr) -> Result<unsafe extern "system" fn() -> isize> {
        // Every caller supplies the CUDA Driver API signature for this symbol.
        let symbol = unsafe { GetProcAddress(self.0, name.as_ptr().cast()) };
        symbol.with_context(|| format!("NVIDIA driver does not export {}", name.to_string_lossy()))
    }

    fn fingerprint(&self, api_version: i32) -> Result<String> {
        let mut path = [0_u16; 32768];
        let length = unsafe { GetModuleFileNameW(self.0, path.as_mut_ptr(), path.len() as u32) };
        if length == 0 || length as usize >= path.len() {
            bail!("could not locate the loaded NVIDIA CUDA driver")
        }
        let path = PathBuf::from(OsString::from_wide(&path[..length as usize]));
        // The CUDA API version alone does not change on every driver update.
        Ok(format!(
            "cuda:{api_version};nvcuda-sha256:{}",
            sha256(&path)?
        ))
    }
}

impl Drop for CudaLibrary {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

fn check(result: i32, operation: &str) -> Result<()> {
    if result != 0 {
        bail!("{operation} failed with CUDA error {result}")
    }
    Ok(())
}

pub fn detect_gpu() -> Result<Option<GpuInfo>> {
    // A driver is optional, but it must never be loaded from a writable app directory.
    let module = unsafe {
        LoadLibraryExW(
            windows_sys::w!("nvcuda.dll"),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(126) {
            return Ok(None);
        }
        return Err(error).context("could not load the NVIDIA CUDA driver");
    }
    let library = CudaLibrary(module);

    let init: unsafe extern "system" fn(u32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuInit")?) };
    let get_count: unsafe extern "system" fn(*mut i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDeviceGetCount")?) };
    let get_device: unsafe extern "system" fn(*mut i32, i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDeviceGet")?) };
    let get_name: unsafe extern "system" fn(*mut u8, i32, i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDeviceGetName")?) };
    let get_uuid: unsafe extern "system" fn(*mut [u8; 16], i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDeviceGetUuid")?) };
    let get_memory: unsafe extern "system" fn(*mut usize, i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDeviceTotalMem_v2")?) };
    let get_attribute: unsafe extern "system" fn(*mut i32, i32, i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDeviceGetAttribute")?) };
    let get_version: unsafe extern "system" fn(*mut i32) -> i32 =
        unsafe { std::mem::transmute(library.symbol(c"cuDriverGetVersion")?) };

    let result = unsafe { init(0) };
    if result == 100 {
        // CUDA_ERROR_NO_DEVICE: the driver exists without an available CUDA GPU.
        return Ok(None);
    }
    check(result, "cuInit")?;

    let mut count = 0;
    check(unsafe { get_count(&mut count) }, "cuDeviceGetCount")?;
    if count == 0 {
        return Ok(None);
    }
    if !(1..=32).contains(&count) {
        bail!("CUDA returned an unsupported GPU count")
    }

    let mut version = 0;
    check(unsafe { get_version(&mut version) }, "cuDriverGetVersion")?;
    let fingerprint = library.fingerprint(version)?;
    let mut gpus = Vec::new();
    for ordinal in 0..count {
        let mut device = 0;
        check(unsafe { get_device(&mut device, ordinal) }, "cuDeviceGet")?;

        let mut name = [0_u8; 256];
        check(
            unsafe { get_name(name.as_mut_ptr(), name.len() as i32, device) },
            "cuDeviceGetName",
        )?;
        let name = CStr::from_bytes_until_nul(&name)
            .context("CUDA returned an unterminated GPU name")?
            .to_str()
            .context("CUDA returned an invalid GPU name")?
            .to_owned();

        let mut uuid = [0_u8; 16];
        check(unsafe { get_uuid(&mut uuid, device) }, "cuDeviceGetUuid")?;
        if uuid.iter().all(|byte| *byte == 0) {
            bail!("CUDA returned a zero GPU UUID")
        }
        let uuid = hex::encode(uuid);
        let gpu_id = format!(
            "GPU-{}-{}-{}-{}-{}",
            &uuid[..8],
            &uuid[8..12],
            &uuid[12..16],
            &uuid[16..20],
            &uuid[20..]
        );

        let mut memory = 0;
        check(
            unsafe { get_memory(&mut memory, device) },
            "cuDeviceTotalMem",
        )?;
        let mut major = 0;
        let mut minor = 0;
        // CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR / _MINOR.
        check(
            unsafe { get_attribute(&mut major, 75, device) },
            "cuDeviceGetAttribute",
        )?;
        check(
            unsafe { get_attribute(&mut minor, 76, device) },
            "cuDeviceGetAttribute",
        )?;
        let capability = format!("{major}.{minor}");

        gpus.push(GpuInfo {
            name,
            vendor: "nvidia".to_owned(),
            backend: RuntimeBackend::Trt,
            discovery_index: ordinal as u32,
            device_count: count as u32,
            gpu_id,
            adapter_luid: None,
            memory_mib: (memory / (1024 * 1024)) as u64,
            supported: matches!(
                capability.as_str(),
                "7.5" | "8.0" | "8.6" | "8.9" | "9.0" | "10.0" | "12.0"
            ),
            capability,
            driver_version: fingerprint.clone(),
        });
    }
    gpus.sort_by_key(|gpu| (gpu.supported, gpu.memory_mib));
    Ok(gpus.pop())
}
