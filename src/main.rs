use clap::Parser;
use std::path::PathBuf;

use thumbnail_processor::batch::{process_directory, EngineConfig};

#[derive(Parser, Debug)]
#[command(name = "thumbnail-processor")]
#[command(about = "Remove backgrounds from images, output transparent PNGs")]
pub struct Cli {
    /// Directory containing input images
    #[arg(short, long)]
    input_dir: PathBuf,

    /// Directory to write transparent PNG outputs
    #[arg(short, long)]
    output_dir: PathBuf,

    /// Number of concurrent workers
    #[arg(short, long, default_value_t = 4)]
    workers: usize,

    /// Background removal engine: `native` or external binary path
    #[arg(short, long, default_value = "native")]
    engine: String,

    /// ONNX model path for native engine
    #[arg(short, long, default_value = "model.onnx")]
    model: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let engine = if cli.engine == "native" {
        EngineConfig::native(cli.model)
    } else {
        EngineConfig::external(cli.engine)
    };
    process_directory(&cli.input_dir, &cli.output_dir, cli.workers, engine).await
}
