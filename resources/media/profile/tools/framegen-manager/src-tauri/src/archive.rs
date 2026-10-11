use crate::catalog::{ArchiveKind, Asset, AssetSource};
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_RANGE, RANGE};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub fn obtain_asset<F>(
    asset: Asset,
    cache: &Path,
    repository: &Path,
    mut progress: F,
) -> Result<PathBuf>
where
    F: FnMut(u64, u64),
{
    let AssetSource::Network {
        url,
        bytes: download_bytes,
    } = asset.source
    else {
        return obtain_bundled_asset(asset, repository, progress);
    };
    fs::create_dir_all(cache)?;
    let destination = cache.join(asset.filename);
    reject_reparse_file(&destination)?;
    if destination.is_file() && sha256(&destination)? == asset.sha256 {
        progress(download_bytes, download_bytes);
        return Ok(destination);
    }
    if destination.exists() {
        fs::remove_file(&destination)?;
    }
    let partial = destination.with_extension("download.partial");
    reject_reparse_file(&partial)?;
    let mut downloaded = fs::metadata(&partial)
        .map(|value| value.len())
        .unwrap_or_default();
    if downloaded > download_bytes {
        fs::remove_file(&partial)?;
        downloaded = 0;
    }
    if downloaded == download_bytes {
        if sha256(&partial)? == asset.sha256 {
            fs::rename(&partial, &destination)?;
            progress(download_bytes, download_bytes);
            return Ok(destination);
        }
        fs::remove_file(&partial)?;
        downloaded = 0;
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(7_200))
        .user_agent("qbutt-framegen-manager/0.1")
        .build()?;
    let mut request = client.get(url);
    if downloaded > 0 {
        request = request.header(RANGE, format!("bytes={downloaded}-"));
    }
    let mut response = request
        .send()
        .with_context(|| format!("failed to download {}", asset.label))?;
    let append = downloaded > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if append {
        let expected_prefix = format!("bytes {downloaded}-");
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .context("resumed download returned no Content-Range")?;
        if !content_range.starts_with(&expected_prefix) {
            bail!("{} returned an incompatible resume range", asset.label)
        }
    } else {
        response = response.error_for_status()?;
        downloaded = 0;
    }
    let expected_response = download_bytes.saturating_sub(downloaded);
    if let Some(length) = response.content_length()
        && length != expected_response
    {
        bail!(
            "{} returned {} bytes; expected {}",
            asset.label,
            length,
            expected_response
        )
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&partial)?;
    let mut output = BufWriter::new(file);
    progress(downloaded, download_bytes);
    let mut buffer = vec![0_u8; 4 * 1024 * 1024];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        downloaded = downloaded.saturating_add(count as u64);
        if downloaded > download_bytes {
            drop(output);
            fs::remove_file(&partial).ok();
            bail!("{} exceeded its pinned download size", asset.label)
        }
        progress(downloaded, download_bytes);
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if downloaded != download_bytes {
        bail!(
            "{} download is incomplete ({downloaded}/{}) and can be resumed",
            asset.label,
            download_bytes
        )
    }
    let actual = sha256(&partial)?;
    if actual != asset.sha256 {
        fs::remove_file(&partial).ok();
        bail!("{} failed SHA-256 verification", asset.label)
    }
    progress(download_bytes, download_bytes);
    fs::rename(&partial, &destination)?;
    Ok(destination)
}

fn obtain_bundled_asset<F>(asset: Asset, repository: &Path, mut progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, u64),
{
    let AssetSource::Bundled { repo_path, bytes } = asset.source else {
        unreachable!()
    };
    let relative = Path::new(repo_path);
    validate_relative_path(relative)?;
    let source = repository.join(relative);
    reject_reparse_file(&source)?;
    let metadata = fs::metadata(&source)
        .with_context(|| format!("bundled asset is missing: {}", source.display()))?;
    if metadata.len() != bytes {
        bail!(
            "{} has {} bytes; expected {}",
            source.display(),
            metadata.len(),
            bytes
        )
    }
    let canonical_repository = repository.canonicalize()?;
    let canonical_source = source.canonicalize()?;
    if !canonical_source.starts_with(&canonical_repository) {
        bail!("bundled asset resolves outside the repository")
    }
    if sha256(&canonical_source)? != asset.sha256 {
        bail!("{} failed SHA-256 verification", asset.label)
    }
    progress(0, 0);
    Ok(canonical_source)
}

