"""Package a host release binary and smoke-test the extracted archive."""

import argparse
import hashlib
import os
from pathlib import Path
import struct
import subprocess
import tarfile
import tempfile
import zipfile
import zlib


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def check_native(binary, work, version):
    # Use absolute paths and an empty executable search directory. Also point
    # the application's explicit GDAL override at a missing executable.
    empty = work / "empty"
    empty.mkdir()
    env = {**os.environ, "PATH": str(empty), "RASTERXYZ_GDAL": str(empty / "missing-gdal")}
    actual = subprocess.check_output([str(binary), "--version"], cwd=work, env=env, text=True)
    if actual.strip() != version:
        raise RuntimeError(f"Unexpected version: {actual.strip()}")

    source = work / "synthetic.png"
    red_row = b"\0" + bytes((255, 0, 0)) * 16
    source.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">2I5B", 16, 16, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(red_row * 16))
        + chunk(b"IEND", b"")
    )
    source.with_suffix(".pgw").write_text("1\n0\n0\n-1\n-7.5\n7.5\n", encoding="ascii")
    source.with_suffix(".prj").write_text(
        'GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],'
        'PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]',
        encoding="ascii",
    )
    tiles = work / "tiles"
    subprocess.run(
        [str(binary), str(source), str(tiles), "--native", "--workers", "2",
         "--min-zoom", "0", "--max-zoom", "0"],
        cwd=work, env=env, check=True,
    )
    pngs = {path.relative_to(tiles).as_posix() for path in tiles.rglob("*.png")}
    if pngs != {"0/0/0.png"}:
        raise RuntimeError(f"Unexpected XYZ output: {pngs}")
    data = (tiles / "0/0/0.png").read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">2I5B", data[16:29]) != (
        256, 256, 8, 6, 0, 0, 0
    ):
        raise RuntimeError("Expected a 256 × 256 RGBA PNG tile")
    print(f"{version}: extracted binary generated XYZ output without GDAL")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", help="Rust target triple for a host executable")
    target = parser.parse_args().target
    root = Path(__file__).resolve().parents[1]
    windows = "windows" in target
    binary = root / "target" / target / "release" / ("rasterxyz.exe" if windows else "rasterxyz")
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    name = f"{version.replace(' ', '-')}-{target}"
    suffix = ".zip" if windows else ".tar.gz"
    with tempfile.TemporaryDirectory(prefix="rasterxyz-package-") as directory:
        work = Path(directory).resolve()
        archive = work / (name + suffix)
        files = [binary, root / "README.md", root / "LICENSE"]
        extracted = work / "extracted"
        if windows:
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as package:
                for file in files:
                    package.write(file, arcname=file.name)
            with zipfile.ZipFile(archive) as package:
                package.extractall(extracted)
        else:
            with tarfile.open(archive, "w:gz") as package:
                for file in files:
                    package.add(file, arcname=file.name)
            with tarfile.open(archive) as package:
                package.extractall(extracted, filter="data")

        check_native(extracted / binary.name, work, version)
        dist = root / "dist"
        dist.mkdir(exist_ok=True)
        data = archive.read_bytes()
        (dist / archive.name).write_bytes(data)
        (dist / (archive.name + ".sha256")).write_text(
            f"{hashlib.sha256(data).hexdigest()}  {archive.name}\n", encoding="ascii"
        )


if __name__ == "__main__":
    main()
