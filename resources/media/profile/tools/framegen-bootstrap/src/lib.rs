#![cfg_attr(not(windows), allow(dead_code))]

use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString, OsStr, c_char, c_int, c_void};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const REQUEST_PROPERTY: &[u8] = b"user-data/framegen-bootstrap-request\0";
const RESULT_PROPERTY: &[u8] = b"user-data/framegen-bootstrap-result\0";
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const RESTART_REQUIRED_PREFIX: &str = "restart-required:";
#[cfg(windows)]
const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x0000_0100;
#[cfg(windows)]
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

type MpvGetProperty = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int;
type MpvSetPropertyString =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type MpvFreeNodeContents = unsafe extern "C" fn(*mut MpvNode);

#[repr(C)]
union MpvNodeValue {
    string: *mut c_char,
    pointer: *mut c_void,
    integer: i64,
    double: f64,
}

#[repr(C)]
pub struct MpvNode {
    value: MpvNodeValue,
    format: c_int,
}

#[unsafe(no_mangle)]
pub static mut pfn_mpv_get_property: Option<MpvGetProperty> = None;
#[unsafe(no_mangle)]
pub static mut pfn_mpv_set_property_string: Option<MpvSetPropertyString> = None;
#[unsafe(no_mangle)]
pub static mut pfn_mpv_free_node_contents: Option<MpvFreeNodeContents> = None;

#[derive(Clone, Debug, Deserialize)]
struct BootstrapRequest {
    schema_version: u32,
    request_id: String,
    runtime_id: String,
    version_dir: PathBuf,
    vsscript_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
struct RuntimeManifest {
    schema_version: u32,
    version: u32,
    runtime_id: String,
    ready: bool,
    backend: String,
    version_dir: PathBuf,
    vsscript_dir: PathBuf,
    dll_dirs: Vec<PathBuf>,
    plugin_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct BootstrapResult {
    schema_version: u32,
    request_id: String,
    ok: bool,
    runtime_id: String,
    version_dir: String,
    vsscript_path: String,
    dll_dirs: Vec<String>,
    error: String,
    restart_required: bool,
}

#[derive(Debug)]
struct ValidatedLayout {
    runtime_id: String,
    version_dir: PathBuf,
    vsscript_path: PathBuf,
    dll_dirs: Vec<PathBuf>,
}

#[cfg(windows)]
#[derive(Debug)]
struct LoadedRuntime {
    vsscript_path: String,
    dll_dirs: Vec<String>,
    _module: isize,
}

#[cfg(windows)]
static LOADED_RUNTIME: Mutex<Option<LoadedRuntime>> = Mutex::new(None);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn GetModuleFileNameW(module: isize, filename: *mut u16, size: u32) -> u32;
    fn GetModuleHandleW(module_name: *const u16) -> isize;
    fn LoadLibraryExW(filename: *const u16, file: isize, flags: u32) -> isize;
}

fn comparable_path(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = value.strip_prefix(r"\\?\UNC\") {
        value = format!(r"\\{stripped}");
    } else if let Some(stripped) = value.strip_prefix(r"\\?\") {
        value = stripped.to_owned();
    }
    value.trim_end_matches('\\').to_lowercase()
}

fn same_path(left: &Path, right: &Path) -> bool {
    comparable_path(left) == comparable_path(right)
}

fn canonical(path: &Path, kind: &str) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|error| format!("{kind} is unavailable: {error}"))
}

#[cfg(windows)]
fn reject_reparse_path(root: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    let relative = target
        .strip_prefix(root)
        .map_err(|_| "runtime path escaped the managed root".to_owned())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("could not inspect {}: {error}", current.display()))?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!(
                "managed runtime path is a reparse point: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn reject_reparse_path(root: &Path, target: &Path) -> Result<(), String> {
    target
        .strip_prefix(root)
        .map(|_| ())
        .map_err(|_| "runtime path escaped the managed root".to_owned())
}