fn reject_reparse_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_reparse_point(&metadata) || !metadata.is_file() => {
            bail!("path is not a normal file: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn extract_asset<F>(asset: Asset, archive: &Path, destination: &Path, allow: F) -> Result<()>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    match asset.kind {
        ArchiveKind::File => extract_file(asset, archive, destination, allow),
        ArchiveKind::Zip | ArchiveKind::Wheel => extract_zip(archive, destination, allow),
        ArchiveKind::SevenZip => extract_7z(archive, destination, allow),
    }
}

fn extract_file<F>(asset: Asset, source: &Path, destination: &Path, allow: F) -> Result<()>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    let relative = allow(asset.filename).context("bundled file has no extraction target")?;
    validate_relative_path(&relative)?;
    let target = destination.join(relative);
    ensure_under(destination, &target)?;
    reject_reparse_file(&target)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
        reject_reparse_ancestors(destination, parent)?;
    }
    fs::copy(source, target)?;
    Ok(())
}

fn extract_zip<F>(archive: &Path, destination: &Path, allow: F) -> Result<()>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    let reader = BufReader::new(File::open(archive)?);
    let mut zip = zip::ZipArchive::new(reader)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        validate_entry_name(&name)?;
        let Some(relative) = allow(&name) else {
            continue;
        };
        validate_relative_path(&relative)?;
        let target = destination.join(relative);
        ensure_under(destination, &target)?;
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
            reject_reparse_ancestors(destination, parent)?;
        }
        let mut output = BufWriter::new(File::create(&target)?);
        io::copy(&mut entry, &mut output)?;
        output.flush()?;
    }
    Ok(())
}

fn extract_7z<F>(archive: &Path, destination: &Path, allow: F) -> Result<()>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    fs::create_dir_all(destination)?;
    let root = destination.to_path_buf();
    sevenz_rust2::decompress_file_with_extract_fn(archive, destination, |entry, reader, _| {
        let name = entry.name().replace('\\', "/");
        validate_entry_name(&name).map_err(sevenz_error)?;
        let Some(relative) = allow(&name) else {
            if !entry.is_directory() {
                io::copy(reader, &mut io::sink())?;
            }
            return Ok(true);
        };
        validate_relative_path(&relative).map_err(sevenz_error)?;
        let target = root.join(relative);
        ensure_under(&root, &target).map_err(sevenz_error)?;
        if entry.is_directory() {
            fs::create_dir_all(&target)?;
            return Ok(true);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
            reject_reparse_ancestors(&root, parent).map_err(sevenz_error)?;
        }
        let mut output = BufWriter::new(File::create(&target)?);
        io::copy(reader, &mut output)?;
        output.flush()?;
        Ok(true)
    })?;
    Ok(())
}

fn sevenz_error(error: anyhow::Error) -> sevenz_rust2::Error {
    std::io::Error::other(error.to_string()).into()
}

fn validate_entry_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('\0') || name.starts_with('/') || name.starts_with('\\') {
        bail!("unsafe archive entry: {name}")
    }
    validate_relative_path(Path::new(name))
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("archive path must be non-empty and relative")
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!(
                "archive path contains an unsafe component: {}",
                path.display()
            )
        }
    }
    Ok(())
}

fn ensure_under(root: &Path, target: &Path) -> Result<()> {
    if !target.starts_with(root) {
        bail!("archive target escapes staging directory")
    }
    Ok(())
}

