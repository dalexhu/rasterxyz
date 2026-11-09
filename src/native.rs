use image::ImageReader;
use png::{BitDepth, ColorType};
use proj4rs::{adaptors::transform_xy, Proj};
use rayon::prelude::*;
use std::{
    error::Error,
    ffi::OsString,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::Path,
};

const TILE: usize = 256;
const MAX_DECODE_BYTES: usize = 2 * 1024 * 1024 * 1024;
const HALF_WORLD: f64 = 20_037_508.342_789_244;

struct Raster {
    width: usize,
    height: usize,
    stride: usize,
    color: ColorType,
    bits: usize,
    data: Vec<u8>,
    palette: Vec<[u8; 4]>,
}

impl Raster {
    fn jpeg(path: &Path) -> Result<Self, Box<dyn Error>> {
        let mut reader = ImageReader::open(path)?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(MAX_DECODE_BYTES as u64);
        reader.limits(limits);
        let image = reader.decode()?.into_rgb8();
        let (width, height) = (image.width() as usize, image.height() as usize);
        Ok(Self {
            width,
            height,
            stride: width * 3,
            color: ColorType::Rgb,
            bits: 8,
            data: image.into_raw(),
            palette: Vec::new(),
        })
    }

    fn png(path: &Path) -> Result<Self, Box<dyn Error>> {
        let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
        let header = decoder.read_header_info()?;
        if header.color_type != ColorType::Indexed {
            decoder.set_transformations(png::Transformations::normalize_to_color8());
        }
        let mut reader = decoder.read_info()?;
        let info = reader.info();
        let alpha = info.trns.as_deref().unwrap_or_default();
        let palette = info
            .palette
            .as_deref()
            .unwrap_or_default()
            .chunks_exact(3)
            .enumerate()
            .map(|(i, rgb)| [rgb[0], rgb[1], rgb[2], alpha.get(i).copied().unwrap_or(255)])
            .collect();
        let bytes = reader
            .output_buffer_size()
            .filter(|&size| size <= MAX_DECODE_BYTES)
            .ok_or("decoded PNG exceeds the prototype's 2 GiB buffer limit")?;
        let mut data = Vec::new();
        data.try_reserve_exact(bytes)?;
        data.resize(bytes, 0);
        let frame = reader.next_frame(&mut data)?;
        Ok(Self {
            width: frame.width as usize,
            height: frame.height as usize,
            stride: frame.line_size,
            color: frame.color_type,
            bits: frame.bit_depth as usize,
            data,
            palette,
        })
    }

    fn pixel(&self, col: usize, row: usize) -> [u8; 4] {
        let line = &self.data[row * self.stride..][..self.stride];
        let offset = col * self.color.samples();
        match self.color {
            ColorType::Indexed => {
                let bit = col * self.bits;
                let shift = 8 - self.bits - bit % 8;
                let index = (line[bit / 8] >> shift) as usize & ((1 << self.bits) - 1);
                self.palette.get(index).copied().unwrap_or([0; 4])
            }
            ColorType::Grayscale => [line[offset], line[offset], line[offset], 255],
            ColorType::GrayscaleAlpha => {
                [line[offset], line[offset], line[offset], line[offset + 1]]
            }
            ColorType::Rgb => [line[offset], line[offset + 1], line[offset + 2], 255],
            ColorType::Rgba => line[offset..offset + 4].try_into().unwrap(),
        }
    }
}

fn read_projection(path: &Path) -> Result<Proj, Box<dyn Error>> {
    let mut aux = OsString::from(path.as_os_str());
    aux.push(".aux.xml");
    let text = match fs::read_to_string(path.with_extension("prj")) {
        Ok(wkt) => wkt,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let xml = fs::read_to_string(aux)?;
            let doc = roxmltree::Document::parse(&xml)?;
            doc.root_element()
                .children()
                .find(|node| node.has_tag_name("SRS"))
                .and_then(|node| node.text())
                .ok_or("missing SRS in PAM XML")?
                .to_owned()
        }
        Err(error) => return Err(error.into()),
    };

    // The existing PAM files use compound CRS metadata. Tiling uses its horizontal
    // projected CRS, so the vertical component has no effect on pixel coordinates.
    let text = text.trim_start();
    let horizontal = if text.starts_with("COMPOUNDCRS[") || text.starts_with("COMPD_CS[") {
        ["PROJCRS[", "PROJCS["]
            .into_iter()
            .filter_map(|prefix| text.find(prefix))
            .min()
            .map(|index| &text[index..])
            .ok_or("compound CRS has no supported projected component")?
    } else {
        text
    };
    let name = horizontal
        .split_once('"')
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(name, _)| {
            name.chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let proj = match name.as_str() {
        "nad83utmzone12n" | "nad1983utmzone12n" => "+proj=utm +zone=12 +ellps=GRS80 +towgs84=0,0,0",
        "wgs84" | "gcswgs1984" if horizontal.starts_with("GEOG") => "+proj=longlat +datum=WGS84",
        _ => {
            return Err(
                "unsupported CRS; expected NAD83 / UTM zone 12N or geographic WGS 84".into(),
            )
        }
    };
    Ok(Proj::from_proj_string(proj)?)
}

