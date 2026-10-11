use crate::archive::{extract_asset, obtain_asset, sha256};
use crate::catalog::{self, Asset, RUNTIME_VERSION};
use crate::gpu_lock::{GpuLock, InstallerLock};
use crate::model::{
    ChecksumContract, DeviceBinding, InstallPlan, ManifestFile, ModelId, OperationResult,
    PlanAction, PlanComponent, ProgressEvent, RuntimeBackend, RuntimeManifest, RuntimeState,
    RuntimeStatus,
};
use crate::platform;
use crate::runtime_notices;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const MANIFEST_NAME: &str = "runtime-manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 3;
const MANIFEST_VERSION: u32 = 3;
const REMOVAL_MARKER: &str = ".framegen-manager-removal.json";
const GC_DIR: &str = "gc";
const TRT_COMMON_FILES: &[&str] = &[
    "nvinfer_10.dll",
    "nvinfer_builder_resource_ptx_10.dll",
    "nvinfer_plugin_10.dll",
    "nvonnxparser_10.dll",
];
const TRT_HARDWARE_RESOURCES: &[&str] = &["sm75", "sm80", "sm86", "sm89", "sm90", "sm100", "sm120"];
const OPENVINO_FILES: &[&str] = &[
    "cache.json",
    "openvino.dll",
    "openvino_c.dll",
    "openvino_intel_gpu_plugin.dll",
    "openvino_onnx_frontend.dll",
    "tbb12.dll",
];

fn hardware_resource(compute_capability: &str) -> Option<String> {
    let resource = format!("sm{}", compute_capability.replace('.', ""));
    TRT_HARDWARE_RESOURCES
        .contains(&resource.as_str())
        .then_some(resource)
}

fn trt_files(resource: &str) -> Vec<String> {
    TRT_COMMON_FILES
        .iter()
        .map(|name| (*name).to_owned())
        .chain(std::iter::once(format!(
            "nvinfer_builder_resource_{resource}_10.dll"
        )))
        .collect()
}

fn is_owned_trt_file(name: &str) -> bool {
    TRT_COMMON_FILES.contains(&name)
        || TRT_HARDWARE_RESOURCES
            .iter()
            .any(|resource| name == format!("nvinfer_builder_resource_{resource}_10.dll"))
}
const PYTHON_FILES: &[&str] = &[
    "python.exe",
    "pythonw.exe",
    "python313.dll",
    "python3.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "LICENSE.txt",
    "pyexpat.pyd",
    "select.pyd",
    "unicodedata.pyd",
    "winsound.pyd",
    "_asyncio.pyd",
    "_bz2.pyd",
    "_ctypes.pyd",
    "_decimal.pyd",
    "_elementtree.pyd",
    "_hashlib.pyd",
    "_lzma.pyd",
    "_multiprocessing.pyd",
    "_overlapped.pyd",
    "_queue.pyd",
    "_socket.pyd",
    "_sqlite3.pyd",
    "_ssl.pyd",
    "_uuid.pyd",
    "_wmi.pyd",
    "_zoneinfo.pyd",
    "libcrypto-3.dll",
    "libffi-8.dll",
    "libssl-3.dll",
    "sqlite3.dll",
    "python313.zip",
    "python313._pth",
    "sitecustomize.py",
    "__pycache__/sitecustomize.cpython-313.pyc",
    "python.cat",
];
const VAPOURSYNTH_FILES: &[&str] = &[
    "vapoursynth-79.dist-info/METADATA",
    "vapoursynth-79.dist-info/WHEEL",
    "vapoursynth-79.dist-info/entry_points.txt",
    "vapoursynth-79.dist-info/licenses/COPYING.LESSER",
    "vapoursynth-79.dist-info/RECORD",
    "vapoursynth/libvapoursynth.dll",
    "vapoursynth/libvapoursynthfilters.dll",
    "vapoursynth/libvapoursynthfilters_avx2.dll",
    "vapoursynth/libvapoursynthfilters_zn4.dll",
    "vapoursynth/vapoursynth.pyd",
    "vapoursynth/vsscript.dll",
    "vapoursynth/__init__.py",
    "vapoursynth/__main__.py",
    "vapoursynth/_cli.py",
    "vapoursynth/_shell.py",
    "vapoursynth/_utils.py",
];

struct SafeRuntime {
    manifest: RuntimeManifest,
    canonical_version: PathBuf,
}

struct EngineNamespaceActivation {
    final_path: PathBuf,
    created: bool,
    committed: bool,
}

struct VersionActivation {
    path: PathBuf,
    committed: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct ResolvedDevice {
    ordinal: u32,
    gpu_id: String,
}

impl VersionActivation {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(mut self) {
        self.committed = true;
    }