fn validate_layout(
    root: &Path,
    request: &BootstrapRequest,
    manifest: RuntimeManifest,
) -> Result<ValidatedLayout, String> {
    if request.schema_version != 1
        || request.request_id.is_empty()
        || request.request_id.len() > 1024
        || request.runtime_id.is_empty()
    {
        return Err("invalid bootstrap request".to_owned());
    }
    if manifest.schema_version != 3
        || manifest.version != 3
        || !manifest.ready
        || manifest.runtime_id.is_empty()
        || manifest.runtime_id != request.runtime_id
        || !same_path(&manifest.version_dir, &request.version_dir)
    {
        return Err("bootstrap request does not match the active v3 runtime".to_owned());
    }
    if manifest.backend != "trt" && manifest.backend != "openvino" {
        return Err("unsupported runtime backend".to_owned());
    }

    let root = canonical(root, "runtime root")?;
    let versions = canonical(&root.join("versions"), "versions directory")?;
    let version_dir = canonical(&manifest.version_dir, "version directory")?;
    let version_parent = version_dir
        .parent()
        .ok_or_else(|| "version directory has no parent".to_owned())?;
    if !same_path(version_parent, &versions) {
        return Err("version directory is not a direct managed runtime child".to_owned());
    }

    let expected_vsscript_dir = version_dir.join("python/Lib/site-packages/vapoursynth");
    let vsscript_dir = canonical(&manifest.vsscript_dir, "VSScript directory")?;
    if !same_path(
        &vsscript_dir,
        &canonical(&expected_vsscript_dir, "expected VSScript directory")?,
    ) {
        return Err("VSScript directory does not match the managed layout".to_owned());
    }
    let active_vsscript = canonical(&vsscript_dir.join("vsscript.dll"), "VSScript DLL")?;
    if !active_vsscript.is_file() {
        return Err("VSScript path is not a file".to_owned());
    }
    let vsscript_path = canonical(&request.vsscript_path, "requested VSScript DLL")?;
    if !same_path(&vsscript_path, &active_vsscript) {
        return Err("requested VSScript is not the active qbutt runtime".to_owned());
    }

    let plugin_dir = canonical(&manifest.plugin_dir, "plugin directory")?;
    if !same_path(
        &plugin_dir,
        &canonical(&version_dir.join("plugins"), "expected plugin directory")?,
    ) {
        return Err("plugin directory does not match the managed layout".to_owned());
    }
    if manifest.dll_dirs.len() != 1 {
        return Err("runtime must declare exactly one backend DLL directory".to_owned());
    }
    let expected_dll_dir = plugin_dir.join(match manifest.backend.as_str() {
        "trt" => "vsmlrt-cuda",
        "openvino" => "vsov",
        _ => unreachable!(),
    });
    let dll_dir = canonical(&manifest.dll_dirs[0], "backend DLL directory")?;
    if !same_path(
        &dll_dir,
        &canonical(&expected_dll_dir, "expected backend DLL directory")?,
    ) {
        return Err("backend DLL directory does not match the managed layout".to_owned());
    }

    for path in [
        &versions,
        &version_dir,
        &vsscript_dir,
        &active_vsscript,
        &vsscript_path,
        &plugin_dir,
        &dll_dir,
    ] {
        reject_reparse_path(&root, path)?;
    }

    Ok(ValidatedLayout {
        runtime_id: manifest.runtime_id,
        version_dir,
        vsscript_path,
        dll_dirs: vec![dll_dir],
    })
}

