use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::{self, BackgroundRemovalEngine, EngineError};

pub struct EngineConfig {
    kind: EngineConfigKind,
}

enum EngineConfigKind {
    External(String),
    Native(PathBuf),
}

impl EngineConfig {
    pub fn external(engine_path: impl Into<String>) -> Self {
        Self {
            kind: EngineConfigKind::External(engine_path.into()),
        }
    }

    pub fn native(model_path: impl Into<PathBuf>) -> Self {
        Self {
            kind: EngineConfigKind::Native(model_path.into()),
        }
    }

    fn build(self) -> Result<BackgroundRemovalEngine, EngineError> {
        match self.kind {
            EngineConfigKind::External(engine_path) => Ok(BackgroundRemovalEngine::external(engine_path)),
            EngineConfigKind::Native(model_path) => BackgroundRemovalEngine::native(model_path),
        }
    }
}

/// Supported input image extensions
const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Process all images in input directory, write transparent PNGs to output directory.
///
/// - Scans input dir for supported image files
/// - Spawns worker pool with given concurrency (uses tokio semaphore)
/// - Writes .png output preserving basename
/// - Skips individual failures, continues batch
pub async fn process_directory(
    input_dir: &Path,
    output_dir: &Path,
    workers: usize,
    engine_config: EngineConfig,
) -> anyhow::Result<()> {
    // Validate input dir exists
    if !input_dir.is_dir() {
        anyhow::bail!("input directory does not exist or is not a directory: {}", input_dir.display());
    }

    // Create output dir if needed
    fs::create_dir_all(output_dir)?;

    // Scan for supported images
    let image_files: Vec<PathBuf> = fs::read_dir(input_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| is_supported_image(&entry.path()))
        .map(|entry| entry.path())
        .collect();

    if image_files.is_empty() {
        eprintln!("no supported images found in {}", input_dir.display());
        return Ok(());
    }

    eprintln!("found {} images in {}", image_files.len(), input_dir.display());

    let engine = engine_config
        .build()
        .map_err(|e| anyhow::anyhow!("failed to initialize engine: {}", e))?;

    // Worker pool via semaphore
    let semaphore = Arc::new(tokio::sync::Semaphore::new(workers));
    let mut handles = Vec::with_capacity(image_files.len());

    for image_path in image_files {
        let semaphore = semaphore.clone();
        let engine = engine.clone();
        let out_dir = output_dir.to_path_buf();

        let handle = tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await.unwrap();
            let file_name = image_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            println!("started: {}", file_name);
            let result = process_single_image(&image_path, &out_dir, engine).await;
            println!("ended: {}", file_name);
            (image_path, result)
        });
        handles.push(handle);
    }

    // Collect results
    let mut succeeded: u32 = 0;
    let mut failed: u32 = 0;

    for handle in handles {
        match handle.await {
            Ok((path, Ok(()))) => {
                succeeded += 1;
                eprintln!("ok: {}", path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default());
            }
            Ok((path, Err(e))) => {
                failed += 1;
                eprintln!(
                    "skip: {} ({})",
                    path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default(),
                    e
                );
            }
            Err(join_err) => {
                failed += 1;
                eprintln!("fail: task panicked: {}", join_err);
            }
        }
    }

    eprintln!("done: {} succeeded, {} failed", succeeded, failed);
    Ok(())
}

