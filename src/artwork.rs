//! Bounded thumbnail sampling on the media worker, never in the drawing loop.
use windows::{
    core::*,
    Win32::{Graphics::Imaging::*, System::Com::*, UI::Shell::SHCreateMemStream},
};

pub fn accent_from_encoded(bytes: &[u8]) -> Option<[f32; 3]> {
    if bytes.is_empty() {
        return None;
    }
    // The caller's media worker owns its COM apartment.
    unsafe { decode_accent(bytes).ok().flatten() }
}

unsafe fn decode_accent(bytes: &[u8]) -> Result<Option<[f32; 3]>> {
    let stream = SHCreateMemStream(Some(bytes)).ok_or_else(Error::from_thread)?;
    let factory: IWICImagingFactory =
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let decoder =
        factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)?;
    let frame = decoder.GetFrame(0)?;
    let scaler = factory.CreateBitmapScaler()?;
    scaler.Initialize(&frame, 48, 48, WICBitmapInterpolationModeFant)?;
    let converter = factory.CreateFormatConverter()?;
    converter.Initialize(
        &scaler,
        &GUID_WICPixelFormat32bppRGBA,
        WICBitmapDitherTypeNone,
        None,
        0.,
        WICBitmapPaletteTypeCustom,
    )?;
    let mut pixels = [0u8; 48 * 48 * 4];
    converter.CopyPixels(std::ptr::null(), 48 * 4, &mut pixels)?;
    Ok(dominant_accent(&pixels))
}

/// Quantized populations preserve a strong album hue instead of averaging it into gray.
fn dominant_accent(pixels: &[u8]) -> Option<[f32; 3]> {
    let mut bins = vec![(0f32, [0f32; 3]); 4096];
    for p in pixels.as_chunks::<4>().0 {
        if p[3] < 128 {
            continue;
        }
        let rgb = [p[0] as f32 / 255., p[1] as f32 / 255., p[2] as f32 / 255.];
        let max = rgb.iter().copied().fold(0f32, f32::max);
        let min = rgb.iter().copied().fold(1f32, f32::min);
        if !(0.06..=0.96).contains(&max) {
            continue;
        }
        let saturation = (max - min) / max;
        let weight = 0.08 + saturation * saturation * 3.;
        let index =
            ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
        let bin = &mut bins[index];
        bin.0 += weight;
        for (sum, channel) in bin.1.iter_mut().zip(rgb) {
            *sum += channel * weight;
        }
    }
    let (weight, sum) = bins.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    if weight == 0. {
        return None;
    }
    let rgb = sum.map(|c| c / weight);
    let peak = rgb.iter().copied().fold(0f32, f32::max);
    // Limit the glow to keep white text legible; retain neutral artwork as neutral.
    Some(rgb.map(|c| c * (0.50 / peak).min(1.6)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wic_decodes_album_pixels_with_correct_channel_order() {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
        }
        // A 2x2, 24-bit BMP with a burgundy album color, including row padding.
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
            bmp[row..row + 6].copy_from_slice(&[95, 35, 200, 95, 35, 200]);
        }
        let result = accent_from_encoded(&bmp);
        unsafe {
            CoUninitialize();
        }
        let [r, g, b] = result.expect("WIC must decode the BMP");
        assert!(r > b && b > g && r <= 0.501);
    }
    #[test]
    fn accent_survives_neutral_background() {
        let mut pixels = [180, 180, 180, 255].repeat(100);
        pixels.extend([200, 35, 95, 255].repeat(25));
        let [r, g, b] = dominant_accent(&pixels).unwrap();
        assert!(r > b && b > g && r <= 0.501);
    }
    #[test]
    fn neutral_and_missing_art_do_not_invent_a_hue() {
        assert_eq!(dominant_accent(&[0, 0, 0, 255, 255, 255, 255, 255]), None);
        assert_eq!(dominant_accent(&[200, 20, 40, 0]), None);
        let [r, g, b] = dominant_accent(&[120, 120, 120, 255]).unwrap();
        assert_eq!(r, g);
        assert_eq!(g, b);
        assert!(accent_from_encoded(b"invalid image").is_none());
    }
}