    fn rollback(&mut self) -> Result<()> {
        if self.path.is_dir() {
            validate_managed_tree(&self.path)?;
            fs::remove_dir_all(&self.path)?;
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for VersionActivation {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.rollback();
        }
    }
}

impl EngineNamespaceActivation {
    fn commit(mut self) {
        self.committed = true;
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.created {
            self.committed = true;
            return Ok(());
        }
        if self.final_path.exists() {
            validate_managed_tree(&self.final_path)?;
            fs::remove_dir_all(&self.final_path)?;
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for EngineNamespaceActivation {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.rollback();
        }
    }
}

pub fn status() -> Result<RuntimeStatus> {
    let root = platform::runtime_root()?;
    let manifest_path = root.join(MANIFEST_NAME);
    let gpu = platform::detect_gpu()?;
    if root.exists()
        && let Err(error) = validate_runtime_root(&root)
    {
        return Ok(RuntimeStatus {
            state: RuntimeState::Damaged,
            runtime_root: root,
            version_dir: None,
            selected_models: vec![],
            problems: vec![format!("Небезопасный runtime: {error:#}")],
            orphaned_bytes: 0,
            gpu,
        });
    }
    if !manifest_path.exists() {
        let orphaned_bytes = managed_size(&root).unwrap_or_default();
        return Ok(RuntimeStatus {
            state: RuntimeState::NotInstalled,
            runtime_root: root,
            version_dir: None,
            selected_models: vec![],
            problems: (orphaned_bytes > 0)
                .then(|| {
                    "Есть файлы незавершенной установки; их можно удалить или продолжить установку"
                        .to_owned()
                })
                .into_iter()
                .collect(),
            orphaned_bytes,
            gpu,
        });
    }

    let manifest = match read_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            return Ok(RuntimeStatus {
                state: RuntimeState::Damaged,
                runtime_root: root,
                version_dir: None,
                selected_models: vec![],
                problems: vec![format!("Manifest поврежден: {error}")],
                orphaned_bytes: 0,
                gpu,
            });
        }
    };
    let mut problems = validate_manifest(&root, &manifest)
        .unwrap_or_else(|error| vec![format!("Runtime не удалось проверить: {error:#}")]);
    let damaged = !problems.is_empty();
    if damaged {
        return Ok(RuntimeStatus {
            state: RuntimeState::Damaged,
            runtime_root: root,
            version_dir: Some(manifest.version_dir),
            selected_models: vec![ModelId::Rife426],
            problems,
            orphaned_bytes: 0,
            gpu,
        });
    }
    let binding_matches = gpu.as_ref().is_some_and(|detected| {
        static_binding_matches(&manifest, detected)
            && prepare_backend_environment(detected.backend).is_ok()
            && resolve_device(&manifest, detected)
                .is_ok_and(|resolved| resolved_binding_matches(&manifest, &resolved))
    });
    if !binding_matches {
        problems.push(
            "Runtime связан с другой GPU, порядком устройств или версией драйвера; обновите runtime"
                .to_owned(),
        );
    }
    Ok(RuntimeStatus {
        state: RuntimeState::Ready,
        runtime_root: root,
        version_dir: Some(manifest.version_dir),
        selected_models: manifest.selected_models,
        problems,
        orphaned_bytes: 0,
        gpu,
    })
}

pub fn plan(models: Vec<ModelId>, action: PlanAction) -> Result<InstallPlan> {
    if matches!(action, PlanAction::Remove) {
        let current = status()?;
        let installed_bytes = managed_size(&current.runtime_root)?;
        return Ok(InstallPlan {
            action,
            selected_models: current.selected_models,
            components: vec![PlanComponent {
                label: "Frame generation runtime и локальные настройки".to_owned(),
                download_bytes: 0,
                installed_bytes,
            }],
            download_bytes: 0,
            installed_bytes,
            runtime_root: current.runtime_root,
            gpu: current.gpu,
        });
    }
    validate_models(&models)?;
    let root = platform::runtime_root()?;
    let gpu = platform::detect_gpu()?;
    let backend = gpu
        .as_ref()
        .map(|gpu| gpu.backend)
        .unwrap_or(RuntimeBackend::Trt);
    let assets = selected_assets(&models, backend);
    let required_downloads = match action {
        PlanAction::Install => {
            let reusable_manifest = read_manifest(&root.join(MANIFEST_NAME))
                .and_then(|manifest| validate_manifest_structure(&root, manifest))
                .ok()
                .filter(|runtime| runtime.manifest.backend == backend)
                .map(|runtime| runtime.manifest);
            download_assets(&models, backend, reusable_manifest.as_ref())
        }
        PlanAction::Repair => {
            let manifest_path = root.join(MANIFEST_NAME);
            if manifest_path.is_file() {
                read_manifest(&manifest_path)
                    .and_then(|manifest| validate_manifest_structure(&root, manifest))
                    .ok()
                    .filter(|runtime| runtime.manifest.backend == backend)
                    .map(|runtime| repair_assets(&runtime, &models))
                    .unwrap_or_else(|| selected_assets(&models, backend))
            } else {
                selected_assets(&models, backend)
            }
        }
        PlanAction::Remove => unreachable!(),
    };
    let downloads = network_downloads(&root.join("cache"), required_downloads);
    let download_ids = downloads
        .iter()
        .map(|asset| asset.id)
        .collect::<BTreeSet<_>>();
    let components = assets
        .iter()
        .map(|asset| PlanComponent {
            label: asset.label.to_owned(),
            download_bytes: if download_ids.contains(asset.id) {
                asset.download_bytes()
            } else {
                0
            },
            installed_bytes: asset.installed_bytes,
        })
        .collect::<Vec<_>>();
    Ok(InstallPlan {
        action,
        selected_models: models,
        download_bytes: downloads.iter().map(|asset| asset.download_bytes()).sum(),
        installed_bytes: assets.iter().map(|asset| asset.installed_bytes).sum(),
        components,
        runtime_root: root,
        gpu,
    })
}

fn managed_size(root: &Path) -> Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    validate_runtime_root(root)?;
    fn visit(path: &Path) -> Result<u64> {
        let mut total = 0_u64;
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if is_reparse_point(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                total = total.saturating_add(visit(&path)?);
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
        Ok(total)
    }
    visit(root)
}

pub fn install(models: Vec<ModelId>) -> Result<OperationResult> {
    install_with_progress(models, |_| {})
}

pub fn repair(models: Vec<ModelId>) -> Result<OperationResult> {
    repair_with_progress(models, |_| {})
}

pub fn install_with_progress<F>(models: Vec<ModelId>, progress: F) -> Result<OperationResult>
where
    F: FnMut(ProgressEvent),
{
    install_or_repair(models, false, progress)
}

pub fn repair_with_progress<F>(models: Vec<ModelId>, progress: F) -> Result<OperationResult>
where
    F: FnMut(ProgressEvent),
{
    install_or_repair(models, true, progress)
}

fn install_or_repair<F>(
    models: Vec<ModelId>,
    repair: bool,
    mut progress: F,
) -> Result<OperationResult>
where
    F: FnMut(ProgressEvent),
{
    validate_models(&models)?;
    let root = platform::runtime_root()?;
    let _lock = InstallerLock::acquire()?;
    validate_runtime_root(&root)?;
    let _ = cleanup_pending_gc(&root);
    if !repair && root.join(MANIFEST_NAME).is_file() {
        let manifest_path = root.join(MANIFEST_NAME);
        if let Some(manifest) = read_current_manifest(&manifest_path)?
            && validate_manifest(&root, &manifest).is_ok_and(|problems| problems.is_empty())
            && manifest.selected_models == models
        {
            let gpu = platform::detect_gpu()?.context("Совместимый GPU не найден")?;
            let deep_ready = if static_binding_matches(&manifest, &gpu)
                && prepare_backend_environment(gpu.backend).is_ok()
            {
                let _gpu_lock = GpuLock::acquire()?;
                resolve_device(&manifest, &gpu).is_ok_and(|resolved| {
                    if !resolved_binding_matches(&manifest, &resolved) {
                        return false;
                    }
                    let mut smoke_manifest = manifest.clone();
                    smoke_manifest.device_binding.device_ordinal = resolved.ordinal;
                    smoke_runtime(&smoke_manifest, gpu.memory_mib).is_ok()
                })
            } else {
                false
            };
            if deep_ready {
                let mut status = status()?;
                let deferred_cleanup = cleanup_inactive_versions(&root, &manifest.version_dir)
                    .and_then(|_| cleanup_unused_engines(&root, &manifest.engine_dir))
                    .err()
                    .map(|error| format!("Отложенная очистка не завершена: {error}"));
                if let Some(warning) = deferred_cleanup {
                    status.problems.push(warning);
                }
                return Ok(OperationResult {
                    status,
                    message: "Runtime уже настроен и готов.".to_owned(),
                });
            }
        }
    }
    let gpu = platform::detect_gpu()?.context("Совместимый GPU не найден")?;
    if !gpu.supported {
        bail!(
            "GPU {} is not supported by the frame generation runtime",
            gpu.name
        )
    }
    let hardware_resource = match gpu.backend {
        RuntimeBackend::Trt => hardware_resource(&gpu.capability)
            .context("TensorRT has no builder resource for this GPU")?,
        RuntimeBackend::Openvino => "intel-gpu".to_owned(),
    };
    prepare_backend_environment(gpu.backend)?;

    let cache = root.join("cache");
    let versions = root.join("versions");
    fs::create_dir_all(&versions)?;
    let staging_parent = root.join("staging");
    fs::create_dir_all(&staging_parent)?;
    let staging = TempDir::new_in(&staging_parent)?;
    let layout = staging.path().join("runtime");
    fs::create_dir_all(&layout)?;

    let current_manifest_path = root.join(MANIFEST_NAME);
    let reusable = if current_manifest_path.is_file() {
        let safe = read_current_manifest(&current_manifest_path)?
            .and_then(|manifest| validate_manifest_structure(&root, manifest).ok());
        let backend_changed = safe
            .as_ref()
            .is_some_and(|runtime| runtime.manifest.backend != gpu.backend);
        if backend_changed { None } else { safe }
    } else {
        None
    };
    let needs_repair = reusable
        .as_ref()
        .map(validate_safe_runtime)
        .transpose()?
        .is_none_or(|problems| !problems.is_empty());
    if let Some(manifest) = &reusable {
        reuse_runtime(manifest, &layout, &models, gpu.backend)?;
    }
    let assets = if repair || needs_repair {
        reusable
            .as_ref()
            .map(|runtime| repair_assets(runtime, &models))
            .unwrap_or_else(|| selected_assets(&models, gpu.backend))
    } else {
        download_assets(
            &models,
            gpu.backend,
            reusable.as_ref().map(|runtime| &runtime.manifest),
        )
    };
    let repository = bundled_repository_root()?;
    let total_download = assets.iter().map(|asset| asset.download_bytes()).sum();
    let mut completed_download = 0_u64;
    let mut sources = Vec::with_capacity(assets.len());
    for asset in &assets {
        let start = completed_download;
        progress(ProgressEvent {
            phase: "download".to_owned(),
            label: format!("Проверяем или скачиваем {}", asset.label),
            current: start,
            total: total_download,
        });
        let archive = obtain_asset(*asset, &cache, &repository, |current, _| {
            progress(ProgressEvent {
                phase: "download".to_owned(),
                label: asset.label.to_owned(),
                current: start.saturating_add(current),
                total: total_download,
            });
        })?;
        completed_download = completed_download.saturating_add(asset.download_bytes());
        sources.push(archive);
    }
    for (index, (asset, source)) in assets.iter().zip(&sources).enumerate() {
        progress(ProgressEvent {
            phase: "extract".to_owned(),
            label: asset.label.to_owned(),
            current: index as u64,
            total: assets.len() as u64,
        });
        remove_component_targets(*asset, &layout)?;
        extract_component(*asset, source, &layout, &hardware_resource)
            .with_context(|| format!("failed to extract {}", asset.label))?;
    }
    progress(ProgressEvent {
        phase: "extract".to_owned(),
        label: "Компоненты распакованы".to_owned(),
        current: assets.len() as u64,
        total: assets.len() as u64,
    });
    runtime_notices::write(&layout, gpu.backend)?;
    configure_embedded_python(&layout)?;
    let mut manifest = build_manifest(&root, &layout, &models, &gpu)?;
    progress(ProgressEvent {
        phase: "verify".to_owned(),
        label: format!("Проверяем VapourSynth и {} на GPU", gpu.backend.as_str()),
        current: 0,
        total: 1,
    });
    let mut engine_activation = {
        let _gpu_lock = GpuLock::acquire()?;
        let resolved = resolve_device(&manifest, &gpu)?;
        manifest.device_binding.device_ordinal = resolved.ordinal;
        manifest.device_binding.gpu_id = resolved.gpu_id;
        prepare_engine_namespace(&root, &mut manifest, gpu.memory_mib)?
    };
    manifest.files = collect_manifest_files(&layout)?;
    progress(ProgressEvent {
        phase: "verify".to_owned(),
        label: "Runtime проверен".to_owned(),
        current: 1,
        total: 1,
    });

    let version_suffix = models
        .iter()
        .map(ModelId::as_str)
        .collect::<Vec<_>>()
        .join("+");
    let build_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let destination = versions.join(format!("{RUNTIME_VERSION}-{version_suffix}-{build_id}"));
    let root_manifest_path = root.join(MANIFEST_NAME);
    let previous_root_manifest = read_optional_normal_file(&root_manifest_path)?;
    fs::rename(&layout, &destination)?;
    let mut version_activation = VersionActivation::new(destination.clone());
    let final_manifest = rebase_manifest(manifest, &destination);
    let activation = (|| -> Result<()> {
        write_json_atomic(&destination.join(MANIFEST_NAME), &final_manifest)?;
        write_json_atomic(&root_manifest_path, &final_manifest)?;
        Ok(())
    })();
    if let Err(error) = activation {
        restore_json(&root_manifest_path, previous_root_manifest.as_deref())?;
        engine_activation.rollback()?;
        version_activation.rollback()?;
        return Err(error);
    }
    engine_activation.commit();
    version_activation.commit();
    let cleanup_warning = cleanup_inactive_versions(&root, &destination)
        .and_then(|_| cleanup_unused_engines(&root, &final_manifest.engine_dir))
        .and_then(|_| cleanup_staging(&staging_parent))
        .and_then(|_| cleanup_cache(&cache))
        .err()
        .map(|error| format!("Runtime активирован, но временные файлы пока не удалены: {error}"));
    progress(ProgressEvent {
        phase: "complete".to_owned(),
        label: "Runtime готов".to_owned(),
        current: 1,
        total: 1,
    });
    let mut status = status().unwrap_or_else(|error| RuntimeStatus {
        state: RuntimeState::Ready,
        runtime_root: root.clone(),
        version_dir: Some(destination.clone()),
        selected_models: vec![ModelId::Rife426],
        problems: vec![format!(
            "Runtime активирован; проверка статуса не завершена: {error:#}"
        )],
        orphaned_bytes: 0,
        gpu: Some(gpu.clone()),
    });
    if let Some(warning) = cleanup_warning {
        status.problems.push(warning);
    }
    Ok(OperationResult {
        status,
        message: if repair {
            "Runtime восстановлен и готов."
        } else {
            "Runtime установлен и готов."
        }
        .to_owned(),
    })
}

fn remove_component_targets(asset: Asset, root: &Path) -> Result<()> {
    let targets = match asset.id {
        "python" => vec![root.join("python")],
        "vapoursynth" => vec![
            root.join("python/Lib/site-packages/vapoursynth"),
            root.join("python/Lib/site-packages/vapoursynth-79.dist-info"),
        ],
        "vsmlrt-script" => vec![root.join("plugins/vsmlrt.py")],
        "vstrt" => vec![
            root.join("plugins/vstrt.dll"),
            root.join("plugins/vsmlrt-cuda/trtexec.exe"),
        ],
        "vsov" => vec![root.join("plugins/vsov.dll")],
        "openvino-runtime" => vec![root.join("plugins/vsov")],
        "tensorrt-libs" => TRT_COMMON_FILES
            .iter()
            .map(|name| root.join("plugins/vsmlrt-cuda").join(name))
            .chain(TRT_HARDWARE_RESOURCES.iter().map(|resource| {
                root.join("plugins/vsmlrt-cuda")
                    .join(format!("nvinfer_builder_resource_{resource}_10.dll"))
            }))
            .collect(),
        "rife-4.26-trt" | "rife-4.26-openvino-s0p5" => {
            vec![root.join("models/rife_v2/rife_v4.26.onnx")]
        }
        _ => bail!("unknown component {}", asset.id),
    };
    for target in targets {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if is_reparse_point(&metadata) => {
                bail!("reparse point at repair target: {}", target.display())
            }
            Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&target)?,
            Ok(_) => fs::remove_file(&target)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn remove() -> Result<OperationResult> {
    let root = platform::runtime_root()?;
    if !root.ends_with(Path::new("media").join("framegen")) {
        bail!("refusing to remove an unexpected runtime root")
    }
    let _lock = InstallerLock::acquire()?;
    if !root.exists() {
        if let Some(parent) = root.parent()
            && parent.is_dir()
        {
            cleanup_removal_quarantines(parent)?;
        }
        return Ok(OperationResult {
            status: status()?,
            message: "Runtime уже удален.".to_owned(),
        });
    }
    if player_is_running()? {
        bail!("Закройте mpv перед удалением runtime; плеер не будет завершен автоматически")
    }
    validate_runtime_root(&root)?;
    validate_managed_tree(&root)?;
    let parent = root.parent().context("runtime root has no parent")?;
    let quarantine = parent.join(format!(
        ".framegen-removing-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    ));
    write_json_atomic(
        &root.join(REMOVAL_MARKER),
        &serde_json::json!({
            "owner": "qbutt-framegen-manager",
            "version": 1,
            "phase": "preparing"
        }),
    )?;
    if let Err(error) = fs::rename(&root, &quarantine) {
        let _ = fs::remove_file(root.join(REMOVAL_MARKER));
        return Err(error)
            .context("runtime is in use and cannot be removed safely; close mpv and retry");
    }
    if let Err(error) = write_json_atomic(
        &quarantine.join(REMOVAL_MARKER),
        &serde_json::json!({
            "owner": "qbutt-framegen-manager",
            "version": 1,
            "phase": "committed"
        }),
    ) {
        fs::rename(&quarantine, &root)
            .context("removal marker failed and runtime rollback also failed")?;
        let _ = fs::remove_file(root.join(REMOVAL_MARKER));
        return Err(error);
    }
    if let Err(error) = fs::remove_dir_all(&quarantine) {
        return Ok(OperationResult {
            status: status()?,
            message: format!("Runtime отключен. Остатки будут удалены после закрытия mpv: {error}"),
        });
    }
    Ok(OperationResult {
        status: status()?,
        message: "Frame generation runtime удален.".to_owned(),
    })
}

fn cleanup_removal_quarantines(parent: &Path) -> Result<()> {
    if player_is_running()? {
        return Ok(());
    }
    if !parent.is_dir() || is_reparse_point(&fs::symlink_metadata(parent)?) {
        bail!("framegen data parent is not a normal directory")
    }
    for entry in fs::read_dir(parent)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(".framegen-removing-") {
            continue;
        }
        let marker = path.join(REMOVAL_MARKER);
        reject_reparse_file(&marker)?;
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&marker)?)?;
        if !is_committed_removal_marker(&value) {
            continue;
        }
        if player_is_running()? {
            return Ok(());
        }
        validate_managed_tree(&path)?;
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn is_committed_removal_marker(value: &serde_json::Value) -> bool {
    value.get("owner").and_then(|value| value.as_str()) == Some("qbutt-framegen-manager")
        && value.get("version").and_then(|value| value.as_u64()) == Some(1)
        && value.get("phase").and_then(|value| value.as_str()) == Some("committed")
}

fn validate_models(models: &[ModelId]) -> Result<()> {
    if models.is_empty() {
        bail!("select at least one interpolation model")
    }
    let unique: BTreeSet<_> = models.iter().map(ModelId::as_str).collect();
    if unique.len() != models.len() {
        bail!("model selection contains duplicates")
    }
    Ok(())
}

fn selected_assets(models: &[ModelId], backend: RuntimeBackend) -> Vec<Asset> {
    let mut assets = catalog::core_assets(backend);
    assets.extend(
        models
            .iter()
            .map(|model| catalog::model_asset(model, backend)),
    );
    assets
}

fn download_assets(
    models: &[ModelId],
    backend: RuntimeBackend,
    reusable_manifest: Option<&RuntimeManifest>,
) -> Vec<Asset> {
    let reusable_models = reusable_manifest
        .filter(|manifest| manifest.backend == backend)
        .map(|manifest| manifest.selected_models.as_slice());
    let mut assets = if reusable_models.is_some() {
        vec![]
    } else {
        catalog::core_assets(backend)
    };
    assets.extend(
        models
            .iter()
            .filter(|model| reusable_models.is_none_or(|current| !current.contains(model)))
            .map(|model| catalog::model_asset(model, backend)),
    );
    assets
}

fn network_downloads(cache: &Path, assets: Vec<Asset>) -> Vec<Asset> {
    assets
        .into_iter()
        .filter(|asset| {
            matches!(asset.source, catalog::AssetSource::Network { .. })
                && !cached_asset_is_valid(cache, *asset)
        })
        .collect()
}

fn cached_asset_is_valid(cache: &Path, asset: Asset) -> bool {
    let Ok(cache_metadata) = fs::symlink_metadata(cache) else {
        return false;
    };
    if !cache_metadata.is_dir() || is_reparse_point(&cache_metadata) {
        return false;
    }
    let path = cache.join(asset.filename);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    metadata.is_file()
        && !is_reparse_point(&metadata)
        && metadata.len() == asset.source_bytes()
        && sha256(&path).is_ok_and(|actual| actual == asset.sha256)
}

fn bundled_repository_root() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let bin = executable
        .parent()
        .context("framegen-manager executable has no parent directory")?;
    if !bin
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
    {
        bail!("framegen-manager must run from the repository tools/bin directory")
    }
    let tools = bin
        .parent()
        .context("framegen-manager tools directory is missing")?;
    if !tools
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("tools"))
    {
        bail!("framegen-manager must run from the repository tools/bin directory")
    }
    Ok(tools
        .parent()
        .context("framegen-manager repository root is missing")?
        .to_path_buf())
}

fn repair_assets(runtime: &SafeRuntime, models: &[ModelId]) -> Vec<Asset> {
    let manifest = &runtime.manifest;
    let tracked_valid = |prefixes: &[&str], excluded_prefixes: &[&str]| {
        let matching = manifest
            .files
            .iter()
            .filter(|(relative, _)| {
                prefixes.iter().any(|prefix| relative.starts_with(prefix))
                    && !excluded_prefixes
                        .iter()
                        .any(|prefix| relative.starts_with(prefix))
            })
            .collect::<Vec<_>>();
        !matching.is_empty()
            && matching
                .into_iter()
                .all(|(relative, file)| manifest_file_valid(runtime, relative, file))
    };
    let mut assets = vec![];
    let python_broken = !tracked_valid(&["python/"], &["python/Lib/site-packages/"]);
    let vapoursynth_broken = !tracked_valid(
        &[
            "python/Lib/site-packages/vapoursynth/",
            "python/Lib/site-packages/vapoursynth-79.dist-info/",
        ],
        &[],
    );
    if python_broken {
        assets.push(catalog::PYTHON);
        assets.push(catalog::VAPOURSYNTH);
    }
    if vapoursynth_broken && !python_broken {
        assets.push(catalog::VAPOURSYNTH);
    }
    match manifest.backend {
        RuntimeBackend::Trt => {
            if !tracked_valid(&["plugins/vsmlrt.py"], &[]) {
                assets.push(catalog::VSMLRT_SCRIPT);
            }
            if !tracked_valid(
                &["plugins/vstrt.dll", "plugins/vsmlrt-cuda/trtexec.exe"],
                &[],
            ) {
                assets.push(catalog::VSTRT);
            }
            if trt_files(&manifest.hardware_resource).iter().any(|name| {
                let relative = format!("plugins/vsmlrt-cuda/{name}");
                manifest
                    .files
                    .get(&relative)
                    .is_none_or(|file| !manifest_file_valid(runtime, &relative, file))
            }) {
                assets.push(catalog::TRT_LIBS);
            }
        }
        RuntimeBackend::Openvino => {
            if !tracked_valid(&["plugins/vsov.dll"], &[]) {
                assets.push(catalog::VSOV);
            }
            if !tracked_valid(&["plugins/vsov/"], &[]) {
                assets.push(catalog::OPENVINO_RUNTIME);
            }
        }
    }
    assets.extend(models.iter().filter_map(|model| {
        let relative = format!("models/rife_v2/{}", model.model_filename());
        manifest
            .files
            .get(&relative)
            .is_none_or(|file| !manifest_file_valid(runtime, &relative, file))
            .then(|| catalog::model_asset(model, manifest.backend))
    }));
    for required in required_files(manifest) {
        let Ok(relative) = required.strip_prefix(&manifest.version_dir) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let valid = manifest
            .files
            .get(&relative)
            .is_some_and(|file| manifest_file_valid(runtime, &relative, file));
        if valid {
            continue;
        }
        let asset = if relative.starts_with("python/Lib/site-packages/vapoursynth") {
            catalog::VAPOURSYNTH
        } else if relative.starts_with("python/") {
            catalog::PYTHON
        } else if relative == "plugins/vsmlrt.py" {
            catalog::VSMLRT_SCRIPT
        } else if relative == "plugins/vstrt.dll" || relative == "plugins/vsmlrt-cuda/trtexec.exe" {
            catalog::VSTRT
        } else if relative.starts_with("plugins/vsmlrt-cuda/") {
            catalog::TRT_LIBS
        } else if relative == "plugins/vsov.dll" {
            catalog::VSOV
        } else if relative.starts_with("plugins/vsov/") {
            catalog::OPENVINO_RUNTIME
        } else if relative.starts_with("models/") {
            let Some(model) = models
                .iter()
                .find(|model| relative.ends_with(model.model_filename()))
            else {
                continue;
            };
            catalog::model_asset(model, manifest.backend)
        } else if relative == "notices/TensorRT-LICENSE.txt" {
            catalog::TRT_LIBS
        } else {
            continue;
        };
        if !assets.iter().any(|existing| existing.id == asset.id) {
            assets.push(asset);
        }
    }
    assets
}

fn manifest_file_valid(runtime: &SafeRuntime, relative: &str, file: &ManifestFile) -> bool {
    let Ok(path) = safe_manifest_file(runtime, relative) else {
        return false;
    };
    fs::symlink_metadata(&path).is_ok_and(|metadata| {
        metadata.is_file()
            && !is_reparse_point(&metadata)
            && metadata.len() == file.size
            && sha256(&path).is_ok_and(|hash| hash == file.sha256)
    })
}

fn reuse_runtime(
    runtime: &SafeRuntime,
    destination: &Path,
    models: &[ModelId],
    backend: RuntimeBackend,
) -> Result<()> {
    if runtime.manifest.backend != backend {
        bail!("cannot reuse a runtime from a different inference backend")
    }
    for (relative, file) in &runtime.manifest.files {
        if !manifest_file_valid(runtime, relative, file) {
            continue;
        }
        if relative.starts_with("models/")
            && !models
                .iter()
                .any(|model| relative.ends_with(model.model_filename()))
        {
            continue;
        }
        let source = safe_manifest_file(runtime, relative)?;
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &target)
            .with_context(|| format!("failed to reuse {}", source.display()))?;
    }
    Ok(())
}

