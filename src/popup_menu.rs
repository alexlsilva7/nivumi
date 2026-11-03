//! Dark, padded native popup menus. Native navigation, submenus and dismissal are retained.
use crate::theme;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{Controls::*, HiDpi::*, WindowsAndMessaging::*},
    },
};

struct Entry {
    label: Vec<u16>,
    separator: bool,
    submenu: bool,
    scale: f32,
}
pub struct DarkMenu {
    // Win32 stores pointers to entries; each allocation must stay fixed while
    // recursively appending submenus can grow the vector.
    #[allow(clippy::vec_box)]
    _entries: Vec<Box<Entry>>,
    brush: HBRUSH,
}
impl DarkMenu {
    /// # Safety
    /// Keep this guard alive until after TrackPopupMenu and DestroyMenu. The owner
    /// must forward WM_MEASUREITEM/WM_DRAWITEM to handle(). Only use with app-owned menus.
    pub unsafe fn attach(menu: HMENU, owner: HWND) -> Self {
        let brush = CreateSolidBrush(COLORREF(theme::colorref(theme::SURFACE)));
        let mut guard = Self {
            _entries: vec![],
            brush,
        };
        let _ = guard.decorate(menu, GetDpiForWindow(owner).max(96) as f32 / 96.);
        guard
    }
    unsafe fn decorate(&mut self, menu: HMENU, scale: f32) -> Result<()> {
        let info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_BACKGROUND,
            hbrBack: self.brush,
            ..Default::default()
        };
        SetMenuInfo(menu, &info)?;
        for index in 0..GetMenuItemCount(Some(menu)) {
            let mut label = vec![0u16; 2048];
            let mut item = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STRING | MIIM_FTYPE | MIIM_SUBMENU,
                dwTypeData: PWSTR(label.as_mut_ptr()),
                cch: label.len() as u32,
                ..Default::default()
            };
            GetMenuItemInfoW(menu, index as u32, true, &mut item)?;
            label.truncate(item.cch as usize);
            let separator = item.fType.contains(MFT_SEPARATOR);
            if !item.hSubMenu.is_invalid() {
                self.decorate(item.hSubMenu, scale)?;
            }
            let mut entry = Box::new(Entry {
                label,
                separator,
                submenu: !item.hSubMenu.is_invalid(),
                scale,
            });
            let update = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_DATA,
                fType: MFT_OWNERDRAW,
                dwItemData: (&mut *entry as *mut Entry) as usize,
                ..Default::default()
            };
            SetMenuItemInfoW(menu, index as u32, true, &update)?;
            self._entries.push(entry);
        }
        Ok(())
    }
}
impl Drop for DarkMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.brush.into());
        }
    }
}

