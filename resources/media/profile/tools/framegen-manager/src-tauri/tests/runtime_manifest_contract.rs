use framegen_manager_lib::model::{
    ChecksumContract, DeviceBinding, ManifestFile, ModelId, RuntimeBackend, RuntimeManifest,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn executor_accepts(value: &Value) -> bool {
    value.get("schema_version").and_then(Value::as_u64) == Some(3)
        && value.get("version").and_then(Value::as_u64) == Some(3)
        && value.get("ready").and_then(Value::as_bool) == Some(true)
        && matches!(
            value.get("backend").and_then(Value::as_str),
            Some("trt" | "openvino")
        )
        && value
            .pointer("/device_binding/discovery_index")
            .and_then(Value::as_u64)
            .is_some()
        && value
            .pointer("/device_binding/device_ordinal")
            .and_then(Value::as_u64)
            .is_some()
        && value
            .pointer("/device_binding/gpu_id")
            .and_then(Value::as_str)
            .is_some()
        && value.get("files").is_some_and(Value::is_object)
        && value
            .pointer("/checksums/algorithm")
            .and_then(Value::as_str)
            == Some("sha256")
        && [
            "vsscript_dir",
            "python_home",
            "plugin_dir",
            "model_dir",
            "engine_dir",
        ]
        .into_iter()
        .all(|field| value.get(field).and_then(Value::as_str).is_some())
}

#[test]
fn manager_manifest_passes_executor_v3_contract() {
    let root = PathBuf::from(r"C:\runtime\versions\active");
    let manifest = RuntimeManifest {
        schema_version: 3,
        version: 3,
        runtime_id: "test-runtime".to_owned(),
        ready: true,
        backend: RuntimeBackend::Trt,
        runtime_version: "test-runtime".to_owned(),
        version_dir: root.clone(),
        vsscript_dir: root.join("python/Lib/site-packages/vapoursynth"),
        python_home: root.join("python"),
        dll_dirs: vec![root.join("plugins/vsmlrt-cuda")],
        plugin_dir: root.join("plugins"),
        model_dir: root.join("models"),
        engine_dir: PathBuf::from(r"C:\runtime\engines\bound-namespace"),
        engine_namespace: "b".repeat(64),
        selected_models: vec![ModelId::Rife426],
        hardware_resource: "sm120".to_owned(),
        device_binding: DeviceBinding {
            vendor: "nvidia".to_owned(),
            discovery_index: 1,
            device_ordinal: 0,
            gpu_id: "GPU-00112233-4455-6677-8899-aabbccddeeff".to_owned(),
            capability: "12.0".to_owned(),
            driver_version: "test".to_owned(),
        },
        files: BTreeMap::from([(
            "models/rife_v2/rife_v4.26.onnx".to_owned(),
            ManifestFile {
                size: 42,
                sha256: "a".repeat(64),
                component: "models".to_owned(),
            },
        )]),
        checksums: ChecksumContract {
            algorithm: "sha256".to_owned(),
        },
    };

    let value = serde_json::to_value(manifest).unwrap();
    assert!(executor_accepts(&value));
    assert_eq!(value["selected_models"], serde_json::json!(["rife-4.26"]));
    assert_eq!(value["device_binding"]["device_ordinal"], 0);

    let executor = include_str!("../../../../scripts/framegen.lua");
    for fragment in [
        "manifest.schema_version ~= 3",
        "manifest.version ~= 3",
        "manifest.device_binding.discovery_index",
        "manifest.device_binding.device_ordinal",
        "manifest.device_binding.gpu_id",
    ] {
        assert!(
            executor.contains(fragment),
            "executor lost contract fragment {fragment}"
        );
    }
}
