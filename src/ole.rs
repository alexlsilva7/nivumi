//! Native OLE Shelf drag/drop. The caller initializes OleInitialize on its STA.
use std::{mem::ManuallyDrop, path::PathBuf, sync::mpsc::Sender};
use windows::{
    core::{implement, Ref, BOOL, HRESULT, HSTRING},
    Win32::{
        Foundation::*,
        System::{Com::*, Memory::*, Ole::*, SystemServices::*},
        UI::Shell::*,
    },
};

#[derive(Clone, Debug)]
pub enum DropPayload {
    Files(Vec<PathBuf>),
    Text(String),
}
fn format(id: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: id,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}
pub struct DropRegistration {
    hwnd: HWND,
    _target: IDropTarget,
}
impl Drop for DropRegistration {
    fn drop(&mut self) {
        unsafe {
            let _ = RevokeDragDrop(self.hwnd);
        }
    }
}
pub fn register_drop_target(
    hwnd: HWND,
    sender: Sender<DropPayload>,
) -> windows::core::Result<DropRegistration> {
    let target: IDropTarget = ShelfTarget {
        sender,
        accepted: Default::default(),
    }
    .into();
    unsafe {
        RegisterDragDrop(hwnd, &target)?;
    }
    Ok(DropRegistration {
        hwnd,
        _target: target,
    })
}
#[implement(IDropTarget)]
struct ShelfTarget {
    sender: Sender<DropPayload>,
    accepted: std::sync::atomic::AtomicU32,
}
impl IDropTarget_Impl for ShelfTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            if effect.is_null() {
                return Err(E_POINTER.into());
            }
            let supported = data.as_ref().is_some_and(|d| {
                d.QueryGetData(&format(CF_HDROP.0)).is_ok()
                    || d.QueryGetData(&format(CF_UNICODETEXT.0)).is_ok()
            });
            let allowed = if supported {
                (*effect).0 & DROPEFFECT_COPY.0
            } else {
                0
            };
            self.accepted
                .store(allowed, std::sync::atomic::Ordering::Relaxed);
            *effect = DROPEFFECT(allowed);
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            if effect.is_null() {
                return Err(E_POINTER.into());
            }
            *effect = DROPEFFECT(self.accepted.load(std::sync::atomic::Ordering::Relaxed));
        }
        Ok(())
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        self.accepted.store(0, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let data = data
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(E_POINTER))?;
        unsafe {
            if effect.is_null() {
                return Err(E_POINTER.into());
            }
            let allowed = (*effect).0
                & self.accepted.swap(0, std::sync::atomic::Ordering::Relaxed)
                & DROPEFFECT_COPY.0;
            *effect = DROPEFFECT_NONE;
            if allowed == 0 {
                return Ok(());
            }
            if let Ok(payload) = read_data(data) {
                if self.sender.send(payload).is_ok() {
                    *effect = DROPEFFECT_COPY;
                }
            }
        }
        Ok(())
    }
}
unsafe fn read_data(data: &IDataObject) -> windows::core::Result<DropPayload> {
    unsafe {
        if let Ok(mut medium) = data.GetData(&format(CF_HDROP.0)) {
            let handle = HDROP(medium.u.hGlobal.0);
            let count = DragQueryFileW(handle, u32::MAX, None);
            let mut files = Vec::new();
            for i in 0..count.min(4096) {
                let len = DragQueryFileW(handle, i, None);
                let mut buffer = vec![0; len as usize + 1];
                DragQueryFileW(handle, i, Some(&mut buffer));
                use std::os::windows::ffi::OsStringExt;
                files.push(PathBuf::from(std::ffi::OsString::from_wide(
                    &buffer[..len as usize],
                )));
            }
            ReleaseStgMedium(&mut medium);
            return Ok(DropPayload::Files(files));
        }
        let mut medium = data.GetData(&format(CF_UNICODETEXT.0))?;
        let size = GlobalSize(medium.u.hGlobal).min(16 * 1024 * 1024);
        let pointer = GlobalLock(medium.u.hGlobal);
        let result = if pointer.is_null() {
            Err(windows::core::Error::from_thread())
        } else {
            let words = std::slice::from_raw_parts(pointer as *const u16, size / 2);
            let end = words.iter().position(|v| *v == 0).unwrap_or(words.len());
            let text = String::from_utf16_lossy(&words[..end]);
            let _ = GlobalUnlock(medium.u.hGlobal);
            Ok(DropPayload::Text(text))
        };
        ReleaseStgMedium(&mut medium);
        result
    }
}

