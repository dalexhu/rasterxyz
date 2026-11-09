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

`cargo test --locked` checks the CLI output directory safeguards. The CI matrix builds and tests the Rust CLI on all three operating systems, then runs GDAL against a synthetic raster to check XYZ paths, coordinate alignment and transparent edges. With Python and GDAL on PATH, run `python tests/gdal_e2e.py` after the release build to repeat this check locally.

[GDAL raster tile documentation](https://gdal.org/en/stable/programs/gdal_raster_tile.html) · [GDAL downloads](https://gdal.org/en/stable/download.html)

## Experimental native backend

Pass `--native` to tile JPEG or PNG with the Rust backend. This mode does not
launch GDAL or require a separate projection library or data installation.
Use `--workers N` to set the native tile worker count. The default is 1.
Workers share the decoded raster and each active task has its own tile buffer.

```sh
cargo run --release --locked -- survey.png survey-tiles --native --workers 8 --min-zoom 14 --max-zoom 17
```

Keep a `.jgw` (JPEG), `.pgw` (PNG), or `.wld` beside the image. Provide
CRS metadata in a matching `.prj` or in the `SRS` element of an
image-specific `.aux.xml`. The prototype recognizes the WKT names for NAD83 /
UTM zone 12N and geographic WGS 84. For the existing compound JPEG metadata,
it uses the horizontal UTM component. It does not interpret arbitrary WKT
projection parameters or datum shifts.

The backend samples source pixels with nearest-neighbor resampling at all
zoom levels. Pixels outside the source footprint are transparent and source
PNG alpha is preserved. The native output can differ from GDAL's default
resampling around image features and boundaries. The source is decoded into
memory, with a 2 GiB decode buffer budget. Indexed PNG retains its palette
indices in memory rather than expanding the full image to RGBA.

With a release build and GDAL on PATH, run `python tests/native_e2e.py` to
compare both backends with synthetic JPEG and PNG inputs. The script generates
its inputs and output in temporary directories.

### Forthills benchmark

Measured on arm64 macOS with a local copy of the same 538 MiB indexed PNG
(33,388 × 41,579 pixels), its world file, and NAD83 / UTM zone 12N metadata.
Each run tiled the full footprint at a single zoom. Both backends used
nearest-neighbor sampling, 256 × 256 Web Mercator XYZ PNG tiles, compression
level 3, and transparent edges. Timings are single warm-cache runs. Peak RSS
is the largest sum for the tiler and its child processes sampled every 50 ms.
Shared memory pages can be counted more than once in that sum.

GDAL 3.13.3 rejected the indexed PNG with its default cubic resampling.
With nearest-neighbor sampling and `--add-alpha`, the direct indexed input
produced grayscale-plus-alpha tiles containing palette indices. For the
color-preserving comparison below, GDAL read an RGBA VRT referencing the
original PNG. The VRT was prepared outside the timed runs.

```sh
gdal_translate -of VRT -expand rgba source.png source-rgba.vrt
gdal raster tile --resampling nearest --tiling-scheme WebMercatorQuad \
  --convention xyz --min-zoom 17 --max-zoom 17 --tile-size 256 \
  --format PNG --add-alpha --skip-blank --co ZLEVEL=3 --webviewer none \
  source-rgba.vrt tiles-gdal
rasterxyz source.png tiles-rust --native --min-zoom 17 --max-zoom 17 --png-compression 3
```

GDAL needs the source CRS in readable metadata before creating the VRT.
For this run a temporary PAM `.aux.xml` supplied the same WKT as the
original `.prj`.

| Zoom | Backend | Wall time | Peak summed RSS | Tiles | PNG bytes (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| 14 | Native Rust | 4.61 s | 1.30 GiB | 171 | 21.15 |
| 14 | GDAL RGBA VRT, default parallelism | 14.59 s | 2.48 GiB | 171 | 21.37 |
| 17 | Native Rust | 121.96 s | 1.30 GiB | 9,179 | 948.94 |
| 17 | GDAL RGBA VRT, default parallelism | 10.07 s | 2.99 GiB | 9,179 | 980.33 |
| 17 | GDAL RGBA VRT, one worker | 70.39 s | 1.97 GiB | 9,179 | 980.33 |

The default-parallelism runs produced identical XYZ path sets at both zooms.
Decoding all corresponding tiles to RGBA found 46 alpha differences out of
11,206,656 pixels at zoom 14 and 41 out of 601,554,944 at zoom 17.
Among pixels visible in both outputs, RGB differed at 437,428 of 9,172,782
pixels (4.77%) at zoom 14 and 436,514 of 587,060,833 (0.074%) at zoom 17.
The output is close in coverage, with measurable color differences.

The native zoom 14 run spent 2.12 s decoding, 1.78 s reprojecting and
sampling, and 0.50 s encoding and writing. At zoom 17 these stages took
2.14 s, 94.27 s, and 25.54 s respectively. Per-pixel reprojection and sampling
accounted for 77% of the native zoom 17 run. GDAL's 17-process run reduced
wall time substantially at a higher aggregate RSS. Its one-worker run was
also faster than native at zoom 17, so parallelism explains only part of
the gap. GDAL stage timings were not measured separately.

#### Native tile workers at zoom 17

Retested on the same arm64 macOS host with the same local Forthills PNG,
nearest-neighbor sampling, and PNG compression level 3. Runs used the same
release build with different `--workers` values. Peak RSS was measured with
macOS `/usr/bin/time -l` for the single multithreaded process.

| Native workers | Wall time | Peak RSS | Tiles |
| ---: | ---: | ---: | ---: |
| 1 | 123.64 s | 1,329.25 MiB | 9,179 |
| 8 | 17.84 s | 1,337.08 MiB | 9,179 |
| 8 (repeat) | 17.79 s | 1,338.22 MiB | 9,179 |

Eight workers reduced wall time by about 6.9× with about 9 MiB additional
peak RSS. Both 8-worker runs and the 1-worker run matched the pre-parallel
output byte for byte across all 9,179 PNGs and their XYZ paths. Each run
produced 995,033,652 bytes of PNG data. Tile scheduling changed while the
pixel sampling and PNG settings stayed the same.
