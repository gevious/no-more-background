use std::io::{Cursor, Write};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use image::{ImageFormat, ImageReader};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("engine binary not found or not executable: {0}")]
    EngineUnavailable(String),
    #[error("engine returned non-zero exit code: {0}")]
    NonZeroExit(i32),
    #[error("failed to spawn engine: {0}")]
    SpawnFailed(#[from] std::io::Error),
    #[error("failed to load native model: {0}")]
    ModelLoad(String),
    #[error("failed to decode input image: {0}")]
    Decode(String),
    #[error("native background removal failed: {0}")]
    Native(String),
    #[error("failed to encode PNG: {0}")]
    Encode(String),
}

#[derive(Clone)]
pub enum BackgroundRemovalEngine {
    External(String),
    Native(Arc<Mutex<rmbg::Rmbg>>),
}

impl BackgroundRemovalEngine {
    pub fn external(engine_path: impl Into<String>) -> Self {
        Self::External(engine_path.into())
    }

    pub fn native(model_path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let remover = rmbg::Rmbg::new(model_path)
            .map_err(|err| EngineError::ModelLoad(err.to_string()))?;
        Ok(Self::Native(Arc::new(Mutex::new(remover))))
    }

    pub fn remove_background(&self, input_bytes: &[u8]) -> Result<Vec<u8>, EngineError> {
        match self {
            Self::External(engine_path) => remove_background_with_external(input_bytes, engine_path),
            Self::Native(remover) => remove_background_with_native(input_bytes, remover),
        }
    }
}

/// Remove background from image bytes using configured engine, return transparent PNG bytes.
pub fn remove_background(
    input_bytes: &[u8],
    engine: &BackgroundRemovalEngine,
) -> Result<Vec<u8>, EngineError> {
    engine.remove_background(input_bytes)
}

fn remove_background_with_external(
    input_bytes: &[u8],
    engine_path: &str,
) -> Result<Vec<u8>, EngineError> {
    let mut child = Command::new(engine_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(EngineError::SpawnFailed)?;

    let stdin = child.stdin.as_mut().ok_or_else(|| {
        EngineError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::Other,
            "failed to open stdin",
        ))
    })?;
    stdin.write_all(input_bytes).map_err(EngineError::SpawnFailed)?;

    let output = child.wait_with_output().map_err(EngineError::SpawnFailed)?;

    if !output.status.success() {
        return Err(EngineError::NonZeroExit(
            output.status.code().unwrap_or(-1),
        ));
    }

    Ok(output.stdout)
}

fn remove_background_with_native(
    input_bytes: &[u8],
    remover: &Arc<Mutex<rmbg::Rmbg>>,
) -> Result<Vec<u8>, EngineError> {
    let image = ImageReader::new(Cursor::new(input_bytes))
        .with_guessed_format()
        .map_err(|err| EngineError::Decode(err.to_string()))?
        .decode()
        .map_err(|err| EngineError::Decode(err.to_string()))?;

    let output = remover
        .lock()
        .map_err(|err| EngineError::Native(err.to_string()))?
        .remove_background(&image)
        .map_err(|err| EngineError::Native(err.to_string()))?;

    let mut png = Cursor::new(Vec::new());
    output
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|err| EngineError::Encode(err.to_string()))?;

    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;
    use tempfile::tempdir;

    /// Helper: create minimal 2x2 PNG bytes for testing
    fn test_image_bytes() -> Vec<u8> {
        let img = DynamicImage::new_rgb8(2, 2);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png)
            .expect("write test image");
        buf.into_inner()
    }

    #[test]
    fn engine_unavailable_returns_error() {
        let bytes = test_image_bytes();
        let engine = BackgroundRemovalEngine::external("/nonexistent/evil-engine");
        let result = remove_background(&bytes, &engine);
        assert!(result.is_err());
        match result.unwrap_err() {
            EngineError::SpawnFailed(_) => {}, // expected: no such file
            err => panic!("expected SpawnFailed, got {err}"),
        }
    }

    #[test]
    fn mock_engine_returns_valid_png() {
        let dir = tempdir().expect("create temp dir");
        let script_path = dir.path().join("fake-rembg");
        let script = r#"#!/bin/bash
# fake-rembg: reads stdin bytes and writes PNG magic + stdin as stdout
printf '\x89PNG\r\n\x1a\n'
cat
"#;
        std::fs::write(&script_path, script).expect("write mock");
        Command::new("chmod")
            .args(["+x", script_path.to_str().unwrap()])
            .status()
            .expect("chmod mock");

        let bytes = test_image_bytes();
        let engine = BackgroundRemovalEngine::external(script_path.to_str().unwrap());
        let output = remove_background(&bytes, &engine);
        assert!(output.is_ok());
        let png = output.unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
