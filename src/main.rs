mod crs;
mod native;

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
    /// Use the experimental pure Rust JPEG/PNG tiler.
    #[arg(long)]
    native: bool,
    /// Number of native tile workers (defaults to 1).
    #[arg(long, requires = "native", value_parser = clap::value_parser!(u16).range(1..))]
    workers: Option<u16>,
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
    // GDAL does not consistently handle the Windows verbatim paths from canonicalize.
    let input = std::path::absolute(&options.input)?;
    if !input.is_file() {
        return Err("input must be a raster file".into());
    }

    let output = std::path::absolute(&options.output)?;
    // Reserve the target so concurrent runs cannot publish to the same directory.
    fs::create_dir(&output)?;

    let result = (|| {
        let staging = tempfile::Builder::new()
            .prefix(".rasterxyz-")
            .tempdir_in(&output)?;
        let tiles = staging.path().join("tiles");
        if options.native {
            native::render(
                &input,
                &tiles,
                options.min_zoom,
                options.max_zoom,
                options.png_compression,
                usize::from(options.workers.unwrap_or(1)),
            )?;
        } else {
            let executable =
                env::var_os("RASTERXYZ_GDAL").unwrap_or_else(|| OsString::from("gdal"));
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
        }
        publish_tiles(&tiles, &output)?;
        Ok(())
    })();
    if result.is_err() {
        // Only remove the reservation when it is empty.
        let _ = fs::remove_dir(&output);
    }
    result
}

fn publish_tiles(tiles: &std::path::Path, output: &std::path::Path) -> io::Result<()> {
    let mut entries = fs::read_dir(tiles)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_unstable();
    for (index, name) in entries.iter().enumerate() {
        let destination = output.join(name);
        let moved = match fs::symlink_metadata(&destination) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "tile destination already exists",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::rename(tiles.join(name), destination)
            }
            Err(error) => Err(error),
        };
        if let Err(error) = moved {
            let mut rollback_error = None;
            for previous in entries[..index].iter().rev() {
                if let Err(error) = fs::rename(output.join(previous), tiles.join(previous)) {
                    rollback_error.get_or_insert(error);
                }
            }
            return Err(match rollback_error {
                Some(rollback) => io::Error::other(format!(
                    "tile publication failed: {error}; rollback failed: {rollback}"
                )),
                None => error,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_publication_rolls_back_moved_levels_and_can_retry() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let staged = temp.path().join("staged");
        let output = temp.path().join("output");
        fs::create_dir_all(staged.join("14"))?;
        fs::create_dir_all(staged.join("17"))?;
        fs::write(staged.join("14/fixture"), b"staged")?;
        fs::create_dir(&output)?;
        fs::write(output.join("17"), b"preexisting")?;

        let error = publish_tiles(&staged, &output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!output.join("14").exists());
        assert_eq!(fs::read(output.join("17"))?, b"preexisting");
        assert_eq!(fs::read(staged.join("14/fixture"))?, b"staged");

        fs::remove_file(output.join("17"))?;
        publish_tiles(&staged, &output)?;
        assert_eq!(fs::read(output.join("14/fixture"))?, b"staged");
        assert!(output.join("17").is_dir());
        Ok(())
    }
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