fn extract_component(
    asset: Asset,
    archive: &Path,
    root: &Path,
    hardware_resource: &str,
) -> Result<()> {
    match asset.id {
        "python" => extract_asset(asset, archive, &root.join("python"), |name| {
            Some(PathBuf::from(name))
        }),
        "vapoursynth" => {
            let wheel = root.join("archives");
            extract_asset(asset, archive, &wheel, |name| {
                (name == "wheel/vapoursynth-79-cp312-abi3-win_amd64.whl")
                    .then(|| PathBuf::from("vapoursynth.whl"))
            })?;
            let wheel_path = wheel.join("vapoursynth.whl");
            let wheel_asset = Asset {
                kind: catalog::ArchiveKind::Wheel,
                ..asset
            };
            extract_asset(
                wheel_asset,
                &wheel_path,
                &root.join("python/Lib/site-packages"),
                |name| {
                    matches!(
                        name,
                        "vapoursynth-79.dist-info/METADATA"
                            | "vapoursynth-79.dist-info/WHEEL"
                            | "vapoursynth-79.dist-info/entry_points.txt"
                            | "vapoursynth-79.dist-info/licenses/COPYING.LESSER"
                            | "vapoursynth-79.dist-info/RECORD"
                            | "vapoursynth/libvapoursynth.dll"
                            | "vapoursynth/libvapoursynthfilters.dll"
                            | "vapoursynth/libvapoursynthfilters_avx2.dll"
                            | "vapoursynth/libvapoursynthfilters_zn4.dll"
                            | "vapoursynth/vapoursynth.pyd"
                            | "vapoursynth/vsscript.dll"
                            | "vapoursynth/__init__.py"
                            | "vapoursynth/__main__.py"
                            | "vapoursynth/_cli.py"
                            | "vapoursynth/_shell.py"
                            | "vapoursynth/_utils.py"
                    )
                    .then(|| PathBuf::from(name))
                },
            )?;
            fs::remove_dir_all(wheel)?;
            Ok(())
        }
        "vsmlrt-script" => extract_asset(asset, archive, &root.join("plugins"), |name| {
            (name == "vsmlrt.py").then(|| PathBuf::from("vsmlrt.py"))
        }),
        "vstrt" => extract_asset(asset, archive, &root.join("plugins"), |name| match name {
            "vstrt.dll" => Some(PathBuf::from("vstrt.dll")),
            "vsmlrt-cuda/trtexec.exe" => Some(PathBuf::from("vsmlrt-cuda/trtexec.exe")),
            _ => None,
        }),
        "vsov" => extract_asset(asset, archive, &root.join("plugins"), |name| {
            (name == asset.filename).then(|| PathBuf::from("vsov.dll"))
        }),
        "openvino-runtime" => extract_asset(asset, archive, &root.join("plugins"), |name| {
            let filename = name.strip_prefix("openvino/libs/")?;
            OPENVINO_FILES
                .contains(&filename)
                .then(|| PathBuf::from("vsov").join(filename))
        }),
        "tensorrt-libs" => extract_asset(asset, archive, root, |name| {
            let filename = name.rsplit('/').next()?;
            if trt_files(hardware_resource)
                .iter()
                .any(|expected| expected == filename)
            {
                Some(PathBuf::from("plugins/vsmlrt-cuda").join(filename))
            } else if name.ends_with(".dist-info/LICENSE.txt") {
                Some(PathBuf::from("notices/TensorRT-LICENSE.txt"))
            } else {
                None
            }
        }),
        "rife-4.26-trt" | "rife-4.26-openvino-s0p5" => {
            let expected = "rife_v4.26.onnx";
            extract_asset(asset, archive, &root.join("models"), |name| {
                (name == expected || name == format!("rife_v2/{expected}"))
                    .then(|| PathBuf::from("rife_v2").join(expected))
            })
        }
        _ => bail!("unknown asset {}", asset.id),
    }
}

