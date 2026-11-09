use std::{fs, process::Command};

#[test]
fn rejects_existing_output_without_touching_contents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.tif");
    let output = temp.path().join("tiles");
    fs::write(&source, b"fixture").unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), b"original").unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_rasterxyz"))
        .arg(&source)
        .arg(&output)
        .status()
        .unwrap();

    assert!(!status.success());
    assert_eq!(fs::read(output.join("keep")).unwrap(), b"original");
}

#[test]
fn cleans_up_output_when_gdal_cannot_start() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.tif");
    let output = temp.path().join("tiles");
    fs::write(&source, b"fixture").unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_rasterxyz"))
        .env("RASTERXYZ_GDAL", temp.path().join("nonexistent-gdal"))
        .arg(&source)
        .arg(&output)
        .status()
        .unwrap();

    assert!(!status.success());
    assert!(!output.exists());
}