#[implement(IDropSource)]
struct ShelfSource {
    right: bool,
}
impl IDropSource_Impl for ShelfSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0
            & (if self.right {
                MK_RBUTTON.0
            } else {
                MK_LBUTTON.0
            })
            == 0
        {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
#[implement(IDataObject)]
struct ShelfData {
    format: u16,
    bytes: Vec<u8>,
}
impl IDataObject_Impl for ShelfData_Impl {
    fn GetData(&self, input: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        unsafe {
            if self.QueryGetData(input).is_err() {
                return Err(DV_E_FORMATETC.into());
            }
            let memory = GlobalAlloc(GMEM_MOVEABLE, self.bytes.len())?;
            let p = GlobalLock(memory);
            if p.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(self.bytes.as_ptr(), p.cast(), self.bytes.len());
            let _ = GlobalUnlock(memory);
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: memory },
                pUnkForRelease: ManuallyDrop::new(None),
            })
        }
    }
    fn QueryGetData(&self, input: *const FORMATETC) -> HRESULT {
        unsafe {
            if input.is_null() {
                return E_POINTER;
            }
            let f = &*input;
            if f.cfFormat == self.format
                && f.tymed & TYMED_HGLOBAL.0 as u32 != 0
                && f.dwAspect == DVASPECT_CONTENT.0
                && f.lindex == -1
            {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        unsafe {
            if !out.is_null() {
                (*out).ptd = std::ptr::null_mut();
            }
        }
        DATA_S_SAMEFORMATETC
    }
    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: BOOL,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&[format(self.format)]) }
    }
    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}
