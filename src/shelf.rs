//! Shelf selection and bounded image decoding, independent from the UI renderer.
use std::{collections::BTreeSet, path::Path, sync::mpsc};
use windows::{
    core::*,
    Win32::{Foundation::E_FAIL, Graphics::Imaging::*, System::Com::*},
};

pub fn select(
    ids: &[u64],
    selected: &mut BTreeSet<u64>,
    anchor: &mut Option<u64>,
    id: u64,
    control: bool,
    shift: bool,
) {
    if shift {
        if let (Some(a), Some(b)) = (
            anchor.and_then(|a| ids.iter().position(|v| *v == a)),
            ids.iter().position(|v| *v == id),
        ) {
            if !control {
                selected.clear();
            }
            selected.extend(ids[a.min(b)..=a.max(b)].iter().copied());
            return;
        }
    }
    if control {
        if !selected.remove(&id) {
            selected.insert(id);
        }
    } else {
        selected.clear();
        selected.insert(id);
    }
    *anchor = Some(id);
}

#[derive(Clone)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
pub struct ThumbnailWorker {
    pub tx: mpsc::SyncSender<(u64, String)>,
    pub rx: mpsc::Receiver<(u64, Option<Thumbnail>)>,
}
impl Default for ThumbnailWorker {
    fn default() -> Self {
        let (tx, requests) = mpsc::sync_channel::<(u64, String)>(32);
        let (results, rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return;
            }
            for (id, path) in requests {
                let image = thumbnail(Path::new(&path)).ok();
                if results.send((id, image)).is_err() {
                    break;
                }
            }
            CoUninitialize();
        });
        Self { tx, rx }
    }
}
pub fn is_image(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            [
                "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "ico", "heic", "avif",
            ]
            .contains(&e.to_ascii_lowercase().as_str())
        })
}
// Only called on the thumbnail worker's COM apartment.
unsafe fn thumbnail(path: &Path) -> Result<Thumbnail> {
    if std::fs::metadata(path)
        .map(|m| m.len() > 32 * 1024 * 1024)
        .unwrap_or(true)
    {
        return Err(E_FAIL.into());
    }
    let factory: IWICImagingFactory =
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let decoder = factory.CreateDecoderFromFilename(
        &HSTRING::from(path.as_os_str()),
        None,
        windows::Win32::Foundation::GENERIC_READ,
        WICDecodeMetadataCacheOnLoad,
    )?;
    let frame = decoder.GetFrame(0)?;
    let (mut width, mut height) = (0, 0);
    frame.GetSize(&mut width, &mut height)?;
    if width == 0 || height == 0 || width as u64 * height as u64 > 100_000_000 {
        return Err(E_FAIL.into());
    }
    let ratio = (144. / width.max(height) as f64).min(1.);
    width = (width as f64 * ratio).round().max(1.) as u32;
    height = (height as f64 * ratio).round().max(1.) as u32;
    let scaler = factory.CreateBitmapScaler()?;
    scaler.Initialize(&frame, width, height, WICBitmapInterpolationModeFant)?;
    let converter = factory.CreateFormatConverter()?;
    converter.Initialize(
        &scaler,
        &GUID_WICPixelFormat32bppPBGRA,
        WICBitmapDitherTypeNone,
        None,
        0.,
        WICBitmapPaletteTypeCustom,
    )?;
    let mut pixels = vec![0; (width * height * 4) as usize];
    converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
    Ok(Thumbnail {
        width,
        height,
        pixels,
    })
}

