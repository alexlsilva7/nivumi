//! Independent native reader on the notch's UI thread. No second media/network service.
use crate::lyrics_view::{fit_bounds, LyricsContent, LyricsPreferences};
use crate::theme;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::*,
        UI::{
            Controls::{RichEdit::*, *},
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

pub const PREFERENCES_CHANGED: u32 = WM_APP + 12;
const FONT_DOWN: usize = 101;
const FONT_UP: usize = 102;
const PIN: usize = 103;
const FOLLOW: usize = 104;
const MODE: usize = 110;
const CAPTIONS: usize = 130;
const OPTIONS: usize = 131;
// COLORREF values use BGR. Keep the caption, toolbar and reader on one surface.
const SURFACE: u32 = theme::colorref(theme::LYRICS_SURFACE);
const MINIMIZE: usize = 140;
const MAXIMIZE: usize = 141;
const CLOSE: usize = 142;
const BACKGROUND: usize = 132;
const COPY_FOR_AI: usize = 133;

// Match the notch's black → 30% artwork accent → full accent stops.
fn lyrics_gradient(accent: [f32; 3], base: u32, t: f32) -> u32 {
    let t = t.clamp(0., 1.);
    let strength = if t <= 0.32 {
        0.
    } else if t <= 0.68 {
        (t - 0.32) / 0.36 * 0.3
    } else {
        0.3 + (t - 0.68) / 0.32 * 0.7
    };
    let mut rgb = 0;
    for (channel, shift) in accent.into_iter().zip([16, 8, 0]) {
        let floor = ((base >> shift) & 255) as f32;
        let value = floor + (channel * 255. - floor).max(0.) * strength;
        rgb |= (value.round().clamp(0., 255.) as u32) << shift;
    }
    rgb
}
fn composite_lyrics_pixel(text: u32, background: u32) -> u32 {
    let mut result = 0;
    for shift in [0, 8, 16] {
        let foreground = (text >> shift) & 255;
        let backdrop = (background >> shift) & 255;
        result |= (foreground + (backdrop * (255 - foreground) + 127) / 255) << shift;
    }
    result
}
struct ReaderSurface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}
impl ReaderSurface {
    unsafe fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 || width as i64 * height as i64 > 16_777_216 {
            return None;
        }
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let _ = DeleteDC(dc);
                return None;
            }
        };
        let previous = SelectObject(dc, bitmap.into());
        Some(Self {
            dc,
            bitmap,
            previous,
            bits: bits.cast(),
            width,
            height,
        })
    }
}
impl Drop for ReaderSurface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct LyricsWindow {
    pub hwnd: HWND,
    edit: HWND,
    buttons: Vec<HWND>,
    owner: HWND,
    prefs: LyricsPreferences,
    content: LyricsContent,
    follow: bool,
    reduced_motion: bool,
    scroll_motion: crate::lyrics_view::FocusScroll,
    scroll_target: POINT,
    english: bool,
    scrollbar_drag: bool,
    accent: [f32; 3],
    surface: std::cell::RefCell<Option<ReaderSurface>>,
    // RichEdit's class stays registered for the lifetime of the process.
    _rich_edit: HMODULE,
}

