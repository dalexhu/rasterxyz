use clap::Parser;
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::PathBuf,
    process::{Command, ExitCode},
};

#[derive(Parser)]
#[command(version, about)]
struct Options {
    /// Georeferenced raster, such as GeoTIFF or JPEG/PNG with GDAL-readable sidecars.
    input: PathBuf,
    /// New output directory. Its parent must exist.
    output: PathBuf,
    #[arg(long, default_value_t = 9, value_parser = clap::value_parser!(u8).range(0..=23))]
    min_zoom: u8,
    #[arg(long, default_value_t = 17, value_parser = clap::value_parser!(u8).range(0..=23))]
    max_zoom: u8,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=9))]
    png_compression: u8,
}

fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    if options.min_zoom > options.max_zoom {
        return Err("--min-zoom must be at most --max-zoom".into());
    }
    let input = options.input.canonicalize()?;
    if !input.is_file() {
        return Err("input must be a raster file".into());
    }

    let output = env::current_dir()?.join(&options.output);
    let name = output
        .file_name()
        .ok_or("output must name a new directory")?;
    let parent = output
        .parent()
        .ok_or("output must have a parent")?
        .canonicalize()?;
    let output = parent.join(name);
    // Reserve the target so concurrent runs cannot publish to the same directory.
    fs::create_dir(&output)?;

    let result = (|| {
        let staging = tempfile::Builder::new()
            .prefix(".rasterxyz-")
            .tempdir_in(&output)?;
        let tiles = staging.path().join("tiles");
        let executable = env::var_os("RASTERXYZ_GDAL").unwrap_or_else(|| OsString::from("gdal"));
        let status = Command::new(executable)
            .args([
                "raster",
                "tile",
                "--tiling-scheme",
                "WebMercatorQuad",
                "--convention",
                "xyz",
            ])
            .args(["--min-zoom", &options.min_zoom.to_string()])
            .args(["--max-zoom", &options.max_zoom.to_string()])
            .args([
                "--tile-size",
                "256",
                "--format",
                "PNG",
                "--add-alpha",
                "--skip-blank",
            ])
            .args([
                "--co",
                &format!("ZLEVEL={}", options.png_compression),
                "--webviewer",
                "none",
            ])
            .arg(input)
            .arg(&tiles)
            .status()
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not launch GDAL 3.11+: {error}"),
                )
            })?;
        if !status.success() {
            return Err(format!("GDAL tile generation failed ({status})").into());
        }
        for entry in fs::read_dir(tiles)? {
            let entry = entry?;
            fs::rename(entry.path(), output.join(entry.file_name()))?;
        }
        Ok(())
    })();
    if result.is_err() {
        // Only remove the reservation when it is empty.
        let _ = fs::remove_dir(&output);
    }
    result
}

fn main() -> ExitCode {
    match run(Options::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rasterxyz: {error}");
            ExitCode::FAILURE
        }
    }
}
