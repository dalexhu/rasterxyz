# RasterXYZ

RasterXYZ is a Rust command-line tool that generates Web Mercator XYZ PNG tiles from georeferenced aerial imagery. GDAL handles raster decoding, reprojection, resampling and PNG encoding.

## Build and run

Build with stable Rust on Windows, macOS or Linux.

```sh
cargo build --release --locked
```

Install GDAL 3.11 or newer for your operating system and verify that `gdal raster tile --help` runs. The CLI looks for `gdal` on PATH. Set `RASTERXYZ_GDAL` to a full executable path if needed.

```sh
cargo run --release --locked -- survey.tif survey-tiles --min-zoom 9 --max-zoom 17 --png-compression 3
```

GeoTIFF can carry its georeferencing internally. For JPEG or PNG, keep the corresponding world file and any GDAL-readable CRS metadata beside the image. A world file supplies the pixel-to-map transform. The source also needs a coordinate reference system.

Tiles are written as `z/x/y.png` with 256-pixel edges, a north-origin Y axis and transparent areas outside the source coverage. Fully blank tiles are skipped. PNG compression defaults to 3. Choose a new output directory whose parent exists. Rerunning against an existing output directory fails without replacing it. Use the output only after the process exits successfully.

`cargo test --locked` checks the CLI output directory safeguards. The CI matrix builds and tests the Rust CLI on all three operating systems.

[GDAL raster tile documentation](https://gdal.org/en/stable/programs/gdal_raster_tile.html) · [GDAL downloads](https://gdal.org/en/stable/download.html)
