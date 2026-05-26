use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Build climatology files from ERA5 NetCDF inputs")]
struct Args {
    /// Dossier contenant les NetCDF annuels (un par année 1991..=2020).
    #[arg(long)]
    input_dir: PathBuf,
    /// Dossier de sortie local pour les 366 OMfiles.
    #[arg(long)]
    output_dir: PathBuf,
    /// Préfixe R2 où uploader (ex: climatology/temperature_2m/era5_1991-2020/arpege_france).
    #[arg(long)]
    r2_prefix: String,
    /// Si présent, n'uploade pas vers R2 (test local).
    #[arg(long)]
    skip_upload: bool,
    #[arg(long, default_value_t = 1991)]
    year_start: i32,
    #[arg(long, default_value_t = 2020)]
    year_end: i32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();
    tracing::info!(?args, "starting climatology build");
    Ok(())
}
