use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub const EDGE_WIDTH: f32 = 96.;
pub const EDGE_HEIGHT: f32 = 10.;

/// Preserve the concave attachment to the screen edge, even at minimum height.
pub fn edge_hint_corners(height: f32) -> (f32, f32) {
    let half = height.max(0.) / 2.;
    (half.min(32.), half.min(12.))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Normal,
    Hint,
    Revealed,
}

#[derive(Default)]
pub struct EdgeReveal {
    showing: bool,
    left_at: Option<Instant>,
}

impl EdgeReveal {
    pub fn show(&mut self) {
        self.showing = true;
        self.left_at = None;
    }

    pub fn update(
        &mut self,
        covered: bool,
        on_edge: bool,
        on_panel: bool,
        interacting: bool,
        now: Instant,
    ) -> Visibility {
        if !covered {
            self.showing = false;
            self.left_at = None;
            return Visibility::Normal;
        }
        if on_edge || interacting {
            self.show();
        } else if self.showing {
            if on_panel {
                self.left_at = None;
            } else if now.duration_since(*self.left_at.get_or_insert(now))
                >= Duration::from_millis(600)
            {
                self.showing = false;
                self.left_at = None;
            }
        }
        if self.showing {
            Visibility::Revealed
        } else {
            Visibility::Hint
        }
    }
}

pub const SHELL_DESKTOP_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

/// Returns true if the window class belongs to the Windows desktop shell or taskbar.
pub fn is_shell_desktop_window(class_name: &str) -> bool {
    SHELL_DESKTOP_CLASSES.contains(&class_name)
}

/// Evaluates whether a window rectangle completely covers the target monitor rectangle.
pub fn rect_covers_monitor(
    window_rect: (i32, i32, i32, i32),
    monitor_rect: (i32, i32, i32, i32),
) -> bool {
    let (w_left, w_top, w_right, w_bottom) = window_rect;
    let (m_left, m_top, m_right, m_bottom) = monitor_rect;
    w_left <= m_left && w_top <= m_top && w_right >= m_right && w_bottom >= m_bottom
}

/// Full-screen and maximized applications both suppress the normal notch.
pub fn is_fullscreen_bounds(
    window_rect: (i32, i32, i32, i32),
    monitor_rect: (i32, i32, i32, i32),
    style: u32,
    is_zoomed: bool,
    class_name: &str,
) -> bool {
    if is_shell_desktop_window(class_name) {
        return false;
    }
    if is_zoomed {
        return window_rect.0 < monitor_rect.2
            && window_rect.2 > monitor_rect.0
            && window_rect.1 < monitor_rect.3
            && window_rect.3 > monitor_rect.1;
    }
    let covers = rect_covers_monitor(window_rect, monitor_rect);
    let no_caption = (style & WS_CAPTION.0) == 0;
    no_caption && covers
}

/// Tracks the external window while the user interacts with the revealed notch.
#[derive(Default)]
pub struct FullscreenTracker {
    target: Option<HWND>,
    pub reveal: EdgeReveal,
}

impl FullscreenTracker {
    pub fn active(&self) -> bool {
        self.target.is_some()
    }

    pub fn target(&self) -> Option<HWND> {
        self.target
    }

    /// # Safety
    /// Call from the UI thread with the live notch window handle.
    pub unsafe fn update(&mut self, notch_hwnd: HWND, enabled: bool) -> bool {
        self.target = if enabled {
            covering_window(notch_hwnd, self.target)
        } else {
            None
        };
        self.active()
    }
}

/// # Safety
/// Call from the UI thread with a live notch window.
pub unsafe fn detect_fullscreen(notch_hwnd: HWND) -> bool {
    covering_window(notch_hwnd, None).is_some()
}