fn reject_reparse_ancestors(root: &Path, target: &Path) -> Result<()> {
    let mut current = target;
    loop {
        if current.exists() && is_reparse_point(&fs::symlink_metadata(current)?) {
            bail!("reparse point in extraction path: {}", current.display())
        }
        if current == root {
            break;
        }
        current = current.parent().context("path escaped staging root")?;
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

pub fn sha256(path: &Path) -> Result<String> {
    let mut input = BufReader::new(File::open(path)?);
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 4 * 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::{
        extract_7z, extract_asset, obtain_asset, sha256, validate_entry_name,
        validate_relative_path,
    };
    use crate::catalog::{ArchiveKind, Asset, AssetSource};
    use std::path::{Path, PathBuf};

    const BUNDLE_SHA256: &str = "1e6ed65d77d6364eeaed5a745ba5c4985ae2b700dd85d7cf7f027bdf294a33fc";

    fn bundled_asset(bytes: u64, hash: &'static str) -> Asset {
        Asset {
            id: "bundled-test",
            label: "Bundled test asset",
            filename: "bundled.bin",
            source: AssetSource::Bundled {
                repo_path: "assets/bundled.bin",
                bytes,
            },
            sha256: hash,
            installed_bytes: bytes,
            kind: ArchiveKind::File,
        }
    }

    #[test]
    fn rejects_traversal_and_absolute_entries() {
        for value in ["../escape", "a/../../escape", "/rooted", "C:/escape"] {
            assert!(validate_entry_name(value).is_err(), "accepted {value}");
        }
        assert!(validate_entry_name("models/rife/model.onnx").is_ok());
        assert!(validate_entry_name("models/./rife/model.onnx").is_ok());
    }

    #[test]
    fn rejects_empty_mapped_path() {
        assert!(validate_relative_path(Path::new("")).is_err());
    }

    #[test]
    fn drains_skipped_solid_entry_before_extracting_allowed_entry() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let archive = temp.path().join("solid.7z");
        let output = temp.path().join("output");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a-skipped.bin"), vec![0x5a; 64 * 1024]).unwrap();
        std::fs::write(source.join("b-kept.bin"), b"kept").unwrap();

        let mut writer = sevenz_rust2::ArchiveWriter::create(&archive).unwrap();
        writer.push_source_path(&source, |_| true).unwrap();
        writer.finish().unwrap();

        extract_7z(&archive, &output, |name| {
            (name == "b-kept.bin").then(|| PathBuf::from("kept.bin"))
        })
        .unwrap();
        assert_eq!(std::fs::read(output.join("kept.bin")).unwrap(), b"kept");
        assert!(!output.join("a-skipped.bin").exists());
    }

    #[test]
    fn bundled_asset_is_verified_without_touching_network_cache() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        let cache = temp.path().join("cache");
        let source = repository.join("assets/bundled.bin");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"bundle").unwrap();
        assert_eq!(sha256(&source).unwrap(), BUNDLE_SHA256);
        let mut progress = vec![];

        let obtained = obtain_asset(
            bundled_asset(6, BUNDLE_SHA256),
            &cache,
            &repository,
            |a, b| {
                progress.push((a, b));
            },
        )
        .unwrap();

        assert_eq!(obtained, source.canonicalize().unwrap());
        assert_eq!(progress, [(0, 0)]);
        assert!(!cache.exists());
    }

    #[test]
    fn bundled_asset_rejects_size_and_hash_drift() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        let source = repository.join("assets/bundled.bin");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"bundle").unwrap();

        assert!(
            obtain_asset(
                bundled_asset(5, BUNDLE_SHA256),
                temp.path(),
                &repository,
                |_, _| {},
            )
            .is_err()
        );
        assert!(
            obtain_asset(
                bundled_asset(
                    6,
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                ),
                temp.path(),
                &repository,
                |_, _| {},
            )
            .is_err()
        );
    }

    #[test]
    fn raw_file_extraction_uses_the_allow_mapping() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bundled.bin");
        let destination = temp.path().join("destination");
        std::fs::write(&source, b"bundle").unwrap();

        extract_asset(
            bundled_asset(6, BUNDLE_SHA256),
            &source,
            &destination,
            |name| (name == "bundled.bin").then(|| PathBuf::from("nested/copied.bin")),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(destination.join("nested/copied.bin")).unwrap(),
            b"bundle"
        );
    }

    #[cfg(windows)]
    #[test]
    fn bundled_asset_rejects_reparse_files() {
        use std::os::windows::fs::symlink_file;

        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        let external = temp.path().join("external.bin");
        let link = repository.join("assets/bundled.bin");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::write(&external, b"bundle").unwrap();
        symlink_file(&external, &link).unwrap();

        assert!(
            obtain_asset(
                bundled_asset(6, BUNDLE_SHA256),
                temp.path(),
                &repository,
                |_, _| {},
            )
            .is_err()
        );
    }
}