impl LyricsWindow {
    /// # Safety
    /// Create and use on the application's initialized UI STA.
    pub unsafe fn open(owner: HWND, prefs: LyricsPreferences, english: bool) -> Result<Box<Self>> {
        let library = LoadLibraryW(w!("Msftedit.dll"))?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: CreateSolidBrush(COLORREF(SURFACE)),
            hIcon: crate::brand::class_icons()?.1,
            hIconSm: crate::brand::class_icons()?.0,
            lpszClassName: w!("Nivumi.LyricsReader"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let mut reader = Box::new(Self {
            hwnd: HWND::default(),
            edit: HWND::default(),
            buttons: vec![],
            owner,
            prefs,
            content: Default::default(),
            follow: true,
            reduced_motion: false,
            scroll_motion: Default::default(),
            scroll_target: POINT::default(),
            english,
            scrollbar_drag: false,
            accent: [0.; 3],
            surface: Default::default(),
            _rich_edit: library,
        });
        let scale = GetDpiForWindow(owner).max(96) as f32 / 96.;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let requested = reader.prefs.window_bounds.unwrap_or([
            100,
            100,
            (640. * scale) as i32,
            (560. * scale) as i32,
        ]);
        let reference = RECT {
            left: requested[0],
            top: requested[1],
            right: requested[0].saturating_add(requested[2]),
            bottom: requested[1].saturating_add(requested[3]),
        };
        GetMonitorInfoW(
            MonitorFromRect(&reference, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .ok()?;
        let work = monitor.rcWork;
        let bounds = fit_bounds(
            requested,
            [work.left, work.top, work.right, work.bottom],
            [(400. * scale) as i32, (260. * scale) as i32],
        );
        reader.hwnd = CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_CONTROLPARENT,
            w!("Nivumi.LyricsReader"),
            w!("Nivumi · Letras"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            bounds[0],
            bounds[1],
            bounds[2],
            bounds[3],
            None,
            None,
            Some(instance.into()),
            Some((&mut *reader as *mut Self).cast()),
        )?;
        crate::brand::apply_window(reader.hwnd)?;
        let dark = BOOL(1);
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            reader.hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const BOOL).cast(),
            std::mem::size_of::<BOOL>() as u32,
        );
        use windows::Win32::Graphics::Dwm::*;
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            reader.hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&corner) as u32,
        );
        for (attribute, color) in [
            (DWMWA_CAPTION_COLOR, SURFACE),
            (DWMWA_BORDER_COLOR, theme::BORDER),
        ] {
            let color = COLORREF(color);
            let _ = DwmSetWindowAttribute(
                reader.hwnd,
                attribute,
                (&color as *const COLORREF).cast(),
                std::mem::size_of_val(&color) as u32,
            );
        }
        reader.edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RICHEDIT50W"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | ES_NOHIDESEL) as u32),
            0,
            0,
            1,
            1,
            Some(reader.hwnd),
            Some(HMENU(120usize as *mut _)),
            Some(instance.into()),
            None,
        )?;
        SendMessageW(
            reader.edit,
            EM_EXLIMITTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(1024 * 1024)),
        );
        SendMessageW(
            reader.edit,
            EM_SETBKGNDCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        SendMessageW(
            reader.edit,
            EM_SETTARGETDEVICE,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        SetWindowSubclass(
            reader.edit,
            Some(edit_proc),
            1,
            (&mut *reader as *mut Self) as usize,
        )
        .ok()?;
        let labels = if english {
            [
                "A−",
                "A+",
                "Pin",
                "Following",
                "Sync",
                "Sync+Trans",
                "Lyrics",
                "Translation",
                "Captions",
                "⋯",
            ]
        } else {
            [
                "A−",
                "A+",
                "Fixar",
                "Acompanhando",
                "Sync",
                "Sync+Trad",
                "Letras",
                "Tradução",
                "Legenda",
                "⋯",
            ]
        };
        for (index, label) in labels.iter().enumerate() {
            let id = if index == 9 || index == 4 {
                OPTIONS
            } else if index == 8 {
                CAPTIONS
            } else if index < 4 {
                FONT_DOWN + index
            } else {
                MODE + index - 4
            };
            let button = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                &HSTRING::from(*label),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | if index == 4 {
                        WS_GROUP
                    } else {
                        WINDOW_STYLE(0)
                    }
                    | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                1,
                1,
                Some(reader.hwnd),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )?;
            SendMessageW(
                button,
                WM_SETFONT,
                Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
                Some(LPARAM(1)),
            );
            reader.buttons.push(button);
        }
        for (id, label) in [(MINIMIZE, "−"), (MAXIMIZE, "□"), (CLOSE, "×")] {
            let button = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                &HSTRING::from(label),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                1,
                1,
                Some(reader.hwnd),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )?;
            reader.buttons.push(button);
        }
        let _ = SetWindowPos(
            reader.hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        reader.layout();
        reader.apply_topmost();
        reader.show();
        Ok(reader)
    }
    /// # Safety
    /// Call on the owning UI thread.
    pub unsafe fn show(&mut self) {
        // Display changes can make even a still-open hidden window unreachable.
        self.ensure_reachable();
        let _ = ShowWindow(self.hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(self.hwnd);
        self.render();
    }
    /// # Safety
    /// Call on the owning UI thread, like update.
    pub unsafe fn set_translation_visibility(&mut self, all: bool) {
        if self.prefs.translation_all_verses != all {
            self.prefs.translation_all_verses = all;
            self.render();
        }
    }
    pub fn preferences(&self) -> &LyricsPreferences {
        &self.prefs
    }
    /// # Safety
    /// Call on the owning UI thread; contents come from the existing media host.
    pub unsafe fn update(&mut self, content: LyricsContent) {
        if self.content == content {
            return;
        }
        let changed_track = self.content.key != content.key;
        let changed_title =
            self.content.title != content.title || self.content.artist != content.artist;
        self.content = content;
        if changed_track {
            self.follow = true;
        }
        if changed_title {
            let _ = SetWindowTextW(
                self.hwnd,
                &HSTRING::from(format!(
                    "{} — {} · Nivumi",
                    self.content.title, self.content.artist
                )),
            );
        }
        if IsWindowVisible(self.hwnd).as_bool() {
            self.render();
        }
    }
    /// # Safety
    /// Call on the owning UI thread before saving application state.
    pub unsafe fn remember_bounds(&mut self) {
        if IsZoomed(self.hwnd).as_bool() || IsIconic(self.hwnd).as_bool() {
            return;
        }
        let mut rect = RECT::default();
        if GetWindowRect(self.hwnd, &mut rect).is_ok() {
            self.prefs.window_bounds = Some([
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ]);
        }
    }
    unsafe fn ensure_reachable(&self) {
        let mut rect = RECT::default();
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetWindowRect(self.hwnd, &mut rect).is_ok()
            && GetMonitorInfoW(
                MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
            .as_bool()
        {
            let work = monitor.rcWork;
            let scale = GetDpiForWindow(self.hwnd) as f32 / 96.;
            let bounds = fit_bounds(
                [
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ],
                [work.left, work.top, work.right, work.bottom],
                [(400. * scale) as i32, (260. * scale) as i32],
            );
            let _ = SetWindowPos(
                self.hwnd,
                None,
                bounds[0],
                bounds[1],
                bounds[2],
                bounds[3],
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    unsafe fn changed(&self) {
        let _ = PostMessageW(Some(self.owner), PREFERENCES_CHANGED, WPARAM(0), LPARAM(0));
    }
    unsafe fn apply_topmost(&self) {
        let _ = SetWindowPos(
            self.hwnd,
            Some(if self.prefs.window_topmost {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        if let Some(button) = self.buttons.get(2) {
            let _ = SetWindowTextW(
                *button,
                &HSTRING::from(match (self.english, self.prefs.window_topmost) {
                    (true, true) => "Unpin",
                    (true, false) => "Pin",
                    (false, true) => "Desafixar",
                    (false, false) => "Fixar",
                }),
            );
        }
    }
    /// Use the artwork accent already extracted by the media host, including resets.
    /// # Safety
    /// Call on the reader's owning UI thread.
    pub unsafe fn set_accent(&mut self, accent: [f32; 3]) {
        let accent = theme::artwork_accent(accent);
        if self.accent != accent {
            self.accent = accent;
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            let _ = InvalidateRect(Some(self.edit), None, false);
        }
    }
    unsafe fn paint_background(&self, dc: HDC, rect: RECT, origin_y: i32, height: i32) {
        for y in rect.top..rect.bottom {
            let rgb = lyrics_gradient(
                self.accent,
                self.background(),
                (origin_y + y) as f32 / height.max(1) as f32,
            );
            let brush = CreateSolidBrush(COLORREF(theme::colorref(rgb)));
            let row = RECT {
                top: y,
                bottom: y + 1,
                ..rect
            };
            let _ = FillRect(dc, &row, brush);
            let _ = DeleteObject(brush.into());
        }
    }
    unsafe fn paint_edit(&self) -> bool {
        let mut rect = RECT::default();
        let _ = GetClientRect(self.edit, &mut rect);
        let (width, height) = (rect.right, rect.bottom);
        if width <= 0 || height <= 0 {
            return false;
        }
        let mut cache = self.surface.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|s| s.width != width || s.height != height)
        {
            *cache = ReaderSurface::new(width, height);
        }
        let Some(surface) = cache.as_mut() else {
            return false;
        };
        // WM_PRINTCLIENT preserves RichEdit's wrapping, scrolling and selection. Its
        // black matte is composited over the window gradient, rather than a flat box.
        let pixels = std::slice::from_raw_parts_mut(surface.bits, width as usize * height as usize);
        pixels.fill(0);
        DefSubclassProc(
            self.edit,
            WM_PRINTCLIENT,
            WPARAM(surface.dc.0 as usize),
            LPARAM((PRF_CLIENT | PRF_ERASEBKGND) as isize),
        );
        let _ = GdiFlush();
        let mut window = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut window);
        let mut edit = RECT::default();
        let _ = GetWindowRect(self.edit, &mut edit);
        let mut origin = POINT {
            x: edit.left,
            y: edit.top,
        };
        let _ = ScreenToClient(self.hwnd, &mut origin);
        for (y, row) in pixels.chunks_exact_mut(width as usize).enumerate() {
            let background = lyrics_gradient(
                self.accent,
                self.background(),
                (origin.y + y as i32) as f32 / window.bottom.max(1) as f32,
            );
            for pixel in row {
                *pixel = composite_lyrics_pixel(*pixel, background);
            }
        }
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(self.edit, &mut paint);
        let _ = BitBlt(dc, 0, 0, width, height, Some(surface.dc), 0, 0, SRCCOPY);
        let _ = EndPaint(self.edit, &paint);
        true
    }
    fn background(&self) -> u32 {
        if self.prefs.window_soft_background {
            theme::CARD
        } else {
            theme::LYRICS_SURFACE
        }
    }
    unsafe fn paint(&self) {
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(self.hwnd, &mut paint);
        let mut rect = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rect);
        let scale = GetDpiForWindow(self.hwnd).max(96) as f32 / 96.;
        self.paint_background(dc, rect, 0, rect.bottom);
        let _ = SetBkMode(dc, TRANSPARENT);
        for (text, top, size, bold, rgb) in [
            (
                self.content.title.as_str(),
                14.,
                16.,
                true,
                theme::LYRICS_TEXT,
            ),
            (
                self.content.artist.as_str(),
                37.,
                12.,
                false,
                theme::LYRICS_MUTED,
            ),
        ] {
            let font = CreateFontW(
                -(size * scale) as i32,
                0,
                0,
                0,
                if bold { 600 } else { 400 },
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let old = SelectObject(dc, font.into());
            let _ = SetTextColor(dc, COLORREF(theme::colorref(rgb)));
            let mut bounds = RECT {
                left: (24. * scale) as i32,
                top: (top * scale) as i32,
                right: rect.right - (148. * scale) as i32,
                bottom: ((top + 22.) * scale) as i32,
            };
            let mut text: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                dc,
                &mut text,
                &mut bounds,
                DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            let _ = DeleteObject(font.into());
        }
        if let Some((top, height, _)) = self.scrollbar() {
            let brush = CreateSolidBrush(COLORREF(theme::colorref(theme::MUTED)));
            let old = SelectObject(dc, brush.into());
            let pen = SelectObject(dc, GetStockObject(NULL_PEN));
            let x = rect.right - (14. * scale) as i32;
            let _ = RoundRect(
                dc,
                x,
                top,
                x + (3. * scale).max(2.) as i32,
                top + height,
                4,
                4,
            );
            SelectObject(dc, old);
            SelectObject(dc, pen);
            let _ = DeleteObject(brush.into());
        }
        let _ = EndPaint(self.hwnd, &paint);
    }
    unsafe fn scrollbar(&self) -> Option<(i32, i32, SCROLLINFO)> {
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        if GetScrollInfo(self.edit, SB_VERT, &mut info).is_err() || info.nMax <= info.nPage as i32 {
            return None;
        }
        let mut rect = RECT::default();
        let _ = GetWindowRect(self.edit, &mut rect);
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        let _ = ScreenToClient(self.hwnd, &mut origin);
        let track = rect.bottom - rect.top;
        let height = ((track as f32 * info.nPage as f32 / (info.nMax + 1) as f32) as i32)
            .max(24)
            .min(track);
        let fraction = info.nPos as f32 / (info.nMax - info.nPage as i32 + 1).max(1) as f32;
        Some((
            origin.y + ((track - height) as f32 * fraction) as i32,
            height,
            info,
        ))
    }
    unsafe fn drag_scrollbar(&mut self, y: i32) {
        if let Some((_, height, info)) = self.scrollbar() {
            let mut rect = RECT::default();
            let _ = GetWindowRect(self.edit, &mut rect);
            let mut origin = POINT {
                x: rect.left,
                y: rect.top,
            };
            let _ = ScreenToClient(self.hwnd, &mut origin);
            let fraction = ((y - origin.y - height / 2) as f32
                / (rect.bottom - rect.top - height).max(1) as f32)
                .clamp(0., 1.);
            self.pause_follow();
            let _ = KillTimer(Some(self.edit), 42);
            self.scroll_motion = Default::default();
            self.set_scroll_position(POINT {
                x: 0,
                y: (fraction * (info.nMax - info.nPage as i32 + 1).max(0) as f32) as i32,
            });
        }
    }
    unsafe fn layout(&self) {
        if self.edit.is_invalid() {
            return;
        }
        let mut rect = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rect);
        let scale = GetDpiForWindow(self.hwnd) as f32 / 96.;
        let width = rect.right as f32 / scale;
        let place = |hwnd, x: f32, y: f32, w: f32, h: f32| {
            let _ = MoveWindow(
                hwnd,
                (x * scale) as i32,
                (y * scale) as i32,
                (w.max(1.) * scale) as i32,
                (h.max(1.) * scale) as i32,
                true,
            );
        };
        for button in &self.buttons {
            let _ = ShowWindow(*button, SW_HIDE);
        }
        // A compact mode capsule, then the same quiet icon controls as the notch.
        let mode_width = (width - 172.).clamp(140., 212.);
        if let Some(button) = self.buttons.get(4) {
            place(*button, 24., 60., mode_width, 36.);
            let _ = ShowWindow(*button, SW_SHOWNOACTIVATE);
        }
        if !self.follow {
            if let Some(button) = self.buttons.get(3) {
                place(*button, 24., 100., (width - 72.).min(210.), 32.);
                let _ = ShowWindow(*button, SW_SHOWNOACTIVATE);
            }
        }
        for (index, x, y) in [
            (8, width - 104., 60.),
            (9, width - 60., 60.),
            (10, width - 132., 12.),
            (11, width - 92., 12.),
            (12, width - 52., 12.),
        ] {
            if let Some(button) = self.buttons.get(index) {
                place(*button, x, y, 36., 36.);
                let _ = ShowWindow(*button, SW_SHOWNOACTIVATE);
            }
        }
        let top = if self.follow { 108. } else { 140. };
        place(
            self.edit,
            24.,
            top,
            width - 56.,
            rect.bottom as f32 / scale - top - 24.,
        );
        let _ = ShowScrollBar(self.edit, SB_BOTH, false);
        let mut text_rect = RECT::default();
        let _ = GetClientRect(self.edit, &mut text_rect);
        text_rect.top += (4. * scale) as i32;
        text_rect.right -= (8. * scale) as i32;
        SendMessageW(
            self.edit,
            EM_SETRECT,
            Some(WPARAM(0)),
            Some(LPARAM((&text_rect as *const RECT) as isize)),
        );
    }
    unsafe fn menu(&mut self) {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        for (mode, label) in [
            "Original sincronizado",
            "Original + tradução",
            "Letra completa",
            "Tradução completa",
        ]
        .iter()
        .enumerate()
        {
            let _ = AppendMenuW(
                menu,
                MF_STRING
                    | if self.content.available_mode(mode) != mode {
                        MF_GRAYED
                    } else {
                        MENU_ITEM_FLAGS(0)
                    }
                    | if mode == self.prefs.window_mode {
                        MF_CHECKED
                    } else {
                        MENU_ITEM_FLAGS(0)
                    },
                MODE + mode,
                &HSTRING::from(*label),
            );
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        for (id, label) in [
            (FONT_DOWN, "Diminuir fonte"),
            (FONT_UP, "Aumentar fonte"),
            (FOLLOW, "Voltar ao verso atual"),
            (CAPTIONS, "Modo legenda"),
        ] {
            let _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(label));
        }
        let _ = AppendMenuW(
            menu,
            MF_STRING
                | if self.prefs.window_topmost {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                },
            PIN,
            w!("Manter sobre outras janelas"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING
                | if self.prefs.window_soft_background {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                },
            BACKGROUND,
            &HSTRING::from(if self.english {
                "Softer background"
            } else {
                "Fundo mais suave"
            }),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            COPY_FOR_AI,
            &HSTRING::from(if self.english {
                "Copy lyrics and translation for AI"
            } else {
                "Copiar letra e tradução para IA"
            }),
        );
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _theme = crate::popup_menu::DarkMenu::attach(menu, self.hwnd);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            self.hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        if choice.0 > 0 {
            self.command(choice.0 as usize);
        }
    }
    unsafe fn command(&mut self, id: usize) {
        match id {
            MINIMIZE => {
                let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
                return;
            }
            MAXIMIZE => {
                let _ = ShowWindow(
                    self.hwnd,
                    if IsZoomed(self.hwnd).as_bool() {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    },
                );
                return;
            }
            CLOSE => {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                return;
            }
            OPTIONS => {
                self.menu();
                return;
            }
            CAPTIONS => {
                let _ = PostMessageW(
                    Some(self.owner),
                    crate::subtitles::OPEN_SUBTITLES,
                    WPARAM(0),
                    LPARAM(0),
                );
                return;
            }
            COPY_FOR_AI => {
                let text = self.content.format_for_ai();
                let _ = crate::ole::copy_text(self.hwnd, &text);
                return;
            }
            FONT_DOWN => self.prefs.window_font = (self.prefs.window_font - 1.).max(12.),
            FONT_UP => self.prefs.window_font = (self.prefs.window_font + 1.).min(36.),
            PIN => {
                self.prefs.window_topmost = !self.prefs.window_topmost;
                self.apply_topmost();
            }
            FOLLOW => self.follow = true,
            BACKGROUND => {
                self.prefs.window_soft_background = !self.prefs.window_soft_background;
                let _ = InvalidateRect(Some(self.edit), None, false);
            }
            id if (MODE..MODE + 4).contains(&id) => {
                self.prefs.window_mode = self.content.available_mode(id - MODE);
                self.follow = true;
            }
            _ => return,
        }
        let _ = InvalidateRect(Some(self.hwnd), None, false);
        self.render();
        self.changed();
    }
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }
    unsafe fn scroll_position(&self) -> POINT {
        let mut point = POINT::default();
        SendMessageW(
            self.edit,
            EM_GETSCROLLPOS,
            Some(WPARAM(0)),
            Some(LPARAM((&mut point as *mut POINT) as isize)),
        );
        point
    }
    unsafe fn set_scroll_position(&self, point: POINT) {
        SendMessageW(
            self.edit,
            EM_SETSCROLLPOS,
            Some(WPARAM(0)),
            Some(LPARAM((&point as *const POINT) as isize)),
        );
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }
    unsafe fn animate_scroll(&mut self, from: POINT, target: POINT) {
        let now = std::time::Instant::now();
        self.scroll_target = target;
        self.scroll_motion.retarget(from.y as f32, now, false);
        if self
            .scroll_motion
            .retarget(target.y as f32, now, !self.reduced_motion)
        {
            self.set_scroll_position(from);
            SetTimer(Some(self.edit), 42, 10, None);
        } else {
            self.set_scroll_position(target);
        }
    }
    unsafe fn pause_follow(&mut self) {
        self.follow = false;
        self.follow_label();
    }
    unsafe fn follow_label(&self) {
        if let Some(button) = self.buttons.get(4) {
            let label = if self.english {
                [
                    "Original ▾",
                    "Original + translation ▾",
                    "Full lyrics ▾",
                    "Translation ▾",
                ]
            } else {
                [
                    "Original ▾",
                    "Original + tradução ▾",
                    "Letra completa ▾",
                    "Tradução ▾",
                ]
            };
            let _ = SetWindowTextW(
                *button,
                &HSTRING::from(label[self.prefs.window_mode.min(3)]),
            );
        }
        if let Some(button) = self.buttons.get(8) {
            let _ = SetWindowTextW(*button, w!("CC"));
        }
        if let Some(button) = self.buttons.get(3) {
            let _ = SetWindowTextW(
                *button,
                &HSTRING::from(if self.english {
                    "Current verse"
                } else {
                    "Voltar ao verso atual"
                }),
            );
        }
        self.layout();
    }
    unsafe fn render(&mut self) {
        self.prefs.window_mode = self.content.available_mode(self.prefs.window_mode);
        if self.edit.is_invalid() {
            return;
        }
        let mut scroll = POINT::default();
        let mut selection = CHARRANGE::default();
        SendMessageW(
            self.edit,
            EM_GETSCROLLPOS,
            Some(WPARAM(0)),
            Some(LPARAM((&mut scroll as *mut POINT) as isize)),
        );
        SendMessageW(
            self.edit,
            EM_EXGETSEL,
            Some(WPARAM(0)),
            Some(LPARAM((&mut selection as *mut CHARRANGE) as isize)),
        );
        SendMessageW(self.edit, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        let verses = self.content.verses(self.prefs.window_mode);
        let mut text = String::new();
        let label = self
            .content
            .offline_label(self.prefs.window_mode, self.english);
        if self.content.status == "Traduzindo offline…"
            || self.content.status == "Translating offline…"
        {
            text.push_str(&self.content.status);
            text.push_str("\r\r");
        }
        if !label.is_empty() {
            text.push_str(label);
            text.push_str("\r\r");
        }
        let mut spans = Vec::new();
        let mut offset = text.encode_utf16().count() as i32;
        let mut active = None;
        for verse in &verses {
            let start = offset;
            text.push_str(&verse.text);
            text.push('\r');
            offset += verse.text.encode_utf16().count() as i32 + 1;
            spans.push((start, offset, false, verse.active));
            if verse.active {
                active = Some(start);
            }
            if let Some(translation) = verse
                .translation
                .as_ref()
                .filter(|_| verse.active || self.prefs.translation_all_verses)
            {
                let start = offset;
                text.push_str(translation);
                text.push('\r');
                offset += translation.encode_utf16().count() as i32 + 1;
                spans.push((start, offset, true, verse.active));
            }
        }
        if text.is_empty() {
            text = self
                .content
                .empty_message(self.prefs.window_mode, self.english)
                .into();
        }
        let set_text = SETTEXTEX {
            flags: ST_DEFAULT,
            codepage: 1200,
        };
        let utf16: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        SendMessageW(
            self.edit,
            EM_SETTEXTEX,
            Some(WPARAM((&set_text as *const SETTEXTEX) as usize)),
            Some(LPARAM(utf16.as_ptr() as isize)),
        );
        self.format(
            0,
            -1,
            theme::colorref(theme::MUTED),
            false,
            self.prefs.window_font,
        );
        for (start, end, translated, highlighted) in spans {
            let color = if translated {
                if highlighted {
                    theme::colorref(theme::LYRICS_TRANSLATION_ACTIVE)
                } else {
                    theme::colorref(theme::LYRICS_TRANSLATION)
                }
            } else if highlighted || self.prefs.window_mode >= 2 {
                theme::colorref(theme::LYRICS_TEXT)
            } else {
                theme::colorref(theme::LYRICS_MUTED)
            };
            self.format(
                start,
                end,
                color,
                highlighted && !translated,
                if translated {
                    (self.prefs.window_font * 0.8).max(12.)
                } else {
                    self.prefs.window_font
                },
            );
        }
        let range = CHARRANGE {
            cpMin: 0,
            cpMax: -1,
        };
        SendMessageW(
            self.edit,
            EM_EXSETSEL,
            Some(WPARAM(0)),
            Some(LPARAM((&range as *const CHARRANGE) as isize)),
        );
        let paragraph = PARAFORMAT2 {
            Base: PARAFORMAT {
                cbSize: std::mem::size_of::<PARAFORMAT2>() as u32,
                dwMask: PFM_SPACEAFTER | PFM_LINESPACING,
                ..Default::default()
            },
            dySpaceAfter: 14 * 15,
            bLineSpacingRule: 5,
            dyLineSpacing: 24,
            ..Default::default()
        };
        SendMessageW(
            self.edit,
            EM_SETPARAFORMAT,
            Some(WPARAM(0)),
            Some(LPARAM((&paragraph as *const PARAFORMAT2) as isize)),
        );
        if self.follow {
            let range = CHARRANGE {
                cpMin: active.unwrap_or(0),
                cpMax: active.unwrap_or(0),
            };
            SendMessageW(
                self.edit,
                EM_EXSETSEL,
                Some(WPARAM(0)),
                Some(LPARAM((&range as *const CHARRANGE) as isize)),
            );
            SendMessageW(self.edit, EM_SCROLLCARET, Some(WPARAM(0)), Some(LPARAM(0)));
            let target = self.scroll_position();
            self.animate_scroll(scroll, target);
        } else {
            SendMessageW(
                self.edit,
                EM_EXSETSEL,
                Some(WPARAM(0)),
                Some(LPARAM((&selection as *const CHARRANGE) as isize)),
            );
            SendMessageW(
                self.edit,
                EM_SETSCROLLPOS,
                Some(WPARAM(0)),
                Some(LPARAM((&scroll as *const POINT) as isize)),
            );
        }
        SendMessageW(self.edit, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
        let _ = InvalidateRect(Some(self.edit), None, false);
        self.follow_label();
        let _ = InvalidateRect(Some(self.hwnd), None, false);
        for (index, button) in self.buttons.iter().enumerate().skip(4).take(4) {
            let _ = EnableWindow(*button, self.content.available_mode(index - 4) == index - 4);
            SendMessageW(
                *button,
                BM_SETCHECK,
                Some(WPARAM(usize::from(index - 4 == self.prefs.window_mode))),
                Some(LPARAM(0)),
            );
            let _ = InvalidateRect(Some(*button), None, false);
        }
    }
    unsafe fn format(&self, start: i32, end: i32, color: u32, bold: bool, font: f32) {
        let range = CHARRANGE {
            cpMin: start,
            cpMax: end,
        };
        let mut format = CHARFORMATW {
            cbSize: std::mem::size_of::<CHARFORMATW>() as u32,
            dwMask: CFM_COLOR | CFM_BOLD | CFM_FACE | CFM_SIZE,
            dwEffects: if bold { CFE_BOLD } else { CFE_EFFECTS(0) },
            // Preferences use the same logical pixels as the notch; RichEdit uses twips.
            yHeight: (font * 15.) as i32,
            crTextColor: COLORREF(color),
            ..Default::default()
        };
        let name: Vec<_> = "Segoe UI".encode_utf16().collect();
        format.szFaceName[..name.len()].copy_from_slice(&name);
        SendMessageW(
            self.edit,
            EM_EXSETSEL,
            Some(WPARAM(0)),
            Some(LPARAM((&range as *const CHARRANGE) as isize)),
        );
        SendMessageW(
            self.edit,
            EM_SETCHARFORMAT,
            Some(WPARAM(SCF_SELECTION as usize)),
            Some(LPARAM((&format as *const CHARFORMATW) as isize)),
        );
    }
}
impl Drop for LyricsWindow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let reader = &mut *(data as *mut LyricsWindow);
    if msg == WM_PAINT && reader.paint_edit() {
        return LRESULT(0);
    }
    if msg == WM_ERASEBKGND {
        return LRESULT(1);
    }
    if msg == WM_TIMER && wp.0 == 42 {
        let now = std::time::Instant::now();
        if reader.reduced_motion || !reader.scroll_motion.is_animating(now) {
            reader.set_scroll_position(reader.scroll_target);
            let _ = KillTimer(Some(hwnd), 42);
        } else {
            reader.set_scroll_position(POINT {
                x: reader.scroll_target.x,
                y: reader.scroll_motion.position(now).round() as i32,
            });
        }
        return LRESULT(0);
    }
    let manual_scroll = msg == WM_MOUSEWHEEL
        || msg == WM_VSCROLL
            && ![
                SB_THUMBTRACK.0 as usize,
                SB_THUMBPOSITION.0 as usize,
                SB_ENDSCROLL.0 as usize,
            ]
            .contains(&(wp.0 & 0xffff))
        || msg == WM_KEYDOWN
            && [
                VK_PRIOR.0, VK_NEXT.0, VK_UP.0, VK_DOWN.0, VK_HOME.0, VK_END.0,
            ]
            .contains(&(wp.0 as u16));
    if manual_scroll {
        reader.pause_follow();
        let from = reader.scroll_position();
        // Let RichEdit resolve wheel settings, keyboard navigation and bounds without
        // displaying its immediate jump before the animated position is restored.
        SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        if reader.scroll_motion.is_animating(std::time::Instant::now()) {
            reader.set_scroll_position(reader.scroll_target);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        let target = reader.scroll_position();
        reader.animate_scroll(from, target);
        SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
        let _ = InvalidateRect(Some(hwnd), None, false);
        return result;
    }
    if msg == WM_VSCROLL || msg == WM_NCDESTROY {
        let _ = KillTimer(Some(hwnd), 42);
        reader.scroll_motion = Default::default();
    }
    if msg == WM_CONTEXTMENU {
        let reader = &mut *(data as *mut LyricsWindow);
        reader.menu();
        return LRESULT(0);
    }
    if matches!(msg, WM_MOUSEWHEEL | WM_VSCROLL)
        || msg == WM_KEYDOWN
            && [
                VK_PRIOR.0, VK_NEXT.0, VK_UP.0, VK_DOWN.0, VK_HOME.0, VK_END.0,
            ]
            .contains(&(wp.0 as u16))
    {
        let reader = &mut *(data as *mut LyricsWindow);
        reader.pause_follow();
    }
    if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), 1);
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = crate::popup_menu::handle(msg, wp, lp) {
        return result;
    }
    if msg == WM_NCCREATE {
        let create = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut LyricsWindow;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let reader = &mut *ptr;
    match msg {
        WM_NCCALCSIZE if wp.0 != 0 => LRESULT(0),
        WM_NCHITTEST => {
            let mut point = POINT {
                x: (lp.0 as u32 & 0xffff) as i16 as i32,
                y: ((lp.0 as u32 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut point);
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
            let edge = (7. * scale) as i32;
            let (left, right, top, bottom) = (
                point.x < edge,
                point.x >= rect.right - edge,
                point.y < edge,
                point.y >= rect.bottom - edge,
            );
            let hit = if !IsZoomed(hwnd).as_bool() && (left || right || top || bottom) {
                match (left, right, top, bottom) {
                    (true, _, true, _) => HTTOPLEFT,
                    (_, true, true, _) => HTTOPRIGHT,
                    (true, _, _, true) => HTBOTTOMLEFT,
                    (_, true, _, true) => HTBOTTOMRIGHT,
                    (true, _, _, _) => HTLEFT,
                    (_, true, _, _) => HTRIGHT,
                    (_, _, true, _) => HTTOP,
                    _ => HTBOTTOM,
                }
            } else if point.y < (56. * scale) as i32 && point.x < rect.right - (140. * scale) as i32
            {
                HTCAPTION
            } else {
                HTCLIENT
            };
            LRESULT(hit as isize)
        }
        WM_PAINT => {
            reader.paint();
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let x = (lp.0 as u32 & 0xffff) as i16 as i32;
            let y = ((lp.0 as u32 >> 16) & 0xffff) as i16 as i32;
            if x >= rect.right - (22. * GetDpiForWindow(hwnd).max(96) as f32 / 96.) as i32
                && reader.scrollbar().is_some()
            {
                reader.scrollbar_drag = true;
                SetCapture(hwnd);
                reader.drag_scrollbar(y);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE if reader.scrollbar_drag => {
            reader.drag_scrollbar(((lp.0 as u32 >> 16) & 0xffff) as i16 as i32);
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            reader.scrollbar_drag = false;
            if msg == WM_LBUTTONUP {
                let _ = ReleaseCapture();
            }
            LRESULT(0)
        }

        WM_CONTEXTMENU => {
            reader.menu();
            LRESULT(0)
        }
        WM_SIZE => {
            reader.layout();
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_DRAWITEM => {
            let item = &*(lp.0 as *const DRAWITEMSTRUCT);
            let selected = (item.CtlID as usize >= MODE
                && item.CtlID as usize == MODE + reader.prefs.window_mode)
                || (item.CtlID as usize == PIN && reader.prefs.window_topmost);
            let mut button_rect = RECT::default();
            let mut window_rect = RECT::default();
            let _ = GetWindowRect(item.hwndItem, &mut button_rect);
            let _ = GetClientRect(hwnd, &mut window_rect);
            let mut origin = POINT {
                x: button_rect.left,
                y: button_rect.top,
            };
            let _ = ScreenToClient(hwnd, &mut origin);
            reader.paint_background(item.hDC, item.rcItem, origin.y, window_rect.bottom);
            let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
            let capsule =
                reader.buttons.get(4) == Some(&item.hwndItem) || item.CtlID as usize == FOLLOW;
            let fill_color = if pressed {
                theme::INPUT
            } else if capsule {
                theme::LYRICS_PILL
            } else {
                reader.background()
            };
            let fill = CreateSolidBrush(COLORREF(theme::colorref(fill_color)));
            let outline = CreatePen(PS_SOLID, 1, COLORREF(theme::colorref(fill_color)));
            let previous_brush = SelectObject(item.hDC, fill.into());
            let previous_pen = SelectObject(item.hDC, outline.into());
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
            let inset = (2. * scale).round() as i32;
            let radius = (36. * scale).round() as i32;
            if capsule || pressed {
                let _ = RoundRect(
                    item.hDC,
                    item.rcItem.left + inset,
                    item.rcItem.top + inset,
                    item.rcItem.right - inset,
                    item.rcItem.bottom - inset,
                    radius,
                    radius,
                );
            }
            SelectObject(item.hDC, previous_brush);
            SelectObject(item.hDC, previous_pen);
            let _ = DeleteObject(fill.into());
            let _ = DeleteObject(outline.into());
            let _ = SetBkMode(item.hDC, TRANSPARENT);
            let _ = SetTextColor(item.hDC, COLORREF(theme::colorref(theme::TEXT)));
            let font = CreateFontW(
                -((if capsule { 14. } else { 20. }) * scale).round() as i32,
                0,
                0,
                0,
                if selected { 600 } else { 400 },
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let old = SelectObject(
                item.hDC,
                if font.is_invalid() {
                    GetStockObject(DEFAULT_GUI_FONT)
                } else {
                    font.into()
                },
            );
            let mut label = [0u16; 128];
            let count = GetWindowTextW(item.hwndItem, &mut label);
            let mut rect = item.rcItem;
            DrawTextW(
                item.hDC,
                &mut label[..count as usize],
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            if item.itemState.0 & ODS_FOCUS.0 != 0 {
                let _ = DrawFocusRect(item.hDC, &rect);
            }
            SelectObject(item.hDC, old);
            if !font.is_invalid() {
                let _ = DeleteObject(font.into());
            }
            LRESULT(1)
        }
        WM_COMMAND => {
            if (wp.0 >> 16) == BN_CLICKED as usize {
                reader.command(wp.0 & 0xffff);
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let scale = GetDpiForWindow(hwnd) as f32 / 96.;
            let info = &mut *(lp.0 as *mut MINMAXINFO);
            info.ptMinTrackSize = POINT {
                x: (400. * scale) as i32,
                y: (260. * scale) as i32,
            };
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            reader.remember_bounds();
            reader.changed();
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            reader.ensure_reachable();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rect = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            reader.layout();
            reader.remember_bounds();
            reader.changed();
            LRESULT(0)
        }
        WM_CLOSE => {
            reader.remember_bounds();
            reader.changed();
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;
    #[test]
    fn artwork_gradient_matches_notch_stops_and_resets_without_artwork() {
        let accent = [0.8, 0.4, 0.2];
        assert_eq!(lyrics_gradient(accent, 0, 0.32), 0);
        assert_eq!(lyrics_gradient(accent, 0, 0.68), 0x3d1f0f);
        assert_eq!(lyrics_gradient(accent, 0, 1.), 0xcc6633);
        assert_eq!(lyrics_gradient([0.; 3], 0, 1.), 0);
        assert_eq!(lyrics_gradient([0.; 3], theme::CARD, 1.), theme::CARD);
    }
    #[test]
    fn compositing_preserves_white_text_and_continuous_background() {
        assert_eq!(composite_lyrics_pixel(0, 0x123456), 0x123456);
        assert_eq!(composite_lyrics_pixel(0xffffff, 0x123456), 0xffffff);
        assert_eq!(composite_lyrics_pixel(0x9a9aa4, 0), 0x9a9aa4);
    }
}