/// Uses the Shell data object so file transfers retain Explorer move/copy semantics.
pub fn file_data(files: &[PathBuf]) -> windows::core::Result<IDataObject> {
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    if files.is_empty() {
        return Err(E_INVALIDARG.into());
    }
    struct Ids(Vec<*mut ITEMIDLIST>);
    impl Drop for Ids {
        fn drop(&mut self) {
            for p in &self.0 {
                unsafe {
                    CoTaskMemFree(Some((*p).cast()));
                }
            }
        }
    }
    let mut ids = Ids(Vec::new());
    for file in files {
        if !file.exists() {
            return Err(windows::core::Error::new(
                E_INVALIDARG,
                "Arquivo nÃ£o encontrado",
            ));
        }
        let mut id = std::ptr::null_mut();
        unsafe {
            SHParseDisplayName(&HSTRING::from(file.as_os_str()), None, &mut id, 0, None)?;
        }
        ids.0.push(id);
    }
    let pointers = ids
        .0
        .iter()
        .map(|p| *p as *const ITEMIDLIST)
        .collect::<Vec<_>>();
    unsafe {
        let items = SHCreateShellItemArrayFromIDLists(&pointers)?;
        items.BindToHandler(None, &BHID_DataObject)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOutcome {
    Cancelled,
    Copied,
    Moved,
}

pub fn drag_files(files: &[PathBuf]) -> windows::core::Result<bool> {
    Ok(drag_files_with_button(files, false, None)? != DragOutcome::Cancelled)
}
pub fn drag_files_with_button(
    files: &[PathBuf],
    right: bool,
    image: Option<&crate::shelf::Thumbnail>,
) -> windows::core::Result<DragOutcome> {
    let data = file_data(files)?;
    let source: IDropSource = ShelfSource { right }.into();
    if let Some(image) = image {
        unsafe {
            attach_drag_image(&data, image);
        }
    }
    let mut effect = DROPEFFECT_NONE;
    let allowed = if right {
        DROPEFFECT_COPY | DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    let result = unsafe { DoDragDrop(&data, &source, allowed, &mut effect) };
    if result == DRAGDROP_S_CANCEL {
        return Ok(DragOutcome::Cancelled);
    }
    result.ok()?;
    Ok(if effect.0 & DROPEFFECT_MOVE.0 != 0 {
        DragOutcome::Moved
    } else if effect.0 & DROPEFFECT_COPY.0 != 0 {
        DragOutcome::Copied
    } else {
        DragOutcome::Cancelled
    })
}
unsafe fn attach_drag_image(data: &IDataObject, image: &crate::shelf::Thumbnail) -> bool {
    use windows::Win32::Graphics::Gdi::*;
    let Ok(helper) =
        CoCreateInstance::<_, IDragSourceHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER)
    else {
        return false;
    };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: image.width as i32,
            biHeight: -(image.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    if let Ok(bitmap) = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
        if bits.is_null() {
            let _ = DeleteObject(bitmap.into());
            return false;
        }
        std::ptr::copy_nonoverlapping(image.pixels.as_ptr(), bits.cast(), image.pixels.len());
        let drag = SHDRAGIMAGE {
            sizeDragImage: SIZE {
                cx: image.width as i32,
                cy: image.height as i32,
            },
            ptOffset: POINT { x: 20, y: 20 },
            hbmpDragImage: bitmap,
            crColorKey: COLORREF(0xffffffff),
        };
        // The drag-image helper takes ownership on success.
        if helper.InitializeFromBitmap(&drag, data).is_err() {
            let _ = DeleteObject(bitmap.into());
            return false;
        }
        return true;
    }
    false
}
pub fn clipboard_files(files: &[PathBuf], cut: bool) -> windows::core::Result<()> {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    let data = file_data(files)?;
    unsafe {
        let id = RegisterClipboardFormatW(windows::core::w!("Preferred DropEffect"));
        if id == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let effect = if cut {
            DROPEFFECT_MOVE
        } else {
            DROPEFFECT_COPY
        };
        let memory = GlobalAlloc(GMEM_MOVEABLE, 4)?;
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(windows::core::Error::from_thread());
        }
        *(pointer as *mut u32) = effect.0;
        let _ = GlobalUnlock(memory);
        let mut medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: ManuallyDrop::new(None),
        };
        if let Err(error) = data.SetData(&format(id as u16), &medium, true) {
            ReleaseStgMedium(&mut medium);
            return Err(error);
        }
        OleSetClipboard(&data)?;
        OleFlushClipboard()
    }
}
pub fn copy_text(hwnd: windows::Win32::Foundation::HWND, text: &str) -> windows::core::Result<()> {
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    unsafe {
        OpenClipboard(Some(hwnd))?;
        let result = (|| {
            EmptyClipboard()?;
            let chars = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let memory = GlobalAlloc(GMEM_MOVEABLE, chars.len() * 2)?;
            let ptr = GlobalLock(memory);
            if ptr.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(chars.as_ptr(), ptr.cast(), chars.len());
            let _ = GlobalUnlock(memory);
            if let Err(e) = SetClipboardData(13, Some(windows::Win32::Foundation::HANDLE(memory.0)))
            {
                let _ = GlobalFree(Some(memory));
                return Err(e);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}
pub fn drag_text(text: &str) -> windows::core::Result<bool> {
    drag_text_with_button(text, false)
}
pub fn drag_text_with_button(text: &str, right: bool) -> windows::core::Result<bool> {
    let bytes = text
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    start_drag(CF_UNICODETEXT.0, bytes, right)
}
fn start_drag(format: u16, bytes: Vec<u8>, right: bool) -> windows::core::Result<bool> {
    let data: IDataObject = ShelfData { format, bytes }.into();
    let source: IDropSource = ShelfSource { right }.into();
    let mut effect = DROPEFFECT_NONE;
    let status = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
    if status == DRAGDROP_S_CANCEL {
        return Ok(false);
    }
    status.ok()?;
    Ok(effect.0 & DROPEFFECT_COPY.0 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_shell_data_preserves_unicode_and_multiple_parent_folders() {
        unsafe {
            OleInitialize(None).unwrap();
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    OleUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let base = std::env::temp_dir().join(format!(
            "nusa-drag-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let a = base.join("origem a");
        let b = base.join("origem b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let files = vec![a.join("imagem ação.txt"), b.join("segundo.txt")];
        for file in &files {
            std::fs::write(file, b"fixture").unwrap();
        }
        let data = file_data(&files).expect("Shell must support files from multiple parents");
        let payload = unsafe { read_data(&data) }.unwrap();
        match payload {
            DropPayload::Files(actual) => assert_eq!(actual, files),
            _ => panic!("expected files"),
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let target: IDropTarget = ShelfTarget {
            sender,
            accepted: Default::default(),
        }
        .into();
        let point = POINTL::default();
        let mut effect = DROPEFFECT_MOVE;
        unsafe {
            target
                .DragEnter(&data, MK_LBUTTON, point, &mut effect)
                .unwrap();
            assert_eq!(effect, DROPEFFECT_NONE);
            target.Drop(&data, MK_LBUTTON, point, &mut effect).unwrap();
            assert!(receiver.try_recv().is_err());
            effect = DROPEFFECT_COPY | DROPEFFECT_MOVE;
            target
                .DragEnter(&data, MK_LBUTTON, point, &mut effect)
                .unwrap();
            assert_eq!(effect, DROPEFFECT_COPY);
            target.Drop(&data, MK_LBUTTON, point, &mut effect).unwrap();
            assert_eq!(effect, DROPEFFECT_COPY);
            assert!(
                matches!(receiver.try_recv().unwrap(),DropPayload::Files(paths) if paths==files)
            );
        }
        let image = crate::shelf::drag_image(None, files.len());
        assert!(
            unsafe { attach_drag_image(&data, &image) },
            "Shell drag-image helper must accept the stack image"
        );
        // Copy/move negotiation must not modify files before a destination accepts.
        assert!(files.iter().all(|p| p.exists()));
        drop(data);
        for file in &files {
            std::fs::remove_file(file).unwrap();
        }
        assert!(file_data(&files).is_err());
        std::fs::remove_dir(a).unwrap();
        std::fs::remove_dir(b).unwrap();
        std::fs::remove_dir(base).unwrap();
        for right in [false, true] {
            let source: IDropSource = ShelfSource { right }.into();
            let held = if right { MK_RBUTTON } else { MK_LBUTTON };
            unsafe {
                assert_eq!(source.QueryContinueDrag(false, held), S_OK);
                assert_eq!(
                    source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(0)),
                    DRAGDROP_S_DROP
                );
                assert_eq!(source.QueryContinueDrag(true, held), DRAGDROP_S_CANCEL);
            }
        }
    }
}