/// Opaque cards on transparent pixels, suitable for the Shell drag-image helper.
pub fn drag_image(image: Option<&Thumbnail>, count: usize) -> Thumbnail {
    let mut result = Thumbnail {
        width: 96,
        height: 96,
        pixels: vec![0; 96 * 96 * 4],
    };
    let mut pixel = |x: usize, y: usize, color: [u8; 4]| {
        let at = (y * 96 + x) * 4;
        result.pixels[at..at + 4].copy_from_slice(&color);
    };
    for layer in (0..count.min(3)).rev() {
        let d = layer * 7;
        for y in 8 + d..68 + d {
            for x in 8 + d..68 + d {
                pixel(
                    x,
                    y,
                    if layer == 0 {
                        [54, 48, 43, 255]
                    } else {
                        [105, 92, 81, 255]
                    },
                );
            }
        }
    }
    if let Some(image) = image {
        let ratio = 52. / image.width.max(image.height) as f32;
        let w = (image.width as f32 * ratio).round().max(1.) as usize;
        let h = (image.height as f32 * ratio).round().max(1.) as usize;
        for y in 0..h {
            for x in 0..w {
                let source = ((y * image.height as usize / h) * image.width as usize
                    + x * image.width as usize / w)
                    * 4;
                let alpha = image.pixels[source + 3] as u16;
                let mut color = [0, 0, 0, 255];
                for (c, channel) in color.iter_mut().enumerate().take(3) {
                    *channel =
                        (image.pixels[source + c] as u16 + 45 * (255 - alpha) / 255).min(255) as u8;
                }
                pixel(12 + (52 - w) / 2 + x, 12 + (52 - h) / 2 + y, color);
            }
        }
    } else {
        for y in 21..54 {
            for x in 25..49 {
                pixel(x, y, [220, 220, 225, 255]);
            }
        }
        for y in [30, 37, 44] {
            for x in 29..45 {
                pixel(x, y, [115, 95, 80, 255]);
            }
        }
    }
    if count > 1 {
        for y in 56usize..86 {
            for x in 56usize..86 {
                if (x as i32 - 71).pow(2) + (y as i32 - 71).pow(2) <= 15 * 15 {
                    pixel(x, y, [220, 135, 30, 255]);
                }
            }
        }
        let digits = [
            [31, 17, 17, 17, 31],
            [4, 12, 4, 4, 14],
            [31, 1, 31, 16, 31],
            [31, 1, 15, 1, 31],
            [17, 17, 31, 1, 1],
            [31, 16, 31, 1, 31],
            [31, 16, 31, 17, 31],
            [31, 1, 2, 4, 4],
            [31, 17, 31, 17, 31],
            [31, 17, 31, 1, 31],
        ];
        let text = count.min(99).to_string();
        let left = 71 - text.len() * 6;
        for (i, d) in text.bytes().enumerate() {
            for (row, bits) in digits[(d - b'0') as usize].iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        for yy in 0..2 {
                            for xx in 0..2 {
                                pixel(
                                    left + i * 12 + col * 2 + xx,
                                    66 + row * 2 + yy,
                                    [255, 255, 255, 255],
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_supports_ranges_and_toggles() {
        let ids = [1, 2, 3, 4, 5];
        let mut selected = BTreeSet::new();
        let mut anchor = None;
        select(&ids, &mut selected, &mut anchor, 2, false, false);
        select(&ids, &mut selected, &mut anchor, 5, false, true);
        assert_eq!(selected, BTreeSet::from([2, 3, 4, 5]));
        select(&ids, &mut selected, &mut anchor, 3, true, false);
        assert_eq!(selected, BTreeSet::from([2, 4, 5]));
        select(&ids, &mut selected, &mut anchor, 1, false, false);
        assert_eq!(selected, BTreeSet::from([1]));
    }
    #[test]
    fn stale_anchor_falls_back_to_single_selection() {
        let mut selected = BTreeSet::from([8]);
        let mut anchor = Some(8);
        select(&[1, 2], &mut selected, &mut anchor, 2, false, true);
        assert_eq!(selected, BTreeSet::from([2]));
    }
    #[test]
    fn image_detection_and_drag_bitmap_are_bounded() {
        assert!(is_image("foto.JPEG"));
        assert!(!is_image("foto.jpg.exe"));
        for count in [1, 2, 5, 1000] {
            let image = drag_image(None, count);
            assert_eq!(image.pixels.len(), 96 * 96 * 4);
            assert_eq!(&image.pixels[..4], &[0, 0, 0, 0]);
            assert!(image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[3] == 0 || p[3] == 255));
        }
    }
}

#[cfg(test)]
mod decoding_tests {
    use super::*;
    #[test]
    fn native_thumbnail_decodes_and_rejects_invalid_images() {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
        }
        let path = std::env::temp_dir().join(format!("nusa-thumbnail-{}.bmp", std::process::id()));
        let mut bmp = vec![0u8; 70];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&70u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&2u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&2u32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        for row in [54, 62] {
            bmp[row..row + 6].copy_from_slice(&[40, 80, 180, 40, 80, 180]);
        }
        std::fs::write(&path, bmp).unwrap();
        let image = unsafe { thumbnail(&path) }.unwrap();
        assert_eq!((image.width, image.height, image.pixels.len()), (2, 2, 16));
        assert_eq!(&image.pixels[..4], &[40, 80, 180, 255]);
        std::fs::write(&path, b"invalid image").unwrap();
        assert!(unsafe { thumbnail(&path) }.is_err());
        std::fs::remove_file(path).unwrap();
        unsafe {
            CoUninitialize();
        }
    }
}