fn read_fixed_layout(request: &BootstrapRequest) -> Result<ValidatedLayout, String> {
    let runtime_root = std::env::var_os("QBUTT_MEDIA_RUNTIME")
        .ok_or_else(|| "QBUTT_MEDIA_RUNTIME is unavailable".to_owned())?;
    let root = PathBuf::from(runtime_root);
    let manifest_path = root.join("runtime-manifest.json");
    let metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|error| format!("runtime manifest is unavailable: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return Err("runtime manifest is not a bounded regular file".to_owned());
    }
    reject_reparse_path(&root, &manifest_path)?;
    let bytes = fs::read(&manifest_path)
        .map_err(|error| format!("could not read runtime manifest: {error}"))?;
    let manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not parse runtime manifest: {error}"))?;
    validate_layout(&root, request, manifest)
}

#[cfg(windows)]
fn wide(value: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain([0]).collect()
}

#[cfg(windows)]
fn loaded_module_path(module: isize) -> Result<PathBuf, String> {
    let mut capacity = 512usize;
    loop {
        let mut buffer = vec![0_u16; capacity];
        let length = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), capacity as u32) };
        if length == 0 {
            return Err(format!("GetModuleFileNameW failed: {}", unsafe {
                GetLastError()
            }));
        }
        if length as usize + 1 < capacity {
            buffer.truncate(length as usize);
            return Ok(PathBuf::from(String::from_utf16_lossy(&buffer)));
        }
        capacity *= 2;
        if capacity > 32 * 1024 {
            return Err("loaded module path is too long".to_owned());
        }
    }
}

#[cfg(windows)]
fn load_runtime(layout: &ValidatedLayout) -> Result<(), String> {
    let requested_vsscript = comparable_path(&layout.vsscript_path);
    let requested_dll_dirs = layout
        .dll_dirs
        .iter()
        .map(|path| comparable_path(path))
        .collect::<Vec<_>>();
    let mut state = LOADED_RUNTIME
        .lock()
        .map_err(|_| "bootstrap state lock is poisoned".to_owned())?;
    if let Some(loaded) = state.as_ref() {
        if loaded.vsscript_path == requested_vsscript && loaded.dll_dirs == requested_dll_dirs {
            return Ok(());
        }
        return Err(format!(
            "{RESTART_REQUIRED_PREFIX} another VapourSynth runtime is already loaded; reopen mpv once after the runtime update"
        ));
    }

    let module_name = wide(OsStr::new("vsscript.dll"));
    let existing = unsafe { GetModuleHandleW(module_name.as_ptr()) };
    if existing != 0 {
        let existing_path = canonical(&loaded_module_path(existing)?, "loaded VSScript DLL")?;
        if !same_path(&existing_path, &layout.vsscript_path) {
            return Err(format!(
                "{RESTART_REQUIRED_PREFIX} mpv already loaded a different VSScript runtime; reopen it once"
            ));
        }
    }

    let module = if existing != 0 {
        existing
    } else {
        let path = wide(layout.vsscript_path.as_os_str());
        let handle = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                0,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if handle == 0 {
            return Err(format!("LoadLibraryExW(vsscript.dll) failed: {}", unsafe {
                GetLastError()
            }));
        }
        handle
    };

    *state = Some(LoadedRuntime {
        vsscript_path: requested_vsscript,
        dll_dirs: requested_dll_dirs,
        _module: module,
    });
    Ok(())
}

#[cfg(not(windows))]
fn load_runtime(_layout: &ValidatedLayout) -> Result<(), String> {
    Err("framegen bootstrap supports Windows only".to_owned())
}