/// Process a single image through the background removal engine.
async fn process_single_image(
    input_path: &Path,
    output_dir: &Path,
    engine: BackgroundRemovalEngine,
) -> anyhow::Result<()> {
    let input_path = input_path.to_path_buf();
    let output_dir = output_dir.to_path_buf();

    // Run blocking I/O in spawn_blocking
    tokio::task::spawn_blocking(move || {
        let input_bytes = fs::read(&input_path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {}", input_path.display(), e))?;

        let png_bytes = engine::remove_background(&input_bytes, &engine)
            .map_err(|e| match e {
                EngineError::EngineUnavailable(msg) => anyhow::anyhow!("engine unavailable: {}", msg),
                EngineError::NonZeroExit(code) => anyhow::anyhow!("engine exit code {}", code),
                EngineError::SpawnFailed(io_err) => anyhow::anyhow!("spawn failed: {}", io_err),
                EngineError::ModelLoad(msg) => anyhow::anyhow!("model load failed: {}", msg),
                EngineError::Decode(msg) => anyhow::anyhow!("decode failed: {}", msg),
                EngineError::Native(msg) => anyhow::anyhow!("native engine failed: {}", msg),
                EngineError::Encode(msg) => anyhow::anyhow!("encode failed: {}", msg),
            })?;

        let output_path = build_output_path(&input_path, &output_dir)?;
        fs::write(&output_path, png_bytes)
            .map_err(|e| anyhow::anyhow!("failed to write {}: {}", output_path.display(), e))?;

        Ok(())
    })
    .await
    .unwrap()
}

/// Build output path: same basename as input but with .png extension.
fn build_output_path(input_path: &Path, output_dir: &Path) -> anyhow::Result<PathBuf> {
    let stem = input_path
        .file_stem()
        .ok_or_else(|| anyhow::anyhow!("cannot extract file stem from {}", input_path.display()))?;
    let mut output_name = stem.to_os_string();
    output_name.push(".png");
    Ok(output_dir.join(output_name))
}

/// Check if a file has a supported image extension.
fn is_supported_image(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    SUPPORTED_EXTENSIONS.contains(&ext.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    /// Create a mock rembg script that echoes stdin with PNG magic
    fn create_mock_engine(dir: &Path) -> PathBuf {
        let script_path = dir.join("mock-rembg");
        let script = r#"#!/bin/bash
printf '\x89PNG\r\n\x1a\n'
cat
"#;
        fs::write(&script_path, script).unwrap();
        Command::new("chmod")
            .args(["+x", script_path.to_str().unwrap()])
            .status()
            .unwrap();
        script_path
    }

    #[test]
    fn is_supported_image_accepts_jpg() {
        assert!(is_supported_image(&PathBuf::from("test.jpg")));
    }

    #[test]
    fn is_supported_image_accepts_png() {
        assert!(is_supported_image(&PathBuf::from("test.PNG")));
    }

    #[test]
    fn is_supported_image_rejects_txt() {
        assert!(!is_supported_image(&PathBuf::from("readme.txt")));
    }

    #[test]
    fn is_supported_image_rejects_no_extension() {
        assert!(!is_supported_image(&PathBuf::from("Makefile")));
    }

    #[test]
    fn build_output_path_converts_to_png() {
        let out = PathBuf::from("/out");
        let path = build_output_path(&PathBuf::from("/in/photo.jpg"), &out).unwrap();
        assert_eq!(path, PathBuf::from("/out/photo.png"));
    }

    #[tokio::test]
    async fn process_directory_empty_dir_returns_ok() {
        let dir = tempdir().unwrap();
        process_directory(
            dir.path(),
            &dir.path().join("out"),
            1,
            EngineConfig::external("rembg"),
        )
            .await
            .expect("empty dir should return Ok");
    }

    #[tokio::test]
    async fn process_directory_missing_input_returns_error() {
        let result = process_directory(
            &PathBuf::from("/nonexistent/input"),
            &PathBuf::from("/tmp/out"),
            1,
            EngineConfig::external("rembg"),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn process_directory_processes_images_with_mock_engine() {
        let dir = tempdir().unwrap();
        let input_dir = dir.path().join("input");
        let output_dir = dir.path().join("output");
        let engine_dir = tempdir().unwrap();

        fs::create_dir(&input_dir).unwrap();
        // Write dummy image files
        fs::write(input_dir.join("a.jpg"), vec![0xFF, 0xD8]).unwrap();
        fs::write(input_dir.join("b.png"), vec![0x89]).unwrap();
        // unsupported — should be skipped silently
        fs::write(input_dir.join("c.txt"), b"not an image").unwrap();

        let engine = create_mock_engine(engine_dir.path());

        process_directory(
            &input_dir,
            &output_dir,
            2,
            EngineConfig::external(engine.to_str().unwrap()),
        )
        .await
        .expect("process should succeed");

        // Verify outputs exist
        assert!(output_dir.join("a.png").exists());
        assert!(output_dir.join("b.png").exists());
        // Verify PNG magic
        let data = fs::read(output_dir.join("a.png")).unwrap();
        assert!(data.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[tokio::test]
    async fn process_directory_completes_when_images_exceed_workers() {
        let dir = tempdir().unwrap();
        let input_dir = dir.path().join("input");
        let output_dir = dir.path().join("output");
        let engine_dir = tempdir().unwrap();

        fs::create_dir(&input_dir).unwrap();
        fs::write(input_dir.join("a.jpg"), vec![0xFF, 0xD8]).unwrap();
        fs::write(input_dir.join("b.jpg"), vec![0xFF, 0xD8]).unwrap();
        fs::write(input_dir.join("c.jpg"), vec![0xFF, 0xD8]).unwrap();

        let engine = create_mock_engine(engine_dir.path());
        let process = process_directory(
            &input_dir,
            &output_dir,
            2,
            EngineConfig::external(engine.to_str().unwrap()),
        );

        tokio::time::timeout(std::time::Duration::from_secs(2), process)
            .await
            .expect("process should not deadlock when image count exceeds workers")
            .expect("process should succeed");

        assert!(output_dir.join("a.png").exists());
        assert!(output_dir.join("b.png").exists());
        assert!(output_dir.join("c.png").exists());
    }
}