fn configure_embedded_python(root: &Path) -> Result<()> {
    let pth = root.join("python/python313._pth");
    let mut contents = fs::read_to_string(&pth)?;
    if !contents.lines().any(|line| line == "Lib\\site-packages") {
        contents.push_str("\nLib\\site-packages\n");
    }
    if let Some(line) = contents.lines().position(|line| line == "#import site") {
        let mut lines = contents.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        lines[line] = "import site".to_owned();
        contents = lines.join("\n");
    }
    fs::write(pth, contents)?;
    fs::write(
        root.join("python/sitecustomize.py"),
        "import sys\nsys.dont_write_bytecode = True\n",
    )?;
    Ok(())
}

fn build_manifest(
    runtime_root: &Path,
    root: &Path,
    models: &[ModelId],
    gpu: &crate::model::GpuInfo,
) -> Result<RuntimeManifest> {
    let python = root.join("python");
    let vapoursynth = python.join("Lib/site-packages/vapoursynth");
    let plugins = root.join("plugins");
    let models_dir = root.join("models");
    let hardware_resource = match gpu.backend {
        RuntimeBackend::Trt => hardware_resource(&gpu.capability)
            .context("TensorRT has no builder resource for this GPU")?,
        RuntimeBackend::Openvino => "intel-gpu".to_owned(),
    };
    let mut manifest = RuntimeManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        version: MANIFEST_VERSION,
        runtime_id: RUNTIME_VERSION.to_owned(),
        ready: true,
        backend: gpu.backend,
        runtime_version: RUNTIME_VERSION.to_owned(),
        version_dir: root.to_path_buf(),
        vsscript_dir: vapoursynth,
        python_home: python,
        dll_dirs: vec![plugins.join(match gpu.backend {
            RuntimeBackend::Trt => "vsmlrt-cuda",
            RuntimeBackend::Openvino => "vsov",
        })],
        plugin_dir: plugins,
        model_dir: models_dir,
        engine_dir: runtime_root.join("engines/pending"),
        engine_namespace: String::new(),
        selected_models: models.to_vec(),
        hardware_resource,
        device_binding: DeviceBinding {
            vendor: gpu.vendor.clone(),
            discovery_index: gpu.discovery_index,
            device_ordinal: u32::MAX,
            gpu_id: gpu.gpu_id.clone(),
            capability: gpu.capability.clone(),
            driver_version: gpu.driver_version.clone(),
        },
        files: BTreeMap::new(),
        checksums: ChecksumContract {
            algorithm: "sha256".to_owned(),
        },
    };
    manifest.files = collect_manifest_files(root)?;
    Ok(manifest)
}

