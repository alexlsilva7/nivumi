//! Per-pixel transparent caption overlay; playback and lyrics remain owned by the media host.
use crate::{
    accessibility::*,
    lyrics::{active_line, LyricLine},
    lyrics_view::fit_bounds,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Gdi::*,
        },
        System::LibraryLoader::*,
        UI::{Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};
use windows_numerics::Vector2;

pub const OPEN_SUBTITLES: u32 = WM_APP + 15;
pub const SUBTITLE_CHANGED: u32 = WM_APP + 16;
pub const CAPTURE_SHORTCUT: u32 = WM_APP + 17;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubtitlePreferences {
    pub font: f32,
    pub translation_font: f32,
    pub opacity: f32,
    pub bounds: Option<[i32; 4]>,
    pub mode: usize,
    pub background: bool,
    pub delay_ms: i64,
    pub shortcut: usize,
    pub sync_hotkeys: [Option<[u32; 2]>; 3],
}
impl Default for SubtitlePreferences {
    fn default() -> Self {
        Self {
            font: 36.,
            translation_font: 28.,
            opacity: 1.,
            bounds: None,
            mode: 0,
            background: false,
            delay_ms: 0,
            shortcut: 0,
            sync_hotkeys: [Some([3, 0xBD]), Some([3, 0xBB]), Some([3, 0x30])],
        }
    }
}
impl SubtitlePreferences {
    pub fn normalize(&mut self) {
        self.font = if self.font.is_finite() {
            self.font.clamp(18., 96.)
        } else {
            36.
        };
        self.translation_font = if self.translation_font.is_finite() {
            self.translation_font.clamp(14., 96.)
        } else {
            28.
        };
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.2, 1.)
        } else {
            1.
        };
        self.mode = self.mode.min(2);
        self.delay_ms = self.delay_ms.clamp(-10000, 10000);
        self.shortcut = self.shortcut.min(2);
        for binding in &mut self.sync_hotkeys {
            if binding.is_some_and(|[modifiers, key]| !valid_binding(modifiers, key)) {
                *binding = None;
            }
        }
    }
    pub fn hotkey(&self) -> (HOT_KEY_MODIFIERS, u32) {
        match self.shortcut {
            1 => (MOD_CONTROL | MOD_ALT, b'S' as u32),
            2 => (MOD_CONTROL | MOD_SHIFT, b'L' as u32),
            _ => (MOD_CONTROL | MOD_ALT, b'L' as u32),
        }
    }
}

