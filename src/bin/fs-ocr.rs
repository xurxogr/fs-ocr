//! Command-line interface for fs-ocr.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use fs_ocr::config::ScanConfig;
use fs_ocr::coordinator::ScanPipeline;
use fs_ocr::enums::ItemFaction;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_BAD_INPUT: u8 = 2;

#[derive(Parser)]
#[command(name = "fs-ocr", version = VERSION, about = "Foxhole Stockpiles OCR scanner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a stockpile screenshot and emit JSON
    Scan(ScanArgs),
    /// Print version
    Version,
}

#[derive(Args)]
struct ScanArgs {
    /// Image path (defaults to stdin if omitted; "-" also means stdin)
    image: Option<String>,

    /// Template database (.h5)
    #[arg(short = 'd', long, value_name = "PATH")]
    database: PathBuf,

    /// Filter: "wardens" or "colonials"
    #[arg(short = 'f', long, value_name = "NAME")]
    faction: Option<String>,

    /// One-line JSON output
    #[arg(long)]
    compact: bool,

    /// Return alternatives within gap
    #[arg(long, value_name = "F64", default_value_t = 0.0)]
    confidence_gap: f64,

    /// pHash hamming distance
    #[arg(long, value_name = "U32", default_value_t = 15)]
    phash_threshold: u32,

    /// NCC candidate limit
    #[arg(long, value_name = "USIZE", default_value_t = 50)]
    max_ncc_candidates: usize,

    /// Tiebreaker threshold
    #[arg(long, value_name = "F64", default_value_t = 0.0015)]
    ncc_tiebreaker: f64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Version => {
            println!("fs-ocr {}", VERSION);
            ExitCode::from(EXIT_OK)
        }
        Command::Scan(args) => run_scan(args),
    }
}

fn run_scan(args: ScanArgs) -> ExitCode {
    let source = args.image.as_deref().unwrap_or("-");
    let img = match load_image(source) {
        Ok(i) => i,
        Err(code) => return code,
    };

    if !args.database.exists() {
        eprintln!(
            "Error: database file not found: {}",
            args.database.display()
        );
        return ExitCode::from(EXIT_BAD_INPUT);
    }

    let config = ScanConfig {
        phash_threshold: args.phash_threshold,
        max_ncc_candidates: args.max_ncc_candidates,
        confidence_gap: args.confidence_gap,
        ncc_tiebreaker_threshold: args.ncc_tiebreaker,
    };

    let data_path = PathBuf::from("data");
    let mut pipeline =
        ScanPipeline::new(args.database.as_path(), data_path.as_path(), config);

    let faction = args
        .faction
        .as_deref()
        .map(|f| ItemFaction::from_string(Some(f)));

    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    let image_data = rgb.into_raw();

    let mut result = match pipeline.scan(&image_data, width as i32, height as i32, faction) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: scan failed: {}", e);
            return ExitCode::from(EXIT_ERROR);
        }
    };

    if std::env::var("FS_OCR_TIMING").ok().as_deref() != Some("1") {
        result.timing = None;
    }

    let json = if args.compact {
        serde_json::to_string(&result)
    } else {
        serde_json::to_string_pretty(&result)
    };

    match json {
        Ok(j) => {
            println!("{}", j);
            ExitCode::from(EXIT_OK)
        }
        Err(e) => {
            eprintln!("Error: JSON serialization failed: {}", e);
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn load_image(source: &str) -> Result<image::DynamicImage, ExitCode> {
    if source == "-" {
        let mut buf = Vec::new();
        if let Err(e) = std::io::stdin().read_to_end(&mut buf) {
            eprintln!("Error: failed to read image from stdin: {}", e);
            return Err(ExitCode::from(EXIT_BAD_INPUT));
        }
        image::load_from_memory(&buf).map_err(|e| {
            eprintln!("Error: invalid image data: {}", e);
            ExitCode::from(EXIT_BAD_INPUT)
        })
    } else {
        let path = Path::new(source);
        if !path.exists() {
            eprintln!("Error: image file not found: {}", source);
            return Err(ExitCode::from(EXIT_BAD_INPUT));
        }
        image::open(path).map_err(|e| {
            eprintln!("Error: failed to load image: {}", e);
            ExitCode::from(EXIT_BAD_INPUT)
        })
    }
}