fn base_engine_namespace(manifest: &RuntimeManifest) -> String {
    let mut digest = Sha256::new();
    let device_ordinal = manifest.device_binding.device_ordinal.to_string();
    let device_placement = match manifest.backend {
        RuntimeBackend::Trt => device_ordinal.as_str(),
        RuntimeBackend::Openvino => "dynamic-uuid",
    };
    let model = catalog::model_asset(&ModelId::Rife426, manifest.backend);
    let mut values = vec![
        manifest.runtime_id.as_str(),
        manifest.backend.as_str(),
        model.sha256,
        match manifest.backend {
            RuntimeBackend::Trt => catalog::VSTRT.sha256,
            RuntimeBackend::Openvino => catalog::VSOV.sha256,
        },
        manifest.device_binding.gpu_id.as_str(),
        manifest.device_binding.driver_version.as_str(),
        device_placement,
        match manifest.backend {
            RuntimeBackend::Trt => "rife-v4.26-fp16-scale1-opt5-cudagraph-stream2",
            RuntimeBackend::Openvino => "rife-v4.26-s0p5-fullwarp-fp16-static-stream4",
        },
    ];
    match manifest.backend {
        RuntimeBackend::Trt => values.push(catalog::VSMLRT_SCRIPT.sha256),
        RuntimeBackend::Openvino => values.push(catalog::OPENVINO_RUNTIME.sha256),
    }
    for value in values {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

fn engine_namespace_path(root: &Path, manifest: &RuntimeManifest) -> PathBuf {
    root.join("engines").join(&manifest.engine_namespace)
}

fn prepare_engine_namespace(
    root: &Path,
    manifest: &mut RuntimeManifest,
    memory_mib: u64,
) -> Result<EngineNamespaceActivation> {
    let engines = root.join("engines");
    fs::create_dir_all(&engines)?;
    reject_reparse_ancestors(root, &engines)?;
    manifest.engine_namespace = base_engine_namespace(manifest);
    let mut final_path = engine_namespace_path(root, manifest);
    if final_path.exists() {
        validate_managed_tree(&final_path)?;
        let has_files = fs::read_dir(&final_path)?.next().transpose()?.is_some();
        if has_files {
            manifest.engine_dir = final_path.clone();
            if smoke_runtime(manifest, memory_mib).is_ok() {
                return Ok(EngineNamespaceActivation {
                    final_path,
                    created: false,
                    committed: false,
                });
            }
        }
        let mut digest = Sha256::new();
        digest.update(manifest.engine_namespace.as_bytes());
        digest.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
                .to_le_bytes(),
        );
        manifest.engine_namespace = hex::encode(digest.finalize());
        final_path = engine_namespace_path(root, manifest);
    }
    fs::create_dir(&final_path)?;
    manifest.engine_dir = final_path.clone();
    let mut activation = EngineNamespaceActivation {
        final_path,
        created: true,
        committed: false,
    };
    if let Err(error) = smoke_runtime(manifest, memory_mib) {
        activation.rollback()?;
        return Err(error);
    }
    Ok(activation)
}

fn collect_manifest_files(root: &Path) -> Result<BTreeMap<String, ManifestFile>> {
    let mut files = BTreeMap::new();
    for relative in collect_runtime_file_paths(root)? {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        let component = owned_runtime_component(&relative)
            .with_context(|| format!("unexpected file in staged runtime: {relative}"))?;
        files.insert(
            relative,
            ManifestFile {
                size: metadata.len(),
                sha256: sha256(&path)?,
                component: component.to_owned(),
            },
        );
    }
    Ok(files)
}

fn collect_runtime_file_paths(root: &Path) -> Result<BTreeSet<String>> {
    fn visit(root: &Path, current: &Path, files: &mut BTreeSet<String>) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if is_reparse_point(&metadata) {
                bail!("reparse point in staged runtime: {}", path.display())
            }
            if metadata.is_dir() {
                if path != root.join("engines") {
                    visit(root, &path, files)?;
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if relative == MANIFEST_NAME {
                continue;
            }
            files.insert(relative);
        }
        Ok(())
    }

    let mut files = BTreeSet::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn owned_runtime_component(relative: &str) -> Option<&'static str> {
    if relative
        .strip_prefix("python/")
        .is_some_and(|name| PYTHON_FILES.contains(&name))
    {
        return Some("python");
    }
    if relative
        .strip_prefix("python/Lib/site-packages/")
        .is_some_and(|name| VAPOURSYNTH_FILES.contains(&name))
    {
        return Some("python");
    }
    if matches!(
        relative,
        "plugins/vsmlrt.py" | "plugins/vstrt.dll" | "plugins/vsov.dll"
    ) || relative
        .strip_prefix("plugins/vsmlrt-cuda/")
        .is_some_and(|name| name == "trtexec.exe" || is_owned_trt_file(name))
        || relative
            .strip_prefix("plugins/vsov/")
            .is_some_and(|name| OPENVINO_FILES.contains(&name))
    {
        return Some("plugins");
    }
    if relative == "models/rife_v2/rife_v4.26.onnx" {
        return Some("models");
    }
    if relative == "notices/TensorRT-LICENSE.txt"
        || relative
            .strip_prefix("notices/")
            .is_some_and(runtime_notices::contains)
    {
        return Some("notices");
    }
    None
}

fn required_files(manifest: &RuntimeManifest) -> Vec<PathBuf> {
    let mut files = vec![
        manifest.python_home.join("python.exe"),
        manifest.python_home.join("python3.dll"),
        manifest.python_home.join("python313.dll"),
        manifest.python_home.join("python313.zip"),
        manifest.python_home.join("python313._pth"),
        manifest.python_home.join("vcruntime140.dll"),
        manifest.python_home.join("vcruntime140_1.dll"),
        manifest.python_home.join("LICENSE.txt"),
        manifest.vsscript_dir.join("vsscript.dll"),
        manifest.vsscript_dir.join("libvapoursynth.dll"),
        manifest.vsscript_dir.join("libvapoursynthfilters.dll"),
        manifest.vsscript_dir.join("libvapoursynthfilters_avx2.dll"),
        manifest.vsscript_dir.join("libvapoursynthfilters_zn4.dll"),
        manifest.vsscript_dir.join("vapoursynth.pyd"),
        manifest.vsscript_dir.join("__init__.py"),
        manifest.vsscript_dir.join("_utils.py"),
        manifest
            .python_home
            .join("Lib/site-packages/vapoursynth-79.dist-info/licenses/COPYING.LESSER"),
    ];
    files.extend(
        runtime_notices::for_backend(manifest.backend)
            .map(|notice| manifest.version_dir.join("notices").join(notice.filename)),
    );
    match manifest.backend {
        RuntimeBackend::Trt => {
            files.push(manifest.version_dir.join("notices/TensorRT-LICENSE.txt"));
            files.push(manifest.plugin_dir.join("vsmlrt.py"));
            files.push(manifest.plugin_dir.join("vstrt.dll"));
            files.push(manifest.plugin_dir.join("vsmlrt-cuda/trtexec.exe"));
            files.extend(
                trt_files(&manifest.hardware_resource)
                    .iter()
                    .map(|name| manifest.plugin_dir.join("vsmlrt-cuda").join(name)),
            );
        }
        RuntimeBackend::Openvino => {
            files.push(manifest.plugin_dir.join("vsov.dll"));
            files.extend(
                OPENVINO_FILES
                    .iter()
                    .map(|name| manifest.plugin_dir.join("vsov").join(name)),
            );
        }
    }
    files.extend(manifest.selected_models.iter().map(|model| {
        manifest
            .model_dir
            .join("rife_v2")
            .join(model.model_filename())
    }));
    files
}

fn resolve_device(
    manifest: &RuntimeManifest,
    gpu: &crate::model::GpuInfo,
) -> Result<ResolvedDevice> {
    match manifest.backend {
        RuntimeBackend::Trt => Ok(ResolvedDevice {
            ordinal: resolve_cuda_ordinal(manifest, gpu.device_count)?,
            gpu_id: gpu.gpu_id.clone(),
        }),
        RuntimeBackend::Openvino => resolve_openvino_device(
            manifest,
            gpu.adapter_luid
                .as_deref()
                .context("DXGI did not report the Intel adapter LUID")?,
        ),
    }
}

fn resolve_cuda_ordinal(manifest: &RuntimeManifest, device_count: u32) -> Result<u32> {
    if device_count == 0 || device_count > 32 {
        bail!("CUDA returned an unsupported GPU count")
    }
    for path in required_files(manifest) {
        if !path.is_file() {
            bail!("runtime smoke failed: {} is missing", path.display())
        }
    }
    let smoke = cuda_resolver_script(manifest, device_count)?;
    let mut command = Command::new(manifest.python_home.join("python.exe"));
    hide_console(&mut command);
    let output = command
        .env_remove("PYTHONDONTWRITEBYTECODE")
        .env_remove("CUDA_VISIBLE_DEVICES")
        .env(
            "PATH",
            format!(
                "{};{}",
                manifest.dll_dirs[0].display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .arg("-c")
        .arg(smoke)
        .output()?;
    if !output.status.success() {
        bail!(
            "VapourSynth import smoke failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    let stdout = String::from_utf8(output.stdout)?;
    let marker = stdout
        .lines()
        .find_map(|line| line.strip_prefix("__FRAMEGEN_CUDA_ORDINAL__="))
        .context("TensorRT smoke did not report a CUDA ordinal")?;
    marker
        .parse::<u32>()
        .context("TensorRT smoke reported an invalid CUDA ordinal")
}

fn resolve_openvino_device(
    manifest: &RuntimeManifest,
    adapter_luid: &str,
) -> Result<ResolvedDevice> {
    for path in required_files(manifest) {
        if !path.is_file() {
            bail!("runtime smoke failed: {} is missing", path.display())
        }
    }
    let script = openvino_device_script(manifest, adapter_luid)?;
    let output = run_runtime_python(manifest, &script)?;
    if !output.status.success() {
        bail!(
            "OpenVINO device smoke failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    let stdout = String::from_utf8(output.stdout)?;
    let marker = stdout
        .lines()
        .find_map(|line| line.strip_prefix("__FRAMEGEN_OPENVINO_DEVICE__="))
        .context("OpenVINO smoke did not report a matched device")?;
    let device: serde_json::Value =
        serde_json::from_str(marker).context("OpenVINO smoke reported invalid device JSON")?;
    let name = device
        .get("name")
        .and_then(|value| value.as_str())
        .context("OpenVINO smoke omitted the device name")?;
    let ordinal = device
        .get("ordinal")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .context("OpenVINO smoke reported an invalid device ordinal")?;
    let gpu_id = device
        .get("gpu_id")
        .and_then(|value| value.as_str())
        .context("OpenVINO smoke omitted the device UUID")?;
    openvino_uuid_bytes(gpu_id)?;
    if !(name == "GPU" || name.starts_with("GPU.")) || ordinal >= 32 {
        bail!("OpenVINO smoke reported an incompatible GPU device")
    }
    Ok(ResolvedDevice {
        ordinal,
        gpu_id: gpu_id.to_owned(),
    })
}

fn openvino_device_script(manifest: &RuntimeManifest, adapter_luid: &str) -> Result<String> {
    let expected_luid = hex::encode(intel_luid_bytes(adapter_luid)?);
    let library = manifest.plugin_dir.join("vsov/openvino_c.dll");
    Ok(format!(
        r#"import ctypes, json, os
runtime=r'{}'
expected_luid='{}'
_dll_dir=os.add_dll_directory(os.path.dirname(runtime))
lib=ctypes.WinDLL(runtime)
class Devices(ctypes.Structure):
    _fields_=[('devices',ctypes.POINTER(ctypes.c_char_p)),('size',ctypes.c_size_t)]
lib.ov_core_create.argtypes=[ctypes.POINTER(ctypes.c_void_p)]
lib.ov_core_create.restype=ctypes.c_int
lib.ov_core_free.argtypes=[ctypes.c_void_p]
lib.ov_core_free.restype=None
lib.ov_core_get_available_devices.argtypes=[ctypes.c_void_p,ctypes.POINTER(Devices)]
lib.ov_core_get_available_devices.restype=ctypes.c_int
lib.ov_available_devices_free.argtypes=[ctypes.POINTER(Devices)]
lib.ov_available_devices_free.restype=None
lib.ov_core_get_property.argtypes=[ctypes.c_void_p,ctypes.c_char_p,ctypes.c_char_p,ctypes.POINTER(ctypes.c_void_p)]
lib.ov_core_get_property.restype=ctypes.c_int
lib.ov_free.argtypes=[ctypes.c_void_p]
lib.ov_free.restype=None
lib.ov_get_last_err_msg.argtypes=[]
lib.ov_get_last_err_msg.restype=ctypes.c_char_p
def check(status):
    if status:
        message=lib.ov_get_last_err_msg()
        raise RuntimeError(message.decode(errors='replace') if message else f'OpenVINO status {{status}}')
def prop(core,name,key):
    value=ctypes.c_void_p()
    check(lib.ov_core_get_property(core,name.encode(),key.encode(),ctypes.byref(value)))
    try:
        return ctypes.string_at(value).decode(errors='replace')
    finally:
        lib.ov_free(value)
core=ctypes.c_void_p()
check(lib.ov_core_create(ctypes.byref(core)))
devices=Devices()
matches=[]
try:
    check(lib.ov_core_get_available_devices(core,ctypes.byref(devices)))
    try:
        names=[devices.devices[index].decode(errors='replace') for index in range(devices.size)]
    finally:
        lib.ov_available_devices_free(ctypes.byref(devices))
    for name in names:
        if name!='GPU' and not name.startswith('GPU.'):
            continue
        try:
            luid=prop(core,name,'DEVICE_LUID').strip().lower()
            ordinal=int(prop(core,name,'DEVICE_ID').strip(),10)
            uuid=prop(core,name,'DEVICE_UUID').strip().lower()
        except (RuntimeError,ValueError):
            continue
        if luid==expected_luid and len(uuid)==32 and all(value in '0123456789abcdef' for value in uuid) and any(value!='0' for value in uuid) and 0<=ordinal<32:
            matches.append({{'name':name,'ordinal':ordinal,'gpu_id':'uuid:'+uuid}})
    assert len(matches)==1, f'GPU LUID not uniquely visible to OpenVINO: {{matches}}'
    print('__FRAMEGEN_OPENVINO_DEVICE__='+json.dumps(matches[0],separators=(',',':')))
finally:
    lib.ov_core_free(core)
"#,
        python_literal(&library),
        expected_luid,
    ))
}

fn cuda_resolver_script(manifest: &RuntimeManifest, device_count: u32) -> Result<String> {
    let plugin = manifest.plugin_dir.join("vstrt.dll");
    let expected_uuid = nvidia_uuid_bytes(&manifest.device_binding.gpu_id)?;
    let expected_python = expected_uuid
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "import vapoursynth as vs\nassert vs.core.core_version.release_major == 79\nvs.core.std.LoadPlugin(path=r'{}')\nassert vs.core.trt.Version()['tensorrt_version']\nexpected=bytes([{}])\nmatches=[]\nfor ordinal in range({}):\n    try:\n        props=vs.core.trt.DeviceProperties(ordinal)\n        raw=props.get('uuid')\n        actual=bytes((int(value) & 0xff) for value in raw) if isinstance(raw, list) and len(raw)==16 else None\n        if actual == expected:\n            matches.append(ordinal)\n    except Exception:\n        continue\nassert len(matches)==1, f'GPU UUID not uniquely visible to TensorRT: {{matches}}'\nprint('__FRAMEGEN_CUDA_ORDINAL__='+str(matches[0]))",
        python_literal(&plugin),
        expected_python,
        device_count,
    ))
}

fn prepare_backend_environment(backend: RuntimeBackend) -> Result<()> {
    if backend == RuntimeBackend::Openvino {
        return Ok(());
    }
    ensure_unmasked_cuda()
}

fn ensure_unmasked_cuda() -> Result<()> {
    if std::env::var_os("CUDA_VISIBLE_DEVICES").is_some_and(|value| !value.is_empty()) {
        bail!(
            "CUDA_VISIBLE_DEVICES changes CUDA ordinals. Clear it and restart Framegen Manager before installing."
        )
    }
    if std::env::var_os("CUDA_DEVICE_ORDER").is_some_and(|value| !value.is_empty()) {
        bail!(
            "CUDA_DEVICE_ORDER changes CUDA ordinals. Clear it and restart Framegen Manager before installing."
        )
    }
    Ok(())
}

fn static_binding_matches(manifest: &RuntimeManifest, gpu: &crate::model::GpuInfo) -> bool {
    manifest.backend == gpu.backend
        && manifest.device_binding.vendor == gpu.vendor
        && manifest.device_binding.capability == gpu.capability
        && manifest.device_binding.driver_version == gpu.driver_version
        && match manifest.backend {
            RuntimeBackend::Trt => {
                manifest.device_binding.discovery_index == gpu.discovery_index
                    && manifest.device_binding.gpu_id == gpu.gpu_id
            }
            RuntimeBackend::Openvino => gpu
                .adapter_luid
                .as_deref()
                .is_some_and(|luid| intel_luid_bytes(luid).is_ok()),
        }
}

fn resolved_binding_matches(manifest: &RuntimeManifest, resolved: &ResolvedDevice) -> bool {
    manifest.device_binding.gpu_id == resolved.gpu_id
        && (manifest.backend == RuntimeBackend::Openvino
            || manifest.device_binding.device_ordinal == resolved.ordinal)
}

fn run_runtime_python(manifest: &RuntimeManifest, script: &str) -> Result<std::process::Output> {
    let mut command = Command::new(manifest.python_home.join("python.exe"));
    hide_console(&mut command);
    Ok(command
        .env_remove("PYTHONDONTWRITEBYTECODE")
        .env_remove("CUDA_VISIBLE_DEVICES")
        .env(
            "PATH",
            format!(
                "{};{}",
                manifest.dll_dirs[0].display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .arg("-c")
        .arg(script)
        .output()?)
}

fn smoke_runtime(manifest: &RuntimeManifest, memory_mib: u64) -> Result<()> {
    let smoke = inference_smoke_script(manifest, memory_mib)?;
    let output = run_runtime_python(manifest, &smoke)?;
    if !output.status.success() {
        bail!(
            "RIFE {} inference smoke failed: {}",
            manifest.backend.as_str(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    if !String::from_utf8_lossy(&output.stdout).contains("__FRAMEGEN_INFERENCE_OK__") {
        bail!("RIFE inference smoke returned no completion marker")
    }
    Ok(())
}

fn inference_smoke_script(manifest: &RuntimeManifest, memory_mib: u64) -> Result<String> {
    let model_dir = manifest.model_dir.clone();
    let model_path = model_dir.join("rife_v2/rife_v4.26.onnx");
    let engine_dir = manifest.engine_dir.clone();
    match manifest.backend {
        RuntimeBackend::Trt => {
            let plugin = manifest.plugin_dir.join("vstrt.dll");
            let wrapper_dir = manifest.plugin_dir.clone();
            let expected_uuid = nvidia_uuid_bytes(&manifest.device_binding.gpu_id)?;
            let expected_python = expected_uuid
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let shapes = if memory_mib >= 6 * 1024 {
                "(64,64),(1280,720),(1920,1080)"
            } else {
                "(64,64),(1280,720)"
            };
            Ok(format!(
                "import os, sys, vapoursynth as vs\nvs.core.std.LoadPlugin(path=r'{}')\nprops=vs.core.trt.DeviceProperties({})\nraw=props.get('uuid')\nassert isinstance(raw,list) and bytes((int(v)&0xff) for v in raw)==bytes([{}])\nsys.path.insert(0,r'{}')\nimport vsmlrt\nvsmlrt.models_path=r'{}'\nmodel_path=r'{}'\nassert os.path.isfile(model_path)\nbackend=vsmlrt.Backend.TRT(fp16=True,device_id={},output_format=1,use_cuda_graph=True,num_streams=2,builder_optimization_level=5,engine_folder=r'{}',custom_env={{'PATH':os.environ['PATH']}})\nfor width,height in ({}):\n left=vs.core.std.BlankClip(width=width,height=height,length=1,format=vs.RGBH,color=[0.0,0.0,0.0])\n right=left.std.BlankClip(color=[1.0,1.0,1.0])\n mask=left.std.BlankClip(format=vs.GRAYH,color=0.5)\n out=vsmlrt.RIFEMerge(clipa=left,clipb=right,mask=mask,model=vsmlrt.RIFEModel.v4_26,backend=backend,_implementation=2)\n frame=out.get_frame(0)\n assert frame.width==width and frame.height==height\n del frame,out,mask,right,left\nprint('__FRAMEGEN_INFERENCE_OK__')",
                python_literal(&plugin),
                manifest.device_binding.device_ordinal,
                expected_python,
                python_literal(&wrapper_dir),
                python_literal(&model_dir),
                python_literal(&model_path),
                manifest.device_binding.device_ordinal,
                python_literal(&engine_dir),
                shapes,
            ))
        }
        RuntimeBackend::Openvino => {
            let plugin = manifest.plugin_dir.join("vsov.dll");
            Ok(format!(
                "import os, vapoursynth as vs\nvs.core.std.LoadPlugin(path=r'{}')\nraw_devices=vs.core.ov.AvailableDevices()\ndevices=[raw_devices.decode(errors='replace')] if isinstance(raw_devices,bytes) else ([raw_devices] if isinstance(raw_devices,str) else [v.decode(errors='replace') if isinstance(v,bytes) else str(v) for v in raw_devices])\nassert 'GPU' in devices, f'OpenVINO GPU device is unavailable: {{devices}}'\nmodel_path=r'{}'\nassert os.path.isfile(model_path)\nfor width,height in ((320,180),(640,360)):\n left=vs.core.std.BlankClip(width=width,height=height,length=1,format=vs.RGBS,color=[0.0,0.0,0.0])\n right=left.std.BlankClip(color=[1.0,1.0,1.0])\n mask=left.std.BlankClip(format=vs.GRAYS,color=0.5)\n out=vs.core.ov.Model([left,right,mask],model_path,device='GPU.{}',builtin=False,fp16=False,config=lambda:{{'NUM_STREAMS':4,'INFERENCE_PRECISION_HINT':'f16','CACHE_DIR':r'{}'}})\n frame=out.get_frame(0)\n assert frame.width==width and frame.height==height\n del frame,out,mask,right,left\nprint('__FRAMEGEN_INFERENCE_OK__')",
                python_literal(&plugin),
                python_literal(&model_path),
                manifest.device_binding.device_ordinal,
                python_literal(&engine_dir),
            ))
        }
    }
}

fn nvidia_uuid_bytes(value: &str) -> Result<[u8; 16]> {
    let uuid = value
        .strip_prefix("GPU-")
        .or_else(|| value.strip_prefix("gpu-"))
        .context("GPU UUID has no NVIDIA prefix")?;
    let hex = uuid
        .chars()
        .filter(|character| *character != '-')
        .collect::<String>();
    if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("GPU UUID is malformed")
    }
    let bytes = hex::decode(hex)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("GPU UUID is not 16 bytes"))
}

fn intel_luid_bytes(value: &str) -> Result<[u8; 8]> {
    let encoded = value
        .strip_prefix("luid:")
        .context("Intel GPU ID has no DXGI LUID prefix")?;
    if encoded.len() != 16 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Intel GPU LUID is malformed")
    }
    let bytes: [u8; 8] = hex::decode(encoded)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Intel GPU LUID is not 8 bytes"))?;
    if bytes.iter().all(|byte| *byte == 0) || value != format!("luid:{}", hex::encode(bytes)) {
        bail!("Intel GPU LUID is not canonical")
    }
    Ok(bytes)
}

fn openvino_uuid_bytes(value: &str) -> Result<[u8; 16]> {
    let encoded = value
        .strip_prefix("uuid:")
        .context("OpenVINO GPU ID has no UUID prefix")?;
    if encoded.len() != 32 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("OpenVINO GPU UUID is malformed")
    }
    let bytes: [u8; 16] = hex::decode(encoded)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("OpenVINO GPU UUID is not 16 bytes"))?;
    if bytes.iter().all(|byte| *byte == 0) || value != format!("uuid:{}", hex::encode(bytes)) {
        bail!("OpenVINO GPU UUID is not canonical")
    }
    Ok(bytes)
}

fn python_literal(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "\\'")
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(windows)]
fn running_process_ids(names: &[&str]) -> Result<Vec<u32>> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error()).context("failed to inspect running players");
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut found = Vec::new();
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        let length = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
        if names
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
        {
            found.push(entry.th32ProcessID);
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    Ok(found)
}

#[cfg(not(windows))]
fn running_process_ids(_names: &[&str]) -> Result<Vec<u32>> {
    Ok(Vec::new())
}

fn player_is_running() -> Result<bool> {
    Ok(!running_process_ids(&["mpv.exe", "mpvnet.exe"])?.is_empty())
}

fn rebase_manifest(mut manifest: RuntimeManifest, destination: &Path) -> RuntimeManifest {
    let old = manifest.version_dir.clone();
    let rebase = |path: &Path| destination.join(path.strip_prefix(&old).unwrap());
    manifest.version_dir = destination.to_path_buf();
    manifest.vsscript_dir = rebase(&manifest.vsscript_dir);
    manifest.python_home = rebase(&manifest.python_home);
    manifest.dll_dirs = manifest.dll_dirs.iter().map(|path| rebase(path)).collect();
    manifest.plugin_dir = rebase(&manifest.plugin_dir);
    manifest.model_dir = rebase(&manifest.model_dir);
    manifest
}

fn validate_manifest(root: &Path, manifest: &RuntimeManifest) -> Result<Vec<String>> {
    let runtime = validate_manifest_structure(root, manifest.clone())?;
    validate_safe_runtime(&runtime)
}

fn validate_manifest_structure(root: &Path, manifest: RuntimeManifest) -> Result<SafeRuntime> {
    let binding_is_valid = match manifest.backend {
        RuntimeBackend::Trt => {
            manifest.device_binding.vendor == "nvidia"
                && hardware_resource(&manifest.device_binding.capability).as_deref()
                    == Some(manifest.hardware_resource.as_str())
                && nvidia_uuid_bytes(&manifest.device_binding.gpu_id).is_ok()
        }
        RuntimeBackend::Openvino => {
            manifest.device_binding.vendor == "intel"
                && manifest.device_binding.capability == "intel-gpu"
                && manifest.hardware_resource == "intel-gpu"
                && openvino_uuid_bytes(&manifest.device_binding.gpu_id).is_ok()
        }
    };
    if !manifest.ready
        || manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.version != MANIFEST_VERSION
        || manifest.runtime_id != RUNTIME_VERSION
        || manifest.runtime_version != RUNTIME_VERSION
        || manifest.checksums.algorithm != "sha256"
        || !binding_is_valid
        || manifest.device_binding.device_ordinal >= 32
    {
        bail!("manifest has an incompatible contract")
    }
    if manifest.selected_models != [ModelId::Rife426] {
        bail!("manifest contains an unsupported model set")
    }
    let versions = root.join("versions");
    let valid_name = manifest
        .version_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(RUNTIME_VERSION));
    if manifest.version_dir.components().any(|component| {
        !matches!(
            component,
            std::path::Component::Prefix(_)
                | std::path::Component::RootDir
                | std::path::Component::Normal(_)
        )
    }) || manifest.version_dir.parent() != Some(versions.as_path())
        || !valid_name
    {
        bail!("runtime version path escapes the managed versions directory")
    }
    match fs::symlink_metadata(&manifest.version_dir) {
        Ok(metadata) if metadata.is_dir() && !is_reparse_point(&metadata) => {}
        _ => bail!("active runtime version is missing or is a reparse point"),
    }
    reject_reparse_ancestors(&versions, &manifest.version_dir)?;
    let canonical_versions = versions.canonicalize()?;
    let canonical_version = manifest.version_dir.canonicalize()?;
    if !canonical_version.starts_with(&canonical_versions) {
        bail!("runtime version resolves outside the managed versions directory")
    }
    let expected_layout = [
        (&manifest.python_home, manifest.version_dir.join("python")),
        (
            &manifest.vsscript_dir,
            manifest
                .version_dir
                .join("python/Lib/site-packages/vapoursynth"),
        ),
        (&manifest.plugin_dir, manifest.version_dir.join("plugins")),
        (&manifest.model_dir, manifest.version_dir.join("models")),
    ];
    if expected_layout
        .iter()
        .any(|(actual, expected)| *actual != expected)
        || manifest.dll_dirs
            != [manifest.version_dir.join(match manifest.backend {
                RuntimeBackend::Trt => "plugins/vsmlrt-cuda",
                RuntimeBackend::Openvino => "plugins/vsov",
            })]
    {
        bail!("manifest contains an unexpected runtime layout")
    }
    let engines = root.join("engines");
    if manifest.engine_namespace.len() != 64
        || !manifest
            .engine_namespace
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || manifest.engine_dir != engine_namespace_path(root, &manifest)
    {
        bail!("manifest has an invalid engine namespace")
    }
    reject_reparse_ancestors(root, &manifest.engine_dir)?;
    match fs::symlink_metadata(&manifest.engine_dir) {
        Ok(engine_metadata) => {
            if !engine_metadata.is_dir() || is_reparse_point(&engine_metadata) {
                bail!("engine namespace is not a normal directory")
            }
            let canonical_engines = engines.canonicalize()?;
            let canonical_engine = manifest.engine_dir.canonicalize()?;
            if !canonical_engine.starts_with(&canonical_engines)
                || canonical_engine == canonical_engines
            {
                bail!("engine namespace escapes the managed engine directory")
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for (path, _) in &expected_layout {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || is_reparse_point(&metadata) {
            bail!("runtime directory is missing or unsafe: {}", path.display())
        }
    }
    if manifest.files.is_empty() {
        bail!("manifest contains no runtime files")
    }
    for (relative, file) in &manifest.files {
        validate_manifest_relative(relative)?;
        let component = owned_runtime_component(relative)
            .with_context(|| format!("manifest tracks an unowned runtime file: {relative}"))?;
        if file.component != component {
            bail!("manifest component does not match owned path: {relative}")
        }
    }
    Ok(SafeRuntime {
        manifest,
        canonical_version,
    })
}

fn validate_safe_runtime(runtime: &SafeRuntime) -> Result<Vec<String>> {
    let manifest = &runtime.manifest;
    let mut problems = vec![];
    match fs::symlink_metadata(&manifest.engine_dir) {
        Ok(metadata) if metadata.is_dir() && !is_reparse_point(&metadata) => {}
        _ => problems.push("Engine cache отсутствует или поврежден".to_owned()),
    }
    for path in required_files(manifest) {
        let relative = path
            .strip_prefix(&manifest.version_dir)?
            .to_string_lossy()
            .replace('\\', "/");
        if !manifest.files.contains_key(&relative) {
            problems.push(format!(
                "Manifest не отслеживает обязательный файл {relative}"
            ));
        }
    }
    for (relative, file) in &manifest.files {
        let path = safe_manifest_file(runtime, relative)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.is_file()
                    && !is_reparse_point(&metadata)
                    && metadata.len() == file.size
                    && file.sha256.len() == 64
                    && file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
            {
                if sha256(&path)? != file.sha256 {
                    problems.push(format!("Изменен файл {relative}"));
                }
            }
            _ => problems.push(format!("Отсутствует файл {relative}")),
        }
    }
    for notice in runtime_notices::for_backend(manifest.backend) {
        let relative = format!("notices/{}", notice.filename);
        match fs::read(manifest.version_dir.join(&relative)) {
            Ok(contents) if contents == notice.contents => {}
            _ => problems.push(format!("Неактуальный обязательный файл {relative}")),
        }
    }
    let actual = collect_runtime_file_paths(&manifest.version_dir)?;
    for relative in &actual {
        if relative != MANIFEST_NAME && !manifest.files.contains_key(relative) {
            problems.push(format!("Неотслеживаемый файл runtime: {relative}"));
        }
    }
    Ok(problems)
}

fn validate_manifest_relative(relative: &str) -> Result<()> {
    if relative.is_empty()
        || relative.contains(':')
        || relative.starts_with(['/', '\\'])
        || relative.contains('\\')
        || Path::new(relative)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("unsafe manifest path: {relative}")
    }
    Ok(())
}

fn safe_manifest_file(runtime: &SafeRuntime, relative: &str) -> Result<PathBuf> {
    validate_manifest_relative(relative)?;
    let path = runtime.manifest.version_dir.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("manifest file is missing: {relative}"))?;
    if !metadata.is_file() || is_reparse_point(&metadata) {
        bail!("manifest file is not a normal file: {relative}")
    }
    reject_reparse_ancestors(&runtime.manifest.version_dir, &path)?;
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(&runtime.canonical_version) {
        bail!("manifest file escapes the runtime: {relative}")
    }
    Ok(path)
}

fn reject_reparse_ancestors(root: &Path, target: &Path) -> Result<()> {
    let mut current = target;
    loop {
        if current.exists() && is_reparse_point(&fs::symlink_metadata(current)?) {
            bail!(
                "reparse point in managed runtime path: {}",
                current.display()
            )
        }
        if current == root {
            break;
        }
        current = current
            .parent()
            .context("runtime path escaped managed versions root")?;
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<RuntimeManifest> {
    reject_reparse_file(path)?;
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn read_current_manifest(path: &Path) -> Result<Option<RuntimeManifest>> {
    reject_reparse_file(path)?;
    Ok(serde_json::from_slice(&fs::read(path)?).ok())
}

fn read_optional_normal_file(path: &Path) -> Result<Option<Vec<u8>>> {
    reject_reparse_file(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path.parent().context("manifest path has no parent")?;
    fs::create_dir_all(parent)?;
    reject_reparse_file(path)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".framegen-json-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), value)?;
    temporary.as_file_mut().write_all(b"\n")?;
    temporary.as_file_mut().sync_all()?;
    let temporary_path = temporary.path().to_path_buf();
    temporary.disable_cleanup(true);
    drop(temporary);
    match replace_file(&temporary_path, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary_path);
            Err(error)
        }
    }
}

fn write_bytes_atomic(path: &Path, value: &[u8]) -> Result<()> {
    let parent = path.parent().context("managed JSON path has no parent")?;
    fs::create_dir_all(parent)?;
    reject_reparse_file(path)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".framegen-json-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(value)?;
    temporary.as_file_mut().sync_all()?;
    let temporary_path = temporary.path().to_path_buf();
    temporary.disable_cleanup(true);
    drop(temporary);
    match replace_file(&temporary_path, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(temporary_path);
            Err(error)
        }
    }
}

fn restore_json(path: &Path, previous: Option<&[u8]>) -> Result<()> {
    match previous {
        Some(bytes) => write_bytes_atomic(path, bytes),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

fn reject_reparse_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_reparse_point(&metadata) || !metadata.is_file() => {
            bail!("managed JSON path is not a normal file: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn cleanup_staging(staging: &Path) -> Result<()> {
    if staging.exists() {
        for entry in fs::read_dir(staging)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if is_reparse_point(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                validate_managed_tree(&path)?;
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

fn cleanup_cache(cache: &Path) -> Result<()> {
    if !cache.exists() {
        return Ok(());
    }
    if is_reparse_point(&fs::symlink_metadata(cache)?) {
        bail!("cache directory is a reparse point")
    }
    for entry in fs::read_dir(cache)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_file() && !is_reparse_point(&metadata) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn cleanup_inactive_versions(root: &Path, active: &Path) -> Result<()> {
    if player_is_running()? {
        return Ok(());
    }
    cleanup_pending_gc(root)?;
    let versions = root.join("versions");
    let canonical_active = canonical_managed_directory(&versions, active)?;
    let gc = root.join(GC_DIR);
    fs::create_dir_all(&gc)?;
    for entry in fs::read_dir(&versions)? {
        if player_is_running()? {
            return Ok(());
        }
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || is_reparse_point(&metadata) {
            continue;
        }
        let canonical_candidate = canonical_managed_directory(&versions, &path)?;
        if canonical_candidate == canonical_active {
            continue;
        }
        let runtime = match validate_gc_version_candidate(root, &path) {
            Ok(runtime) => runtime,
            Err(_) => continue,
        };
        ensure_gc_runtime_self_binding(&runtime, &canonical_candidate)?;
        validate_managed_tree(&path)?;
        let token = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        let data_name = format!("{token}.data");
        let marker = gc.join(format!("{token}.json"));
        let destination = gc.join(&data_name);
        write_json_atomic(
            &marker,
            &serde_json::json!({
                "owner": "qbutt-framegen-manager",
                "version": 1,
                "source": path,
                "data_name": data_name,
            }),
        )?;
        if player_is_running()? {
            fs::remove_file(marker)?;
            return Ok(());
        }
        if let Err(error) = fs::rename(&path, &destination) {
            let _ = fs::remove_file(marker);
            return Err(error.into());
        }
        if let Err(error) = fs::remove_dir_all(&destination) {
            return Err(error.into());
        }
        fs::remove_file(marker)?;
    }
    Ok(())
}

fn cleanup_pending_gc(root: &Path) -> Result<()> {
    if player_is_running()? {
        return Ok(());
    }
    let gc = root.join(GC_DIR);
    if !gc.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&gc)?;
    if !metadata.is_dir() || is_reparse_point(&metadata) {
        bail!("runtime GC directory is unsafe")
    }
    let versions = root.join("versions");
    let canonical_versions = versions.canonicalize()?;
    let active = if root.join(MANIFEST_NAME).is_file() {
        Some(
            validate_manifest_structure(root, read_manifest(&root.join(MANIFEST_NAME))?)?
                .canonical_version,
        )
    } else {
        None
    };
    for entry in fs::read_dir(&gc)? {
        let marker = entry?.path();
        if marker.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        reject_reparse_file(&marker)?;
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&marker)?)?;
        if value.get("owner").and_then(|value| value.as_str()) != Some("qbutt-framegen-manager")
            || value.get("version").and_then(|value| value.as_u64()) != Some(1)
        {
            continue;
        }
        let source = value
            .get("source")
            .and_then(|value| value.as_str())
            .map(PathBuf::from)
            .context("GC marker has no source")?;
        let data_name = value
            .get("data_name")
            .and_then(|value| value.as_str())
            .context("GC marker has no data name")?;
        let valid_data_name = data_name.strip_suffix(".data").is_some_and(|token| {
            !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit())
        });
        if Path::new(data_name).components().count() != 1 || !valid_data_name {
            bail!("GC marker has an unsafe data name")
        }
        if !is_normal_direct_child(&versions, &source) {
            bail!("GC marker points outside inactive runtime versions")
        }
        let destination = gc.join(data_name);
        if !destination.exists() && source.exists() {
            let canonical_source = canonical_managed_directory(&versions, &source)?;
            if canonical_source.parent() != Some(canonical_versions.as_path())
                || active.as_ref() == Some(&canonical_source)
            {
                bail!("GC marker points at the active runtime")
            }
            let runtime = validate_gc_version_candidate(root, &source)?;
            ensure_gc_runtime_self_binding(&runtime, &canonical_source)?;
            validate_managed_tree(&source)?;
            if player_is_running()? {
                return Ok(());
            }
            fs::rename(&source, &destination)?;
        }
        if destination.exists() {
            let _ = canonical_managed_directory(&gc, &destination)?;
            validate_managed_tree(&destination)?;
            if player_is_running()? {
                return Ok(());
            }
            fs::remove_dir_all(&destination)?;
        }
        fs::remove_file(marker)?;
    }
    Ok(())
}

fn cleanup_unused_engines(root: &Path, active: &Path) -> Result<()> {
    if player_is_running()? {
        return Ok(());
    }
    let _gpu_lock = GpuLock::acquire()?;
    if player_is_running()? {
        return Ok(());
    }
    let engines = root.join("engines");
    if !engines.exists() {
        return Ok(());
    }
    let canonical_active = canonical_managed_directory(&engines, active)?;
    for entry in fs::read_dir(&engines)? {
        if player_is_running()? {
            return Ok(());
        }
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.len() != 64 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let canonical_candidate = canonical_managed_directory(&engines, &path)?;
        if canonical_candidate == canonical_active {
            continue;
        }
        validate_managed_tree(&path)?;
        if player_is_running()? {
            return Ok(());
        }
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn validate_gc_version_candidate(root: &Path, candidate: &Path) -> Result<SafeRuntime> {
    let runtime =
        validate_manifest_structure(root, read_manifest(&candidate.join(MANIFEST_NAME))?)?;
    let canonical_candidate = candidate.canonicalize()?;
    ensure_gc_runtime_self_binding(&runtime, &canonical_candidate)?;
    Ok(runtime)
}

fn ensure_gc_runtime_self_binding(runtime: &SafeRuntime, canonical_candidate: &Path) -> Result<()> {
    if runtime.canonical_version != canonical_candidate {
        bail!("inactive runtime manifest does not bind its containing directory")
    }
    Ok(())
}

fn canonical_managed_directory(parent: &Path, path: &Path) -> Result<PathBuf> {
    if !is_normal_direct_child(parent, path) {
        bail!(
            "managed directory is not a direct child: {}",
            path.display()
        )
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || is_reparse_point(&metadata) {
        bail!("managed directory is unsafe: {}", path.display())
    }
    reject_reparse_ancestors(parent, path)?;
    let canonical_parent = parent.canonicalize()?;
    let canonical_path = path.canonicalize()?;
    if canonical_path.parent() != Some(canonical_parent.as_path()) {
        bail!("managed directory resolves outside its parent")
    }
    Ok(canonical_path)
}

fn is_normal_direct_child(parent: &Path, path: &Path) -> bool {
    path.is_absolute()
        && path.parent() == Some(parent)
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

fn validate_runtime_root(root: &Path) -> Result<()> {
    let expected = platform::runtime_root()?;
    if root != expected || !root.is_absolute() {
        bail!("runtime root does not match the owned qbutt profile path")
    }
    if let Some(parent) = root.parent()
        && parent.exists()
    {
        let metadata = fs::symlink_metadata(parent)?;
        if !metadata.is_dir() || is_reparse_point(&metadata) {
            bail!("qbutt media data directory is a reparse point")
        }
    }
    if root.exists() {
        let metadata = fs::symlink_metadata(root)?;
        if !metadata.is_dir() || is_reparse_point(&metadata) {
            bail!("runtime root is not a normal directory")
        }
    }
    for child in ["cache", "staging", "versions", "engines", GC_DIR] {
        let path = root.join(child);
        if path.exists() {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || is_reparse_point(&metadata) {
                bail!(
                    "managed runtime child is a reparse point: {}",
                    path.display()
                )
            }
        }
    }
    Ok(())
}

fn validate_managed_tree(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || is_reparse_point(&metadata) {
        bail!("managed path is not a normal directory: {}", root.display())
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if is_reparse_point(&metadata) {
            bail!("reparse point inside managed path: {}", path.display())
        }
        if metadata.is_dir() {
            validate_managed_tree(&path)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error()).context("atomic manifest replacement failed");
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination)?;
    Ok(())
}
