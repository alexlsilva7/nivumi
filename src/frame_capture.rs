//! Debug-only readback of the app's own GPU surface, for repeatable visual QA.
//! Composites transparent pixels over cream; never captures another application.
use super::*;
use std::{cell::RefCell, path::PathBuf};
thread_local! { static DESTINATION: RefCell<Option<PathBuf>> = const { RefCell::new(None) }; }

pub fn request(path: PathBuf) {
    DESTINATION.with(|slot| *slot.borrow_mut() = Some(path));
}

pub(super) unsafe fn capture(renderer: &Renderer) -> Result<()> {
    let Some(path) = DESTINATION.with(|slot| slot.borrow_mut().take()) else {
        return Ok(());
    };
    let source: ID3D11Texture2D = renderer.swap.GetBuffer(0)?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    source.GetDesc(&mut desc);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    let mut copy = None;
    renderer
        ._device
        .CreateTexture2D(&desc, None, Some(&mut copy))?;
    let copy = copy.ok_or_else(Error::from_thread)?;
    let context = renderer._device.GetImmediateContext()?;
    context.CopyResource(&copy, &source);
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context.Map(&copy, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    let width = desc.Width as usize;
    let height = desc.Height as usize;
    let size = width * height * 4;
    let mut bmp = Vec::with_capacity(54 + size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((54 + size) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(width as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(height as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    for y in 0..height {
        let row = std::slice::from_raw_parts(
            (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
            width * 4,
        );
        for px in row.as_chunks::<4>().0 {
            let alpha = px[3] as u32;
            for (channel, background) in px[..3].iter().zip([239u32, 243, 246]) {
                bmp.push(
                    (*channel as u32 + (background * (255 - alpha) + 127) / 255).min(255) as u8,
                );
            }
            bmp.push(255);
        }
    }
    context.Unmap(&copy, 0);
    std::fs::write(path, bmp)
        .map_err(|e| Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))
}
