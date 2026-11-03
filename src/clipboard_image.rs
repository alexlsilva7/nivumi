//! Clipboard screenshots persisted as ordinary Shelf files. Never modifies the clipboard.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{CF_DIB, CF_DIBV5},
        },
    },
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid or unsupported clipboard image",
    )
}

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Must run on the window's UI thread. Copy the borrowed clipboard data before closing it.
pub fn paste(hwnd: HWND, directory: &Path) -> io::Result<Option<PathBuf>> {
    let image = unsafe {
        OpenClipboard(Some(hwnd)).map_err(io::Error::other)?;
        let _clipboard = Clipboard;
        let png = RegisterClipboardFormatW(w!("PNG"));
        let mut image = None;
        let mut last_error = None;
        for (format, is_png) in [
            (png, true),
            (CF_DIBV5.0 as u32, false),
            (CF_DIB.0 as u32, false),
        ] {
            if format == 0 || IsClipboardFormatAvailable(format).is_err() {
                continue;
            }
            let result = read_global(format).and_then(|bytes| {
                if is_png {
                    validate_png(&bytes)?;
                    validate_encoded(&bytes)?;
                    Ok((bytes, "png"))
                } else {
                    let bmp = dib_to_bmp(&bytes)?;
                    validate_encoded(&bmp)?;
                    Ok((bmp, "bmp"))
                }
            });
            match result {
                Ok(value) => {
                    image = Some(value);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        match (image, last_error) {
            (Some(image), _) => Some(image),
            (None, Some(error)) => return Err(error),
            (None, None) => None,
        }
    };
    image
        .map(|(bytes, extension)| save(directory, &bytes, extension))
        .transpose()
}

unsafe fn read_global(format: u32) -> io::Result<Vec<u8>> {
    let handle = GetClipboardData(format).map_err(io::Error::other)?;
    let memory = HGLOBAL(handle.0);
    let size = GlobalSize(memory);
    if size == 0 || size > MAX_BYTES {
        return Err(invalid());
    }
    let pointer = GlobalLock(memory);
    if pointer.is_null() {
        return Err(io::Error::other(windows::core::Error::from_thread()));
    }
    let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec();
    let _ = GlobalUnlock(memory);
    Ok(bytes)
}

fn validate_encoded(bytes: &[u8]) -> io::Result<()> {
    use windows::{
        core::Interface,
        Win32::{Graphics::Imaging::*, System::Com::*},
    };
    let result = unsafe {
        (|| -> windows::core::Result<()> {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let stream = factory.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let input: IStream = stream.cast()?;
            let decoder = factory.CreateDecoderFromStream(
                &input,
                std::ptr::null(),
                WICDecodeMetadataCacheOnLoad,
            )?;
            let frame = decoder.GetFrame(0)?;
            let (mut width, mut height) = (0, 0);
            frame.GetSize(&mut width, &mut height)?;
            if width == 0 || height == 0 || width as u64 * height as u64 > 100_000_000 {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            Ok(())
        })()
    };
    result.map_err(io::Error::other)
}

fn validate_png(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() < 33 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return Err(invalid());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width as u64 * height as u64 > 100_000_000 {
        return Err(invalid());
    }
    Ok(())
}

// CF_DIB has no BITMAPFILEHEADER. Keep the original pixels, masks and orientation intact.
fn dib_to_bmp(dib: &[u8]) -> io::Result<Vec<u8>> {
    if dib.len() < 40 || dib.len() > MAX_BYTES {
        return Err(invalid());
    }
    let word = |at| u16::from_le_bytes(dib[at..at + 2].try_into().unwrap());
    let dword = |at| u32::from_le_bytes(dib[at..at + 4].try_into().unwrap());
    let header = dword(0) as usize;
    if ![40, 52, 56, 108, 124].contains(&header) || header > dib.len() {
        return Err(invalid());
    }
    let width = dword(4) as i32;
    let height = dword(8) as i32;
    let bits = word(14);
    let compression = dword(16);
    if width <= 0
        || height == 0
        || word(12) != 1
        || ![1, 4, 8, 16, 24, 32].contains(&bits)
        || ![0, 3, 6].contains(&compression)
        || (compression != 0 && ![16, 32].contains(&bits))
    {
        return Err(invalid());
    }
    if width as u64 * height.unsigned_abs() as u64 > 100_000_000 {
        return Err(invalid());
    }
    let colors = if dword(32) != 0 {
        dword(32) as u64
    } else if bits <= 8 {
        1u64 << bits
    } else {
        0
    };
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let mut offset = header as u64 + masks + colors * 4;
    // Packed V5 clipboard DIBs place an optional color profile before the pixels.
    if header == 124 {
        let profile_at = dword(112) as u64;
        let profile_size = dword(116) as u64;
        if profile_size != 0 {
            if profile_at < offset {
                return Err(invalid());
            }
            offset = profile_at + profile_size;
        }
    }
    let stride = (width as u64 * bits as u64).div_ceil(32) * 4;
    let pixel_bytes = stride * height.unsigned_abs() as u64;
    if offset + pixel_bytes > dib.len() as u64 {
        return Err(invalid());
    }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((14 + offset) as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

fn save(directory: &Path, bytes: &[u8], extension: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    for suffix in 0..100 {
        let path = directory.join(format!("Captura-{stamp}-{suffix}.{extension}"));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Cannot create unique screenshot file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dib(header: usize, height: i32, bits: u16, compression: u32, extra: usize) -> Vec<u8> {
        let mut bytes = vec![0; header + extra + 8]; // 2x1 RGB/BGRA, including stride padding
        bytes[0..4].copy_from_slice(&(header as u32).to_le_bytes());
        bytes[4..8].copy_from_slice(&2i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&height.to_le_bytes());
        bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&bits.to_le_bytes());
        bytes[16..20].copy_from_slice(&compression.to_le_bytes());
        bytes
    }
    #[test]
    fn bmp_keeps_orientation_pixels_and_bitfield_offsets() {
        for height in [-1, 1] {
            let mut input = dib(40, height, 24, 0, 0);
            input[40..48].copy_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
            let output = dib_to_bmp(&input).unwrap();
            assert_eq!(&output[..2], b"BM");
            assert_eq!(u32::from_le_bytes(output[10..14].try_into().unwrap()), 54);
            assert_eq!(&output[14..], input);
        }
        let output = dib_to_bmp(&dib(40, -1, 32, 3, 12)).unwrap();
        assert_eq!(u32::from_le_bytes(output[10..14].try_into().unwrap()), 66);
        let output = dib_to_bmp(&dib(124, -1, 32, 3, 0)).unwrap();
        assert_eq!(u32::from_le_bytes(output[10..14].try_into().unwrap()), 138);
        let output = dib_to_bmp(&dib(40, 1, 8, 0, 1024)).unwrap();
        assert_eq!(u32::from_le_bytes(output[10..14].try_into().unwrap()), 1078);
    }
    #[test]
    fn malformed_or_truncated_images_are_rejected() {
        for length in 0..40 {
            assert!(dib_to_bmp(&vec![0; length]).is_err());
        }
        let mut input = dib(40, 1, 24, 0, 0);
        input.pop();
        assert!(dib_to_bmp(&input).is_err());
        let mut input = dib(40, i32::MIN, 32, 0, 0);
        input[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(dib_to_bmp(&input).is_err());
        assert!(validate_png(b"not a PNG").is_err());
    }
    #[test]
    fn saved_images_survive_reload_and_have_unique_paths() {
        let directory = std::env::temp_dir().join(format!(
            "nusa-clipboard-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bytes = dib_to_bmp(&dib(40, 1, 24, 0, 0)).unwrap();
        let first = save(&directory, &bytes, "bmp").unwrap();
        let second = save(&directory, &bytes, "bmp").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), bytes);
        assert_eq!(fs::read(&second).unwrap(), bytes);
        assert!(crate::shelf::is_image(first.to_str().unwrap()));
        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn native_windows_decoder_accepts_png_and_screenshot_bmp() {
        use windows::Win32::System::Com::*;
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).unwrap();
        }
        let bmp = dib_to_bmp(&dib(40, -1, 32, 0, 0)).unwrap();
        validate_encoded(&bmp).unwrap();
        // A complete 1x1 PNG, with valid chunk checksums.
        let hex = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b49444154789c636000020000050001a5f645400000000049454e44ae426082";
        let png: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        validate_png(&png).unwrap();
        validate_encoded(&png).unwrap();
        assert!(validate_encoded(b"invalid").is_err());
        unsafe {
            CoUninitialize();
        }
    }
}
