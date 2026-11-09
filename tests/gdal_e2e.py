"""Exercise the release binary using a synthetic georeferenced raster."""

import math
import os
from pathlib import Path
import subprocess
import tempfile


def make_raster(directory):
    size = 128
    row = bytes((255, 0, 0)) * (size // 2) + bytes((0, 255, 0)) * (size // 2)
    (directory / "pixels.raw").write_bytes(row * size)
    bands = []
    for index, color in enumerate(("Red", "Green", "Blue")):
        bands.append(f"""
        <VRTRasterBand dataType="Byte" band="{index + 1}" subClass="VRTRawRasterBand">
          <ColorInterp>{color}</ColorInterp>
          <SourceFilename relativeToVRT="1">pixels.raw</SourceFilename>
          <ImageOffset>{index}</ImageOffset>
          <PixelOffset>3</PixelOffset>
          <LineOffset>{size * 3}</LineOffset>
        </VRTRasterBand>""")
    source = directory / "synthetic.vrt"
    source.write_text(f"""
    <VRTDataset rasterXSize="{size}" rasterYSize="{size}">
      <SRS>EPSG:4326</SRS>
      <GeoTransform>-70,0.3125,0,55,0,-0.234375</GeoTransform>
      {''.join(bands)}
    </VRTDataset>
    """.strip(), encoding="utf-8")
    return source


def read_pixel(tiles, zoom, lon, lat):
    world_size = 256 * 2**zoom
    x = int(world_size * (lon + 180) / 360)
    y = int(world_size * (1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2)
    tile = tiles / str(zoom) / str(x // 256) / f"{y // 256}.png"
    output = subprocess.check_output(
        ["gdallocationinfo", "-valonly", str(tile), str(x % 256), str(y % 256)],
        text=True,
    )
    return tuple(map(int, output.split()))


def main():
    binary = Path(__file__).resolve().parents[1] / "target" / "release" / (
        "rasterxyz.exe" if os.name == "nt" else "rasterxyz"
    )
    subprocess.run(["gdal", "--version"], check=True)
    with tempfile.TemporaryDirectory(prefix="rasterxyz-e2e-") as temp:
        directory = Path(temp)
        source = make_raster(directory)
        tiles = directory / "tiles"
        subprocess.run(
            [str(binary), str(source), str(tiles), "--min-zoom", "2", "--max-zoom", "3"],
            check=True,
        )
        expected = {"2/1/1.png", "3/2/2.png", "3/2/3.png", "3/3/2.png", "3/3/3.png"}
        actual = {path.relative_to(tiles).as_posix() for path in tiles.rglob("*.png")}
        assert actual == expected, f"Expected XYZ paths {expected}, got {actual}"

        # The image spans -70 to -30 longitude and 25 to 55 latitude.
        for zoom in (2, 3):
            for lat in (34, 48):
                for lon, expected_pixel in ((-60, (255, 0, 0, 255)), (-40, (0, 255, 0, 255))):
                    pixel = read_pixel(tiles, zoom, lon, lat)
                    assert pixel == expected_pixel, (zoom, lon, lat, pixel, expected_pixel)
            for lon, lat in ((-75, 48), (-25, 34), (-60, 57), (-40, 23)):
                pixel = read_pixel(tiles, zoom, lon, lat)
                assert len(pixel) == 4 and pixel[3] == 0, (zoom, lon, lat, pixel)

    print("Passed 5 XYZ path, 8 coordinate alignment, and 8 transparency checks")


if __name__ == "__main__":
    main()