unsafe fn covering_window(notch_hwnd: HWND, previous: Option<HWND>) -> Option<HWND> {
    let own_mon = MonitorFromWindow(notch_hwnd, MONITOR_DEFAULTTONEAREST);
    let foreground = GetForegroundWindow();
    let mut foreground_pid = 0;
    if !foreground.is_invalid() {
        let _ = GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
    }
    let fg = if foreground_pid == std::process::id() {
        previous?
    } else {
        foreground
    };
    // 2. Foreground window validation
    if fg.is_invalid() || fg == notch_hwnd || !IsWindow(Some(fg)).as_bool() {
        return None;
    }
    if !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() {
        return None;
    }

    // Ignore our own process windows (popups, settings dialog, etc.)
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(fg, Some(&mut pid));
    if pid == std::process::id() {
        return None;
    }

    // Ignore cloaked windows (UWP background apps, other virtual desktops)
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(
        fg,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut _,
        std::mem::size_of::<u32>() as u32,
    )
    .is_ok()
        && cloaked != 0
    {
        return None;
    }

    // 3. Monitor match: must be on the same monitor as Nivumi
    let fg_mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
    if fg_mon != own_mon {
        return None;
    }

    // 4. Class name check: ignore shell / desktop / taskbars
    let mut cls_buf = [0u16; 128];
    let n = GetClassNameW(fg, &mut cls_buf);
    let class_name = String::from_utf16_lossy(&cls_buf[..n.max(0) as usize]);
    if is_shell_desktop_window(&class_name) {
        return None;
    }

    // 5. Window bounds & style
    let style = GetWindowLongW(fg, GWL_STYLE) as u32;
    let is_zoomed = IsZoomed(fg).as_bool();

    let mut r = RECT::default();
    let mut m = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetWindowRect(fg, &mut r).is_ok() && GetMonitorInfoW(own_mon, &mut m).as_bool() {
        let win_rect = (r.left, r.top, r.right, r.bottom);
        let mon_rect = (
            m.rcMonitor.left,
            m.rcMonitor.top,
            m.rcMonitor.right,
            m.rcMonitor.bottom,
        );
        if is_fullscreen_bounds(win_rect, mon_rect, style, is_zoomed, &class_name)
            || matches!(
                SHQueryUserNotificationState(),
                Ok(QUNS_RUNNING_D3D_FULL_SCREEN)
            )
            || (rect_covers_monitor(win_rect, mon_rect)
                && matches!(
                    SHQueryUserNotificationState(),
                    Ok(QUNS_BUSY | QUNS_PRESENTATION_MODE)
                ))
        {
            return Some(fg);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS_POPUP: u32 = 0x80000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_CAPTION_VAL: u32 = 0x00C00000;
    const WS_THICKFRAME: u32 = 0x00040000;

    #[test]
    fn edge_reveals_panel_and_leaving_waits_before_hiding() {
        let mut reveal = EdgeReveal::default();
        let now = Instant::now();
        assert_eq!(
            reveal.update(true, false, true, false, now),
            Visibility::Hint
        );
        assert_eq!(
            reveal.update(true, true, false, false, now),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, true, false, now),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, false, false, now),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, false, false, now + Duration::from_millis(599)),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, false, false, now + Duration::from_millis(600)),
            Visibility::Hint
        );
    }

    #[test]
    fn returning_to_panel_and_dragging_reset_the_hide_delay() {
        let mut reveal = EdgeReveal::default();
        let now = Instant::now();
        reveal.show();
        reveal.update(true, false, false, false, now);
        reveal.update(true, false, true, false, now + Duration::from_millis(500));
        assert_eq!(
            reveal.update(true, false, false, false, now + Duration::from_millis(900)),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, false, true, now + Duration::from_secs(5)),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, false, false, now + Duration::from_secs(6)),
            Visibility::Revealed
        );
        assert_eq!(
            reveal.update(true, false, false, false, now + Duration::from_secs(7)),
            Visibility::Hint
        );
    }

    #[test]
    fn leaving_covered_mode_resets_reveal_for_the_next_maximized_window() {
        let mut reveal = EdgeReveal::default();
        let now = Instant::now();
        reveal.show();
        assert_eq!(
            reveal.update(false, false, false, false, now),
            Visibility::Normal
        );
        assert_eq!(
            reveal.update(true, false, false, false, now),
            Visibility::Hint
        );
    }

    #[test]
    fn windowed_chrome_and_maximized_window_on_another_monitor_do_not_hide() {
        let style = WS_VISIBLE | WS_CAPTION_VAL | WS_THICKFRAME;
        assert!(!is_fullscreen_bounds(
            (100, 100, 1400, 900),
            (0, 0, 1920, 1080),
            style,
            false,
            "Chrome_WidgetWin_1"
        ));
        assert!(!is_fullscreen_bounds(
            (-1928, -8, 8, 1048),
            (1920, 0, 3840, 1080),
            style,
            true,
            "Chrome_WidgetWin_1"
        ));
        assert!(is_fullscreen_bounds(
            (-1928, 32, 8, 1048),
            (-1920, 40, 0, 1080),
            style,
            true,
            "Chrome_WidgetWin_1"
        ));
    }

    #[test]
    fn test_shell_desktop_exclusion() {
        assert!(is_shell_desktop_window("Progman"));
        assert!(is_shell_desktop_window("WorkerW"));
        assert!(is_shell_desktop_window("Shell_TrayWnd"));
        assert!(is_shell_desktop_window("Shell_SecondaryTrayWnd"));
        assert!(!is_shell_desktop_window("Chrome_WidgetWin_1"));
        assert!(!is_shell_desktop_window("CabinetWClass"));
        assert!(!is_shell_desktop_window("Notepad"));
    }

    #[test]
    fn test_rect_covers_monitor() {
        let mon = (0, 0, 1920, 1080);
        assert!(rect_covers_monitor((0, 0, 1920, 1080), mon));
        assert!(rect_covers_monitor((-1, -1, 1921, 1081), mon));
        assert!(!rect_covers_monitor((10, 10, 1900, 1000), mon));
        assert!(!rect_covers_monitor((0, 0, 1920, 1040), mon)); // Taskbar area uncovered
    }

    #[test]
    fn maximized_chrome_is_suppressed_even_with_a_visible_taskbar() {
        let mon = (0, 0, 1920, 1080);
        let win = (0, 0, 1920, 1040);
        let style = WS_VISIBLE | WS_CAPTION_VAL | WS_THICKFRAME;
        // Maximized Chrome retains WS_CAPTION and leaves the taskbar uncovered.
        assert!(is_fullscreen_bounds(
            win,
            mon,
            style,
            true,
            "Chrome_WidgetWin_1"
        ));
    }

    #[test]
    fn test_borderless_fullscreen_game_or_video() {
        let mon = (0, 0, 1920, 1080);
        let win = (0, 0, 1920, 1080);
        let style = WS_VISIBLE | WS_POPUP; // No WS_CAPTION
        assert!(is_fullscreen_bounds(
            win,
            mon,
            style,
            false,
            "Chrome_WidgetWin_1"
        ));
        assert!(is_fullscreen_bounds(win, mon, style, true, "UnrealWindow"));
    }

    #[test]
    fn test_multi_monitor_bounds() {
        let mon2 = (-1920, 0, 0, 1080);
        let win2 = (-1920, 0, 0, 1080);
        let style = WS_VISIBLE | WS_POPUP;
        assert!(is_fullscreen_bounds(win2, mon2, style, false, "GameWindow"));
        // Window on primary monitor doesn't cover secondary monitor
        let win1 = (0, 0, 1920, 1080);
        assert!(!is_fullscreen_bounds(
            win1,
            mon2,
            style,
            false,
            "GameWindow"
        ));
    }
}
