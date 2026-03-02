use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use tar::Archive;
use zip::ZipArchive;

use crate::errors::{AppError, AppResult};

const ORT_VERSION: &str = "1.23.2";
const ORT_DYLIB_PATH_ENV: &str = "ORT_DYLIB_PATH";
static ORT_INITIALIZED: OnceLock<PathBuf> = OnceLock::new();

pub fn initialize_ort_dylib_env(cache_dir: Option<PathBuf>) -> AppResult<()> {
    if ORT_INITIALIZED.get().is_some() {
        return Ok(());
    }

    let dylib_path = resolve_ort_dylib_path(cache_dir)?;
    // SAFETY: This function is intended to run once during process startup,
    // before the async runtime starts handling concurrent requests.
    unsafe {
        std::env::set_var(ORT_DYLIB_PATH_ENV, &dylib_path);
    }
    let _ = ORT_INITIALIZED.set(dylib_path);
    Ok(())
}

pub fn ensure_ort_dylib_configured() -> AppResult<()> {
    let value = std::env::var_os(ORT_DYLIB_PATH_ENV).ok_or_else(|| {
        AppError::ConfigMissing("ORT runtime not initialized; ORT_DYLIB_PATH is unset".to_string())
    })?;
    let path = PathBuf::from(value);
    if !path.exists() {
        return Err(AppError::ConfigMissing(format!(
            "ORT runtime not initialized; ORT_DYLIB_PATH points to missing file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn resolve_ort_dylib_path(cache_dir: Option<PathBuf>) -> AppResult<PathBuf> {
    if let Some(existing) = std::env::var_os(ORT_DYLIB_PATH_ENV) {
        let existing_path = PathBuf::from(existing);
        if existing_path.exists() {
            return Ok(existing_path);
        }
    }

    let spec = target_spec()?;
    let cache_root = ort_cache_root(cache_dir)?;
    let target_dir = cache_root.join(format!("{}-{}", spec.target, ORT_VERSION));
    let dylib_path = target_dir.join("lib").join(spec.lib_name);

    if !dylib_path.exists() {
        fs::create_dir_all(&target_dir).map_err(|error| {
            AppError::Dependency(format!("failed creating ORT cache dir: {error}"))
        })?;
        download_ort_bundle(&spec, &target_dir)?;
        if !dylib_path.exists() {
            return Err(AppError::Dependency(format!(
                "ONNX Runtime archive downloaded but library missing: {}",
                dylib_path.display()
            )));
        }
    }

    Ok(dylib_path)
}

struct OrtTargetSpec {
    target: &'static str,
    archive_name: String,
    lib_name: &'static str,
    archive_kind: ArchiveKind,
}

enum ArchiveKind {
    Tgz,
    Zip,
}

fn target_spec() -> AppResult<OrtTargetSpec> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok(OrtTargetSpec {
            target: "osx-x86_64",
            archive_name: format!("onnxruntime-osx-x86_64-{ORT_VERSION}.tgz"),
            lib_name: "libonnxruntime.dylib",
            archive_kind: ArchiveKind::Tgz,
        }),
        ("macos", "aarch64") => Ok(OrtTargetSpec {
            target: "osx-arm64",
            archive_name: format!("onnxruntime-osx-arm64-{ORT_VERSION}.tgz"),
            lib_name: "libonnxruntime.dylib",
            archive_kind: ArchiveKind::Tgz,
        }),
        ("linux", "x86_64") => Ok(OrtTargetSpec {
            target: "linux-x64",
            archive_name: format!("onnxruntime-linux-x64-{ORT_VERSION}.tgz"),
            lib_name: "libonnxruntime.so",
            archive_kind: ArchiveKind::Tgz,
        }),
        ("linux", "aarch64") => Ok(OrtTargetSpec {
            target: "linux-aarch64",
            archive_name: format!("onnxruntime-linux-aarch64-{ORT_VERSION}.tgz"),
            lib_name: "libonnxruntime.so",
            archive_kind: ArchiveKind::Tgz,
        }),
        ("windows", "x86_64") => Ok(OrtTargetSpec {
            target: "win-x64",
            archive_name: format!("onnxruntime-win-x64-{ORT_VERSION}.zip"),
            lib_name: "onnxruntime.dll",
            archive_kind: ArchiveKind::Zip,
        }),
        _ => Err(AppError::Dependency(
            "unsupported target for ONNX Runtime auto-download".to_string(),
        )),
    }
}

fn ort_cache_root(cache_dir: Option<PathBuf>) -> AppResult<PathBuf> {
    if let Some(path) = cache_dir {
        return Ok(path.join("onnxruntime"));
    }

    if let Some(path) = std::env::var_os("HF_HUB_CACHE") {
        return Ok(PathBuf::from(path).join("onnxruntime"));
    }
    if let Some(path) = std::env::var_os("HF_HOME") {
        return Ok(PathBuf::from(path).join("onnxruntime"));
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(path).join("onnxruntime"));
    }
    if let Some(path) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(path).join(".cache").join("onnxruntime"));
    }

    std::env::current_dir()
        .map(|path| path.join(".cache").join("onnxruntime"))
        .map_err(|error| AppError::Dependency(format!("failed to resolve cache root: {error}")))
}