pub fn valid_binding(modifiers: u32, key: u32) -> bool {
    modifiers & !7 == 0
        && modifiers & 3 != 0
        && (0x20..=0xFE).contains(&key)
        && ![0x5B, 0x5C].contains(&key)
}
fn binding_label(binding: Option<[u32; 2]>) -> String {
    let Some([mods, key]) = binding else {
        return "desativado".into();
    };
    let mut parts = vec![];
    if mods & 2 != 0 {
        parts.push("Ctrl".into());
    }
    if mods & 1 != 0 {
        parts.push("Alt".into());
    }
    if mods & 4 != 0 {
        parts.push("Shift".into());
    }
    parts.push(match key {
        0xBD => "−".into(),
        0xBB => "+".into(),
        0x25 => "Esquerda".into(),
        0x27 => "Direita".into(),
        0x26 => "Cima".into(),
        0x28 => "Baixo".into(),
        0x70..=0x87 => format!("F{}", key - 0x6F),
        0x30..=0x5A => char::from_u32(key).unwrap().to_string(),
        _ => format!("Tecla {key}"),
    });
    parts.join("+")
}
pub fn caption_motion(elapsed_ms: u64) -> (f32, f32) {
    let t = (elapsed_ms as f32 / 180.).clamp(0., 1.);
    let ease = 1. - (1. - t).powi(3);
    (0.75 + 0.25 * ease, 3. * (1. - ease))
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyncFeedback {
    pub delay_ms: i64,
    pub previous_ms: i64,
    pub start: std::time::Instant,
}
impl SyncFeedback {
    pub fn new(delay_ms: i64, previous_ms: i64) -> Self {
        Self {
            delay_ms,
            previous_ms,
            start: std::time::Instant::now(),
        }
    }
    pub fn progress(&self, reduced_motion: bool) -> (f32, f32, f32) {
        sync_hud_motion(
            self.start.elapsed().as_millis() as u64,
            self.previous_ms,
            self.delay_ms,
            reduced_motion,
        )
    }
}
pub fn sync_hud_motion(
    elapsed_ms: u64,
    previous_ms: i64,
    target_ms: i64,
    reduced_motion: bool,
) -> (f32, f32, f32) {
    if elapsed_ms >= 1500 {
        return (0., 0., target_ms as f32);
    }
    if reduced_motion {
        return (1., 0., target_ms as f32);
    }
    let alpha = if elapsed_ms < 160 {
        let t = elapsed_ms as f32 / 160.;
        1. - (1. - t).powi(3)
    } else if elapsed_ms < 1150 {
        1.
    } else {
        1. - ((elapsed_ms - 1150) as f32 / 350.).clamp(0., 1.)
    };
    let offset_y = if elapsed_ms < 160 {
        let t = elapsed_ms as f32 / 160.;
        (1. - (1. - (1. - t).powi(3))) * 6.
    } else {
        0.
    };
    let meter_t = (elapsed_ms as f32 / 140.).clamp(0., 1.);
    let meter_ease = 1. - (1. - meter_t).powi(3);
    let current_ms = previous_ms as f32 + (target_ms - previous_ms) as f32 * meter_ease;
    (alpha, offset_y, current_ms)
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptionText {
    pub primary: String,
    pub secondary: String,
}
pub fn caption_text(
    lines: &[LyricLine],
    position_ms: i64,
    prefs: &SubtitlePreferences,
) -> CaptionText {
    let Some(line) =
        active_line(lines, position_ms.saturating_add(prefs.delay_ms)).map(|index| &lines[index])
    else {
        return CaptionText::default();
    };
    if line.text.trim().is_empty() || line.text.trim() == "♪" || line.text.starts_with('[') {
        return CaptionText::default();
    }
    match prefs.mode {
        1 => CaptionText {
            primary: line.translation.clone().unwrap_or_default(),
            secondary: String::new(),
        },
        2 => CaptionText {
            primary: line.text.clone(),
            secondary: line.translation.clone().unwrap_or_default(),
        },
        _ => CaptionText {
            primary: line.text.clone(),
            secondary: String::new(),
        },
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    #[cfg(test)]
    bits: *mut u32,
}
impl Surface {
    unsafe fn new(width: i32, height: i32) -> Result<Self> {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return Err(Error::from_thread());
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
            Err(error) => {
                let _ = DeleteDC(dc);
                return Err(error);
            }
        };
        let previous = SelectObject(dc, bitmap.into());
        Ok(Self {
            dc,
            bitmap,
            previous,
            #[cfg(test)]
            bits: bits.cast(),
        })
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct SubtitleWindow {
    pub hwnd: HWND,
    owner: HWND,
    prefs: SubtitlePreferences,
    editing: bool,
    capturing: Option<usize>,
    transition: Option<std::time::Instant>,
    sync_feedback: Option<SyncFeedback>,
    reduced_motion: bool,
    allow_show: bool,
    return_focus: HWND,
    #[cfg(test)]
    alpha_stats: (usize, usize),
    text: CaptionText,
    factory: ID2D1Factory,
    write: IDWriteFactory,
    dragging: Option<(POINT, RECT, i32)>,
    accessible: Arc<Mutex<Vec<AccessibleItem>>>,
    provider: Option<IRawElementProviderSimple>,
}
impl SubtitleWindow {
    /// # Safety
    /// All methods and destruction must run on the application's UI STA.
    pub unsafe fn open(owner: HWND, prefs: SubtitlePreferences) -> Result<Box<Self>> {
        Self::create(owner, prefs, true)
    }
    unsafe fn create(
        owner: HWND,
        mut prefs: SubtitlePreferences,
        allow_show: bool,
    ) -> Result<Box<Self>> {
        prefs.normalize();
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: crate::brand::class_icons()?.1,
            hIconSm: crate::brand::class_icons()?.0,
            lpszClassName: w!("Nivumi.Subtitles"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let mut window = Box::new(Self {
            hwnd: HWND::default(),
            owner,
            prefs,
            editing: true,
            capturing: None,
            transition: None,
            sync_feedback: None,
            reduced_motion: false,
            allow_show,
            return_focus: GetForegroundWindow(),
            #[cfg(test)]
            alpha_stats: (0, 0),
            text: Default::default(),
            factory: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
            write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            dragging: None,
            accessible: Arc::new(Mutex::new(vec![])),
            provider: None,
        });
        let scale = GetDpiForWindow(owner).max(96) as f32 / 96.;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .ok()?;
        let work = monitor.rcWork;
        let width = ((1000. * scale) as i32).min(work.right - work.left - 32);
        let initial = window.prefs.bounds.unwrap_or([
            work.left + (work.right - work.left - width) / 2,
            work.bottom - (240. * scale) as i32,
            width,
            (180. * scale) as i32,
        ]);
        window.hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("Nivumi.Subtitles"),
            w!("Nivumi · Legenda"),
            WS_POPUP,
            initial[0],
            initial[1],
            initial[2].max(200),
            initial[3].max(100),
            None,
            None,
            Some(instance.into()),
            Some((&mut *window as *mut Self).cast()),
        )?;
        crate::brand::apply_window(window.hwnd)?;
        window.provider = Some(provider(window.hwnd, window.accessible.clone()));
        window.reachable();
        window.render()?;
        if allow_show {
            let _ = ShowWindow(window.hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(window)
    }
    /// # Safety
    /// Call on the overlay's UI thread.
    pub unsafe fn set_reduced_motion(&mut self, reduced: bool) {
        if reduced {
            let had_transition = self.transition.take().is_some();
            let had_sync = self.sync_feedback.take().is_some();
            if had_transition || had_sync {
                let _ = KillTimer(Some(self.hwnd), 1);
                let _ = self.render();
            }
        }
        self.reduced_motion = reduced;
    }
    /// # Safety
    /// Call on the overlay's UI thread. None resets the compensation.
    pub unsafe fn adjust_sync(&mut self, delta: Option<i64>) {
        let previous = self.prefs.delay_ms;
        let current_animated = self
            .sync_feedback
            .map(|f| f.progress(self.reduced_motion).2.round() as i64)
            .unwrap_or(previous);
        let new_delay = delta
            .map(|d| self.prefs.delay_ms.saturating_add(d).clamp(-10000, 10000))
            .unwrap_or(0);
        self.prefs.delay_ms = new_delay;
        self.sync_feedback = Some(SyncFeedback::new(new_delay, current_animated));
        SetTimer(Some(self.hwnd), 1, 16, None);
        let _ = self.render();
        self.changed();
    }
    pub fn sync_feedback(&self) -> Option<&SyncFeedback> {
        self.sync_feedback.as_ref()
    }
    unsafe fn end_capture(&mut self) {
        self.capturing = None;
        let _ = PostMessageW(Some(self.owner), CAPTURE_SHORTCUT, WPARAM(0), LPARAM(0));
        let _ = self.render();
    }
    /// Change song compensation without carrying the previous song's HUD into it.
    pub fn set_song_delay(&mut self, delay_ms: i64) {
        let delay_ms = delay_ms.clamp(-10000, 10000);
        if self.prefs.delay_ms != delay_ms {
            self.prefs.delay_ms = delay_ms;
            self.sync_feedback = None;
        }
    }
    pub fn preferences(&self) -> &SubtitlePreferences {
        &self.prefs
    }
    /// # Safety
    /// Call on the owning UI thread using the player's current position.
    pub unsafe fn update(&mut self, lines: &[LyricLine], position: i64) {
        let text = caption_text(lines, position, &self.prefs);
        if self.text != text {
            self.text = text;
            self.transition = if self.allow_show
                && !self.reduced_motion
                && !self.text.primary.is_empty()
                && IsWindowVisible(self.hwnd).as_bool()
            {
                Some(std::time::Instant::now())
            } else {
                None
            };
            if self.transition.is_some() {
                SetTimer(Some(self.hwnd), 1, 16, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), 1);
            }
            let _ = self.render();
        }
    }
    /// # Safety
    /// Call on the owning UI thread.
    pub unsafe fn edit(&mut self, editing: bool) {
        if editing && GetForegroundWindow() != self.hwnd {
            self.return_focus = GetForegroundWindow();
        }
        self.editing = editing;
        let mut style = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
        if editing {
            style &= !(WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0);
        } else {
            style |= WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0;
        }
        SetWindowLongW(self.hwnd, GWL_EXSTYLE, style as i32);
        let _ = SetWindowPos(
            self.hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        self.reachable();
        let _ = self.render();
        if self.allow_show {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            if editing {
                let _ = SetForegroundWindow(self.hwnd);
            } else if GetForegroundWindow() == self.hwnd
                && IsWindow(Some(self.return_focus)).as_bool()
            {
                let _ = SetForegroundWindow(self.return_focus);
            }
        }
    }
    /// # Safety
    /// Call on the owning UI thread. Hidden windows reopen in adjustment mode.
    pub unsafe fn toggle(&mut self) {
        self.edit(!self.editing || !IsWindowVisible(self.hwnd).as_bool());
    }
    unsafe fn changed(&self) {
        let _ = PostMessageW(Some(self.owner), SUBTITLE_CHANGED, WPARAM(0), LPARAM(0));
    }
    unsafe fn remember(&mut self) {
        let mut rect = RECT::default();
        if GetWindowRect(self.hwnd, &mut rect).is_ok() {
            self.prefs.bounds = Some([
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ]);
        }
    }
    unsafe fn reachable(&self) {
        let mut rect = RECT::default();
        let _ = GetWindowRect(self.hwnd, &mut rect);
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            let w = monitor.rcWork;
            let bounds = fit_bounds(
                [
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ],
                [w.left, w.top, w.right, w.bottom],
                [
                    (360. * GetDpiForWindow(self.hwnd).max(96) as f32 / 96.) as i32,
                    (100. * GetDpiForWindow(self.hwnd).max(96) as f32 / 96.) as i32,
                ],
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
    unsafe fn layout(&self, text: &str, width: f32, size: f32) -> Result<IDWriteTextLayout> {
        let format = self.write.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("pt-BR"),
        )?;
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        self.write.CreateTextLayout(
            &text.encode_utf16().collect::<Vec<_>>(),
            &format,
            width.max(1.),
            100000.,
        )
    }
    unsafe fn menu(&mut self) {
        if !self.editing {
            return;
        }
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        type MenuEntry = (usize, String, bool);
        let groups: [(&str, Vec<MenuEntry>); 5] = [
            (
                "Tamanho do texto",
                vec![
                    (
                        100,
                        format!(
                            "Diminuir fonte ({} → {})",
                            self.prefs.font,
                            (self.prefs.font - 2.).max(18.)
                        ),
                        false,
                    ),
                    (101, "Aumentar fonte".into(), false),
                ],
            ),
            (
                "Mostrar",
                vec![
                    (102, "Original".into(), self.prefs.mode == 0),
                    (103, "Tradução".into(), self.prefs.mode == 1),
                    (104, "Original e tradução".into(), self.prefs.mode == 2),
                ],
            ),
            (
                "Sincronização",
                vec![
                    (106, "Atrasar 0,1 segundo".into(), false),
                    (107, "Adiantar 0,1 segundo".into(), false),
                    (
                        203,
                        format!(
                            "Zerar ajuste ({:+.1} s)",
                            self.prefs.delay_ms as f64 / 1000.
                        ),
                        false,
                    ),
                ],
            ),
            (
                "Atalho para ajustar / travar",
                ["Ctrl+Alt+L", "Ctrl+Alt+S", "Ctrl+Shift+L"]
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        (200 + index, (*label).into(), self.prefs.shortcut == index)
                    })
                    .collect(),
            ),
            ("Atalhos de sincronização", {
                let names = ["Atrasar", "Adiantar", "Zerar"];
                let mut entries: Vec<MenuEntry> = names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        (
                            300 + index,
                            format!(
                                "{name}: {} — alterar",
                                binding_label(self.prefs.sync_hotkeys[index])
                            ),
                            false,
                        )
                    })
                    .collect();
                entries.extend(names.iter().enumerate().map(|(index, name)| {
                    (
                        310 + index,
                        format!("Desativar: {name}"),
                        self.prefs.sync_hotkeys[index].is_none(),
                    )
                }));
                entries
            }),
        ];
        for (label, entries) in groups {
            if let Ok(submenu) = CreatePopupMenu() {
                for (id, text, checked) in entries {
                    let _ = AppendMenuW(
                        submenu,
                        MF_STRING
                            | if checked {
                                MF_CHECKED
                            } else {
                                MENU_ITEM_FLAGS(0)
                            },
                        id,
                        &HSTRING::from(text),
                    );
                }
                let _ = AppendMenuW(menu, MF_POPUP, submenu.0 as usize, &HSTRING::from(label));
            }
        }
        let _ = AppendMenuW(
            menu,
            MF_STRING
                | if self.prefs.background {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                },
            105,
            w!("Faixa escura atrás do texto"),
        );
        for (id, label) in [
            (204, "Diminuir fonte da tradução"),
            (205, "Aumentar fonte da tradução"),
            (206, "Diminuir opacidade"),
            (207, "Aumentar opacidade"),
        ] {
            let _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(label));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 109, w!("Travar legenda"));
        let _ = AppendMenuW(menu, MF_STRING, 110, w!("Ocultar legenda"));
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(self.hwnd);
        let _menu_theme = crate::popup_menu::DarkMenu::attach(menu, self.hwnd);
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
        match choice.0 as usize {
            id @ 300..=302 => {
                self.capturing = Some(id - 300);
                self.transition = None;
                let _ = PostMessageW(Some(self.owner), CAPTURE_SHORTCUT, WPARAM(1), LPARAM(0));
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
                let _ = self.render();
            }
            id @ 310..=312 => {
                self.prefs.sync_hotkeys[id - 310] = None;
                self.changed();
            }
            id @ 100..=110 => self.command(id - 100),
            id @ 200..=202 => {
                self.prefs.shortcut = id - 200;
                self.changed();
            }
            id @ 204..=207 => {
                match id {
                    204 => {
                        self.prefs.translation_font = (self.prefs.translation_font - 2.).max(14.)
                    }
                    205 => {
                        self.prefs.translation_font = (self.prefs.translation_font + 2.).min(96.)
                    }
                    206 => self.prefs.opacity = (self.prefs.opacity - 0.1).max(0.2),
                    _ => self.prefs.opacity = (self.prefs.opacity + 0.1).min(1.),
                }
                let _ = self.render();
                self.changed();
            }
            203 => {
                self.adjust_sync(None);
            }
            _ => {}
        }
    }
    unsafe fn command(&mut self, id: usize) {
        match id {
            0 => self.prefs.font = (self.prefs.font - 2.).max(18.),
            1 => self.prefs.font = (self.prefs.font + 2.).min(96.),
            2..=4 => self.prefs.mode = id - 2,
            5 => self.prefs.background = !self.prefs.background,
            6 => self.adjust_sync(Some(-100)),
            7 => self.adjust_sync(Some(100)),
            9 => self.edit(false),
            10 => {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            _ => return,
        }
        let _ = self.render();
        self.remember();
        self.changed();
    }
    unsafe fn render(&mut self) -> Result<()> {
        let scale = GetDpiForWindow(self.hwnd).max(96) as f32 / 96.;
        let mut rect = RECT::default();
        GetWindowRect(self.hwnd, &mut rect)?;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(
            MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .ok()?;
        let work = monitor.rcWork;
        let width =
            (rect.right - rect.left).clamp(200.min(work.right - work.left), work.right - work.left);
        let logical_width = width as f32 / scale;
        let capture_hint = self.capturing.map(|index| {
            format!(
                "{}: pressione Ctrl ou Alt + uma tecla (Esc cancela)",
                ["Atrasar", "Adiantar", "Zerar"][index]
            )
        });
        let display_text = if let Some(hint) = &capture_hint {
            hint
        } else if self.text.primary.is_empty() {
            "♪"
        } else {
            &self.text.primary
        };
        let primary = self.layout(display_text, logical_width - 32., self.prefs.font)?;
        let secondary = self.layout(
            &self.text.secondary,
            logical_width - 32.,
            self.prefs.translation_font,
        )?;
        let mut pm = DWRITE_TEXT_METRICS::default();
        primary.GetMetrics(&mut pm)?;
        let mut sm = DWRITE_TEXT_METRICS::default();
        secondary.GetMetrics(&mut sm)?;
        let toolbar = 40.;
        let text_height = if display_text.is_empty() {
            0.
        } else {
            pm.height
                + if self.text.secondary.is_empty() {
                    0.
                } else {
                    sm.height + 6.
                }
        };
        let height = (((toolbar + text_height + 24.).max(32.) * scale).ceil() as i32)
            .min(work.bottom - work.top);
        rect.left = rect.left.clamp(work.left, work.right - width);
        rect.top = rect.top.clamp(work.top, work.bottom - height);
        let surface = Surface::new(width, height)?;
        let target = self
            .factory
            .CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: scale * 96.,
                dpiY: scale * 96.,
                ..Default::default()
            })?;
        target.BindDC(
            surface.dc,
            &RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
        )?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        target.BeginDraw();
        target.Clear(Some(&D2D1_COLOR_F {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        }));
        let brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 0.,
                g: 0.,
                b: 0.,
                a: 0.65,
            },
            None,
        )?;
        if self.editing || self.prefs.background && !self.text.primary.is_empty() {
            if !self.prefs.background {
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: 0.02,
                });
            }
            target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.,
                        top: toolbar,
                        right: logical_width,
                        bottom: height as f32 / scale,
                    },
                    radiusX: 12.,
                    radiusY: 12.,
                },
                &brush,
            );
        }
        let mut items = vec![];
        if self.editing {
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.75,
                g: 0.8,
                b: 0.9,
                a: 0.3,
            });
            target.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 1.,
                        top: toolbar + 1.,
                        right: logical_width - 1.,
                        bottom: height as f32 / scale - 1.,
                    },
                    radiusX: 10.,
                    radiusY: 10.,
                },
                &brush,
                1.,
                None,
            );
            items.push(AccessibleItem {
                label: "Configurar legenda · botão direito ou Shift+F10".into(),
                action_id: 11,
                enabled: true,
                rect: RECT {
                    left: 0,
                    top: (toolbar * scale) as i32,
                    right: width,
                    bottom: height,
                },
            });
            if self.text.primary.is_empty() {
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.8,
                    g: 0.85,
                    b: 0.9,
                    a: 0.8,
                });
                target.DrawTextLayout(
                    Vector2 { X: 16., Y: 5. },
                    &self.layout("", logical_width - 32., 14.)?,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
        if !display_text.is_empty() {
            let (fade, offset) = self
                .transition
                .map(|start| caption_motion(start.elapsed().as_millis() as u64))
                .unwrap_or((1., 0.));
            let paint = |layout: &IDWriteTextLayout, y: f32, translated: bool| {
                let y = y + offset;
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: 0.65 * fade,
                });
                target.DrawTextLayout(
                    Vector2 { X: 18., Y: y + 3. },
                    layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: fade,
                });
                for (dx, dy) in [
                    (-1.5, 0.),
                    (1.5, 0.),
                    (0., -1.5),
                    (0., 1.5),
                    (-1., -1.),
                    (1., 1.),
                    (-1., 1.),
                    (1., -1.),
                ] {
                    target.DrawTextLayout(
                        Vector2 {
                            X: 16. + dx,
                            Y: y + dy,
                        },
                        layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
                brush.SetColor(&D2D1_COLOR_F {
                    r: if translated { 0.7 } else { 1. },
                    g: if translated { 0.9 } else { 1. },
                    b: 1.,
                    a: fade,
                });
                target.DrawTextLayout(
                    Vector2 { X: 16., Y: y },
                    layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            };
            paint(&primary, toolbar + 8., false);
            if !self.text.secondary.is_empty() {
                paint(&secondary, toolbar + 14. + pm.height, true);
            }
        }
        if let Some(feedback) = &self.sync_feedback {
            self.draw_sync_hud(&target, &brush, logical_width, feedback)?;
        }
        target.EndDraw(None, None)?;
        #[cfg(test)]
        {
            let pixels = std::slice::from_raw_parts(surface.bits, (width * height) as usize);
            self.alpha_stats = (
                pixels.iter().filter(|pixel| **pixel >> 24 == 0).count(),
                pixels.iter().filter(|pixel| **pixel >> 24 == 255).count(),
            );
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (self.prefs.opacity * 255.).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            self.hwnd,
            None,
            Some(&POINT {
                x: rect.left,
                y: rect.top,
            }),
            Some(&SIZE {
                cx: width,
                cy: height,
            }),
            Some(surface.dc),
            Some(&POINT::default()),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )?;
        if let Ok(mut shared) = self.accessible.lock() {
            *shared = items;
        }
        let _ = SetWindowTextW(
            self.hwnd,
            &HSTRING::from(format!("Nivumi · Legenda — {}", self.text.primary)),
        );
        Ok(())
    }
    unsafe fn draw_sync_hud(
        &self,
        target: &ID2D1DCRenderTarget,
        brush: &ID2D1SolidColorBrush,
        logical_width: f32,
        feedback: &SyncFeedback,
    ) -> Result<()> {
        let (alpha, offset_y, current_ms) = feedback.progress(self.reduced_motion);
        if alpha <= 0.001 {
            return Ok(());
        }
        let pill_width = 270.0f32.min(logical_width - 16.0);
        let pill_height = 32.0f32;
        let pill_x = (logical_width - pill_width) / 2.0;
        let pill_y = 4.0 + offset_y;

        brush.SetColor(&D2D1_COLOR_F {
            r: 0.08,
            g: 0.09,
            b: 0.12,
            a: 0.92 * alpha,
        });
        let pill_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: pill_x,
                top: pill_y,
                right: pill_x + pill_width,
                bottom: pill_y + pill_height,
            },
            radiusX: 16.0,
            radiusY: 16.0,
        };
        target.FillRoundedRectangle(&pill_rect, brush);

        brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.20 * alpha,
        });
        target.DrawRoundedRectangle(&pill_rect, brush, 1.0, None);

        let delay_sec = current_ms / 1000.0;
        let is_zero = current_ms.abs() < 10.0;
        let is_advanced = current_ms >= 10.0;
        let (label, accent_color) = if is_zero {
            (
                "Sincronizado: 0,0 s".to_string(),
                D2D1_COLOR_F {
                    r: 0.30,
                    g: 0.90,
                    b: 0.55,
                    a: alpha,
                },
            )
        } else if is_advanced {
            (
                format!("Adiantado: +{:.1} s", delay_sec).replace('.', ","),
                D2D1_COLOR_F {
                    r: 0.35,
                    g: 0.75,
                    b: 1.0,
                    a: alpha,
                },
            )
        } else {
            (
                format!("Atrasado: {:.1} s", delay_sec).replace('.', ","),
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.68,
                    b: 0.28,
                    a: alpha,
                },
            )
        };

        let icon_cx = pill_x + 18.0;
        let icon_cy = pill_y + 16.0;
        brush.SetColor(&accent_color);
        if is_zero {
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: Vector2 {
                        X: icon_cx,
                        Y: icon_cy,
                    },
                    radiusX: 4.0,
                    radiusY: 4.0,
                },
                brush,
            );
        } else if is_advanced {
            target.DrawLine(
                Vector2 {
                    X: icon_cx - 4.0,
                    Y: icon_cy - 4.5,
                },
                Vector2 {
                    X: icon_cx - 0.5,
                    Y: icon_cy,
                },
                brush,
                1.8,
                None,
            );
            target.DrawLine(
                Vector2 {
                    X: icon_cx - 0.5,
                    Y: icon_cy,
                },
                Vector2 {
                    X: icon_cx - 4.0,
                    Y: icon_cy + 4.5,
                },
                brush,
                1.8,
                None,
            );
            target.DrawLine(
                Vector2 {
                    X: icon_cx + 1.0,
                    Y: icon_cy - 4.5,
                },
                Vector2 {
                    X: icon_cx + 4.5,
                    Y: icon_cy,
                },
                brush,
                1.8,
                None,
            );
            target.DrawLine(
                Vector2 {
                    X: icon_cx + 4.5,
                    Y: icon_cy,
                },
                Vector2 {
                    X: icon_cx + 1.0,
                    Y: icon_cy + 4.5,
                },
                brush,
                1.8,
                None,
            );
        } else {
            target.DrawLine(
                Vector2 {
                    X: icon_cx - 0.5,
                    Y: icon_cy - 4.5,
                },
                Vector2 {
                    X: icon_cx - 4.0,
                    Y: icon_cy,
                },
                brush,
                1.8,
                None,
            );
            target.DrawLine(
                Vector2 {
                    X: icon_cx - 4.0,
                    Y: icon_cy,
                },
                Vector2 {
                    X: icon_cx - 0.5,
                    Y: icon_cy + 4.5,
                },
                brush,
                1.8,
                None,
            );
            target.DrawLine(
                Vector2 {
                    X: icon_cx + 4.5,
                    Y: icon_cy - 4.5,
                },
                Vector2 {
                    X: icon_cx + 1.0,
                    Y: icon_cy,
                },
                brush,
                1.8,
                None,
            );
            target.DrawLine(
                Vector2 {
                    X: icon_cx + 1.0,
                    Y: icon_cy,
                },
                Vector2 {
                    X: icon_cx + 4.5,
                    Y: icon_cy + 4.5,
                },
                brush,
                1.8,
                None,
            );
        }

        let label_format = self.write.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            12.0,
            w!("pt-BR"),
        )?;
        label_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        label_format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let label_layout = self.write.CreateTextLayout(
            &label.encode_utf16().collect::<Vec<_>>(),
            &label_format,
            145.0,
            24.0,
        )?;
        brush.SetColor(&D2D1_COLOR_F {
            r: 0.95,
            g: 0.96,
            b: 0.98,
            a: alpha,
        });
        target.DrawTextLayout(
            Vector2 {
                X: pill_x + 32.0,
                Y: pill_y + 8.0,
            },
            &label_layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );

        let track_w = 70.0f32;
        let track_h = 4.0f32;
        let track_x = pill_x + pill_width - 14.0 - track_w;
        let track_y = pill_y + (pill_height - track_h) / 2.0;
        brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.16 * alpha,
        });
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: track_x,
                    top: track_y,
                    right: track_x + track_w,
                    bottom: track_y + track_h,
                },
                radiusX: 2.0,
                radiusY: 2.0,
            },
            brush,
        );

        let cx = track_x + track_w / 2.0;
        brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.45 * alpha,
        });
        target.DrawLine(
            Vector2 {
                X: cx,
                Y: track_y - 2.0,
            },
            Vector2 {
                X: cx,
                Y: track_y + track_h + 2.0,
            },
            brush,
            1.5,
            None,
        );

        let fraction = (current_ms / 5000.0).clamp(-1.0, 1.0);
        if fraction > 0.01 {
            let fill_w = fraction * (track_w / 2.0);
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.35,
                g: 0.75,
                b: 1.0,
                a: 0.95 * alpha,
            });
            target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx,
                        top: track_y,
                        right: cx + fill_w,
                        bottom: track_y + track_h,
                    },
                    radiusX: 2.0,
                    radiusY: 2.0,
                },
                brush,
            );
        } else if fraction < -0.01 {
            let fill_w = fraction.abs() * (track_w / 2.0);
            brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 0.68,
                b: 0.28,
                a: 0.95 * alpha,
            });
            target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - fill_w,
                        top: track_y,
                        right: cx,
                        bottom: track_y + track_h,
                    },
                    radiusX: 2.0,
                    radiusY: 2.0,
                },
                brush,
            );
        } else if is_zero {
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.30,
                g: 0.90,
                b: 0.55,
                a: 0.95 * alpha,
            });
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: Vector2 {
                        X: cx,
                        Y: track_y + 2.0,
                    },
                    radiusX: 3.5,
                    radiusY: 3.5,
                },
                brush,
            );
        }

        Ok(())
    }
}
impl Drop for SubtitleWindow {
    fn drop(&mut self) {
        unsafe {
            if let Some(provider) = &self.provider {
                disconnect(provider);
            }
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = crate::popup_menu::handle(msg, wp, lp) {
        return result;
    }
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp.0 as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SubtitleWindow;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let window = &mut *ptr;
    match msg {
        WM_GETOBJECT if lp.0 as i32 == UiaRootObjectId => {
            if let Some(provider) = &window.provider {
                return UiaReturnRawElementProvider(hwnd, wp, lp, provider);
            }
            LRESULT(0)
        }
        ACCESSIBLE_INVOKE => {
            if wp.0 == 11 {
                window.menu();
            }
            LRESULT(0)
        }
        ACCESSIBLE_FOCUS => {
            let _ = SetFocus(Some(hwnd));
            LRESULT(0)
        }
        WM_LBUTTONDOWN if window.editing => {
            let point = POINT {
                x: (lp.0 as u32 & 0xffff) as i16 as i32,
                y: ((lp.0 as u32 >> 16) & 0xffff) as i16 as i32,
            };
            let mut screen = POINT::default();
            let _ = GetCursorPos(&mut screen);
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);
            let edge = if point.x < 12 {
                -1
            } else if point.x > rect.right - rect.left - 12 {
                1
            } else {
                0
            };
            window.dragging = Some((screen, rect, edge));
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_CONTEXTMENU if window.editing => {
            window.menu();
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some((start, rect, edge)) = window.dragging {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let dx = point.x - start.x;
                let dy = point.y - start.y;
                let min_width = (360. * GetDpiForWindow(hwnd).max(96) as f32 / 96.) as i32;
                let width = if edge == 0 {
                    rect.right - rect.left
                } else {
                    (rect.right - rect.left + edge * dx).max(min_width)
                };
                let x = if edge == 0 {
                    rect.left + dx
                } else if edge < 0 {
                    rect.right - width
                } else {
                    rect.left
                };
                let y = if edge == 0 { rect.top + dy } else { rect.top };
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = window.render();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            if window.dragging.take().is_some() {
                let _ = ReleaseCapture();
                window.reachable();
                window.remember();
                window.changed();
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == 1 => {
            let mut keep = false;
            if let Some(start) = window.transition {
                if start.elapsed().as_millis() < 180 && !window.reduced_motion {
                    keep = true;
                } else {
                    window.transition = None;
                }
            }
            if let Some(feedback) = window.sync_feedback {
                if feedback.start.elapsed().as_millis() < 1500 {
                    keep = true;
                } else {
                    window.sync_feedback = None;
                }
            }
            if !keep {
                let _ = KillTimer(Some(hwnd), 1);
            }
            let _ = window.render();
            LRESULT(0)
        }
        WM_KILLFOCUS if window.capturing.is_some() => {
            window.end_capture();
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN if window.capturing.is_some() => {
            if wp.0 as u16 == VK_ESCAPE.0 {
                window.end_capture();
            } else {
                let modifiers = (u32::from(GetKeyState(VK_CONTROL.0 as i32) < 0) * 2)
                    | u32::from(GetKeyState(VK_MENU.0 as i32) < 0)
                    | (u32::from(GetKeyState(VK_SHIFT.0 as i32) < 0) * 4);
                let key = wp.0 as u32;
                if valid_binding(modifiers, key) {
                    let index = window.capturing.unwrap();
                    let binding = Some([modifiers, key]);
                    for existing in &mut window.prefs.sync_hotkeys {
                        if *existing == binding {
                            *existing = None;
                        }
                    }
                    window.prefs.sync_hotkeys[index] = binding;
                    window.changed();
                    window.end_capture();
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wp.0 as u16 {
                key if key == VK_RETURN.0
                    || key == VK_SPACE.0
                    || key == VK_APPS.0
                    || key == VK_F10.0 && GetKeyState(VK_SHIFT.0 as i32) < 0 =>
                {
                    window.menu()
                }
                key if key == VK_ESCAPE.0 => window.edit(false),
                _ => {}
            }
            let _ = window.render();
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            window.reachable();
            let _ = window.render();
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
            window.reachable();
            let _ = window.render();
            window.remember();
            window.changed();
            LRESULT(0)
        }
        WM_CLOSE => {
            if window.capturing.is_some() {
                window.end_capture();
            }
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
mod tests {
    use super::*;
    #[test]
    fn motion_settles_quickly_and_never_overshoots() {
        assert_eq!(caption_motion(0), (0.75, 3.));
        let mut previous = caption_motion(0);
        for elapsed in 1..=180 {
            let current = caption_motion(elapsed);
            assert!(current.0 >= previous.0 && current.0 <= 1.);
            assert!(current.1 <= previous.1 && current.1 >= 0.);
            previous = current;
        }
        assert_eq!(caption_motion(180), (1., 0.));
        assert_eq!(caption_motion(5000), (1., 0.));
    }
    #[test]
    fn sync_shortcuts_load_compatibly_validate_and_persist() {
        let mut prefs: SubtitlePreferences = serde_json::from_str(r#"{"font":40}"#).unwrap();
        assert_eq!(prefs.sync_hotkeys[0], Some([3, 0xBD]));
        assert!(!valid_binding(0, b'A' as u32));
        assert!(!valid_binding(8, b'A' as u32));
        assert!(valid_binding(6, 0x74));
        prefs.sync_hotkeys = [Some([6, 0x74]), None, Some([99, 0])];
        prefs.normalize();
        assert_eq!(prefs.sync_hotkeys[2], None);
        assert_eq!(binding_label(prefs.sync_hotkeys[0]), "Ctrl+Shift+F5");
        let restored: SubtitlePreferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored.sync_hotkeys, prefs.sync_hotkeys);
    }
    #[test]
    fn hidden_native_overlay_keeps_text_opaque_and_background_transparent_when_locked() {
        unsafe {
            use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
            OleInitialize(None).unwrap();
            let mut window =
                SubtitleWindow::create(HWND::default(), SubtitlePreferences::default(), false)
                    .unwrap();
            let lines =
                crate::lyrics::parse_lrc("[00:01]Canção de teste com contorno e sombra\n[00:02]");
            window.update(&lines, 1000);
            window.edit(false);
            let style = GetWindowLongW(window.hwnd, GWL_EXSTYLE) as u32;
            assert_ne!(style & WS_EX_TRANSPARENT.0, 0);
            assert_ne!(style & WS_EX_NOACTIVATE.0, 0);
            assert!(
                window.alpha_stats.0 > 0,
                "Background must contain truly transparent pixels"
            );
            assert!(
                window.alpha_stats.1 > 0,
                "Glyphs must contain fully opaque pixels"
            );
            window.update(&lines, 2000);
            assert!(
                window.alpha_stats.1 > 0,
                "Instrumental gaps show the music symbol"
            );
            window.edit(true);
            assert_eq!(
                GetWindowLongW(window.hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0,
                0
            );
            assert!(
                !IsWindowVisible(window.hwnd).as_bool(),
                "The test must not show a window"
            );
            drop(window);
            OleUninitialize();
        }
    }
    #[test]
    fn captions_follow_seek_delay_and_instrumental_gaps_without_inventing_translation() {
        let mut lines = crate::lyrics::parse_lrc("[00:01]One\n[00:02]\n[00:03]Three");
        lines[0].translation = Some("Um".into());
        let mut prefs = SubtitlePreferences::default();
        assert_eq!(caption_text(&lines, 999, &prefs), CaptionText::default());
        assert_eq!(caption_text(&lines, 1000, &prefs).primary, "One");
        assert_eq!(caption_text(&lines, 2000, &prefs).primary, "");
        assert_eq!(caption_text(&lines, 3000, &prefs).primary, "Three");
        prefs.mode = 1;
        assert_eq!(caption_text(&lines, 3000, &prefs).primary, "");
        prefs.delay_ms = 100;
        assert_eq!(caption_text(&lines, 900, &prefs).primary, "Um");
        prefs.mode = 2;
        assert_eq!(caption_text(&lines, 1000, &prefs).secondary, "Um");
    }
    #[test]
    fn defaults_and_preferences_round_trip() {
        let mut prefs: SubtitlePreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.font, 36.);
        prefs.font = f32::NAN;
        prefs.delay_ms = i64::MAX;
        prefs.shortcut = 999;
        prefs.normalize();
        assert_eq!(prefs.font, 36.);
        assert_eq!(prefs.delay_ms, 10000);
        assert_eq!(prefs.shortcut, 2);
        let restored: SubtitlePreferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(prefs, restored);
    }
    #[test]
    fn sync_hud_motion_progresses_smoothly_and_fades_out() {
        // At 0 ms
        let (alpha_0, offset_0, current_0) = sync_hud_motion(0, 0, 300, false);
        assert_eq!(alpha_0, 0.0);
        assert_eq!(offset_0, 6.0);
        assert_eq!(current_0, 0.0);

        // At 80 ms (during entry)
        let (alpha_80, offset_80, current_80) = sync_hud_motion(80, 0, 300, false);
        assert!(alpha_80 > 0.0 && alpha_80 < 1.0);
        assert!(offset_80 > 0.0 && offset_80 < 6.0);
        assert!(current_80 > 0.0 && current_80 < 300.0);

        // At 160 ms (fully settled)
        let (alpha_160, offset_160, current_160) = sync_hud_motion(160, 0, 300, false);
        assert_eq!(alpha_160, 1.0);
        assert_eq!(offset_160, 0.0);
        assert_eq!(current_160, 300.0);

        // At 500 ms (still fully visible)
        let (alpha_500, offset_500, current_500) = sync_hud_motion(500, 0, 300, false);
        assert_eq!(alpha_500, 1.0);
        assert_eq!(offset_500, 0.0);
        assert_eq!(current_500, 300.0);

        // At 1325 ms (fading out)
        let (alpha_1325, offset_1325, current_1325) = sync_hud_motion(1325, 0, 300, false);
        assert!(alpha_1325 > 0.0 && alpha_1325 < 1.0);
        assert_eq!(offset_1325, 0.0);
        assert_eq!(current_1325, 300.0);

        // At 1500 ms (completely invisible)
        let (alpha_1500, offset_1500, current_1500) = sync_hud_motion(1500, 0, 300, false);
        assert_eq!(alpha_1500, 0.0);
        assert_eq!(offset_1500, 0.0);
        assert_eq!(current_1500, 300.0);

        // Reduced motion: no slide offset, immediate snap
        let (rm_alpha, rm_offset, rm_current) = sync_hud_motion(50, 0, 300, true);
        assert_eq!(rm_alpha, 1.0);
        assert_eq!(rm_offset, 0.0);
        assert_eq!(rm_current, 300.0);

        let (rm_end_alpha, _, _) = sync_hud_motion(1500, 0, 300, true);
        assert_eq!(rm_end_alpha, 0.0);
    }
    #[test]
    fn sync_adjustments_update_state_clamp_and_activate_hud() {
        unsafe {
            use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
            OleInitialize(None).unwrap();
            let mut window =
                SubtitleWindow::create(HWND::default(), SubtitlePreferences::default(), false)
                    .unwrap();
            assert_eq!(window.preferences().delay_ms, 0);
            assert!(window.sync_feedback().is_none());

            // Advance by 100 ms
            window.adjust_sync(Some(100));
            assert_eq!(window.preferences().delay_ms, 100);
            assert!(window.sync_feedback().is_some());
            assert_eq!(window.sync_feedback().unwrap().delay_ms, 100);
            assert_eq!(window.sync_feedback().unwrap().previous_ms, 0);

            // Advance again by 100 ms
            window.adjust_sync(Some(100));
            assert_eq!(window.preferences().delay_ms, 200);
            assert_eq!(window.sync_feedback().unwrap().delay_ms, 200);

            // Reset sync
            window.adjust_sync(None);
            assert_eq!(window.preferences().delay_ms, 0);
            assert_eq!(window.sync_feedback().unwrap().delay_ms, 0);

            // Delay by 100 ms
            window.adjust_sync(Some(-100));
            assert_eq!(window.preferences().delay_ms, -100);

            // Extreme clamping check
            window.adjust_sync(Some(50000));
            assert_eq!(window.preferences().delay_ms, 10000);
            window.adjust_sync(Some(-50000));
            assert_eq!(window.preferences().delay_ms, -10000);

            drop(window);
            OleUninitialize();
        }
    }
}
