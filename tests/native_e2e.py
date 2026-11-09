"""Compare native JPEG/PNG sidecar tiling with the GDAL backend."""

import os
from pathlib import Path
import subprocess
import tempfile

from gdal_e2e import read_pixel


def check(binary, folder, kind, crs, transform, samples, sidecar_crs=None):
    folder.mkdir()
    size = 128
    channels = bytes((255, 0, 0)) * 64 + bytes((0, 255, 0)) * 64
    ppm = folder / "source.ppm"
    ppm.write_bytes(b"P6\n128 128\n255\n" + channels * size)
    source = folder / f"raster.{kind}"
    subprocess.run(["gdal_translate", "-q", str(ppm), str(source)], check=True)
    (folder / f"raster.{'jgw' if kind == 'jpg' else 'pgw'}").write_text(
        "\n".join(map(str, transform)), encoding="utf-8"
    )
    wkt = subprocess.check_output(["gdalsrsinfo", "-o", "wkt2" if sidecar_crs else "wkt_esri", sidecar_crs or crs], text=True).strip()
    if kind == "jpg":
        Path(f"{source}.aux.xml").write_text(
            f"<PAMDataset><SRS>{wkt}</SRS></PAMDataset>", encoding="utf-8"
        )
    else:
        source.with_suffix(".prj").write_text(wkt, encoding="utf-8")
    baseline = folder / "reference.tif"
    subprocess.run(
        ["gdal_translate", "-q", "-a_srs", crs, str(source), str(baseline)], check=True
    )

    def tiles(path):
        return {p.relative_to(path).as_posix() for p in path.rglob("*.png")}

    zoom = 2 if crs == "EPSG:4326" else 11
    options = ["--min-zoom", str(zoom), "--max-zoom", str(zoom + 1)]
    native, reference = folder / "native", folder / "reference"
    subprocess.run(
        [str(binary), str(source), str(native), "--native", *options],
        env={**os.environ, "RASTERXYZ_GDAL": str(folder / "nonexistent-gdal")},
        check=True,
    )
    parallel = folder / "parallel"
    subprocess.run(
        [str(binary), str(source), str(parallel), "--native", "--workers", "4", *options],
        env={**os.environ, "RASTERXYZ_GDAL": str(folder / "nonexistent-gdal")},
        check=True,
    )
    assert tiles(native) == tiles(parallel)
    for path in tiles(native):
        assert (native / path).read_bytes() == (parallel / path).read_bytes(), path
    subprocess.run([str(binary), str(baseline), str(reference), *options], check=True)
    assert tiles(native) == tiles(reference) and tiles(native), (tiles(native), tiles(reference))
    for z in (zoom, zoom + 1):
        for lon, lat, expected in samples:
            a = read_pixel(native, z, lon, lat)
            b = read_pixel(reference, z, lon, lat)
            assert len(a) == len(b) == 4, (z, a, b)
            assert a[3] == b[3] == (0 if expected is None else 255), (z, lon, lat, a, b)
            if expected is not None:
                assert a[expected] >= 245 and b[expected] >= 245, (z, lon, lat, a, b)
                assert max(abs(x - y) for x, y in zip(a, b)) <= 4, (z, lon, lat, a, b)
    print(f"{kind}: {len(tiles(native))} matching XYZ paths, 8 matching RGBA samples", flush=True)


def main():
    binary = Path(__file__).resolve().parents[1] / "target" / "release" / (
        "rasterxyz.exe" if os.name == "nt" else "rasterxyz"
    )
    with tempfile.TemporaryDirectory(prefix="rasterxyz-native-") as tmp:
        folder = Path(tmp)
        check(
            binary, folder / "jpeg", "jpg", "EPSG:4326",
            (0.3125, 0, 0, -0.234375, -70 + 0.3125 / 2, 55 - 0.234375 / 2),
            [(-60, 48, 0), (-40, 34, 1), (-75, 48, None), (-25, 34, None)],
        )
        # Independently convert sample locations from UTM to lon/lat with GDAL.
        points = [(493000, 6380000, 0), (507000, 6380000, 1),
                  (486300, 6380000, None), (513000, 6380000, None)]
        locations = subprocess.check_output(
            ["gdaltransform", "-s_srs", "EPSG:26912", "-t_srs", "EPSG:4326"],
            input="".join(f"{x} {y}\n" for x, y, _ in points), text=True,
        )
        samples = [(*map(float, line.split()[:2]), point[2])
                   for line, point in zip(locations.splitlines(), points, strict=True)]
        check(
            binary, folder / "compound-jpeg", "jpg", "EPSG:26912",
            (200, 0, 0, -200, 487300, 6392700), samples,
            sidecar_crs="EPSG:26912+5703",
        )
        check(
            binary, folder / "png", "png", "EPSG:26912",
            (200, 0, 0, -200, 487300, 6392700), samples,
        )


if __name__ == "__main__":
    main()
