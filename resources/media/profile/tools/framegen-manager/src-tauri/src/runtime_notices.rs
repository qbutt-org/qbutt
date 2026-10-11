use crate::model::RuntimeBackend;
use anyhow::Result;
use std::{fs, path::Path};

#[derive(Clone, Copy)]
pub(crate) struct RuntimeNotice {
    pub(crate) filename: &'static str,
    pub(crate) contents: &'static [u8],
}

const COMMON: &[RuntimeNotice] = &[
    RuntimeNotice {
        filename: "THIRD-PARTY-NOTICES.md",
        contents: include_bytes!("../../notices/THIRD-PARTY-NOTICES.md"),
    },
    RuntimeNotice {
        filename: "vs-mlrt-GPL-3.0.txt",
        contents: include_bytes!("../../notices/vs-mlrt-GPL-3.0.txt"),
    },
    RuntimeNotice {
        filename: "Practical-RIFE-MIT.txt",
        contents: include_bytes!("../../notices/Practical-RIFE-MIT.txt"),
    },
    RuntimeNotice {
        filename: "vs-rife-MIT.txt",
        contents: include_bytes!("../../notices/vs-rife-MIT.txt"),
    },
];

const TENSORRT_OSS: &[RuntimeNotice] = &[
    RuntimeNotice {
        filename: "TensorRT-OSS-LICENSE.txt",
        contents: include_bytes!("../../notices/TensorRT-OSS-LICENSE.txt"),
    },
    RuntimeNotice {
        filename: "TensorRT-OSS-NOTICE.txt",
        contents: include_bytes!("../../notices/TensorRT-OSS-NOTICE.txt"),
    },
];

const OPENVINO: &[RuntimeNotice] = &[
    RuntimeNotice {
        filename: "RIFE-Intel-PROVENANCE.json",
        contents: include_bytes!("../../../../framegen/models/openvino/PROVENANCE.json"),
    },
    RuntimeNotice {
        filename: "RIFE-Intel-REBUILD.md",
        contents: include_bytes!("../../../framegen-models/README.md"),
    },
    RuntimeNotice {
        filename: "VSOV-PROVENANCE.json",
        contents: include_bytes!("../../../framegen-vsov/PROVENANCE.json"),
    },
    RuntimeNotice {
        filename: "VSOV-BUILDING.md",
        contents: include_bytes!("../../../framegen-vsov/BUILDING.md"),
    },
    RuntimeNotice {
        filename: "VSOV-MODIFICATIONS.md",
        contents: include_bytes!("../../../framegen-vsov/source/MODIFICATIONS.md"),
    },
    RuntimeNotice {
        filename: "VSOV-SOURCE-DOWNLOADS.json",
        contents: include_bytes!("../../../framegen-vsov/source/SOURCE-DOWNLOADS.json"),
    },
    RuntimeNotice {
        filename: "VSOV-VapourSynth-R54-LGPL-2.1.txt",
        contents: include_bytes!("../../../framegen-vsov/licenses/VapourSynth-R54-LGPL-2.1.txt"),
    },
    RuntimeNotice {
        filename: "VSOV-VapourSynth-R54-VSHelper-WTFPL-2.0.txt",
        contents: include_bytes!(
            "../../../framegen-vsov/licenses/VapourSynth-R54-VSHelper-WTFPL-2.0.txt"
        ),
    },
    RuntimeNotice {
        filename: "VSOV-ONNX-Apache-2.0.txt",
        contents: include_bytes!("../../../framegen-vsov/licenses/onnx-Apache-2.0.txt"),
    },
    RuntimeNotice {
        filename: "VSOV-Protobuf-BSD-3-Clause.txt",
        contents: include_bytes!("../../../framegen-vsov/licenses/protobuf-BSD-3-Clause.txt"),
    },
    RuntimeNotice {
        filename: "VSOV-Abseil-Apache-2.0.txt",
        contents: include_bytes!("../../../framegen-vsov/licenses/abseil-Apache-2.0.txt"),
    },
    RuntimeNotice {
        filename: "VSOV-utf8-range-MIT.txt",
        contents: include_bytes!("../../../framegen-vsov/licenses/utf8-range-MIT.txt"),
    },
    RuntimeNotice {
        filename: "OpenVINO-Apache-2.0.txt",
        contents: include_bytes!("../../notices/OpenVINO-Apache-2.0.txt"),
    },
    RuntimeNotice {
        filename: "OpenVINO-third-party-programs.txt",
        contents: include_bytes!("../../notices/OpenVINO-third-party-programs.txt"),
    },
    RuntimeNotice {
        filename: "OpenVINO-oneTBB-third-party-programs.txt",
        contents: include_bytes!("../../notices/OpenVINO-oneTBB-third-party-programs.txt"),
    },
    RuntimeNotice {
        filename: "OpenVINO-oneDNN-third-party-programs.txt",
        contents: include_bytes!("../../notices/OpenVINO-oneDNN-third-party-programs.txt"),
    },
];

pub(crate) fn for_backend(backend: RuntimeBackend) -> impl Iterator<Item = &'static RuntimeNotice> {
    let backend_notices = match backend {
        RuntimeBackend::Trt => TENSORRT_OSS,
        RuntimeBackend::Openvino => OPENVINO,
    };
    COMMON.iter().chain(backend_notices)
}

pub(crate) fn contains(filename: &str) -> bool {
    COMMON
        .iter()
        .chain(TENSORRT_OSS)
        .chain(OPENVINO)
        .any(|notice| notice.filename == filename)
}

pub(crate) fn write(root: &Path, backend: RuntimeBackend) -> Result<()> {
    let notices = root.join("notices");
    fs::create_dir_all(&notices)?;
    let stale = match backend {
        RuntimeBackend::Trt => OPENVINO,
        RuntimeBackend::Openvino => TENSORRT_OSS,
    };
    for notice in stale {
        match fs::remove_file(notices.join(notice.filename)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    for notice in for_backend(backend) {
        fs::write(notices.join(notice.filename), notice.contents)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_is_backend_specific_and_repairable() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), RuntimeBackend::Trt).unwrap();

        let common = temp.path().join("notices/vs-mlrt-GPL-3.0.txt");
        assert_eq!(fs::read(&common).unwrap(), COMMON[1].contents);
        assert!(
            !temp
                .path()
                .join("notices/OpenVINO-third-party-programs.txt")
                .exists()
        );
        assert!(temp.path().join("notices/TensorRT-OSS-NOTICE.txt").exists());
        assert!(!temp.path().join("notices/TensorRT-LICENSE.txt").exists());

        fs::write(&common, b"damaged").unwrap();
        write(temp.path(), RuntimeBackend::Trt).unwrap();
        assert_eq!(fs::read(&common).unwrap(), COMMON[1].contents);

        write(temp.path(), RuntimeBackend::Openvino).unwrap();
        assert!(!temp.path().join("notices/TensorRT-OSS-NOTICE.txt").exists());
        for notice in OPENVINO {
            assert_eq!(
                fs::read(temp.path().join("notices").join(notice.filename)).unwrap(),
                notice.contents
            );
        }

        let provenance = OPENVINO
            .iter()
            .find(|notice| notice.filename == "VSOV-PROVENANCE.json")
            .unwrap();
        let installed_provenance = temp.path().join("notices").join(provenance.filename);
        fs::write(&installed_provenance, b"damaged").unwrap();
        write(temp.path(), RuntimeBackend::Openvino).unwrap();
        assert_eq!(fs::read(installed_provenance).unwrap(), provenance.contents);
    }
}
