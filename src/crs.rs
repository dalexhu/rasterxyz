use proj4rs::Proj;
use std::error::Error;

// Read only the structural fields needed by the two supported CRS definitions.
fn fields(node: &str) -> Option<Vec<&str>> {
    let (_, body) = node.split_once('[')?;
    let body = body.strip_suffix(']')?;
    let mut fields = Vec::new();
    let (mut depth, mut quoted, mut start) = (0_usize, false, 0);
    for (index, ch) in body.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '[' if !quoted => depth += 1,
            ']' if !quoted => depth = depth.checked_sub(1)?,
            ',' if !quoted && depth == 0 => {
                fields.push(body[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || quoted {
        return None;
    }
    fields.push(body[start..].trim());
    Some(fields)
}

fn keyword(node: &str) -> &str {
    node.split_once('[').map_or("", |(key, _)| key.trim())
}

fn child<'a>(fields: &[&'a str], keys: &[&str]) -> Option<&'a str> {
    let mut matches = fields
        .iter()
        .copied()
        .filter(|node| keys.contains(&keyword(node)));
    let node = matches.next()?;
    matches.next().is_none().then_some(node)
}

fn name(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn equals(value: Option<&&str>, expected: f64) -> bool {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(|value| {
            value.is_finite() && (value - expected).abs() <= 1e-12 * expected.abs().max(1.0)
        })
}

fn unit(nodes: &[&str], keys: &[&str], factor: f64) -> bool {
    child(nodes, keys)
        .and_then(fields)
        .is_some_and(|values| equals(values.get(1), factor))
}

const DEGREE: f64 = std::f64::consts::PI / 180.0;

fn geographic(nodes: &[&str], nad83: bool) -> Option<()> {
    let datum = fields(child(nodes, &["DATUM", "GEODETICDATUM", "ENSEMBLE"])?)?;
    if has_extension(nodes) || has_extension(&datum) {
        return None;
    }
    let datum_name = name(datum.first()?);
    let supported = if nad83 {
        matches!(
            datum_name.as_str(),
            "dnorthamerican1983" | "northamericandatum1983" | "northamerican1983"
        )
    } else {
        matches!(
            datum_name.as_str(),
            "dwgs1984"
                | "wgs1984"
                | "wgs84"
                | "worldgeodeticsystem1984"
                | "worldgeodeticsystem1984ensemble"
        )
    };
    let ellipsoid = fields(child(&datum, &["SPHEROID", "ELLIPSOID"])?)?;
    let prime = fields(child(nodes, &["PRIMEM"])?)?;
    let flattening = if nad83 {
        298.257_222_101
    } else {
        298.257_223_563
    };
    let shift_ok = datum
        .iter()
        .filter(|node| keyword(node) == "TOWGS84")
        .all(|node| {
            fields(node).is_some_and(|values| {
                matches!(values.len(), 3 | 7) && values.iter().all(|value| equals(Some(value), 0.0))
            })
        });
    (supported
        && shift_ok
        && equals(ellipsoid.get(1), 6_378_137.0)
        && equals(ellipsoid.get(2), flattening)
        && equals(prime.get(1), 0.0)
        && (keyword(child(&datum, &["SPHEROID", "ELLIPSOID"])?) == "SPHEROID"
            || unit(&ellipsoid, &["LENGTHUNIT"], 1.0)))
    .then_some(())
}

fn axes_valid(nodes: &[&str], units: Option<(&str, f64)>) -> bool {
    let axes = nodes
        .iter()
        .filter(|node| keyword(node) == "AXIS")
        .map(|node| fields(node))
        .collect::<Option<Vec<_>>>();
    match axes.as_deref() {
        Some([]) => units.is_none(), // WKT1 permits omitting axes.
        Some([first, second]) => {
            let directions = (first.get(1).copied(), second.get(1).copied());
            matches!(
                directions,
                (Some("east" | "EAST"), Some("north" | "NORTH"))
                    | (Some("north" | "NORTH"), Some("east" | "EAST"))
            ) && units.is_none_or(|(key, factor)| {
                unit(first, &[key], factor) && unit(second, &[key], factor)
            })
        }
        _ => false,
    }
}

fn has_extension(nodes: &[&str]) -> bool {
    nodes.iter().any(|node| keyword(node) == "EXTENSION")
}

fn supported(node: &str) -> Option<&'static str> {
    let mut kind = keyword(node);
    let mut root = fields(node)?;
    if matches!(kind, "COMPOUNDCRS" | "COMPD_CS") {
        let horizontal = child(&root, &["PROJCRS", "PROJCS", "GEOGCRS", "GEOGCS"])?;
        kind = keyword(horizontal);
        root = fields(horizontal)?;
    }

    if has_extension(&root) {
        return None;
    }
    if matches!(kind, "GEOGCS" | "GEOGCRS") {
        geographic(&root, false)?;
        let degrees = if kind == "GEOGCS" {
            unit(&root, &["UNIT"], DEGREE) && axes_valid(&root, None)
        } else {
            axes_valid(&root, Some(("ANGLEUNIT", DEGREE)))
        };
        return degrees.then_some("+proj=longlat +datum=WGS84");
    }

    let (base, conversion, method, wkt2) = match kind {
        "PROJCS" if unit(&root, &["UNIT"], 1.0) && axes_valid(&root, None) => {
            let base = fields(child(&root, &["GEOGCS"])?)?;
            if !unit(&base, &["UNIT"], DEGREE) {
                return None;
            }
            let method = fields(child(&root, &["PROJECTION"])?)?;
            (base, root, method, false)
        }
        "PROJCRS" if axes_valid(&root, Some(("LENGTHUNIT", 1.0))) => {
            let base = fields(child(&root, &["BASEGEOGCRS", "BASEGEODCRS"])?)?;
            let conversion = fields(child(&root, &["CONVERSION"])?)?;
            let method = fields(child(&conversion, &["METHOD"])?)?;
            (base, conversion, method, true)
        }
        _ => return None,
    };
    geographic(&base, true)?;
    if name(method.first()?) != "transversemercator" {
        return None;
    }

    let mut seen = [false; 5];
    for node in conversion
        .iter()
        .filter(|node| keyword(node) == "PARAMETER")
    {
        let param = fields(node)?;
        let (index, value, unit_key, factor) = match name(param.first()?).as_str() {
            "latitudeoforigin" | "latitudeofnaturalorigin" => (0, 0.0, "ANGLEUNIT", DEGREE),
            "centralmeridian" | "longitudeofnaturalorigin" => (1, -111.0, "ANGLEUNIT", DEGREE),
            "scalefactor" | "scalefactoratnaturalorigin" => (2, 0.9996, "SCALEUNIT", 1.0),
            "falseeasting" => (3, 500_000.0, "LENGTHUNIT", 1.0),
            "falsenorthing" => (4, 0.0, "LENGTHUNIT", 1.0),
            _ => return None,
        };
        if seen[index]
            || !equals(param.get(1), value)
            || (wkt2 && !unit(&param, &[unit_key], factor))
        {
            return None;
        }
        seen[index] = true;
    }
    seen.into_iter()
        .all(|found| found)
        .then_some("+proj=utm +zone=12 +ellps=GRS80 +towgs84=0,0,0")
}

pub fn parse(wkt: &str) -> Result<Proj, Box<dyn Error>> {
    let definition = supported(wkt.trim().trim_start_matches('\u{feff}'))
        .ok_or("unsupported CRS definition; expected NAD83 / UTM zone 12N or geographic WGS 84")?;
    Ok(Proj::from_proj_string(definition)?)
}