pub fn render(
    input: &Path,
    output: &Path,
    min_zoom: u8,
    max_zoom: u8,
    compression: u8,
    workers: usize,
) -> Result<(), Box<dyn Error>> {
    let png_input = match input
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => true,
        Some("jpg" | "jpeg") => false,
        _ => return Err("native backend expects JPEG or PNG".into()),
    };
    let world = input.with_extension(if png_input { "pgw" } else { "jgw" });
    let coefficients = match fs::read_to_string(world) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::read_to_string(input.with_extension("wld"))?
        }
        Err(error) => return Err(error.into()),
    };
    let values = coefficients
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()?;
    let [a, d, b, e, c, f]: [f64; 6] = values
        .try_into()
        .map_err(|_| "world file must contain six numbers")?;
    let det = a * e - b * d;
    if ![a, d, b, e, c, f, det].iter().all(|v| v.is_finite()) || det == 0.0 {
        return Err("invalid world-file affine transform".into());
    }
    let source = read_projection(input)?;
    let mercator = Proj::from_proj_string("+proj=merc +a=6378137 +b=6378137 +units=m")?;
    let decode_start = std::time::Instant::now();
    let raster = if png_input {
        Raster::png(input)?
    } else {
        Raster::jpeg(input)?
    };
    eprintln!(
        "Decoded {}×{} in {:.2}s",
        raster.width,
        raster.height,
        decode_start.elapsed().as_secs_f64()
    );

    let (mut west, mut east, mut south, mut north) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    // World files reference pixel centers. Sample the outer edges for the tile envelope.
    for step in 0..=128 {
        let t = f64::from(step) / 128.0;
        let (w, h) = (raster.width as f64, raster.height as f64);
        for (col, row) in [(w * t, 0.0), (w * t, h), (0.0, h * t), (w, h * t)] {
            let (col, row) = (col - 0.5, row - 0.5);
            let x = a * col + b * row + c;
            let y = d * col + e * row + f;
            let (x, y) = if source.is_latlong() {
                (x.to_radians(), y.to_radians())
            } else {
                (x, y)
            };
            let (x, y) = transform_xy(&source, &mercator, x, y)?;
            west = west.min(x);
            east = east.max(x);
            south = south.min(y);
            north = north.max(y);
        }
    }
    if ![west, east, south, north].iter().all(|v| v.is_finite())
        || east <= -HALF_WORLD
        || west >= HALF_WORLD
        || north <= -HALF_WORLD
        || south >= HALF_WORLD
    {
        return Err("raster does not intersect the Web Mercator world".into());
    }

    fs::create_dir(output)?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()?;
    for zoom in min_zoom..=max_zoom {
        let count = 1_u32 << zoom;
        let span = 2.0 * HALF_WORLD / f64::from(count);
        let index = |v: f64| (v / span).floor().clamp(0.0, f64::from(count - 1)) as u32;
        let scale = span / TILE as f64;
        let min_x = index(west + HALF_WORLD);
        let max_x = index(east + HALF_WORLD);
        let min_y = index(HALF_WORLD - north);
        let rows = u64::from(index(HALF_WORLD - south) - min_y + 1);
        let jobs = u64::from(max_x - min_x + 1) * rows;
        let result = pool.install(|| {
            (0..jobs).into_par_iter().try_for_each_init(
                || vec![0; TILE * TILE * 4],
                |rgba, job| -> Result<(), Box<dyn Error + Send + Sync>> {
                    let x = min_x + (job / rows) as u32;
                    let y = min_y + (job % rows) as u32;
                    let mut nonempty = false;
                    for (i, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                        let mx =
                            f64::from(x) * span - HALF_WORLD + ((i % TILE) as f64 + 0.5) * scale;
                        let my =
                            HALF_WORLD - f64::from(y) * span - ((i / TILE) as f64 + 0.5) * scale;
                        pixel.fill(0);
                        if let Ok((sx, sy)) = transform_xy(&mercator, &source, mx, my) {
                            let (sx, sy) = if source.is_latlong() {
                                (sx.to_degrees(), sy.to_degrees())
                            } else {
                                (sx, sy)
                            };
                            let (dx, dy) = (sx - c, sy - f);
                            let col = ((e * dx - b * dy) / det + 0.5).floor();
                            let row = ((a * dy - d * dx) / det + 0.5).floor();
                            if col >= 0.0
                                && row >= 0.0
                                && col < raster.width as f64
                                && row < raster.height as f64
                            {
                                let color = raster.pixel(col as usize, row as usize);
                                if color[3] != 0 {
                                    pixel.copy_from_slice(&color);
                                    nonempty = true;
                                }
                            }
                        }
                    }
                    if nonempty {
                        let dir = output.join(zoom.to_string()).join(x.to_string());
                        fs::create_dir_all(&dir)?;
                        let mut encoder = png::Encoder::new(
                            BufWriter::new(File::create(dir.join(format!("{y}.png")))?),
                            TILE as u32,
                            TILE as u32,
                        );
                        encoder.set_color(ColorType::Rgba);
                        encoder.set_depth(BitDepth::Eight);
                        encoder
                            .set_deflate_compression(png::DeflateCompression::Level(compression));
                        let mut writer = encoder.write_header()?;
                        writer.write_image_data(rgba)?;
                        writer.finish()?;
                    }
                    Ok(())
                },
            )
        });
        result.map_err(|error| -> Box<dyn Error> { error })?;
    }
    Ok(())
}
