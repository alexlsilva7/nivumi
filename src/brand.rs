//! Embedded Windows identity. Icon handles live for the UI thread's lifetime.
use std::{cell::RefCell, collections::BTreeMap};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            Shell::{
                DefSubclassProc, RemoveWindowSubclass, SetCurrentProcessExplicitAppUserModelID,
                SetWindowSubclass,
            },
            WindowsAndMessaging::*,
        },
    },
};

pub const APP_ID: &str = "Nivumi.Desktop";
const ICON_SUBCLASS: usize = 0x4e495655;
struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct Icons {
    small: OwnedIcon,
    large: OwnedIcon,
    tray: OwnedIcon,
}
thread_local! { static ICONS: RefCell<BTreeMap<u32, Icons>> = const { RefCell::new(BTreeMap::new()) }; }

unsafe fn load(id: usize, width: i32, height: i32) -> Result<OwnedIcon> {
    let instance = GetModuleHandleW(None)?;
    let icon = LoadImageW(
        Some(instance.into()),
        PCWSTR(id as *const u16),
        IMAGE_ICON,
        width,
        height,
        LR_DEFAULTCOLOR,
    )?;
    Ok(OwnedIcon(HICON(icon.0)))
}
unsafe fn icons(dpi: u32) -> Result<(HICON, HICON, HICON)> {
    ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(dpi) {
            let small_w = GetSystemMetricsForDpi(SM_CXSMICON, dpi);
            let small_h = GetSystemMetricsForDpi(SM_CYSMICON, dpi);
            entry.insert(Icons {
                small: load(1, small_w, small_h)?,
                large: load(
                    1,
                    GetSystemMetricsForDpi(SM_CXICON, dpi),
                    GetSystemMetricsForDpi(SM_CYICON, dpi),
                )?,
                tray: load(2, small_w, small_h)?,
            });
        }
        let set = &cache[&dpi];
        Ok((set.small.0, set.large.0, set.tray.0))
    })
}
/// # Safety
/// Call once on the UI thread before creating any top-level window.
pub unsafe fn initialize() -> Result<()> {
    SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID))?;
    icons(96)?;
    Ok(())
}
/// # Safety
/// Call on the window's owning UI thread. Handles are borrowed, never destroy them.
pub unsafe fn class_icons() -> Result<(HICON, HICON)> {
    let (small, large, _) = icons(96)?;
    Ok((small, large))
}
/// # Safety
/// Call on the notification window's owning UI thread. Handle is borrowed.
pub unsafe fn tray_icon(hwnd: HWND) -> Result<HICON> {
    Ok(icons(GetDpiForWindow(hwnd).max(96))?.2)
}
unsafe fn apply_dpi(hwnd: HWND, dpi: u32) -> Result<()> {
    let (small, large, _) = icons(dpi)?;
    SendMessageW(
        hwnd,
        WM_SETICON,
        Some(WPARAM(ICON_SMALL as usize)),
        Some(LPARAM(small.0 as isize)),
    );
    SendMessageW(
        hwnd,
        WM_SETICON,
        Some(WPARAM(ICON_BIG as usize)),
        Some(LPARAM(large.0 as isize)),
    );
    Ok(())
}
/// # Safety
/// Call on the owning UI thread after creating a top-level window.
pub unsafe fn apply_window(hwnd: HWND) -> Result<()> {
    apply_dpi(hwnd, GetDpiForWindow(hwnd).max(96))?;
    if !SetWindowSubclass(hwnd, Some(icon_proc), ICON_SUBCLASS, 0).as_bool() {
        return Err(Error::from_thread());
    }
    Ok(())
}
unsafe extern "system" fn icon_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == WM_DPICHANGED {
        let _ = apply_dpi(hwnd, (wp.0 & 0xffff) as u32);
    }
    if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(icon_proc), ICON_SUBCLASS);
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_icons_load_at_standard_windows_scales() {
        unsafe {
            for dpi in [96, 144, 192] {
                let (small, large, tray) = icons(dpi).unwrap();
                assert!(!small.is_invalid() && !large.is_invalid() && !tray.is_invalid());
                assert_eq!(icons(dpi).unwrap().0, small);
                use windows::Win32::Graphics::Gdi::*;
                let mut info = ICONINFO::default();
                GetIconInfo(small, &mut info).unwrap();
                let mut bitmap = BITMAP::default();
                assert_ne!(
                    GetObjectW(
                        info.hbmColor.into(),
                        std::mem::size_of::<BITMAP>() as i32,
                        Some((&mut bitmap as *mut BITMAP).cast())
                    ),
                    0
                );
                assert_eq!(bitmap.bmWidth, GetSystemMetricsForDpi(SM_CXSMICON, dpi));
                assert_eq!(bitmap.bmHeight, GetSystemMetricsForDpi(SM_CYSMICON, dpi));
                let _ = DeleteObject(info.hbmColor.into());
                let _ = DeleteObject(info.hbmMask.into());
            }
        }
    }
}
