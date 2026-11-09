# RasterXYZ v0.1.0 release notes

RasterXYZ converts georeferenced rasters into 256 × 256 Web Mercator XYZ PNG
tiles. The first release includes an experimental pure Rust backend and a
GDAL backend for broader input format and CRS support.

- Use `--native` to tile JPEG or PNG without installing GDAL. Keep the world
  file and CRS sidecar beside the image. See the README for supported metadata.
- Use `--workers N` with `--native` to process multiple tiles in parallel.
  The default is one worker. Tile output is identical across worker counts.
- The default backend runs GDAL 3.11 or newer and requires it on PATH.
- Native inputs are limited to NAD83 / UTM zone 12N and geographic WGS 84.
  The native backend rejects imagery that crosses the antimeridian.

## Packages

| Platform | Architecture | Archive |
| --- | --- | --- |
| Windows | x64 | ZIP |
| macOS | Apple Silicon | TAR.GZ |
| macOS | Intel | TAR.GZ |
| Linux | x64 | TAR.GZ |

Unpack the archive and run `rasterxyz --help` (or `rasterxyz.exe --help` on
Windows). Run the tool in native mode using, for example:

```sh
./rasterxyz survey.png tiles --native --workers 4 --min-zoom 9 --max-zoom 17
```

The Linux package is built and tested on Ubuntu 22.04. macOS packages are
tested on macOS 15 and are unsigned. Each archive has a matching SHA-256
file. Archives include the binary, README and license.

## Maintainer checklist

1. Confirm both backend regressions on Windows, macOS and Linux for the
   candidate revision. Verify that Cargo.toml reports version 0.1.0.
2. Run **Release packages** for that revision and download its four Actions
   artifacts. Each packaging job checks the extracted binary with GDAL
   unavailable and uploads the archive and checksum after the check passes.
3. Check the four archive hashes and contents. Keep the archive filenames
   unchanged so the corresponding checksum files resolve.
4. Once publication is approved, create the GitHub Release for tag
   `v0.1.0` targeting the verified revision. Attach the four archives and
   their four checksum files, and use the notes above.