fn download_ort_bundle(spec: &OrtTargetSpec, out_dir: &Path) -> AppResult<()> {
    let url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{ORT_VERSION}/{}",
        spec.archive_name
    );

    let client = Client::builder().build().map_err(|error| {
        AppError::Dependency(format!("failed creating download client: {error}"))
    })?;
    let response = client
        .get(url)
        .send()
        .and_then(|resp| resp.error_for_status())
        .map_err(|error| {
            AppError::Dependency(format!("failed downloading ONNX Runtime: {error}"))
        })?;
    let bytes = response.bytes().map_err(|error| {
        AppError::Dependency(format!("failed reading ONNX Runtime download: {error}"))
    })?;

    match spec.archive_kind {
        ArchiveKind::Tgz => extract_tgz_bundle(&bytes, out_dir),
        ArchiveKind::Zip => extract_zip_bundle(&bytes, out_dir),
    }
}

fn extract_tgz_bundle(bytes: &[u8], out_dir: &Path) -> AppResult<()> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|error| AppError::Dependency(format!("failed reading tgz entries: {error}")))?
    {
        let mut entry = entry
            .map_err(|error| AppError::Dependency(format!("failed reading tgz entry: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| AppError::Dependency(format!("failed reading tgz path: {error}")))?;
        let relative = strip_top_component(&path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output_path = out_dir.join(relative);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::Dependency(format!("failed creating parent dir: {error}"))
            })?;
        }
        entry.unpack(&output_path).map_err(|error| {
            AppError::Dependency(format!(
                "failed unpacking tgz entry {}: {error}",
                output_path.display()
            ))
        })?;
    }

    Ok(())
}

fn extract_zip_bundle(bytes: &[u8], out_dir: &Path) -> AppResult<()> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)
        .map_err(|error| AppError::Dependency(format!("failed opening zip archive: {error}")))?;

    for idx in 0..archive.len() {
        let mut file = archive
            .by_index(idx)
            .map_err(|error| AppError::Dependency(format!("failed reading zip entry: {error}")))?;
        let entry_path = Path::new(file.name());
        let relative = strip_top_component(entry_path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output_path = out_dir.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                AppError::Dependency(format!("failed creating extracted dir: {error}"))
            })?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::Dependency(format!("failed creating parent dir: {error}"))
            })?;
        }

        let mut output = fs::File::create(&output_path).map_err(|error| {
            AppError::Dependency(format!("failed creating extracted file: {error}"))
        })?;
        std::io::copy(&mut file, &mut output).map_err(|error| {
            AppError::Dependency(format!("failed writing extracted file: {error}"))
        })?;
    }

    Ok(())
}

fn strip_top_component(path: &Path) -> AppResult<PathBuf> {
    let mut components = path.components().peekable();
    while matches!(components.peek(), Some(Component::CurDir)) {
        components.next();
    }
    components.next();
    let relative: PathBuf = components.collect();

    if relative.as_os_str().is_empty() {
        return Ok(PathBuf::new());
    }
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AppError::Dependency(
            "unexpected path in archive".to_string(),
        ));
    }

    Ok(relative)
}