unsafe fn get_request(handle: *mut c_void) -> Result<BootstrapRequest, String> {
    const MPV_FORMAT_STRING: c_int = 1;
    const MPV_FORMAT_NODE: c_int = 6;
    let get = unsafe { pfn_mpv_get_property }
        .ok_or_else(|| "mpv_get_property is unavailable".to_owned())?;
    let free = unsafe { pfn_mpv_free_node_contents }
        .ok_or_else(|| "mpv_free_node_contents is unavailable".to_owned())?;
    let mut node = MpvNode {
        value: MpvNodeValue {
            pointer: std::ptr::null_mut(),
        },
        format: 0,
    };
    let status = unsafe {
        get(
            handle,
            REQUEST_PROPERTY.as_ptr().cast(),
            MPV_FORMAT_NODE,
            (&raw mut node).cast(),
        )
    };
    if status < 0 {
        return Err(format!(
            "bootstrap request property is unavailable: {status}"
        ));
    }
    if node.format != MPV_FORMAT_STRING {
        unsafe { free(&raw mut node) };
        return Err("bootstrap request property is not a string".to_owned());
    }
    let pointer = unsafe { node.value.string };
    if pointer.is_null() {
        unsafe { free(&raw mut node) };
        return Err("bootstrap request property is empty".to_owned());
    }
    let bytes = unsafe { CStr::from_ptr(pointer) }.to_bytes().to_vec();
    unsafe { free(&raw mut node) };
    if bytes.len() > MAX_REQUEST_BYTES {
        Err("bootstrap request is too large".to_owned())
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid bootstrap request: {error}"))
    }
}

unsafe fn publish_result(handle: *mut c_void, result: &BootstrapResult) -> Result<(), String> {
    let set = unsafe { pfn_mpv_set_property_string }
        .ok_or_else(|| "mpv_set_property_string is unavailable".to_owned())?;
    let value = serde_json::to_string(result)
        .map_err(|error| format!("could not serialize bootstrap result: {error}"))?;
    let value = CString::new(value).map_err(|_| "bootstrap result contains NUL".to_owned())?;
    let status = unsafe { set(handle, RESULT_PROPERTY.as_ptr().cast(), value.as_ptr()) };
    if status < 0 {
        return Err(format!("mpv rejected bootstrap result property: {status}"));
    }
    Ok(())
}

#[unsafe(no_mangle)]
/// mpv C-plugin entry point.
///
/// # Safety
///
/// `handle` must be the live client handle supplied by mpv. The host must
/// initialize the exported `pfn_mpv_*` slots before calling this function.
pub unsafe extern "C" fn mpv_open_cplugin(handle: *mut c_void) -> c_int {
    let request = match unsafe { get_request(handle) } {
        Ok(request) => request,
        Err(error) => {
            let result = BootstrapResult {
                schema_version: 1,
                request_id: String::new(),
                ok: false,
                runtime_id: String::new(),
                version_dir: String::new(),
                vsscript_path: String::new(),
                dll_dirs: Vec::new(),
                error,
                restart_required: false,
            };
            return if unsafe { publish_result(handle, &result) }.is_ok() {
                0
            } else {
                -1
            };
        }
    };

    let request_id = request.request_id.clone();
    let outcome = read_fixed_layout(&request).and_then(|layout| {
        load_runtime(&layout)?;
        Ok(layout)
    });
    let (outcome, restart_required) = match outcome {
        Err(error) if error.starts_with(RESTART_REQUIRED_PREFIX) => (
            Err(error[RESTART_REQUIRED_PREFIX.len()..].trim().to_owned()),
            true,
        ),
        outcome => (outcome, false),
    };
    let result = match outcome {
        Ok(layout) => BootstrapResult {
            schema_version: 1,
            request_id,
            ok: true,
            runtime_id: layout.runtime_id,
            version_dir: layout.version_dir.to_string_lossy().into_owned(),
            vsscript_path: layout.vsscript_path.to_string_lossy().into_owned(),
            dll_dirs: layout
                .dll_dirs
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            error: String::new(),
            restart_required: false,
        },
        Err(error) => BootstrapResult {
            schema_version: 1,
            request_id,
            ok: false,
            runtime_id: request.runtime_id,
            version_dir: request.version_dir.to_string_lossy().into_owned(),
            vsscript_path: String::new(),
            dll_dirs: Vec::new(),
            error,
            restart_required,
        },
    };
    if unsafe { publish_result(handle, &result) }.is_ok() {
        0
    } else {
        -1
    }
}
