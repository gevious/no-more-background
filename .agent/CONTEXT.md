# Context — thumbnail-selfie-processor

## Domain

**Remove-bg CLI** — Rust CLI that takes a directory of images, removes backgrounds via native ONNX inference or an external engine, and writes transparent PNGs.

## Terms

| Term | Definition |
|------|-----------|
| Input dir | Directory containing images to process (jpg, jpeg, png, webp) |
| Output dir | Directory where transparent PNGs are written |
| BG engine | Background removal adapter. Native default uses RMBG ONNX model; external mode reads stdin image bytes and writes stdout PNG |
| ONNX model | Native engine model file supplied by `--model` (default: `model.onnx`) |
| Worker pool | tokio semaphore-based concurrency limiter (default: 4 workers) |

## Module-seam relationships

- **CLI** (`main.rs`) → seam to clap for arg parsing
- **engine** (`engine.rs`) → seam to native RMBG ONNX or external BG binary. Interface: `remove_background(bytes, engine) → Result<Vec<u8>, EngineError>`
- **batch** (`batch.rs`) → seam to engine + filesystem. Interface: `process_directory(input, output, workers, engine_config) → anyhow::Result<()>`

## Canonical entry points

- `src/main.rs` — binary: parses args, delegates to `process_directory`
- `src/lib.rs` — library: exports `engine` and `batch` modules

## Module interfaces

### engine
- `BackgroundRemovalEngine` — native ONNX model or external subprocess adapter
- `EngineError` — engine-level errors (unavailable, non-zero exit, spawn fail, model/decode/native/encode fail)
- `remove_background(input_bytes, engine)` — returns transparent PNG bytes

### batch
- `EngineConfig` — user-selected engine config (`native(model_path)` or `external(path)`)
- `process_directory(input_dir, output_dir, workers, engine_config)` — scan, spawn tokio tasks, write outputs
- `is_supported_image(path)` — extension filter (jpg/jpeg/png/webp)
- `build_output_path(input, output_dir)` — basename → `.png`

## Dependencies

| Crate | Role |
|--|--|
| clap | CLI argument parsing |
| tokio | async runtime + semaphore worker pool |
| image | image decode/encode and test image helpers |
| rmbg | native RMBG ONNX background removal |
| ort / ort-sys | ONNX Runtime bindings and runtime binary download/copy |
| thiserror | EngineError derive |
| anyhow | application-level error propagation |
| tempfile (dev) | temp dirs in tests |

## Constraints

- Must compile for musl target
- All output forced to `.png`
- Individual failures skipped, batch continues
- Native engine requires compatible RMBG ONNX model file
- External engine interface: read from stdin, write to stdout PNG
