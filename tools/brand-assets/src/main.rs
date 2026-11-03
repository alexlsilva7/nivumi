//! Development-only SVG rasterizer and multi-frame ICO writer. Not linked into Nivumi.
use resvg::{tiny_skia, usvg};
use std::{error::Error, fs, path::Path};

const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

fn raster(svg: &str, size: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())?;
    let mut pixels = tiny_skia::Pixmap::new(size, size).ok_or("Invalid size")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(size as f32 / 24., size as f32 / 24.),
        &mut pixels.as_mut(),
    );
    Ok(pixels.encode_png()?)
}

fn ico(path: &Path, frames: &[(u32, Vec<u8>)]) -> std::io::Result<()> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    let mut offset = 6 + frames.len() * 16;
    for (size, png) in frames {
        out.extend_from_slice(&[*size as u8, *size as u8, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in frames {
        out.extend_from_slice(png);
    }
    fs::write(path, out)
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let brand = root.join("assets/brand");
    let mark = fs::read_to_string(brand.join("mark.svg"))?;
    let small = fs::read_to_string(brand.join("mark-small.svg"))?;
    let mut app = Vec::new();
    let mut tray = Vec::new();
    for size in SIZES {
        let source = if size <= 24 { &small } else { &mark };
        let paths = source
            .split_once('>')
            .ok_or("Invalid SVG")?
            .1
            .trim_end()
            .trim_end_matches("</svg>");
        let tile = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="5.5" fill="#000000"/><g transform="translate(2.4 2.4) scale(0.8)">{paths}</g></svg>"##
        );
        let png = raster(&tile, size)?;
        fs::write(brand.join(format!("icon-{size}.png")), &png)?;
        app.push((size, png));
        tray.push((size, raster(source, size)?));
        if size == 256 {
            fs::write(brand.join("icon.svg"), tile)?;
        }
    }
    ico(&brand.join("app.ico"), &app)?;
    ico(&brand.join("tray.ico"), &tray)?;
    println!("Generated Nivumi ICOs (9 frames each), SVG tile and PNGs.");
    Ok(())
}
