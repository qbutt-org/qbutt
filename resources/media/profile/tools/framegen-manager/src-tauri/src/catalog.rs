use serde::Serialize;

use crate::model::{ModelId, RuntimeBackend};

pub const RUNTIME_VERSION: &str = "py3.13.15-vsR79-vsmlrt15.16-multibackend2-ov2026.2.1";
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ArchiveKind {
    File,
    Zip,
    Wheel,
    SevenZip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetSource {
    Network { url: &'static str, bytes: u64 },
    Bundled { repo_path: &'static str, bytes: u64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Asset {
    pub id: &'static str,
    pub label: &'static str,
    pub filename: &'static str,
    pub source: AssetSource,
    pub sha256: &'static str,
    pub installed_bytes: u64,
    pub kind: ArchiveKind,
}

impl Asset {
    pub const fn download_bytes(self) -> u64 {
        match self.source {
            AssetSource::Network { bytes, .. } => bytes,
            AssetSource::Bundled { .. } => 0,
        }
    }

    pub const fn source_bytes(self) -> u64 {
        match self.source {
            AssetSource::Network { bytes, .. } | AssetSource::Bundled { bytes, .. } => bytes,
        }
    }
}

pub const PYTHON: Asset = Asset {
    id: "python",
    label: "Python 3.13.15",
    filename: "python-3.13.15-embed-amd64.zip",
    source: AssetSource::Network {
        url: "https://www.python.org/ftp/python/3.13.15/python-3.13.15-embed-amd64.zip",
        bytes: 11_009_825,
    },
    sha256: "d1f04d990aee1253d8569e8e5104e30fa9f5fa830899f14843448872d936a2cf",
    installed_bytes: 21_337_150,
    kind: ArchiveKind::Zip,
};

pub const VAPOURSYNTH: Asset = Asset {
    id: "vapoursynth",
    label: "VapourSynth R79",
    filename: "VapourSynth64-Portable-R79.zip",
    source: AssetSource::Network {
        url: "https://github.com/vapoursynth/vapoursynth/releases/download/R79/VapourSynth64-Portable-R79.zip",
        bytes: 23_160_674,
    },
    sha256: "625b3410d903943107291592e90d6f521f829ebb8291d952ee91b8d674bbb153",
    installed_bytes: 10_087_954,
    kind: ArchiveKind::Zip,
};

pub const VSMLRT_SCRIPT: Asset = Asset {
    id: "vsmlrt-script",
    label: "vs-mlrt 15.16",
    filename: "scripts.v15.16.7z",
    source: AssetSource::Network {
        url: "https://github.com/AmusementClub/vs-mlrt/releases/download/v15.16/scripts.v15.16.7z",
        bytes: 17_732,
    },
    sha256: "d07dae0a00cb8dbf4f00358f640f630ff5d933de44d27050e7acce4f31cc3560",
    installed_bytes: 116_325,
    kind: ArchiveKind::SevenZip,
};

pub const VSTRT: Asset = Asset {
    id: "vstrt",
    label: "vs-mlrt TensorRT plugin",
    filename: "VSTRT-Windows-x64.v15.16.7z",
    source: AssetSource::Network {
        url: "https://github.com/AmusementClub/vs-mlrt/releases/download/v15.16/VSTRT-Windows-x64.v15.16.7z",
        bytes: 486_704,
    },
    sha256: "c4e64e69e87553bf15a7acd29d76debd006954e39ee07ef0e6d526947c5d34b1",
    installed_bytes: 2_292_736,
    kind: ArchiveKind::SevenZip,
};

pub const VSOV: Asset = Asset {
    id: "vsov",
    label: "vs-mlrt OpenVINO 2026.2.1 plugin",
    filename: "framegen-vsov.dll",
    source: AssetSource::Bundled {
        repo_path: "tools/bin/framegen-vsov.dll",
        bytes: 4_231_168,
    },
    sha256: "25229f2263d6dfe2794e7140c8cdc6093e40c24f5fc840225b73e974db65cd22",
    installed_bytes: 4_231_168,
    kind: ArchiveKind::File,
};

pub const OPENVINO_RUNTIME: Asset = Asset {
    id: "openvino-runtime",
    label: "OpenVINO 2026.2.1 GPU runtime",
    filename: "openvino-2026.2.1-21919-cp313-cp313-win_amd64.whl",
    source: AssetSource::Network {
        url: "https://files.pythonhosted.org/packages/ee/e5/13b9bfc20c02706d015e96e5b3e932227d658b3826b4aa5007cdc2455ed4/openvino-2026.2.1-21919-cp313-cp313-win_amd64.whl",
        bytes: 76_171_396,
    },
    sha256: "197435104d18657a7cc4d05850615b913618ebb0d3baaec052fda25a2d831c74",
    installed_bytes: 66_669_201,
    kind: ArchiveKind::Wheel,
};

pub const TRT_LIBS: Asset = Asset {
    id: "tensorrt-libs",
    label: "TensorRT 10.16 [CUDA 13]",
    filename: "tensorrt_cu13_libs-10.16.0.72-py3-none-win_amd64.whl",
    source: AssetSource::Network {
        url: "https://pypi.nvidia.com/tensorrt-cu13-libs/tensorrt_cu13_libs-10.16.0.72-py3-none-win_amd64.whl",
        bytes: 1_911_163_963,
    },
    sha256: "11c6703d440f929f057be91e18d6425d657bd6f5f3635cd5a85ea5015e00f310",
    installed_bytes: 925_662_256,
    kind: ArchiveKind::Wheel,
};

pub const RIFE_REGULAR: Asset = Asset {
    id: "rife-4.26-trt",
    label: "RIFE 4.26",
    filename: "rife_v4.26.7z",
    source: AssetSource::Network {
        url: "https://github.com/AmusementClub/vs-mlrt/releases/download/external-models/rife_v4.26.7z",
        bytes: 19_591_599,
    },
    sha256: "dfdabd84a2a3db773f87604b8cc255e94a6a72f13550d910ccd3b4ee2606cd4f",
    installed_bytes: 22_748_049,
    kind: ArchiveKind::SevenZip,
};

pub const RIFE_OPENVINO: Asset = Asset {
    id: "rife-4.26-openvino-s0p5",
    label: "RIFE 4.26 [Intel optimized]",
    filename: "rife_v4.26.onnx",
    source: AssetSource::Bundled {
        repo_path: "framegen/models/openvino/rife_v2/rife_v4.26.onnx",
        bytes: 22_751_073,
    },
    sha256: "3a958289369a5645339b2fbd5f3a228bc82075866c2b4723e4ca485fda72fc6b",
    installed_bytes: 22_751_073,
    kind: ArchiveKind::File,
};

const COMMON_ASSETS: &[Asset] = &[PYTHON, VAPOURSYNTH];

pub fn core_assets(backend: RuntimeBackend) -> Vec<Asset> {
    let mut assets = COMMON_ASSETS.to_vec();
    match backend {
        RuntimeBackend::Trt => assets.extend([VSMLRT_SCRIPT, VSTRT, TRT_LIBS]),
        RuntimeBackend::Openvino => assets.extend([OPENVINO_RUNTIME, VSOV]),
    }
    assets
}

pub fn model_asset(id: &ModelId, backend: RuntimeBackend) -> Asset {
    match (id, backend) {
        (ModelId::Rife426, RuntimeBackend::Trt) => RIFE_REGULAR,
        (ModelId::Rife426, RuntimeBackend::Openvino) => RIFE_OPENVINO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_artifact_is_backend_specific() {
        let trt = model_asset(&ModelId::Rife426, RuntimeBackend::Trt);
        let openvino = model_asset(&ModelId::Rife426, RuntimeBackend::Openvino);
        assert_eq!(trt.id, "rife-4.26-trt");
        assert_eq!(openvino.id, "rife-4.26-openvino-s0p5");
        assert_ne!(trt.sha256, openvino.sha256);
        assert!(matches!(trt.source, AssetSource::Network { .. }));
        assert!(matches!(openvino.source, AssetSource::Bundled { .. }));
    }

    #[test]
    fn openvino_assets_are_wheel_custom_plugin_and_custom_model() {
        let mut assets = core_assets(RuntimeBackend::Openvino);
        assets.push(model_asset(&ModelId::Rife426, RuntimeBackend::Openvino));
        let ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "python",
                "vapoursynth",
                "openvino-runtime",
                "vsov",
                "rife-4.26-openvino-s0p5",
            ]
        );
        assert!(!ids.contains(&"vstrt"));
        assert!(!ids.contains(&"rife-4.26-trt"));
        assert_eq!(VSOV.download_bytes(), 0);
        assert_eq!(RIFE_OPENVINO.download_bytes(), 0);
    }
}