/// # Safety
/// Forward only owner-drawn messages for menus decorated by DarkMenu in this app.
pub unsafe fn handle(msg: u32, _: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if msg == WM_MEASUREITEM {
        let item = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
        if item.CtlType != ODT_MENU || item.itemData == 0 {
            return None;
        }
        let entry = &*(item.itemData as *const Entry);
        let dc = GetDC(None);
        let font = menu_font(entry.scale);
        let previous = SelectObject(dc, font.into());
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &entry.label, &mut size);
        SelectObject(dc, previous);
        let _ = DeleteObject(font.into());
        let _ = ReleaseDC(None, dc);
        item.itemWidth = (size.cx as f32 + 64. * entry.scale).max(220. * entry.scale) as u32;
        item.itemHeight = (if entry.separator { 10. } else { 34. } * entry.scale) as u32;
        return Some(LRESULT(1));
    }
    if msg == WM_DRAWITEM {
        let item = &*(lp.0 as *const DRAWITEMSTRUCT);
        if item.CtlType != ODT_MENU || item.itemData == 0 {
            return None;
        }
        let entry = &*(item.itemData as *const Entry);
        let brush = CreateSolidBrush(COLORREF(theme::colorref(theme::SURFACE)));
        let _ = FillRect(item.hDC, &item.rcItem, brush);
        let _ = DeleteObject(brush.into());
        let scale = entry.scale;
        let mut rect = item.rcItem;
        if entry.separator {
            let pen = CreatePen(PS_SOLID, 1, COLORREF(theme::colorref(theme::BORDER)));
            let previous = SelectObject(item.hDC, pen.into());
            let middle = (rect.top + rect.bottom) / 2;
            let _ = MoveToEx(item.hDC, rect.left + (12. * scale) as i32, middle, None);
            let _ = LineTo(item.hDC, rect.right - (12. * scale) as i32, middle);
            SelectObject(item.hDC, previous);
            let _ = DeleteObject(pen.into());
        } else {
            if item.itemState.0 & ODS_SELECTED.0 != 0 {
                let brush = CreateSolidBrush(COLORREF(theme::colorref(theme::SELECTED)));
                let old = SelectObject(item.hDC, brush.into());
                let pen = SelectObject(item.hDC, GetStockObject(NULL_PEN));
                let pad = (3. * scale) as i32;
                let _ = RoundRect(
                    item.hDC,
                    rect.left + pad,
                    rect.top + pad,
                    rect.right - pad,
                    rect.bottom - pad,
                    (10. * scale) as i32,
                    (10. * scale) as i32,
                );
                SelectObject(item.hDC, old);
                SelectObject(item.hDC, pen);
                let _ = DeleteObject(brush.into());
            }
            let _ = SetBkMode(item.hDC, TRANSPARENT);
            let _ = SetTextColor(
                item.hDC,
                COLORREF(if item.itemState.0 & ODS_DISABLED.0 != 0 {
                    theme::colorref(theme::MUTED)
                } else {
                    theme::colorref(theme::TEXT)
                }),
            );
            let font = menu_font(scale);
            let old = SelectObject(item.hDC, font.into());
            rect.left += (30. * scale) as i32;
            rect.right -= (24. * scale) as i32;
            let mut label = entry.label.clone();
            DrawTextW(
                item.hDC,
                &mut label,
                &mut rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            if item.itemState.0 & ODS_CHECKED.0 != 0 {
                let mut check: Vec<_> = "✓".encode_utf16().collect();
                let mut bounds = item.rcItem;
                bounds.left += (8. * scale) as i32;
                bounds.right = rect.left;
                DrawTextW(
                    item.hDC,
                    &mut check,
                    &mut bounds,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
            if entry.submenu {
                let mut arrow: Vec<_> = "›".encode_utf16().collect();
                let mut bounds = item.rcItem;
                bounds.left = bounds.right - (20. * scale) as i32;
                DrawTextW(
                    item.hDC,
                    &mut arrow,
                    &mut bounds,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
            SelectObject(item.hDC, old);
            let _ = DeleteObject(font.into());
        }
        return Some(LRESULT(1));
    }
    None
}
unsafe fn menu_font(scale: f32) -> HFONT {
    CreateFontW(
        -(theme::BODY as f32 * scale).round() as i32,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        0,
        w!("Segoe UI"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoration_preserves_commands_checks_and_submenus_with_stable_measurement_data() {
        unsafe {
            let root = CreatePopupMenu().unwrap();
            let child = CreatePopupMenu().unwrap();
            AppendMenuW(child, MF_STRING | MF_CHECKED, 42, w!("Tradução")).unwrap();
            AppendMenuW(root, MF_POPUP, child.0 as usize, w!("Mostrar")).unwrap();
            AppendMenuW(root, MF_SEPARATOR, 0, None).unwrap();
            for id in 100..160 {
                AppendMenuW(root, MF_STRING, id, w!("Configuração")).unwrap();
            }
            let theme = DarkMenu::attach(root, HWND::default());
            let mut item = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_ID | MIIM_STATE | MIIM_DATA | MIIM_FTYPE,
                ..Default::default()
            };
            GetMenuItemInfoW(child, 0, true, &mut item).unwrap();
            assert_eq!(item.wID, 42);
            assert!(item.fState.contains(MFS_CHECKED));
            assert!(item.fType.contains(MFT_OWNERDRAW));
            let mut measure = MEASUREITEMSTRUCT {
                CtlType: ODT_MENU,
                itemData: item.dwItemData,
                ..Default::default()
            };
            assert_eq!(
                handle(
                    WM_MEASUREITEM,
                    WPARAM(0),
                    LPARAM((&mut measure as *mut MEASUREITEMSTRUCT) as isize)
                ),
                Some(LRESULT(1))
            );
            assert!(measure.itemWidth >= 220);
            assert_eq!(measure.itemHeight, 34);
            let entry = &*(item.dwItemData as *const Entry);
            assert_eq!(String::from_utf16(&entry.label).unwrap(), "Tradução");
            DestroyMenu(root).unwrap();
            drop(theme);
        }
    }
}
