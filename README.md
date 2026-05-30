# thumbnail-selfie-processor

Rust CLI for removing backgrounds from a directory of selfie images and writing transparent PNGs.

## Requirements

- Rust/Cargo, if building from source
- Native mode requires an RMBG ONNX model file
- No Python required

## Get model

Download the RMBG-1.4 ONNX model into the repo root:

```bash
wget -O model.onnx \
'https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model.onnx'
```

Default model path is `./model.onnx`.

## Build

```bash
cargo build --release
cp target/release/thumbnail-processor ./thumbnail-processor
```

## Run native background removal

```bash
./thumbnail-processor \
  --input-dir /path/to/dir/with/images \
  --output-dir /path/to/final/output \
  --model /path/to/model.onnx
```

## Options

```text
-i, --input-dir <INPUT_DIR>    Directory containing input images
-o, --output-dir <OUTPUT_DIR>  Directory to write transparent PNG outputs
-w, --workers <WORKERS>        Concurrent workers [default: 4]
-e, --engine <ENGINE>          native or external binary path [default: native]
-m, --model <MODEL>            ONNX model path for native engine [default: model.onnx]
```

## Progress output

The CLI prints per-file progress to stdout:

```text
started: Neutral_Front_1.JPG
ended: Neutral_Front_1.JPG
```

Summary prints at end:

```text
done: 73 succeeded, 0 failed
```

## Supported input formats

- `.jpg`
- `.jpeg`
- `.png`
- `.webp`

Outputs always use `.png`, preserving input basename.
