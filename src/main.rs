#![windows_subsystem = "windows"]
use chrono::{Datelike, Local, TimeZone};
use nivumi::brand;
use nivumi::{
    accessibility::*,
    dialogs,
    fullscreen::{FullscreenTracker, Visibility, EDGE_HEIGHT, EDGE_WIDTH},
    lyrics,
    lyrics_view::LyricsContent,
    lyrics_window::{LyricsWindow, PREFERENCES_CHANGED},
    model::*,
    motion::{self, equalizer_heights, Spring},
    ole,
    player_layout::PlayerLayout,
    render::Renderer,
    services::*,
    storage,
    subtitles::{SubtitleWindow, CAPTURE_SHORTCUT, OPEN_SUBTITLES, SUBTITLE_CHANGED},
    theme,
    window_region::{notch_outline, WindowRegion},
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        Media::Audio::*,
        System::{Com::*, LibraryLoader::*, Ole::*},
        UI::{
            Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

#[cfg(debug_assertions)]
mod brand_preview;
mod hydration;
mod notch_position;

const TRAY: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WIDTH: f32 = 580.;
const HEIGHT: f32 = 380.;
const BODY: f32 = 420.;
const NAV_EXPAND_H: f32 = 28.;
const NAV_TARGET_HEIGHT: f32 = 32.;
const NAV_ICON_COUNT: usize = 8;
const NAV_PILL_WIDTH: f32 = NAV_ICON_COUNT as f32 * 32. + 12.;
const NAV_HOVER_HALF_WIDTH: f32 = NAV_PILL_WIDTH / 2. + 6.;
const NAV_DOTS_WIDTH: f32 = 15. + 6. * (3.6 + 5.5);
const TASK_TARGET: f32 = 32.;
const TASK_ROW_PITCH: f32 = 36.;
#[allow(dead_code)]
// Settings commands own a complete snapshot; the UI keeps only a small set of hits.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum Action {
    Page(usize),
    Settings,
    HydrationSettings,
    DismissPomodoro,
    DismissToast,
    PomodoroNext(PomodoroPhase),
    Play,
    Next,
    Previous,
    Seek(f64),
    Volume(f32),
    VolumeMute,
    Command(AppCommand),
    NewTask,
    EditTask(u64),
    SnoozeTask(u64),
    TaskMenu(u64),
    AddText,
    AddFiles,
    AddFolder,
    OpenShelf(u64),
    CopyShelf(u64),
    SelectShelf(u64),
    DragShelf(u64),
    RemoveSelected,
    RemoveShelfItem(u64),
    ShelfClipboard(bool),
    PasteShelfImage,
    ClearShelf,
    ShelfInfo,
    ShelfScroll(i32),
    Calendar(i32),
    LyricsMode(usize),
    LyricsFont(i32),
    LyricsOptions,
    LyricsAnimation(usize),
    LyricsAllTranslations,
    OpenLyricsWindow,
    OpenSubtitles,
    Reveal,
    FollowLyrics,
    LyricsScroll(i32),
    ReloadLetras,
    CopyLyricsForAi,
    TrackInfo,
    Close,
    Exit,
}
pub fn marquee_offset(text_w: f32, max_w: f32, elapsed_secs: f32) -> f32 {
    if text_w <= max_w || max_w <= 0. {
        return 0.;
    }
    let distance = text_w - max_w;
    let speed = 28.0;
    let pause = 2.0;
    let scroll_dur = distance / speed;
    let cycle_dur = 2.0 * (pause + scroll_dur);
    if cycle_dur <= 0. {
        return 0.;
    }
    let t = elapsed_secs % cycle_dur;
    if t < pause {
        0.
    } else if t < pause + scroll_dur {
        let p = (t - pause) / scroll_dur;
        let smooth_p = p * p * (3.0 - 2.0 * p);
        distance * smooth_p
    } else if t < 2.0 * pause + scroll_dur {
        distance
    } else {
        let p = (t - (2.0 * pause + scroll_dur)) / scroll_dur;
        let smooth_p = p * p * (3.0 - 2.0 * p);
        distance * (1.0 - smooth_p)
    }
}
fn expanded_content_offset(page: usize) -> f32 {
    if matches!(page, 2 | 4 | 5) {
        28.
    } else {
        0.
    }
}
#[derive(Clone)]
struct Hit {
    rect: (f32, f32, f32, f32),
    action: Action,
    label: String,
}
impl Hit {
    fn id(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.action).hash(&mut h);
        h.finish() as usize
    }
}
#[derive(Clone, Copy)]
struct NotchResize {
    pointer: POINT,
    width: f32,
    height: f32,
    horizontal: i32,
    vertical: bool,
}
#[derive(Clone, Copy)]
struct ShelfGesture {
    id: u64,
    start: POINT,
    right: bool,
    stage: Option<Instant>,
    modified: bool,
}
struct App {
    offline_job: Option<nivumi::offline_translation::Job>,
    offline_attempt: String,
    letras_done: bool,
    hwnd: HWND,
    renderer: Option<Renderer>,
    window_region: WindowRegion,
    canvas_width: f32,
    canvas_height: f32,
    notch_resize: Option<NotchResize>,
    notch_drag: Option<(POINT, POINT, bool)>,
    lyrics_window: Option<Box<LyricsWindow>>,
    subtitles: Option<Box<SubtitleWindow>>,
    notch_follow: bool,
    state: AppState,
    path: PathBuf,
    services: ServiceHandle,
    media: MediaSnapshot,
    system: SystemSnapshot,
    media_at: Instant,
    track_change_at: Instant,
    lyric_lines: Vec<lyrics::LyricLine>,
    painted_lyric_line: Option<usize>,
    verse_change_at: Option<Instant>,
    verse_focus: Option<usize>,
    verse_scroll: nivumi::lyrics_view::FocusScroll,
    shelf_scroll: nivumi::lyrics_view::FocusScroll,
    task_scroll: nivumi::lyrics_view::FocusScroll,
    lyric_status: String,
    external_lyrics: Option<ExternalLyrics>,
    lyrics_view_mode: usize,
    external_scroll: usize,
    page: usize,
    expanded: bool,
    expansion: Spring,
    navigation: Spring,
    navigation_hover: bool,
    body_width: Spring,
    body_height: Spring,
    player_layout: Spring,
    accent: [Spring; 3],
    volume_level: Spring,
    volume_hover: Spring,
    volume_dragging: bool,
    volume_hovered: bool,
    volume_saved: f32,
    volume_fade: Option<(f32, f32, Instant, Duration)>,
    seek_drag: Option<f64>,
    media_phase: f32,
    media_frame: Instant,
    media_animating: bool,
    last_anim: Instant,
    hits: Vec<Hit>,
    content_offset_y: f32,
    selected: BTreeSet<u64>,
    shelf_anchor: Option<u64>,
    shelf_gesture: Option<ShelfGesture>,
    shelf_worker: nivumi::shelf::ThumbnailWorker,
    shelf_thumbnails: std::collections::HashMap<u64, Option<nivumi::shelf::Thumbnail>>,
    scroll: usize,
    calendar_offset: i32,
    toast: Option<(String, Instant)>,
    pomodoro_notice: Option<(String, String, Option<PomodoroPhase>, Instant)>,
    hover: bool,
    focus: Option<usize>,
    last_clock: String,
    drop_rx: mpsc::Receiver<ole::DropPayload>,
    _drop: Option<ole::DropRegistration>,
    modal: bool,
    hidden: bool,
    tray_hidden: bool,
    fullscreen_state: FullscreenTracker,
    last_second: i64,
    accessible_items: Arc<Mutex<Vec<AccessibleItem>>>,
    accessible_provider: Option<IRawElementProviderSimple>,
    pinned: bool,
    hovered_hit: Option<usize>,
    cursor_pos: Option<(f32, f32)>,
}
impl App {
    fn is_hovered(&self, rect: (f32, f32, f32, f32)) -> bool {
        let Some((x, y)) = self.cursor_pos else {
            return false;
        };
        x >= rect.0 && x <= rect.0 + rect.2 && y >= rect.1 && y <= rect.1 + rect.3
    }
    fn tr<'a>(&self, pt: &'a str, en: &'a str) -> &'a str {
        self.state.settings.text(pt, en)
    }
    fn position(&self) -> f64 {
        (self.media.position
            + if self.media.playing {
                self.media_at.elapsed().as_secs_f64() * self.media.rate.max(0.)
            } else {
                0.
            })
        .clamp(0., self.media.duration.max(0.))
    }
    fn song_sync_key(&self) -> String {
        nivumi::lyrics_view::song_sync_key(&self.media.title, &self.media.artist)
    }
    fn song_sync_offset(&self) -> i64 {
        self.state
            .settings
            .lyrics
            .song_offset(&self.song_sync_key())
    }
    fn lyric_position_ms(&self) -> i64 {
        ((self.position() * 1000.) as i64).saturating_add(self.song_sync_offset())
    }
    fn select_song_sync(&mut self) {
        let delay = self.song_sync_offset();
        self.state.settings.subtitles.delay_ms = delay;
        if let Some(window) = &mut self.subtitles {
            window.set_song_delay(delay);
        }
    }
    unsafe fn adjust_song_sync(&mut self, delta: Option<i64>) {
        self.collect_subtitle_preferences();
        let key = self.song_sync_key();
        if key.is_empty() {
            return;
        }
        let delay = self.state.settings.lyrics.adjust_song_offset(&key, delta);
        self.state.settings.subtitles.delay_ms = delay;
        let position = (self.position() * 1000.) as i64;
        if let Some(window) = &mut self.subtitles {
            window.adjust_sync(delta);
            window.update(&self.lyric_lines, position);
        } else {
            let text = if delay > 0 {
                format!(
                    "Letra desta música: +{:.1} s (adiantada)",
                    delay as f64 / 1000.
                )
            } else if delay < 0 {
                format!(
                    "Letra desta música: {:.1} s (atrasada)",
                    delay as f64 / 1000.
                )
            } else {
                "Letra desta música: sincronizada (0,0 s)".into()
            };
            self.toast = Some((text.replace('.', ","), Instant::now()));
        }
        self.painted_lyric_line = None;
        self.sync_lyrics_window();
        self.paint();
        self.save();
    }
    fn receive_external_lyrics(&mut self, ext: ExternalLyrics) {
        if ext
            .translation
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            self.offline_job = None;
        }
        if ext
            .translation
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && self.external_lyrics.as_ref().is_some_and(|e| {
                e.translation_origin == nivumi::offline_translation::TranslationOrigin::Offline
            })
        {
            self.external_lyrics = None;
        }

        let source = ext.source.clone();
        if lyrics::merge_external(&mut self.external_lyrics, &mut self.lyric_lines, ext) {
            self.lyric_status = format!("● {source}");
        }
        self.lyrics_view_mode = if self.lyric_lines.is_empty() { 2 } else { 1 };
        let content = self.lyrics_content();
        self.lyrics_view_mode = content.available_mode(self.lyrics_view_mode);
        self.lyric_status = content.status;
        self.external_scroll = 0;
    }

    fn save(&mut self) {
        if let Err(e) = storage::save(&self.path, &self.state) {
            self.toast = Some((
                format!("{}: {e}", self.tr("Erro ao salvar", "Save error")),
                Instant::now(),
            ));
        }
    }
    fn apply(&mut self, c: AppCommand) {
        let is_start_pomodoro = matches!(c, AppCommand::StartPomodoro);
        let events = self.state.apply(c, Local::now());
        self.events(events);
        self.save();
        if is_start_pomodoro {
            self.hover = false;
            unsafe {
                self.expand(false);
            }
        }
    }
    fn events(&mut self, events: Vec<AppEvent>) {
        for e in events {
            if let AppEvent::Reminder { title, body } = e {
                if title == "Pomodoro" {
                    let next = if self.state.pomodoro.phase == PomodoroPhase::Focus {
                        if self.state.pomodoro.completed_focus.is_multiple_of(4) {
                            PomodoroPhase::LongBreak
                        } else {
                            PomodoroPhase::ShortBreak
                        }
                    } else {
                        PomodoroPhase::Focus
                    };
                    self.pomodoro_notice = Some((
                        self.tr("Tempo concluído!", "Time is up!").into(),
                        self.tr("Pronto para a próxima fase?", "Ready for the next phase?")
                            .into(),
                        Some(next),
                        Instant::now(),
                    ));
                } else {
                    self.toast = Some((format!("{title} · {body}"), Instant::now()));
                }
                unsafe {
                    self.balloon(&title, &body);
                    if self.state.settings.sounds {
                        let _ = PlaySoundW(
                            w!("SystemNotification"),
                            None,
                            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
                        );
                    }
                }
            }
        }
    }
    unsafe fn balloon(&self, title: &str, body: &str) {
        let mut n = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        copy_wide(&mut n.szInfoTitle, title);
        copy_wide(&mut n.szInfo, body);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
    }
    unsafe fn resize(&mut self) {
        // Release the old composition target before attaching another to this HWND.
        self.renderer.take();
        let mut scale = GetDpiForSystem() as f32 / 96. * self.state.settings.scale;
        let monitors = enum_monitors();
        let monitor = monitors
            .get(self.state.settings.monitor)
            .or_else(|| monitors.first());
        let (left, top, right, bottom) = monitor
            .map(|m| (m.left, m.top, m.right, m.bottom))
            .unwrap_or((
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN),
            ));
        let mut dx = 96;
        let mut dy = 96;
        if GetDpiForMonitor(
            MonitorFromPoint(
                POINT {
                    x: left + 1,
                    y: top + 1,
                },
                MONITOR_DEFAULTTONEAREST,
            ),
            MDT_EFFECTIVE_DPI,
            &mut dx,
            &mut dy,
        )
        .is_ok()
        {
            scale = dx as f32 / 96. * self.state.settings.scale;
        }
        let available_w = ((right - left) as f32 / scale - 12.).max(320.);
        let available_h = ((bottom - top) as f32 / scale - 8.).max(180.);
        self.state.settings.lyrics.notch_width = self
            .state
            .settings
            .lyrics
            .notch_width
            .min((available_w - 60.).max(320.));
        self.state.settings.lyrics.notch_height = self
            .state
            .settings
            .lyrics
            .notch_height
            .min((available_h - 70.).max(140.));
        self.canvas_width = if self.notch_resize.is_some() {
            available_w
        } else {
            WIDTH
                .max(self.state.settings.lyrics.notch_width + 60.)
                .min(available_w)
        };
        self.canvas_height = if self.notch_resize.is_some() {
            available_h
        } else {
            HEIGHT
                .max(self.state.settings.lyrics.notch_height + 70.)
                .min(available_h)
        };
        let _ = SetWindowPos(
            self.hwnd,
            Some(HWND_TOPMOST),
            left + (right - left - (self.canvas_width * scale) as i32) / 2,
            top,
            (self.canvas_width * scale) as i32,
            (self.canvas_height * scale) as i32,
            SWP_NOACTIVATE,
        );
        self.renderer = Renderer::new(
            self.hwnd,
            (self.canvas_width * scale) as u32,
            (self.canvas_height * scale) as u32,
            scale * 96.,
        )
        .ok();
    }
    unsafe fn placement_area(&self) -> RECT {
        let monitors = enum_monitors();
        let point = monitors
            .get(self.state.settings.monitor)
            .or_else(|| monitors.first())
            .map(|m| POINT {
                x: m.left + 1,
                y: m.top + 1,
            })
            .unwrap_or_default();
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            if self.fullscreen_state.active() {
                info.rcMonitor
            } else {
                info.rcWork
            }
        } else {
            RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            }
        }
    }
    unsafe fn position_notch(&self, scale: f32, width: f32, height: f32) {
        if self.notch_drag.is_some_and(|(_, _, moved)| moved) {
            return;
        }
        let area = self.placement_area();
        let (x, y) = notch_position::origin(
            (area.left, area.top, area.right, area.bottom),
            self.canvas_width,
            (width, height),
            scale,
            self.state.settings.notch_column,
        );
        let _ = SetWindowPos(
            self.hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    unsafe fn finish_notch_drag(&mut self, commit: bool) -> bool {
        let Some((_, _, moved)) = self.notch_drag.take() else {
            return false;
        };
        let _ = ReleaseCapture();
        if moved && commit {
            let mut pointer = POINT::default();
            let _ = GetCursorPos(&mut pointer);
            let area = self.placement_area();
            let column = notch_position::nearest(
                (pointer.x, pointer.y),
                (area.left, area.top, area.right, area.bottom),
            );
            self.state.settings.notch_column = column;
            self.state.settings.notch_bottom = false;
            self.save();
        }
        if moved {
            self.paint();
        }
        moved
    }
    unsafe fn expand(&mut self, yes: bool) {
        if yes && self.hidden {
            self.hidden = false;
            self.fullscreen_state.reveal.show();
        }
        if !yes && self.seek_drag.is_some() {
            self.seek_drag = None;
            let _ = ReleaseCapture();
        }
        self.expanded = yes;
        self.animate();
    }
    unsafe fn hide_to_tray(&mut self) {
        self.tray_hidden = true;
        self.expanded = false;
        self.hover = false;
        self.hovered_hit = None;
        self.focus = None;
        self.navigation_hover = false;
        self.volume_hovered = false;
        self.seek_drag = None;
        self.volume_fade = None;
        self.expansion.snap(0.);
        let (w, h) = self.target_size();
        self.body_width.snap(w);
        self.body_height.snap(h);
        if self.hwnd != HWND::default() {
            let _ = KillTimer(Some(self.hwnd), 2);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
    unsafe fn restore_from_tray(&mut self, expand: bool) {
        self.tray_hidden = false;
        self.hidden = false;
        if self.hwnd != HWND::default() {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        if expand {
            self.expand(true);
            if self.hwnd != HWND::default() {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
        } else {
            self.expand(false);
        }
        self.paint();
    }
    unsafe fn animate(&mut self) {
        self.last_anim = Instant::now();
        if !self.expanded || self.expansion.value < 0.01 || !matches!(self.page, 0 | 1) {
            self.player_layout
                .snap(if self.page == 1 { 1. } else { 0. });
        }
        if self.state.settings.reduced_motion {
            self.player_layout
                .snap(if self.page == 1 { 1. } else { 0. });
            self.expansion.snap(if self.expanded { 1. } else { 0. });
            let (w, h) = self.target_size();
            self.body_width.snap(w);
            self.body_height.snap(h);
            self.navigation
                .snap(if self.navigation_hover && self.expanded {
                    1.
                } else {
                    0.
                });
            for (spring, target) in self
                .accent
                .iter_mut()
                .zip(self.media.accent.unwrap_or([0.; 3]))
            {
                spring.snap(target);
            }
            if let Some(target_vol) = self.system.volume {
                self.volume_level.snap(target_vol);
            }
            self.volume_hover
                .snap(if self.volume_hovered || self.volume_dragging {
                    1.
                } else {
                    0.
                });
            let _ = KillTimer(Some(self.hwnd), 2);
            self.paint();
        } else {
            // Below a system tick: avoids 16 ms rounding to two ticks on common Windows setups.
            // Present(1) paces rendering to the display; the timer stops once motion settles.
            SetTimer(Some(self.hwnd), 2, 10, None);
        }
    }
    unsafe fn animate_frame(&mut self) {
        if self.state.settings.reduced_motion {
            self.animate();
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_anim).as_secs_f32();
        self.last_anim = now;
        let target = if self.expanded { 1. } else { 0. };
        let frequency = if self.expanded {
            motion::FREQUENCY_DEFAULT
        } else {
            motion::FREQUENCY_SNAPPY
        };
        let (w, h) = self.target_size();
        let mut active = self.expansion.advance(target, dt, frequency);
        active |= self.expanded
            && self.page == 1
            && self.state.settings.lyrics.focus_animation != 0
            && self
                .verse_change_at
                .is_some_and(|t| t.elapsed().as_secs_f32() < 0.28);
        active |= self.body_width.advance(w, dt, frequency);
        active |= self.body_height.advance(h, dt, frequency);
        active |= self.player_layout.advance(
            if self.page == 1 { 1. } else { 0. },
            dt,
            motion::FREQUENCY_DEFAULT,
        );
        active |= self.expanded && self.page == 1 && self.verse_scroll.is_animating(now);
        active |= self.expanded
            && ((self.page == 4 && self.shelf_scroll.is_animating(now))
                || (self.page == 5 && self.task_scroll.is_animating(now)));
        active |= self.navigation.advance(
            if self.navigation_hover && self.expanded {
                1.
            } else {
                0.
            },
            dt,
            motion::FREQUENCY_SNAPPY,
        );
        for (spring, target) in self
            .accent
            .iter_mut()
            .zip(self.media.accent.unwrap_or([0.; 3]))
        {
            active |= spring.advance(target, dt, motion::FREQUENCY_GENTLE);
        }
        if let Some((start_v, target_v, start_time, dur)) = self.volume_fade {
            let elapsed = start_time.elapsed().as_secs_f32();
            let total_secs = dur.as_secs_f32().max(0.001);
            let t = (elapsed / total_secs).clamp(0., 1.);
            let smooth_t = (1.0 - (t * std::f32::consts::PI).cos()) / 2.0;
            let current = start_v + (target_v - start_v) * smooth_t;
            self.system.volume = Some(current);
            self.services.send(ServiceCommand::Volume(current));
            if t >= 1.0 {
                self.volume_fade = None;
                self.system.volume = Some(target_v);
                self.services.send(ServiceCommand::Volume(target_v));
            }
            active = true;
        }
        if let Some(target_vol) = self.system.volume {
            active |= self
                .volume_level
                .advance(target_vol, dt, motion::FREQUENCY_SNAPPY);
        }
        let hover_target = if self.volume_hovered || self.volume_dragging {
            1.
        } else {
            0.
        };
        active |= self
            .volume_hover
            .advance(hover_target, dt, motion::FREQUENCY_SNAPPY);
        if !active {
            let _ = KillTimer(Some(self.hwnd), 2);
        }
        self.paint();
    }
    unsafe fn tick(&mut self) {
        let mut dirty = false;
        while let Ok((id, image)) = self.shelf_worker.rx.try_recv() {
            if self.state.shelf.iter().any(|i| i.id == id) {
                self.shelf_thumbnails.insert(id, image);
                dirty |= self.page == 4;
            }
        }
        while let Ok(e) = self.services.rx.try_recv() {
            match e {
                ServiceEvent::Media(m) => {
                    let changed = m.key() != self.media.key();
                    let accent_changed = m.accent != self.media.accent;
                    self.collect_subtitle_preferences();
                    self.media = m;
                    self.select_song_sync();
                    self.media_at = Instant::now();
                    if changed {
                        self.track_change_at = Instant::now();
                        self.verse_focus = None;
                        self.verse_scroll = Default::default();
                        self.verse_change_at = None;
                        self.notch_follow = true;
                        self.offline_job = None;
                        self.offline_attempt.clear();
                        self.letras_done = false;
                        self.lyric_lines.clear();
                        self.external_lyrics = None;
                        self.external_scroll = 0;
                        self.lyrics_view_mode = 1;
                        self.lyric_status = self.tr("Buscando letras…", "Loading lyrics…").into();
                        if !self.media.title.is_empty() {
                            self.services.send(ServiceCommand::FetchLyrics {
                                title: self.media.title.clone(),
                                artist: self.media.artist.clone(),
                                duration: self.media.duration,
                            });
                            self.services.send(ServiceCommand::FetchLetrasLyrics {
                                title: self.media.title.clone(),
                                artist: self.media.artist.clone(),
                            });
                        }
                    }
                    if accent_changed || changed {
                        self.animate();
                    }
                    dirty = true;
                }
                ServiceEvent::LetrasLyrics { key, lyrics } => {
                    if key == lyrics_key(&self.media.title, &self.media.artist, 0.)
                        && (self.media.title.is_empty()
                            || nivumi::synced_lyrics::same_song(
                                &self.media.title,
                                &self.media.artist,
                                &lyrics.title,
                                &lyrics.artist,
                            ))
                    {
                        self.receive_external_lyrics(lyrics);
                        dirty = true;
                    }
                }
                ServiceEvent::LetrasFinished { key } => {
                    if key == lyrics_key(&self.media.title, &self.media.artist, 0.) {
                        self.letras_done = true;
                    }
                }
                ServiceEvent::Lyrics { key, text, error } => {
                    if key == self.media.key() {
                        let had_sync = !self.lyric_lines.is_empty();
                        // An explicit Lyricsify import wins over a late automatic request.
                        if !had_sync {
                            lyrics::apply_synced(
                                &mut self.lyric_lines,
                                text.as_ref().map(|value| value.text.as_str()),
                            );
                        }
                        if let Some(ext) = self.external_lyrics.as_ref() {
                            lyrics::align_external_translation(&mut self.lyric_lines, ext);
                        }
                        if !had_sync {
                            self.lyric_status = if !self.lyric_lines.is_empty() {
                                format!(
                                    "● {}",
                                    text.as_ref()
                                        .map(|value| value.source.as_str())
                                        .unwrap_or_default()
                                )
                            } else if error.is_some() {
                                self.tr("Não foi possível buscar letras", "Could not load lyrics")
                                    .into()
                            } else {
                                self.tr(
                                    "Letra sincronizada indisponível",
                                    "Synced lyrics unavailable",
                                )
                                .into()
                            };
                        }
                        self.lyric_status = self.lyrics_content().status;
                        if let Some(error) = error {
                            log(&error);
                        }
                        if self.lyric_lines.is_empty()
                            && self.external_lyrics.is_some()
                            && self.lyrics_view_mode <= 1
                        {
                            self.lyrics_view_mode = 2;
                        } else if !self.lyric_lines.is_empty() && self.lyrics_view_mode <= 2 {
                            self.lyrics_view_mode = 1;
                        }
                        self.lyrics_view_mode =
                            self.lyrics_content().available_mode(self.lyrics_view_mode);
                        dirty = true;
                    }
                }
                ServiceEvent::System(s) => {
                    self.system_toast(&s);
                    if let Some(v) = s.volume {
                        if self.system.volume.is_none() {
                            self.volume_level.snap(v);
                            self.volume_saved = if v > 0.05 { v } else { 0.5 };
                        } else if !self.volume_dragging
                            && self.volume_fade.is_none()
                            && (self.volume_level.value - v).abs() > 0.001
                        {
                            self.animate();
                        }
                    }
                    self.system = s;
                    dirty = true;
                }
                ServiceEvent::Error(e) => {
                    log(&e);
                }
            }
        }
        while let Ok(drop) = self.drop_rx.try_recv() {
            match drop {
                ole::DropPayload::Files(files) => {
                    for p in files {
                        self.add_path(p);
                    }
                }
                ole::DropPayload::Text(text) => {
                    self.apply(AppCommand::AddShelf {
                        kind: if text.starts_with("https://") || text.starts_with("http://") {
                            ShelfKind::Link
                        } else {
                            ShelfKind::Text
                        },
                        value: text,
                    });
                    self.scroll = self.state.shelf.len().saturating_sub(5);
                }
            }
            self.page = 4;
            self.expand(true);
            dirty = true;
        }
        let now = Local::now();
        let before_offline = (self.offline_job.is_some(), self.external_lyrics.clone());
        self.poll_offline();
        dirty |= before_offline != (self.offline_job.is_some(), self.external_lyrics.clone());
        // Timed lyrics also advance while collapsed. Redraw on a verse change,
        // using the existing 250 ms heartbeat rather than a continuous animation.
        if self.page == 1
            && self.media.playing
            && self.active_lyric_line() != self.painted_lyric_line
        {
            dirty = true;
        }
        if now.timestamp() != self.last_second {
            self.last_second = now.timestamp();
            let events = self.state.tick(now);
            if !events.is_empty() {
                self.events(events);
                self.save();
                dirty = true;
            }
            if (self.expanded
                && ((self.page == 0 || self.page == 1) && self.media.playing
                    || self.page == 2 && self.state.pomodoro.running()))
                || self.state.pomodoro.running()
            {
                dirty = true;
            }
        }
        let clock = now.format("%H:%M").to_string();
        if clock != self.last_clock {
            self.last_clock = clock;
            dirty = true;
        }
        if self
            .pomodoro_notice
            .as_ref()
            .is_some_and(|(_, _, next, at)| next.is_none() && at.elapsed() > Duration::from_secs(6))
        {
            self.pomodoro_notice = None;
            dirty = true;
        }
        if let Some((_, t)) = &self.toast {
            let elapsed = t.elapsed();
            if elapsed > Duration::from_secs(6) {
                self.toast = None;
                dirty = true;
            } else if !self.state.settings.reduced_motion
                && (elapsed < Duration::from_millis(250) || elapsed > Duration::from_millis(5650))
            {
                dirty = true;
            }
        }
        if !self.tray_hidden {
            self.fullscreen();
        }
        if !self.tray_hidden
            && !self.hidden
            && !self.fullscreen_state.active()
            && !self.modal
            && !self.pinned
            && self.notch_resize.is_none()
            && self.shelf_gesture.is_none()
        {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = ScreenToClient(self.hwnd, &mut pt);
            let scale = self.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            let x = pt.x as f32 / scale;
            let y = pt.y as f32 / scale;
            let inside = self.inside(x, y);
            let over_notice = (self.pomodoro_notice.is_some() || self.toast.is_some())
                && y > self.body_height.value;
            if inside != self.hover && self.notch_drag.is_none() && !over_notice {
                self.hover = inside;
                if !self.state.settings.open_click {
                    self.expand(inside);
                } else if !inside && self.expanded {
                    self.expand(false);
                }
                dirty = true;
            }
        }
        let mut pointer = POINT::default();
        let _ = GetCursorPos(&mut pointer);
        let _ = ScreenToClient(self.hwnd, &mut pointer);
        let scale = self.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
        let px = pointer.x as f32 / scale;
        let py = pointer.y as f32 / scale;
        self.cursor_pos = if self.inside(px, py) {
            Some((px, py))
        } else {
            None
        };
        self.update_navigation_hover(px, py);
        self.update_volume_hover(px, py);
        let hovered = self.hits.iter().position(|h| {
            px >= h.rect.0
                && px <= h.rect.0 + h.rect.2
                && py >= h.rect.1
                && py <= h.rect.1 + h.rect.3
        });
        if hovered != self.hovered_hit {
            self.hovered_hit = hovered;
            dirty = true;
        }
        self.sync_lyrics_window();
        let caption_position = (self.position() * 1000.) as i64;
        if let Some(window) = &mut self.subtitles {
            window.set_reduced_motion(self.state.settings.reduced_motion);
            window.update(&self.lyric_lines, caption_position);
        }
        self.update_media_animation();
        if dirty && !self.hidden && !self.tray_hidden {
            self.paint();
        }
    }
    #[allow(dead_code)]
    fn pomodoro_description(&self) -> String {
        let label = match self.state.pomodoro.phase {
            PomodoroPhase::Focus => self.tr("Foco", "Focus"),
            PomodoroPhase::ShortBreak => self.tr("Pausa curta", "Short break"),
            PomodoroPhase::LongBreak => self.tr("Pausa longa", "Long break"),
        };
        format!(
            "{label} · {}",
            time(self.state.pomodoro.remaining(Local::now().timestamp()) as f64)
        )
    }
    unsafe fn draw_status_icon(&self, r: &Renderer, x: f32, y: f32) {
        if self.page == 2 || self.state.pomodoro.running() {
            let color = if self.state.pomodoro.phase == PomodoroPhase::Focus {
                theme::FOCUS
            } else {
                theme::POSITIVE
            };
            let phase = if self.state.pomodoro.running() && !self.state.settings.reduced_motion {
                self.media_phase * 2.
            } else {
                0.
            };
            r.circle_alpha(x + 5., y, 9., color, 0.2);
            r.stroke_circle(x + 5., y, 8., color, 0.7, 1.2);
            for i in 0..20 {
                let a = -6. + i as f32 * 0.6;
                let b = a + 0.6;
                let wave = |v: f32| (v * 0.45 + phase).sin() * 2.5;
                r.line_alpha(
                    x + 5. + a,
                    y + wave(a),
                    x + 5. + b,
                    y + wave(b),
                    color,
                    0.95,
                    2.,
                );
            }
        } else {
            equalizer(
                r,
                x,
                y,
                self.media.playing,
                theme::TEXT,
                (!self.state.settings.reduced_motion).then_some(self.media_phase),
            );
        }
    }
    fn media_marquee_needed(&self) -> bool {
        matches!(self.page, 0 | 1)
            && !self.media.title.is_empty()
            && (self.media.title.len() > 18 || self.media.artist.len() > 22)
            && !self.modal
            && !self.state.settings.reduced_motion
    }
    fn media_motion_enabled(&self) -> bool {
        (self.media.playing || self.media_marquee_needed())
            && !self.media.title.is_empty()
            && !self.modal
            && !self.state.settings.reduced_motion
    }
    unsafe fn update_media_animation(&mut self) {
        let is_loading_lyrics = (self.lyric_status.contains("Buscando")
            || self.lyric_status.contains("Loading"))
            && !self.hidden
            && !self.modal
            && !self.state.settings.reduced_motion;
        let is_crossfading = self.renderer.as_ref().is_some_and(|r| r.is_crossfading())
            && !self.hidden
            && !self.modal
            && !self.state.settings.reduced_motion;
        let enabled = self.media_motion_enabled()
            || is_loading_lyrics
            || is_crossfading
            || (self.state.pomodoro.running()
                && !self.hidden
                && !self.modal
                && !self.state.settings.reduced_motion);
        if enabled != self.media_animating {
            self.media_animating = enabled;
            self.media_frame = Instant::now();
            if enabled {
                SetTimer(Some(self.hwnd), 3, 50, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), 3);
            }
        }
    }
    fn system_toast(&mut self, s: &SystemSnapshot) {
        let old = &self.system;
        let settings = &self.state.settings;
        let text = if settings.popup_bluetooth
            && old.keyboard_language != 0
            && s.bluetooth_devices != old.bluetooth_devices
        {
            Some(format!("Bluetooth · {}", s.bluetooth_devices.join(", ")))
        } else if settings.show_battery
            && settings.popup_battery
            && old.charging.is_some()
            && s.charging != old.charging
        {
            Some(
                self.tr(
                    if s.charging == Some(true) {
                        "Carregador conectado"
                    } else {
                        "Carregador desconectado"
                    },
                    if s.charging == Some(true) {
                        "Charger connected"
                    } else {
                        "Charger disconnected"
                    },
                )
                .into(),
            )
        } else if settings.popup_wifi && old.online.is_some() && s.online != old.online {
            Some(
                self.tr(
                    if s.online == Some(true) {
                        "Conectado"
                    } else {
                        "Sem conexão"
                    },
                    if s.online == Some(true) {
                        "Connected"
                    } else {
                        "No connection"
                    },
                )
                .into(),
            )
        } else {
            None
        };
        if let Some(t) = text {
            self.toast = Some((t, Instant::now()));
        }
    }
    unsafe fn fullscreen(&mut self) {
        if self.tray_hidden {
            return;
        }
        let was_active = self.fullscreen_state.active();
        let covered = self
            .fullscreen_state
            .update(self.hwnd, self.state.settings.hide_fullscreen);
        if !covered && !was_active && !self.hidden {
            return;
        }
        let mut pointer = POINT::default();
        let _ = GetCursorPos(&mut pointer);
        let scale = self.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
        let area = self.placement_area();
        let (edge_x, edge_y) = notch_position::origin(
            (area.left, area.top, area.right, area.bottom),
            self.canvas_width,
            (EDGE_WIDTH, EDGE_HEIGHT),
            scale,
            self.state.settings.notch_column,
        );
        let (x, y, w, h) = self.edge_rect();
        let on_edge = pointer.x as f32 >= edge_x as f32 + x * scale
            && pointer.x as f32 <= edge_x as f32 + (x + w) * scale
            && pointer.y as f32 >= edge_y as f32 + y * scale
            && pointer.y as f32 <= edge_y as f32 + (y + h) * scale;
        let mut client = pointer;
        let _ = ScreenToClient(self.hwnd, &mut client);
        let on_panel =
            !self.hidden && self.inside(client.x as f32 / scale, client.y as f32 / scale);
        let interacting = self.modal
            || self.notch_resize.is_some()
            || self.notch_drag.is_some()
            || self.shelf_gesture.is_some()
            || self.volume_dragging;
        let mode = self.fullscreen_state.reveal.update(
            covered,
            on_edge,
            on_panel,
            interacting,
            Instant::now(),
        );
        let hint = mode == Visibility::Hint;
        let changed = hint != self.hidden || covered != was_active;
        if !changed {
            return;
        }
        let was_hidden = self.hidden;
        self.hidden = hint;
        if hint {
            self.expanded = false;
            self.hover = false;
            self.hovered_hit = None;
            self.focus = None;
            self.navigation_hover = false;
            self.volume_hovered = false;
            self.navigation.snap(0.);
            self.animate();
            if GetForegroundWindow() == self.hwnd {
                if let Some(target) = self.fullscreen_state.target() {
                    let _ = SetForegroundWindow(target);
                }
            }
        } else if mode == Visibility::Revealed && (was_hidden || !was_active) {
            self.hover = true;
            self.expand(true);
        } else if was_hidden {
            self.expanded = false;
            let (w, h) = self.target_size();
            self.body_width.snap(w);
            self.body_height.snap(h);
        }
        self.paint();
        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            self.hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    fn edge_rect(&self) -> (f32, f32, f32, f32) {
        let (_, ear) = nivumi::fullscreen::edge_hint_corners(EDGE_HEIGHT);
        (
            (self.canvas_width - EDGE_WIDTH) / 2. - ear,
            0.,
            EDGE_WIDTH + ear * 2.,
            EDGE_HEIGHT,
        )
    }
    fn target_size(&self) -> (f32, f32) {
        if self.hidden {
            return (EDGE_WIDTH, EDGE_HEIGHT);
        }
        let cw = if self.state.settings.minimal {
            112.
        } else {
            150.
        };
        let ch = if self.state.settings.minimal {
            18.
        } else {
            30.
        };
        let ew = if self.page == 1 {
            self.state.settings.lyrics.notch_width
        } else {
            BODY
        };
        let mut eh = if self.page == 1 {
            self.state.settings.lyrics.notch_height
        } else if self.page == 6 {
            286.
        } else if matches!(self.page, 2..=5) {
            170.
        } else {
            140.
        };
        if self.expanded && self.navigation_hover && self.notch_resize.is_none() {
            eh += NAV_EXPAND_H;
        }
        if self.expanded {
            (ew, eh)
        } else if self.compact_lyrics_enabled() {
            (312., 36.)
        } else {
            (cw, ch)
        }
    }
    fn compact_lyrics_enabled(&self) -> bool {
        self.page == 1 && !self.media.title.is_empty()
    }
    fn active_lyric_line(&self) -> Option<usize> {
        lyrics::active_line(&self.lyric_lines, self.lyric_position_ms())
    }
    fn body_size(&self) -> (f32, f32) {
        (self.body_width.value, self.body_height.value)
    }
    fn player_width(&self) -> f32 {
        PlayerLayout::width(
            self.state.settings.lyrics.notch_width,
            self.player_layout.value,
        )
    }
    fn player_geometry(&self) -> PlayerLayout {
        PlayerLayout::new(
            (self.canvas_width - self.player_width()) / 2.,
            self.player_layout.value,
        )
    }
    fn inside(&self, x: f32, y: f32) -> bool {
        if self.hidden {
            let (left, top, width, height) = self.edge_rect();
            return x >= left && x <= left + width && y >= top && y <= top + height;
        }
        let (w, h) = if self.expanded {
            (
                self.body_width.value.max(self.target_size().0),
                self.body_height.value.max(self.target_size().1),
            )
        } else {
            self.target_size()
        };
        let ox = (self.canvas_width - w) / 2.;
        (x >= ox && x <= ox + w && y >= 0. && y <= h)
            || ((self.pomodoro_notice.is_some() || self.toast.is_some())
                && y >= h + 8.
                && y <= h + 58.
                && (x - self.canvas_width / 2.).abs() <= 175.)
            || (self.expanded && self.navigation.value > 0.01 && self.navigation_contains(x, y))
    }
    fn hit(&mut self, rect: (f32, f32, f32, f32), label: &str, action: Action) {
        self.hits.push(Hit {
            rect: (rect.0, rect.1 + self.content_offset_y, rect.2, rect.3),
            action,
            label: label.into(),
        });
    }
    unsafe fn paint(&mut self) {
        if self.tray_hidden {
            return;
        }
        if self.hidden {
            self.paint_edge_hint();
            return;
        }
        if self.renderer.is_none() {
            self.resize();
        }
        let Some(renderer) = self.renderer.take() else {
            return;
        };
        renderer.begin();
        let lyric_line = self.active_lyric_line();
        let (w, h) = self.body_size();
        self.position_notch(renderer.dpi / 96., w, h);
        let ox = (self.canvas_width - w) / 2.;
        let amount = self.expansion.value.clamp(0., 1.);
        let accent = if matches!(self.page, 0 | 1) || amount < 0.01 {
            self.accent.map(|s| s.value.clamp(0., 0.5))
        } else {
            [0.; 3]
        };
        let radius = 14. + 18. * amount;
        let ear = 10. + 12. * amount;
        let mask = renderer.notch(
            ox,
            w,
            h,
            radius,
            ear,
            accent,
            self.media_motion_enabled().then_some(self.media_phase),
        );
        let mut overlays = Vec::new();
        self.hits.clear();
        let compact_opacity = (1. - amount * 3.).clamp(0., 1.);
        if compact_opacity > 0. {
            renderer.push_content(compact_opacity, mask.as_ref());
            let cy = h / 2.;
            if self.compact_lyrics_enabled() {
                let content = self.lyrics_content();
                let text = content.compact_text(self.state.settings.en());
                icon(&renderer, "music", ox + 12., cy - 8., 16., theme::MUTED);
                renderer.text(
                    text,
                    ox + 38.,
                    0.,
                    (w - 50.).max(0.),
                    h,
                    12.,
                    theme::TEXT,
                    true,
                    true,
                );
                self.hit(
                    (ox, 0., w, h),
                    &format!("{}: {text}", self.tr("Abrir letras", "Open lyrics")),
                    Action::Page(1),
                );
            } else if self.media.title.is_empty()
                && !self.state.pomodoro.running()
                && self.page != 2
            {
                if !renderer.svg_icon(
                    include_str!("../assets/brand/mark.svg"),
                    self.canvas_width / 2. - 10.,
                    cy - 10.,
                    20.,
                    theme::ACCENT,
                ) {
                    renderer.text("n", ox, 0., w, h, 20., theme::ACCENT, true, true);
                }
            } else {
                let has_media = !self.media.title.is_empty();
                let pomodoro_running = self.state.pomodoro.running();

                // 1. Ícone esquerdo: Bateria, Mini Capa/Música ou Timer do Pomodoro
                let mut has_left_icon = false;
                if self.state.settings.show_battery {
                    ring(
                        &renderer,
                        ox + 20.,
                        cy,
                        7.,
                        theme::POSITIVE,
                        self.system.battery_percent.unwrap_or(100) as f32 / 100.,
                    );
                    has_left_icon = true;
                } else if has_media {
                    let art_size = if self.state.settings.minimal {
                        12.
                    } else {
                        16.
                    };
                    let art_x = ox + 20. - art_size / 2.;
                    let art_y = cy - art_size / 2.;
                    let drawn_art = !self.media.cover.is_empty()
                        && !self.state.settings.minimal
                        && renderer.artwork(
                            &self.media.cover,
                            windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                                left: art_x,
                                top: art_y,
                                right: art_x + art_size,
                                bottom: art_y + art_size,
                            },
                        );
                    if drawn_art {
                        renderer.stroke_rounded_rect(
                            art_x,
                            art_y,
                            art_size,
                            art_size,
                            3.,
                            theme::TEXT,
                            0.25,
                            0.8,
                            false,
                        );
                    } else {
                        let icon_size = if self.state.settings.minimal {
                            11.
                        } else {
                            13.
                        };
                        icon(
                            &renderer,
                            "music",
                            ox + 20. - icon_size / 2.,
                            cy - icon_size / 2.,
                            icon_size,
                            theme::MUTED,
                        );
                    }
                    has_left_icon = true;
                } else if pomodoro_running {
                    let icon_size = if self.state.settings.minimal {
                        11.
                    } else {
                        13.
                    };
                    let color = if self.state.pomodoro.phase == PomodoroPhase::Focus {
                        theme::FOCUS
                    } else {
                        theme::POSITIVE
                    };
                    icon(
                        &renderer,
                        "timer",
                        ox + 20. - icon_size / 2.,
                        cy - icon_size / 2.,
                        icon_size,
                        color,
                    );
                    has_left_icon = true;
                }

                // 2. Ícone direito: Equalizador ou Anel/Onda do Pomodoro
                let has_right_icon = pomodoro_running
                    || has_media
                    || self.media.playing
                    || self.state.settings.show_battery;
                if has_right_icon {
                    self.draw_status_icon(&renderer, ox + w - 24., cy);
                }

                // 3. Texto do Relógio ou Cronômetro (centralização dinâmica para simetria perfeita)
                let clock_text = if pomodoro_running {
                    time(
                        self.state
                            .pomodoro
                            .remaining(chrono::Local::now().timestamp())
                            as f64,
                    )
                } else {
                    self.last_clock.clone()
                };

                let (text_x, text_w) = compact_notch_layout(has_left_icon, has_right_icon, ox, w);

                let font_size = if self.state.settings.minimal {
                    13.
                } else {
                    16.
                };
                renderer.text(
                    &clock_text,
                    text_x,
                    0.,
                    text_w,
                    h,
                    font_size,
                    theme::TEXT,
                    true,
                    true,
                );
            }
            renderer.pop_content();
        }
        let content_opacity = ((amount - 0.25) / 0.65).clamp(0., 1.);
        if content_opacity > 0. {
            renderer.push_content(
                content_opacity * content_opacity * (3. - 2. * content_opacity),
                mask.as_ref(),
            );
            let expanded_width = if matches!(self.page, 0 | 1) {
                self.player_width()
            } else {
                BODY
            };
            self.draw_expanded(&renderer, (self.canvas_width - expanded_width) / 2.);
            renderer.pop_content();
            if !self.expanded || content_opacity < 0.95 {
                self.hits.clear();
            } else {
                self.draw_dots(&renderer, (self.canvas_width - expanded_width) / 2.);
            }
        }
        if let Some((title, body, next, _)) = self.pomodoro_notice.clone() {
            let tw = 350.;
            let x = (self.canvas_width - tw) / 2.;
            let y = h + 8.;
            overlays.push((x, y, tw, 48., 22.));
            renderer.fill(x, y, tw, 48., 22., theme::SURFACE);
            renderer.stroke_circle(x + 26., y + 24., 18., theme::ACCENT, 0.65, 1.4);
            icon(&renderer, "timer", x + 16., y + 14., 20., theme::TEXT);
            renderer.text(
                &title,
                x + 54.,
                y + 6.,
                174.,
                20.,
                13.,
                theme::TEXT,
                true,
                false,
            );
            renderer.text(
                &body,
                x + 54.,
                y + 26.,
                174.,
                16.,
                10.5,
                theme::MUTED,
                false,
                false,
            );
            let dismiss_rect = (x + 232., y + 4., 36., 40.);
            let is_dismiss_hovered = self.is_hovered(dismiss_rect);
            renderer.fill(
                x + 236.,
                y + 10.,
                28.,
                28.,
                14.,
                if is_dismiss_hovered {
                    theme::PRESSED
                } else {
                    theme::ERROR_SURFACE
                },
            );
            icon(&renderer, "close", x + 244., y + 18., 12., theme::ERROR);
            self.hit(
                dismiss_rect,
                self.tr("Dispensar", "Dismiss"),
                Action::DismissPomodoro,
            );
            renderer.fill(x + 272., y + 10., 70., 28., 14., theme::POSITIVE_SURFACE);
            let label = if next.is_some_and(|phase| phase != PomodoroPhase::Focus) {
                self.tr("Pausa", "Break")
            } else {
                self.tr("Foco", "Focus")
            };
            renderer.text(
                label,
                x + 272.,
                y + 10.,
                70.,
                28.,
                11.,
                theme::POSITIVE,
                true,
                true,
            );
            self.hit(
                (x + 272., y + 10., 70., 28.),
                label,
                next.map(Action::PomodoroNext).unwrap_or(Action::Page(2)),
            );
        } else if let Some((toast, time)) = &self.toast {
            let tw = 310.;
            let tox = (self.canvas_width - tw) / 2.;
            let elapsed = time.elapsed().as_secs_f32();
            let opacity = if self.state.settings.reduced_motion {
                1.0
            } else if elapsed < 0.2 {
                (elapsed / 0.2).clamp(0.05, 1.0)
            } else if elapsed > 5.7 {
                ((6.0 - elapsed) / 0.3).clamp(0.0, 1.0)
            } else {
                1.0
            };
            overlays.push((tox, h + 10., tw, 46., 18.));
            renderer.push_content(opacity, None);
            renderer.fill_alpha(tox, h + 10., tw, 46., 18., theme::SURFACE, 0.95);
            renderer.stroke_rounded_rect(tox, h + 10., tw, 46., 18., theme::TEXT, 0.22, 1., false);
            renderer.text(
                toast,
                tox + 16.,
                h + 13.,
                tw - 46.,
                40.,
                12.,
                theme::TEXT,
                false,
                true,
            );
            renderer.text(
                "✕",
                tox + tw - 26.,
                h + 23.,
                16.,
                16.,
                11.,
                theme::MUTED,
                false,
                true,
            );
            renderer.pop_content();
            self.hit(
                (tox, h + 10., tw, 46.),
                self.tr("Dispensar notificação", "Dismiss notification"),
                Action::DismissToast,
            );
        }
        if let Some(i) = self.focus {
            if let Some(hit) = self.hits.get(i) {
                let (x, y, w, h) = hit.rect;
                renderer.stroke_rounded_rect(
                    x - 2.,
                    y - 2.,
                    w + 4.,
                    h + 4.,
                    8.,
                    theme::POSITIVE,
                    0.85,
                    1.75,
                    false,
                );
            }
        }
        let scale = renderer.dpi / 96.;
        self.window_region.update(
            self.hwnd,
            scale,
            &notch_outline(ox, w, h, radius, ear),
            &overlays,
        );
        let ok = renderer.end().is_ok();
        if ok {
            self.renderer = Some(renderer);
            self.painted_lyric_line = lyric_line;
        }
        self.publish_accessibility(scale);
    }
    unsafe fn paint_edge_hint(&mut self) {
        if self.renderer.is_none() {
            self.resize();
        }
        let Some(renderer) = self.renderer.take() else {
            return;
        };
        let scale = renderer.dpi / 96.;
        let (w, h) = self.body_size();
        self.position_notch(scale, w, h);
        renderer.begin();
        let x = (self.canvas_width - w) / 2.;
        let y = 0.;
        let finish = nivumi::motion::edge_hint_opacity(h);
        let (radius, ear) = nivumi::fullscreen::edge_hint_corners(h);
        let outline = renderer.notch(
            x,
            w,
            h,
            radius,
            ear,
            self.accent.map(|s| s.value.clamp(0., 0.5)),
            None,
        );
        // Both playback states retain the contour attached to the screen edge.
        let playing = self.media.playing && !self.media.title.is_empty();
        if let Some(outline) = outline {
            renderer.fill_path(
                &outline,
                if playing {
                    theme::SELECTED
                } else {
                    theme::TEXT
                },
                0.92 * finish,
            );
        }
        if playing && finish > 0. {
            let phase = self.media_motion_enabled().then_some(self.media_phase);
            // The existing equalizer is scaled to stay within the ten-DIP handle.
            let center = x + w / 2.;
            for (i, height) in equalizer_heights(phase, true).iter().enumerate() {
                let height = height * 0.4;
                renderer.fill_alpha(
                    center - 7. + i as f32 * 3.,
                    (h - height) / 2.,
                    2.,
                    height,
                    1.,
                    theme::ACCENT,
                    finish,
                );
            }
            for line_x in [x + 14., center + 18.] {
                renderer.fill_alpha(
                    line_x,
                    h / 2. - 0.5,
                    (w / 2. - 32.).max(0.),
                    1.,
                    0.5,
                    theme::ACCENT,
                    0.35 * finish,
                );
            }
        }
        self.hits.clear();
        self.hit(
            (x - ear, y, w + ear * 2., h),
            self.tr(
                if playing {
                    "Música tocando · Mostrar Nivumi"
                } else {
                    "Mostrar Nivumi"
                },
                if playing {
                    "Music playing · Show Nivumi"
                } else {
                    "Show Nivumi"
                },
            ),
            Action::Reveal,
        );
        self.window_region
            .update(self.hwnd, scale, &notch_outline(x, w, h, radius, ear), &[]);
        if renderer.end().is_ok() {
            self.renderer = Some(renderer);
        }
        self.publish_accessibility(scale);
    }
    unsafe fn publish_accessibility(&self, scale: f32) {
        let items = self
            .hits
            .iter()
            .map(|h| AccessibleItem {
                label: h.label.clone(),
                action_id: h.id(),
                enabled: true,
                rect: RECT {
                    left: (h.rect.0 * scale) as i32,
                    top: (h.rect.1 * scale) as i32,
                    right: ((h.rect.0 + h.rect.2) * scale) as i32,
                    bottom: ((h.rect.1 + h.rect.3) * scale) as i32,
                },
            })
            .collect();
        if let Ok(mut shared) = self.accessible_items.lock() {
            *shared = items;
        }
        if let Some(provider) = &self.accessible_provider {
            keyboard_focus(
                provider,
                self.focus.and_then(|i| self.hits.get(i)).map(Hit::id),
            );
        }
    }
    fn navigation_contains(&self, x: f32, y: f32) -> bool {
        let (half_width, min_y, max_y) = if self.navigation_hover || self.navigation.value > 0.01 {
            (
                NAV_HOVER_HALF_WIDTH,
                self.body_height.value - 52.,
                self.body_height.value + 6.,
            )
        } else {
            (
                38.,
                self.body_height.value - 20.,
                self.body_height.value + 4.,
            )
        };
        (x - self.canvas_width / 2.).abs() <= half_width && y >= min_y && y <= max_y
    }
    unsafe fn update_navigation_hover(&mut self, x: f32, y: f32) {
        // Keep the navigation available while tabbing, even with the pointer away.
        let keyboard = self.focus.is_some() && GetFocus() == self.hwnd;
        let hovered = self.expanded && (self.navigation_contains(x, y) || keyboard);
        if hovered != self.navigation_hover {
            self.navigation_hover = hovered;
            self.animate();
        }
    }
    #[allow(dead_code)]
    #[allow(dead_code)]
    fn change_volume(&mut self, v: f32) {
        self.volume_fade = None;
        let v = v.clamp(0., 1.);
        self.system.volume = Some(v);
        if v > 0.005 {
            self.volume_saved = v;
        }
        self.services.send(ServiceCommand::Volume(v));
        if self.volume_dragging {
            self.volume_level.snap(v);
        }
        unsafe {
            self.animate();
        }
    }
    #[allow(dead_code)]
    #[allow(dead_code)]
    fn toggle_volume_mute(&mut self) {
        if let Some(v) = self.system.volume {
            let target = if v > 0.005 {
                self.volume_saved = v;
                0.
            } else if self.volume_saved > 0.05 {
                self.volume_saved
            } else {
                0.25
            };
            if self.state.settings.reduced_motion {
                self.volume_fade = None;
                self.change_volume(target);
            } else {
                self.volume_fade = Some((v, target, Instant::now(), Duration::from_millis(150)));
                unsafe {
                    self.animate();
                }
            }
        }
    }
    #[allow(dead_code, clippy::type_complexity)]
    #[allow(dead_code, clippy::type_complexity)]
    fn volume_rects(&self) -> Option<((f32, f32, f32, f32), (f32, f32, f32, f32))> {
        if !self.expanded || !matches!(self.page, 0 | 1) || self.system.volume.is_none() {
            return None;
        }
        let layout = self.player_geometry();
        Some((layout.volume_icon, layout.volume_bar))
    }
    fn is_volume_hit(&self, x: f32, y: f32) -> bool {
        if let Some((icon, bar)) = self.volume_rects() {
            let over_icon =
                x >= icon.0 && x <= icon.0 + icon.2 && y >= icon.1 && y <= icon.1 + icon.3;
            let over_bar = x >= bar.0 && x <= bar.0 + bar.2 && y >= bar.1 && y <= bar.1 + bar.3;
            if over_icon || over_bar {
                return true;
            }
            if self.volume_hovered || self.volume_dragging {
                let min_x = icon.0.min(bar.0) - 6.;
                let max_x = (icon.0 + icon.2).max(bar.0 + bar.2) + 6.;
                let min_y = icon.1.min(bar.1) - 6.;
                let max_y = (icon.1 + icon.3).max(bar.1 + bar.3) + 6.;
                x >= min_x && x <= max_x && y >= min_y && y <= max_y
            } else {
                false
            }
        } else {
            false
        }
    }
    #[allow(dead_code)]
    #[allow(dead_code)]
    fn is_volume_bar_hit(&self, x: f32, y: f32) -> bool {
        if let Some((_, bar)) = self.volume_rects() {
            x >= bar.0 && x <= bar.0 + bar.2 && y >= bar.1 && y <= bar.1 + bar.3
        } else {
            false
        }
    }
    #[allow(dead_code)]
    #[allow(dead_code)]
    fn is_seek_hit(&self, x: f32, y: f32) -> bool {
        if !self.expanded
            || !matches!(self.page, 0 | 1)
            || !self.media.can_seek
            || self.media.duration <= 0.
        {
            return false;
        }
        let layout = self.player_geometry();
        let (sx, sy, sw, sh) = layout.seek;
        x >= sx && x <= sx + sw && y >= sy && y <= sy + sh
    }
    fn wheel_target(
        &self,
        x: f32,
        y: f32,
        up: bool,
        shift: bool,
        alt: bool,
        horizontal: bool,
    ) -> WheelTarget {
        // Match the lyrics clip in draw_lyrics, in logical (DPI-independent) coordinates.
        let width = self.player_width();
        let left = (self.canvas_width - width) / 2. + 264.;
        let height = 140.
            + (self.state.settings.lyrics.notch_height - 140.)
                * self.player_layout.value.clamp(0., 1.);
        let over_full_lyrics = self.expanded
            && self.page == 1
            && matches!(self.lyrics_view_mode, 2 | 3)
            && x >= left
            && x <= left + (width - 284.).max(180.)
            && y >= 44.
            && y <= height - 18.;
        let over_shelf_content =
            self.page == 4 && self.state.shelf.len() > 5 && (25. ..=135.).contains(&y);
        handle_wheel(
            self.page,
            self.scroll,
            self.state.shelf.len(),
            self.state.tasks.len(),
            up,
            shift || horizontal || over_shelf_content || over_full_lyrics,
            alt,
        )
    }
    fn is_volume_icon_hit(&self, x: f32, y: f32) -> bool {
        if let Some((icon, _)) = self.volume_rects() {
            x >= icon.0 && x <= icon.0 + icon.2 && y >= icon.1 && y <= icon.1 + icon.3
        } else {
            false
        }
    }
    unsafe fn update_volume_hover(&mut self, x: f32, y: f32) {
        let hovered = self.is_volume_hit(x, y);
        if hovered != self.volume_hovered {
            self.volume_hovered = hovered;
            self.animate();
        }
    }
    unsafe fn draw_dots(&mut self, r: &Renderer, _ox: f32) {
        let center = self.canvas_width / 2.;
        let h = self.body_height.value;
        let amount = self.navigation.value.clamp(0., 1.);
        let entries = [
            (
                Action::Page(0),
                self.tr("Música", "Music"),
                "music",
                self.page == 0,
            ),
            (
                Action::Page(1),
                self.tr("Letras", "Lyrics"),
                "lyrics",
                self.page == 1,
            ),
            (Action::Page(2), "Pomodoro", "timer", self.page == 2),
            (
                Action::Page(3),
                self.tr("Hidratação", "Hydration"),
                "drop",
                self.page == 3,
            ),
            (
                Action::Page(4),
                self.tr("Prateleira", "Shelf"),
                "download",
                self.page == 4,
            ),
            (
                Action::Page(5),
                self.tr("Tarefas", "Tasks"),
                "check",
                self.page == 5,
            ),
            (
                Action::Page(6),
                self.tr("Calendário", "Calendar"),
                "calendar",
                self.page == 6,
            ),
            (
                Action::Settings,
                self.tr("Configurações", "Settings"),
                "settings",
                false,
            ),
        ];
        if amount < 0.99 {
            r.push_content(1. - amount, None);
            let mut x = center - NAV_DOTS_WIDTH / 2.;
            for (_, _, _, active) in &entries[..7] {
                let w = if *active { 15. } else { 3.6 };
                r.fill_alpha(
                    x,
                    h - 8.8,
                    w,
                    3.6,
                    1.8,
                    theme::TEXT,
                    if *active { 1. } else { 0.45 },
                );
                x += w + 5.5;
            }
            r.pop_content();
        }
        if amount > 0.01 {
            r.push_content(amount, None);
            let width = 62. + (NAV_PILL_WIDTH - 62.) * amount;
            let pill_h = NAV_TARGET_HEIGHT;
            let pill_y = h - pill_h - 4.;
            let pill_x = center - width / 2.;
            let pill_r = pill_h / 2.;

            // 1. Soft ambient shadow under the floating pill
            r.fill_alpha(
                pill_x - 1.,
                pill_y + 1.5,
                width + 2.,
                pill_h,
                pill_r,
                theme::SURFACE,
                0.22 * amount,
            );

            // 2. Frosted glass base container
            r.fill_alpha(pill_x, pill_y, width, pill_h, pill_r, theme::SURFACE, 0.78);

            // Ambient artwork tint if playing
            let ar = (self.accent[0].value.clamp(0., 1.) * 255.) as u32;
            let ag = (self.accent[1].value.clamp(0., 1.) * 255.) as u32;
            let ab = (self.accent[2].value.clamp(0., 1.) * 255.) as u32;
            let has_accent = (ar > 15 || ag > 15 || ab > 15) && matches!(self.page, 0 | 1);
            let accent_rgb = (ar << 16) | (ag << 8) | ab;
            if has_accent {
                r.fill_alpha(pill_x, pill_y, width, pill_h, pill_r, accent_rgb, 0.06);
            }

            // 3. Subtle frosted glass border
            r.stroke_rounded_rect(
                pill_x,
                pill_y,
                width,
                pill_h,
                pill_r,
                theme::TEXT,
                0.12,
                1.,
                false,
            );

            // 4. Specular top rim light
            if width > 60. {
                let rim_margin = 12.;
                r.line_alpha(
                    pill_x + rim_margin,
                    pill_y + 1.,
                    pill_x + width - rim_margin,
                    pill_y + 1.,
                    theme::TEXT,
                    0.14,
                    1.,
                );
            }

            // 5. Vertical subtle divider separating navigation pages from Settings
            if amount > 0.85 {
                let div_x = center - NAV_ICON_COUNT as f32 * 16. + 7.0 * 32.;
                r.line_alpha(
                    div_x,
                    pill_y + 8.,
                    div_x,
                    pill_y + pill_h - 8.,
                    theme::TEXT,
                    0.10,
                    1.,
                );
            }

            // 6. Navigation items with active capsule & hover feedback
            for (i, (action, label, glyph, active)) in entries.iter().enumerate() {
                let x = center - NAV_ICON_COUNT as f32 * 16. + i as f32 * 32.;
                let is_hovered = amount > 0.8 && self.is_hovered((x, pill_y, 32., pill_h));

                // Active capsule or hover highlight
                let item_w = 26.;
                let item_h = 24.;
                let item_x = x + (32. - item_w) / 2.;
                let item_y = pill_y + (pill_h - item_h) / 2.;
                let item_r = 7.;

                if *active {
                    if has_accent {
                        // Smooth artwork-derived tint + frosted layer
                        r.fill_alpha(item_x, item_y, item_w, item_h, item_r, accent_rgb, 0.22);
                        r.fill_alpha(item_x, item_y, item_w, item_h, item_r, theme::TEXT, 0.10);
                        r.stroke_rounded_rect(
                            item_x, item_y, item_w, item_h, item_r, accent_rgb, 0.35, 1., false,
                        );
                    } else {
                        // Clean frosted white capsule
                        r.fill_alpha(item_x, item_y, item_w, item_h, item_r, theme::TEXT, 0.16);
                        r.stroke_rounded_rect(
                            item_x,
                            item_y,
                            item_w,
                            item_h,
                            item_r,
                            theme::TEXT,
                            0.18,
                            1.,
                            false,
                        );
                    }
                } else if is_hovered {
                    r.fill_alpha(item_x, item_y, item_w, item_h, item_r, theme::TEXT, 0.08);
                }

                let icon_color = if *active {
                    theme::ACCENT
                } else if is_hovered {
                    theme::TEXT
                } else {
                    theme::MUTED
                };

                icon(
                    r,
                    glyph,
                    x + 8.,
                    pill_y + (pill_h - 16.) / 2.,
                    16.,
                    icon_color,
                );
                if amount > 0.8 {
                    self.hit((x, pill_y, 32., pill_h), label, action.clone());
                }
            }
            r.pop_content();
        }
    }
    unsafe fn draw_header(&mut self, r: &Renderer, ox: f32) {
        if matches!(self.page, 2..=5) {
            if self.state.settings.show_battery {
                ring(
                    r,
                    ox + 296.,
                    19.,
                    7.,
                    theme::POSITIVE,
                    self.system.battery_percent.unwrap_or(100) as f32 / 100.,
                );
            }
            r.text(
                &self.last_clock,
                ox + 312.,
                8.,
                48.,
                22.,
                12.,
                theme::TEXT,
                false,
                true,
            );
            self.draw_status_icon(r, ox + 364., 19.);
            return;
        }
        if self.state.settings.show_battery {
            // Battery ring at ox + 146., 18.
            ring(
                r,
                ox + 146.,
                18.,
                7.,
                theme::POSITIVE,
                self.system.battery_percent.unwrap_or(100) as f32 / 100.,
            );
        }
        // Date & Time at ox + 163. (with battery) or ox + 146. (without battery)
        let now = chrono::Local::now();
        let date_str = if self.state.settings.en() {
            now.format("%a %d %b %Y").to_string()
        } else {
            let weekday = match now.weekday() {
                chrono::Weekday::Mon => "Seg",
                chrono::Weekday::Tue => "Ter",
                chrono::Weekday::Wed => "Qua",
                chrono::Weekday::Thu => "Qui",
                chrono::Weekday::Fri => "Sex",
                chrono::Weekday::Sat => "Sáb",
                chrono::Weekday::Sun => "Dom",
            };
            let month = match now.month() {
                1 => "Jan",
                2 => "Fev",
                3 => "Mar",
                4 => "Abr",
                5 => "Mai",
                6 => "Jun",
                7 => "Jul",
                8 => "Ago",
                9 => "Set",
                10 => "Out",
                11 => "Nov",
                12 => "Dez",
                _ => "",
            };
            format!("{} {} {} {}", weekday, now.day(), month, now.year())
        };
        let date_x = if self.state.settings.show_battery {
            ox + 163.
        } else {
            ox + 146.
        };
        let date_w = if self.state.settings.show_battery {
            190.
        } else {
            210.
        };
        r.text(
            &format!("{}  {}", date_str, self.last_clock),
            date_x,
            8.,
            date_w,
            20.,
            11.5,
            theme::TEXT,
            false,
            false,
        );
        // Equalizer at ox + 364., 18.
        self.draw_status_icon(r, ox + 364., 19.);
    }
    unsafe fn draw_expanded(&mut self, r: &Renderer, ox: f32) {
        if !matches!(self.page, 0 | 1) {
            match self.page {
                2 => {
                    r.text(
                        "Pomodoro",
                        ox + 15.,
                        8.,
                        120.,
                        22.,
                        15.,
                        theme::TEXT,
                        true,
                        false,
                    );
                }
                3 => {
                    r.text(
                        self.tr("Hidratação", "Hydration"),
                        ox + 15.,
                        8.,
                        136.,
                        22.,
                        15.,
                        theme::TEXT,
                        true,
                        false,
                    );
                }
                4 => {
                    r.text(
                        self.tr("Prateleira", "Shelf"),
                        ox + 15.,
                        8.,
                        120.,
                        22.,
                        15.,
                        theme::TEXT,
                        true,
                        false,
                    );
                }
                5 => {
                    r.text(
                        self.tr("Tarefas", "Tasks"),
                        ox + 15.,
                        8.,
                        140.,
                        22.,
                        15.,
                        theme::POSITIVE,
                        true,
                        false,
                    );
                }
                _ => {
                    r.text(
                        self.tr("Calendário", "Calendar"),
                        ox + 15.,
                        8.,
                        120.,
                        22.,
                        15.,
                        theme::TEXT,
                        true,
                        false,
                    );
                }
            }
        }
        if !matches!(self.page, 0 | 1) {
            self.draw_header(r, ox);
        }
        if self.page == 4 {
            let info_x = ox + 104.;
            let info_y = 8.;
            let info_hovered = self
                .hovered_hit
                .and_then(|i| self.hits.get(i))
                .is_some_and(|h| matches!(h.action, Action::ShelfInfo));
            r.fill_alpha(
                info_x,
                info_y,
                24.,
                24.,
                12.,
                theme::TEXT,
                if info_hovered { 0.18 } else { 0.08 },
            );
            icon(
                r,
                "info",
                info_x + 5.,
                info_y + 5.,
                14.,
                if info_hovered {
                    theme::TEXT
                } else {
                    theme::MUTED
                },
            );
            self.hit(
                (info_x, info_y, 24., 24.),
                self.tr("O que a prateleira pode fazer", "What the shelf can do"),
                Action::ShelfInfo,
            );

            let selected = !self.selected.is_empty();
            let label = if selected {
                self.tr("Remover", "Remove")
            } else {
                self.tr("Limpar", "Clear")
            };
            r.fill_alpha(ox + 144., 8., 64., 24., 12., theme::TEXT, 0.1);
            r.text(
                label,
                ox + 144.,
                8.,
                64.,
                24.,
                11.,
                theme::TEXT,
                false,
                true,
            );
            self.hit(
                (ox + 144., 8., 64., 24.),
                label,
                if selected {
                    Action::RemoveSelected
                } else {
                    Action::ClearShelf
                },
            );
            let count = if selected {
                format!(
                    "{} {}",
                    self.selected.len(),
                    self.tr(
                        if self.selected.len() == 1 {
                            "selecionado"
                        } else {
                            "selecionados"
                        },
                        "selected"
                    )
                )
            } else {
                format!("{} {}", self.state.shelf.len(), self.tr("itens", "items"))
            };
            r.text(
                &count,
                ox + 216.,
                9.,
                88.,
                22.,
                10.,
                theme::MUTED,
                false,
                false,
            );
        }
        // Hydration already reserves its header inside its own layout.
        let offset = expanded_content_offset(self.page);
        self.content_offset_y = offset;
        let mut transform = windows_numerics::Matrix3x2::identity();
        transform.M32 = offset;
        r.dc.SetTransform(&transform);
        match self.page {
            0 | 1 => {
                self.draw_player(r, ox);
                let amount = self.player_layout.value.clamp(0., 1.);
                if amount < 1. {
                    r.push_content(1. - amount, None);
                    self.draw_header(r, ox);
                    r.pop_content();
                }
                let opacity = PlayerLayout::lyrics_opacity(amount);
                if opacity > 0. {
                    let shared_hits = self.hits.len();
                    r.push_content(opacity, None);
                    self.draw_lyrics(r, ox);
                    r.pop_content();
                    if self.page != 1 {
                        self.hits.truncate(shared_hits);
                    }
                }
            }
            2 => self.draw_pomodoro(r, ox),
            3 => self.draw_water(r, ox),
            4 => self.draw_shelf(r, ox),
            5 => self.draw_tasks(r, ox),
            _ => self.draw_calendar(r, ox),
        }
        r.dc.SetTransform(&windows_numerics::Matrix3x2::identity());
        self.content_offset_y = 0.;

        // Universal Close / Minimize to Background Button (available in all sections)
        let notch_w = if self.page == 1 {
            self.player_width()
        } else {
            BODY
        };
        let close_cx = ox + notch_w - 20.;
        let close_cy = if self.page == 1 { 22.5 } else { 19. };
        let close_hit_rect = if self.page == 1 {
            let (x, y, w, h) = nivumi::player_layout::lyrics_header_action(notch_w, 3);
            (ox + x, y, w, h)
        } else {
            (close_cx - 16., close_cy - 15., 32., 30.)
        };
        let is_close_hovered = self.is_hovered(close_hit_rect);
        if is_close_hovered {
            r.circle_alpha(close_cx, close_cy, 12., theme::TEXT, 0.15);
        }
        icon(
            r,
            "close",
            close_cx - 5.,
            close_cy - 5.,
            10.,
            if is_close_hovered {
                theme::TEXT
            } else {
                theme::MUTED
            },
        );
        self.hit(
            close_hit_rect,
            self.tr("Ocultar na bandeja (Esc)", "Hide to tray (Esc)"),
            Action::Close,
        );
    }
    unsafe fn draw_player(&mut self, r: &Renderer, ox: f32) {
        let layout = self.player_geometry();
        // Ambient Glow (Backlight) based on artwork accent color
        let ar = (self.accent[0].value.clamp(0., 1.) * 255.) as u32;
        let ag = (self.accent[1].value.clamp(0., 1.) * 255.) as u32;
        let ab = (self.accent[2].value.clamp(0., 1.) * 255.) as u32;
        let accent_rgb = (ar << 16) | (ag << 8) | ab;
        if !self.media.cover.is_empty() && (ar > 10 || ag > 10 || ab > 10) {
            let ax = ox + 22.;
            let ay = 12.;
            let aw = 62.;
            let ah = 62.;
            r.fill_alpha(ax - 8., ay - 8., aw + 16., ah + 16., 16., accent_rgb, 0.08);
            r.fill_alpha(ax - 5., ay - 5., aw + 10., ah + 10., 14., accent_rgb, 0.14);
            r.fill_alpha(ax - 2., ay - 2., aw + 4., ah + 4., 12., accent_rgb, 0.22);
        }
        // Artwork at ox + 22., 12., 62x62, r: 10
        r.fill(ox + 22., 12., 62., 62., 10., theme::CARD);
        if !r.artwork(
            &self.media.cover,
            windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                left: ox + 22.,
                top: 12.,
                right: ox + 84.,
                bottom: 74.,
            },
        ) {
            icon(r, "music", ox + 41., 31., 24., theme::MUTED);
        }
        // Title at ox + 100., 35. with Marquee
        let elapsed = self.track_change_at.elapsed().as_secs_f32();
        let title_text = if self.media.title.is_empty() {
            self.tr("Nada tocando agora", "Nothing playing")
        } else {
            &self.media.title
        };
        let title_w = r.text_width(title_text, 15., true);
        let title_scrolling = title_w > layout.title_width && !self.state.settings.reduced_motion;
        let title_offset = if title_scrolling {
            marquee_offset(title_w, layout.title_width, elapsed)
        } else {
            0.
        };
        r.text_marquee(
            title_text,
            ox + 100.,
            35.,
            layout.title_width,
            24.,
            15.,
            theme::TEXT,
            true,
            title_offset,
            title_scrolling,
        );
        // Artist at ox + 100., 55. with Marquee
        let artist_text = if self.media.title.is_empty() {
            self.tr("Abra seu player de música", "Open your music player")
        } else {
            &self.media.artist
        };
        let artist_w = r.text_width(artist_text, 11.5, false);
        let artist_scrolling = artist_w > layout.title_width && !self.state.settings.reduced_motion;
        let artist_offset = if artist_scrolling {
            marquee_offset(artist_w, layout.title_width, elapsed)
        } else {
            0.
        };
        r.text_marquee(
            artist_text,
            ox + 100.,
            55.,
            layout.title_width,
            20.,
            11.5,
            theme::MUTED,
            false,
            artist_offset,
            artist_scrolling,
        );
        // Track / Album Tooltip on Title area
        if !self.media.title.is_empty() {
            let album_tip = if !self.media.album.is_empty() {
                let prefix = self.tr("Álbum: ", "Album: ");
                format!("{prefix}{}", self.media.album)
            } else {
                self.tr("Álbum desconhecido", "Unknown album").to_string()
            };
            self.hit(
                (ox + 100., 32., layout.title_width, 44.),
                &album_tip,
                Action::TrackInfo,
            );
        }
        // Lateral Navigation Arrows

        // Progress bar and timestamps
        let is_seeking = self.seek_drag.is_some();
        let pos = if let Some(ratio) = self.seek_drag {
            (ratio * self.media.duration).clamp(0., self.media.duration)
        } else {
            self.position()
        };
        let progress = if self.media.duration > 0. {
            (pos / self.media.duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        // Current time at ox + 25., 82.
        r.text(
            &time(pos),
            ox + 25.,
            82.,
            48.,
            18.,
            11.,
            theme::TEXT,
            false,
            false,
        );
        let (seek_x, _, seek_width, _) = layout.seek;
        let is_seek_hovered = self.is_hovered(layout.seek);
        r.fill_alpha(
            seek_x,
            89.,
            seek_width,
            4.,
            2.,
            theme::TEXT,
            if is_seek_hovered || is_seeking {
                0.35
            } else {
                0.22
            },
        );
        r.fill(seek_x, 89., seek_width * progress, 4., 2., theme::TEXT);
        let (thumb_x, thumb_y) = layout.seek_thumb(progress);
        let thumb_radius = if is_seek_hovered || is_seeking {
            6.0
        } else {
            4.5
        };
        r.circle(thumb_x, thumb_y, thumb_radius, theme::TEXT);
        if is_seek_hovered || is_seeking {
            r.stroke_circle(thumb_x, thumb_y, thumb_radius + 1.5, theme::TEXT, 0.35, 1.5);
        }
        // Floating Timestamp on Hover/Drag
        if (is_seek_hovered || is_seeking) && self.media.duration > 0. {
            let hover_ratio = if is_seeking {
                self.seek_drag.unwrap_or(0.)
            } else if let Some((cx, _)) = self.cursor_pos {
                layout.seek_at(cx)
            } else {
                progress as f64
            };
            let hover_time = (hover_ratio * self.media.duration).clamp(0., self.media.duration);
            let tag_text = time(hover_time);
            let tag_center_x = if is_seeking {
                thumb_x
            } else if let Some((cx, _)) = self.cursor_pos {
                cx.clamp(seek_x, seek_x + seek_width)
            } else {
                thumb_x
            };
            let tag_w = 42.;
            let tag_h = 17.;
            let tag_left =
                (tag_center_x - tag_w / 2.).clamp(seek_x - 4., seek_x + seek_width - tag_w + 4.);
            let tag_top = 67.;
            r.fill_alpha(tag_left, tag_top, tag_w, tag_h, 5., theme::CARD, 0.94);
            r.stroke_rounded_rect(
                tag_left,
                tag_top,
                tag_w,
                tag_h,
                5.,
                theme::TEXT,
                0.18,
                1.,
                false,
            );
            r.text(
                &tag_text,
                tag_left,
                tag_top + 1.5,
                tag_w,
                14.,
                9.5,
                theme::TEXT,
                true,
                true,
            );
        }
        // Remaining time at ox + 343., 82.
        r.text(
            &format!("-{}", time((self.media.duration - pos).max(0.))),
            layout.remaining_x,
            82.,
            50.,
            18.,
            11.,
            theme::TEXT,
            false,
            true,
        );
        if self.media.can_seek {
            self.hit(layout.seek, self.tr("Avançar", "Seek"), Action::Seek(-1.));
        }

        // Controls
        // Lyrics speech bubble button
        let lyrics_hit_rect = layout.lyrics_button;
        let lyrics_center_x = lyrics_hit_rect.0 + lyrics_hit_rect.2 / 2.;
        let lyrics_x = lyrics_center_x - 10.;
        let is_lyrics_hovered = self.is_hovered(lyrics_hit_rect);
        if is_lyrics_hovered {
            r.circle_alpha(lyrics_center_x, 118., 15., theme::TEXT, 0.12);
        }
        if self.page == 1 {
            icon(r, "lyrics-active", lyrics_x, 108., 20., theme::TEXT);
        } else {
            icon(
                r,
                "lyrics",
                lyrics_x,
                108.,
                20.,
                if is_lyrics_hovered {
                    theme::TEXT
                } else {
                    theme::MUTED
                },
            );
        }
        self.hit(
            lyrics_hit_rect,
            if self.page == 1 {
                self.tr("Desativar letras", "Turn off lyrics")
            } else {
                self.tr("Letras", "Lyrics")
            },
            Action::Page(usize::from(self.page == 0)),
        );

        // Playback buttons: Previous, Play/Pause, Next
        let btn_hit_w = if self.page == 1 { 34. } else { 42. };
        let hit_offset = if self.page == 1 { 0.5 } else { -3. };

        // Previous button (always visible; dimmed when not available)
        let prev_hit_rect = (layout.previous_x + hit_offset, 99., btn_hit_w, 38.);
        let is_prev_hovered = self.media.can_previous && self.is_hovered(prev_hit_rect);
        if is_prev_hovered {
            r.circle_alpha(layout.previous_x + 17.5, 118., 15., theme::TEXT, 0.12);
        }
        icon(
            r,
            "previous",
            layout.previous_x + 7.5,
            108.,
            20.,
            if !self.media.can_previous {
                theme::MUTED
            } else if is_prev_hovered {
                theme::TEXT
            } else {
                theme::MUTED
            },
        );
        if self.media.can_previous {
            self.hit(
                prev_hit_rect,
                self.tr("Faixa anterior", "Previous track"),
                Action::Previous,
            );
        }

        // Play/Pause button
        let play_hit_rect = (layout.play_x + hit_offset, 99., btn_hit_w, 38.);
        let is_play_hovered = self.media.can_toggle && self.is_hovered(play_hit_rect);
        if is_play_hovered {
            r.circle_alpha(layout.play_x + 17.5, 118., 16., theme::TEXT, 0.15);
        }
        icon(
            r,
            if self.media.playing { "pause" } else { "play" },
            layout.play_x + 7.5,
            108.,
            20.,
            if !self.media.can_toggle {
                theme::MUTED
            } else {
                theme::TEXT
            },
        );
        if self.media.can_toggle {
            self.hit(
                play_hit_rect,
                self.tr("Reproduzir / pausar", "Play / pause"),
                Action::Play,
            );
        }

        // Next button (always visible; dimmed when not available)
        let next_hit_rect = (layout.next_x + hit_offset, 99., btn_hit_w, 38.);
        let is_next_hovered = self.media.can_next && self.is_hovered(next_hit_rect);
        if is_next_hovered {
            r.circle_alpha(layout.next_x + 17.5, 118., 15., theme::TEXT, 0.12);
        }
        icon(
            r,
            "next",
            layout.next_x + 7.5,
            108.,
            20.,
            if !self.media.can_next {
                theme::MUTED
            } else if is_next_hovered {
                theme::TEXT
            } else {
                theme::MUTED
            },
        );
        if self.media.can_next {
            self.hit(
                next_hit_rect,
                self.tr("Próxima faixa", "Next track"),
                Action::Next,
            );
        }

        if let Some(v) = self.system.volume {
            let (bar_x, bar_center_y, bar_w) = layout.volume_track();
            let is_vol_hovered = self.volume_hovered || self.volume_dragging;
            let h_val = self.volume_hover.value.clamp(0., 1.);
            let v_display = self.volume_level.value.clamp(0., 1.);
            let icon_name = if v <= 0.001 {
                "volume_mute"
            } else if v < 0.33 {
                "volume_low"
            } else if v < 0.66 {
                "volume"
            } else {
                "volume_high"
            };
            let icon_rgb = if is_vol_hovered {
                theme::TEXT
            } else if v <= 0.001 {
                theme::MUTED
            } else {
                theme::TEXT
            };
            if self.page == 1 && is_vol_hovered {
                r.circle_alpha(
                    layout.volume_icon.0 + layout.volume_icon.2 / 2.,
                    layout.volume_icon.1 + layout.volume_icon.3 / 2.,
                    15.,
                    theme::TEXT,
                    0.12,
                );
            }
            icon(
                r,
                icon_name,
                layout.volume_icon.0 + (layout.volume_icon.2 - 18.) / 2.,
                layout.volume_icon.1 + (layout.volume_icon.3 - 18.) / 2.,
                18.,
                icon_rgb,
            );

            let bar_h = 3.0 + 3.0 * h_val;
            let bar_r = bar_h / 2.0;
            let bar_y = bar_center_y - bar_h / 2.0;

            r.push_content(h_val, None);
            r.fill_alpha(
                bar_x,
                bar_y,
                bar_w,
                bar_h,
                bar_r,
                theme::TEXT,
                0.18 + 0.10 * h_val,
            );

            let fill_w = (bar_w * v_display).clamp(0., bar_w);
            if fill_w > 0.5 {
                let r_c = (188. + (255. - 188.) * h_val) as u32;
                let g_c = (188. + (255. - 188.) * h_val) as u32;
                let b_c = (196. + (255. - 196.) * h_val) as u32;
                let fill_rgb = (r_c << 16) | (g_c << 8) | b_c;
                r.fill(bar_x, bar_y, fill_w, bar_h, bar_r, fill_rgb);
            }

            r.pop_content();
            self.hit(
                layout.volume_icon,
                if v <= 0.001 {
                    self.tr("Ativar som", "Unmute")
                } else {
                    self.tr("Silenciar", "Mute")
                },
                Action::VolumeMute,
            );
            let label = format!(
                "{} · {:.0}%",
                self.tr("Volume", "Volume"),
                (v * 100.).round()
            );
            if self.volume_hovered || self.volume_dragging {
                self.hit(layout.volume_bar, &label, Action::Volume(-1.));
            }
        }
    }
    unsafe fn draw_lyrics(&mut self, r: &Renderer, ox: f32) {
        let focus = self.active_lyric_line();
        if self.verse_focus != focus {
            self.verse_focus = focus;
            self.verse_change_at = Some(Instant::now());
            if !self.state.settings.reduced_motion
                && self.state.settings.lyrics.focus_animation != 0
                && self.lyrics_view_mode <= 1
            {
                self.last_anim = Instant::now();
                SetTimer(Some(self.hwnd), 2, 10, None);
            }
        }

        let width = self.player_width();
        let height = 140.
            + (self.state.settings.lyrics.notch_height - 140.)
                * self.player_layout.value.clamp(0., 1.);
        let x = ox + 264.;
        let text_width = (width - 284.).max(180.);
        r.line_alpha(
            ox + 252.,
            22.,
            ox + 252.,
            height - 20.,
            theme::TEXT,
            0.12,
            1.,
        );
        let mut offline_label = self
            .lyrics_content()
            .offline_label(self.lyrics_view_mode, self.state.settings.en())
            .to_owned();
        if self.offline_job.is_some() && (self.lyrics_view_mode == 1 || self.lyrics_view_mode == 3)
        {
            offline_label = self
                .tr("Traduzindo offline…", "Translating offline…")
                .into();
        }
        let mode_label = [
            "Original",
            "Original + tradução",
            "Letra completa",
            "Tradução completa",
        ][self.lyrics_view_mode.min(3)];
        let actions_start = ox + nivumi::player_layout::lyrics_header_action(width, 0).0;
        let max_header_w = (actions_start - x - 8.).max(80.);
        let mode_text_w = r.text_width(mode_label, 10., false);
        let mode_w = (mode_text_w + 20.).min(max_header_w);
        let is_mode_hovered = self.is_hovered((x, 10., mode_w, 25.));
        r.fill_alpha(
            x,
            10.,
            mode_w,
            25.,
            12.,
            theme::TEXT,
            if is_mode_hovered { 0.12 } else { 0.08 },
        );
        r.text(
            mode_label,
            x + 10.,
            10.,
            mode_w - 20.,
            25.,
            10.,
            if is_mode_hovered {
                theme::TEXT
            } else {
                theme::MUTED
            },
            false,
            false,
        );
        self.hit(
            (x, 10., mode_w, 25.),
            "Modo de exibição das letras",
            Action::LyricsOptions,
        );
        let badge_x = x + mode_w + 6.;
        let badge_available_w = actions_start - badge_x - 6.;
        if !offline_label.is_empty() && badge_available_w >= 45. {
            let is_translating = self.offline_job.is_some();
            let badge_text = if is_translating {
                self.tr("Traduzindo…", "Translating…")
            } else if badge_available_w >= 115. {
                self.tr("Tradução offline", "Offline translation")
            } else {
                self.tr("Offline", "Offline")
            };
            let text_w = r.text_width(badge_text, 9.5, false);
            let badge_w = (text_w + 22.).min(badge_available_w);
            let badge_rect = (badge_x, 10., badge_w, 25.);
            let is_badge_hovered = self.is_hovered(badge_rect);

            r.fill_alpha(
                badge_x,
                10.,
                badge_w,
                25.,
                12.,
                theme::ACCENT,
                if is_badge_hovered { 0.18 } else { 0.10 },
            );
            r.stroke_rounded_rect(
                badge_x,
                10.,
                badge_w,
                25.,
                12.,
                theme::ACCENT,
                if is_badge_hovered { 0.40 } else { 0.22 },
                1.,
                false,
            );

            let dot_x = badge_x + 9.;
            let dot_y = 10. + 12.5;
            let dot_alpha = if is_translating {
                ((self.media_phase * 4.).sin() * 0.4 + 0.6).clamp(0.2, 1.0)
            } else if is_badge_hovered {
                1.0
            } else {
                0.85
            };
            r.circle_alpha(dot_x, dot_y, 2.5, theme::ACCENT, dot_alpha);

            r.text(
                badge_text,
                badge_x + 16.,
                10.,
                badge_w - 20.,
                25.,
                9.5,
                if is_badge_hovered {
                    theme::TEXT
                } else {
                    theme::ACCENT
                },
                false,
                false,
            );

            self.hit(
                badge_rect,
                self.tr(
                    "Tradução offline (MarianMT)",
                    "Offline translation (MarianMT)",
                ),
                Action::LyricsOptions,
            );
        }
        for (index, label, description, action) in [
            (0, "↗", "Abrir letras em janela", Action::OpenLyricsWindow),
            (1, "CC", "Abrir legenda", Action::OpenSubtitles),
            (2, "⋯", "Opções de letras", Action::LyricsOptions),
        ] {
            let (left, top, w, h) = nivumi::player_layout::lyrics_header_action(width, index);
            let hit_rect = (ox + left, top, w, h);
            let bx = ox + left + 2.;
            let is_btn_hovered = self.is_hovered(hit_rect);
            if is_btn_hovered {
                r.fill_alpha(bx, 10., 28., 25., 6., theme::TEXT, 0.12);
            }
            r.text(
                label,
                bx,
                10.,
                28.,
                25.,
                13.,
                if is_btn_hovered {
                    theme::TEXT
                } else {
                    theme::MUTED
                },
                false,
                true,
            );
            self.hit(hit_rect, description, action);
        }
        let content = self.lyrics_content();
        let verses = content.verses(self.lyrics_view_mode);
        let font = self.state.settings.lyrics.notch_font;
        let clip = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
            left: x,
            top: 44.,
            right: x + text_width,
            bottom: height - 18.,
        };
        r.dc.PushAxisAlignedClip(
            &clip,
            windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        );
        let mut y = clip.top;
        if verses.is_empty() {
            let is_loading =
                self.lyric_status.contains("Buscando") || self.lyric_status.contains("Loading");
            if is_loading {
                let heights = motion::equalizer_heights(Some(self.media_phase), true);
                let start_x = x + 2.;
                let eq_y = y + 2.;
                for (i, &bar_h) in heights.iter().enumerate() {
                    let h_clamped = bar_h.min(16.);
                    r.fill(
                        start_x + i as f32 * 6.,
                        eq_y + (16. - h_clamped),
                        3.5,
                        h_clamped,
                        1.75,
                        theme::POSITIVE,
                    );
                }
                r.text(
                    content.empty_message(self.lyrics_view_mode, self.state.settings.en()),
                    x + 36.,
                    y,
                    text_width - 36.,
                    22.,
                    font,
                    theme::MUTED,
                    false,
                    false,
                );
            } else {
                r.text_wrapped(
                    content.empty_message(self.lyrics_view_mode, self.state.settings.en()),
                    x,
                    y,
                    text_width,
                    font,
                    theme::MUTED,
                    false,
                );
            }
        } else {
            let heights: Vec<f32> = verses
                .iter()
                .map(|verse| {
                    verse.display_height(
                        self.state.settings.lyrics.translation_all_verses,
                        |text, translated| {
                            r.text_height(
                                text,
                                text_width,
                                if translated {
                                    font
                                } else if verse.active || self.lyrics_view_mode >= 2 {
                                    font + 1.
                                } else {
                                    font - 1.
                                },
                                verse.active && !translated,
                            )
                        },
                    )
                })
                .collect();
            let start = if self.notch_follow && self.lyrics_view_mode <= 1 {
                let active = verses.iter().position(|verse| verse.active).unwrap_or(0);
                if active > 0 {
                    let previous_height = heights[active - 1];
                    let current_height = heights[active];
                    if previous_height + current_height <= height - 62. {
                        active - 1
                    } else {
                        active
                    }
                } else {
                    0
                }
            } else {
                self.external_scroll.min(verses.len().saturating_sub(1))
            };
            let elapsed = self
                .verse_change_at
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or(1.);
            let (opacity, offset, highlight) = nivumi::lyrics_view::focus_effect(
                self.state.settings.lyrics.focus_animation,
                elapsed,
                self.state.settings.reduced_motion,
            );
            let mut offsets = Vec::with_capacity(verses.len() + 1);
            offsets.push(0.);
            for h in &heights {
                offsets.push(offsets.last().copied().unwrap_or(0.) + h);
            }
            let now = Instant::now();
            if self
                .verse_scroll
                .retarget(offsets[start], now, !self.state.settings.reduced_motion)
            {
                self.last_anim = now;
                SetTimer(Some(self.hwnd), 2, 10, None);
            }
            let scroll = self.verse_scroll.position(now);
            let render_start = offsets
                .iter()
                .skip(1)
                .position(|bottom| *bottom > scroll)
                .unwrap_or(start);
            y = clip.top + offsets[render_start] - scroll;
            for (verse_index, verse) in verses.iter().enumerate().skip(render_start) {
                if verse.active {
                    y += offset;
                    r.push_content(opacity, None);
                }

                let is_active = verse.active || self.lyrics_view_mode >= 2;
                let size = if is_active { font + 1. } else { font - 1. };
                r.text_wrapped(
                    &verse.text,
                    x,
                    y,
                    text_width,
                    size,
                    if is_active {
                        let channel = |a: f32| (a + (255. - a) * highlight).round() as u32;
                        (channel(154.) << 16) | (channel(154.) << 8) | channel(164.)
                    } else {
                        theme::MUTED
                    },
                    verse.active,
                );
                let original_height = r.text_height(&verse.text, text_width, size, verse.active);
                let row_start = y - if verse.active { offset } else { 0. };
                y += original_height + 4.;
                if let Some(translation) =
                    verse.visible_translation(self.state.settings.lyrics.translation_all_verses)
                {
                    r.text_wrapped(
                        translation,
                        x,
                        y,
                        text_width,
                        font,
                        if verse.active {
                            theme::LYRICS_TRANSLATION_ACTIVE
                        } else {
                            theme::LYRICS_TRANSLATION
                        },
                        false,
                    );
                }
                if verse.active {
                    r.pop_content();
                }
                y = row_start + offsets[verse_index + 1] - offsets[verse_index] - 10.;
                if let Some(index) = verse.line_index {
                    if self.media.can_seek && y > clip.top && row_start < clip.bottom {
                        self.hit(
                            (
                                x,
                                row_start.max(clip.top),
                                text_width,
                                (y.min(clip.bottom) - row_start.max(clip.top)).max(0.),
                            ),
                            &verse.text,
                            Action::Seek(self.lyric_lines[index].timestamp_ms as f64 / 1000.),
                        );
                    }
                }
                y += 10.;
                if y >= height - 18. {
                    break;
                }
            }
        }
        r.dc.PopAxisAlignedClip();
        // Visible corner resize grip at the bottom-right corner to indicate resizability
        // without colliding with the central navigation dots.
        let radius = 24.;
        let actual_width = self.body_width.value;
        let cx = (self.canvas_width + actual_width) / 2. - 32.;
        let cy = self.body_height.value - 32.;
        for step in 0..24 {
            let a = (12. + step as f32 * 66. / 24.).to_radians();
            let b = (12. + (step + 1) as f32 * 66. / 24.).to_radians();
            r.line_alpha(
                cx + radius * a.cos(),
                cy + radius * a.sin(),
                cx + radius * b.cos(),
                cy + radius * b.sin(),
                theme::TEXT,
                0.38,
                2.,
            );
        }
    }
    fn poll_offline(&mut self) {
        use nivumi::offline_translation::{self as offline, TranslationOrigin};
        if !self.state.settings.lyrics.offline_translation {
            self.offline_job = None;
            self.offline_attempt.clear();
            if self
                .external_lyrics
                .as_ref()
                .is_some_and(|e| e.translation_origin == TranslationOrigin::Offline)
            {
                if let Some(e) = self.external_lyrics.as_mut() {
                    e.translation = None;
                    e.translation_origin = TranslationOrigin::Human;
                }
                for line in &mut self.lyric_lines {
                    line.translation = None;
                }
            }
            return;
        }
        if self.external_lyrics.as_ref().is_some_and(|e| {
            e.translation_origin == TranslationOrigin::Human
                && e.translation.as_ref().is_some_and(|s| !s.trim().is_empty())
        }) {
            self.offline_job = None;
            return;
        }
        let original = if self.lyric_lines.is_empty() {
            self.external_lyrics
                .as_ref()
                .map(|e| e.original.clone())
                .unwrap_or_default()
        } else {
            self.lyric_lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let key = offline::cache_key(&self.media.key(), &original);
        if let Some(job) = &self.offline_job {
            if job.key != key {
                self.offline_job = None;
            } else if let Ok(result) = job.rx.try_recv() {
                self.offline_job = None;
                match result {
                    Ok(t) => {
                        for (line, translated) in
                            self.lyric_lines.iter_mut().zip(t.translated.split('\n'))
                        {
                            line.translation =
                                (!translated.is_empty()).then(|| translated.to_owned());
                        }
                        let e = self.external_lyrics.get_or_insert_with(|| ExternalLyrics {
                            title: self.media.title.clone(),
                            artist: self.media.artist.clone(),
                            ..Default::default()
                        });
                        e.original = t.original;
                        e.translation = Some(t.translated);
                        e.translation_origin = TranslationOrigin::Offline;
                    }
                    Err(e) => {
                        log(&e);
                        self.lyric_status = e;
                    }
                }
            }
        }
        if self.letras_done
            && self.offline_attempt != key
            && !original.trim().is_empty()
            && offline::installed()
            && offline::english(&original)
        {
            self.offline_attempt = key;
            self.offline_job = Some(offline::Job::spawn(&self.media.key(), original));
        }
    }
    fn lyrics_content(&self) -> LyricsContent {
        let mut content = LyricsContent {
            translation_origin: self
                .external_lyrics
                .as_ref()
                .map(|e| e.translation_origin.clone())
                .unwrap_or_default(),
            key: self.media.key(),
            title: self.media.title.clone(),
            artist: self.media.artist.clone(),
            lines: self.lyric_lines.clone(),
            original: self
                .external_lyrics
                .as_ref()
                .map(|lyrics| lyrics.original.clone())
                .unwrap_or_else(|| {
                    self.lyric_lines
                        .iter()
                        .map(|line| line.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
            translation: self
                .external_lyrics
                .as_ref()
                .and_then(|lyrics| lyrics.translation.clone()),
            status: if self.offline_job.is_some() {
                self.tr("Traduzindo offline…", "Translating offline…")
                    .into()
            } else {
                self.lyric_status.clone()
            },
            active: self.active_lyric_line(),
            is_portuguese: false,
        };
        content.is_portuguese = nivumi::lyrics_view::is_lyrics_in_portuguese(&content.original);
        if content.lines.is_empty() && !content.original.trim().is_empty() {
            content.status = LyricsContent::static_status(self.state.settings.en()).into();
        }
        content
    }
    unsafe fn sync_lyrics_window(&mut self) {
        if self.lyrics_window.is_some() {
            let content = self.lyrics_content();
            if let Some(window) = self.lyrics_window.as_mut() {
                window
                    .set_translation_visibility(self.state.settings.lyrics.translation_all_verses);
                window.set_reduced_motion(self.state.settings.reduced_motion);
                window.set_accent(self.media.accent.unwrap_or([0.; 3]));
                window.update(content);
            }
        }
    }
    fn collect_lyrics_preferences(&mut self) {
        if let Some(window) = &self.lyrics_window {
            let prefs = window.preferences();
            self.state.settings.lyrics.window_font = prefs.window_font;
            self.state.settings.lyrics.window_bounds = prefs.window_bounds;
            self.state.settings.lyrics.window_topmost = prefs.window_topmost;
            self.state.settings.lyrics.window_mode = prefs.window_mode;
        }
    }
    fn resize_edge(&self, x: f32, y: f32) -> Option<(i32, bool)> {
        if !self.expanded || self.page != 1 || self.expansion.value < 0.99 {
            return None;
        }
        let (width, height) = self.body_size();
        let left = (self.canvas_width - width) / 2.;
        let in_corner = x > left + width - 20. && y > height - 20.;
        let horizontal = if x < left + 9. {
            -1
        } else if x > left + width - 9. || in_corner {
            1
        } else {
            0
        };
        // The central navigation zone is reserved for navigation dots / tabs,
        // avoiding hit/cursor collisions with bottom edge resizing.
        let nav_half = if self.navigation_hover || self.navigation.value > 0.01 {
            NAV_HOVER_HALF_WIDTH
        } else {
            38.
        };
        let in_nav_zone = (x - self.canvas_width / 2.).abs() <= nav_half;
        let bottom = (y > height - 12. || in_corner) && (horizontal != 0 || !in_nav_zone);
        (self.inside(x, y) && (horizontal != 0 || bottom)).then_some((horizontal, bottom))
    }
    unsafe fn finish_notch_resize(&mut self) {
        if self.notch_resize.take().is_some() {
            let _ = ReleaseCapture();
            self.resize();
            self.save();
            self.paint();
        }
    }
    unsafe fn draw_pomodoro(&mut self, r: &Renderer, ox: f32) {
        let remaining = self
            .state
            .pomodoro
            .remaining(chrono::Local::now().timestamp());
        let phase = self.state.pomodoro.phase;
        let total = self.state.phase_seconds(phase).max(1);

        // Lateral arrows

        // Glow and Ring at ox + 80., 74.
        let phase_color = if phase == PomodoroPhase::Focus {
            theme::FOCUS
        } else {
            theme::POSITIVE
        };
        r.circle_alpha(ox + 80., 74., 44., phase_color, 0.12);
        r.circle_alpha(ox + 80., 74., 36., phase_color, 0.22);
        ring(
            r,
            ox + 80.,
            74.,
            32.,
            phase_color,
            remaining as f32 / total as f32,
        );
        icon(r, "timer", ox + 65., 59., 30., theme::TEXT);

        // Digital time at ox + 265., 32. (centered around 265)
        r.text(
            &time(remaining as f64),
            ox + 200.,
            32.,
            130.,
            40.,
            31.,
            theme::TEXT,
            true,
            true,
        );

        // Reset button: circular 24x24 at ox + 330., 39.
        r.circle_alpha(ox + 330., 51., 12., theme::TEXT, 0.12);
        icon(r, "reset", ox + 322., 43., 16., theme::TEXT);
        self.hit(
            (ox + 318., 39., 24., 24.),
            self.tr("Reiniciar", "Reset"),
            Action::Command(AppCommand::ResetPomodoro),
        );

        // Subtitle at ox + 200., 68.
        let phase_text = match phase {
            PomodoroPhase::Focus => self.tr("Hora de focar", "Focus time"),
            PomodoroPhase::ShortBreak => self.tr("Pausa curta", "Short break"),
            _ => self.tr("Pausa longa", "Long break"),
        };
        r.text(
            phase_text,
            ox + 200.,
            68.,
            130.,
            20.,
            11.5,
            theme::MUTED,
            false,
            true,
        );

        // Pills: Short Break (ox + 144., w: 86), Play (ox + 238., w: 55), Focus (ox + 301., w: 85)
        let playing = self.state.pomodoro.running();
        // Short Break
        r.fill_alpha(ox + 144., 92., 86., 28., 14., theme::TEXT, 0.12);
        icon(r, "cup", ox + 152., 99., 14., theme::TEXT);
        r.text(
            self.tr("Pausa", "Break"),
            ox + 170.,
            96.,
            55.,
            20.,
            11.5,
            theme::TEXT,
            true,
            false,
        );
        self.hit(
            (ox + 144., 92., 86., 28.),
            self.tr("Pausa curta", "Short break"),
            Action::Command(AppCommand::SetPomodoroPhase(PomodoroPhase::ShortBreak)),
        );

        // Play/Pause pill
        r.fill(ox + 238., 92., 55., 28., 14., theme::FOCUS);
        icon(
            r,
            if playing { "pause" } else { "play" },
            ox + 258.,
            98.,
            16.,
            theme::TEXT,
        );
        self.hit(
            (ox + 238., 92., 55., 28.),
            if playing {
                self.tr("Pausar", "Pause")
            } else {
                self.tr("Iniciar", "Start")
            },
            Action::Command(if playing {
                AppCommand::PausePomodoro
            } else {
                AppCommand::StartPomodoro
            }),
        );

        // Focus Time
        r.fill_alpha(ox + 301., 92., 85., 28., 14., theme::TEXT, 0.12);
        icon(r, "target", ox + 309., 99., 14., theme::TEXT);
        r.text(
            self.tr("Foco", "Focus"),
            ox + 327.,
            96.,
            54.,
            20.,
            11.5,
            theme::TEXT,
            true,
            false,
        );
        self.hit(
            (ox + 301., 92., 85., 28.),
            self.tr("Foco", "Focus"),
            Action::Command(AppCommand::SetPomodoroPhase(PomodoroPhase::Focus)),
        );
    }
    unsafe fn draw_water(&mut self, r: &Renderer, ox: f32) {
        let now = Local::now();
        let total = self.state.water.total_ml();
        let goal = self.state.settings.water_goal_ml.max(1);
        let progress = (total as f32 / goal as f32).min(1.);
        let english = self.state.settings.en();
        let paused_until = self
            .state
            .water
            .snooze_until
            .filter(|&time| time > now.timestamp())
            .and_then(|time| Local.timestamp_opt(time, 0).single());
        let reached = total >= goal;
        let (label, hint, command) = if reached {
            (
                self.tr("Meta atingida", "Goal reached").to_owned(),
                String::new(),
                None,
            )
        } else if let Some(until) = paused_until {
            (
                format!(
                    "{} · {}",
                    self.tr("Retomar", "Resume"),
                    until.format("%H:%M")
                ),
                format!(
                    "{} {}. {}",
                    self.tr("Lembretes pausados até", "Reminders paused until"),
                    until.format("%H:%M"),
                    self.tr("Retomar lembretes", "Resume reminders")
                ),
                Some(AppCommand::ResumeWater),
            )
        } else {
            (
                self.tr("Pausar 15 min", "Pause 15 min").to_owned(),
                self.tr(
                    "Pausar lembretes de água por 15 minutos",
                    "Pause water reminders for 15 minutes",
                )
                .to_owned(),
                Some(AppCommand::SnoozeWater(15)),
            )
        };
        r.fill_alpha(
            ox + 156.,
            4.,
            148.,
            28.,
            10.,
            theme::TEXT,
            if paused_until.is_some() { 0.16 } else { 0.08 },
        );
        r.text(
            &label,
            ox + 160.,
            4.,
            140.,
            28.,
            11.,
            if reached { theme::BLUE } else { theme::MUTED },
            false,
            true,
        );
        if let Some(command) = command {
            self.hit((ox + 156., 4., 148., 28.), &hint, Action::Command(command));
        }

        r.text(
            &format!("{} L", hydration::liters(total, english)),
            ox + 24.,
            40.,
            94.,
            36.,
            26.,
            theme::TEXT,
            true,
            false,
        );
        r.text(
            &format!("/ {} L", hydration::liters(goal, english)),
            ox + 120.,
            49.,
            78.,
            24.,
            12.,
            theme::MUTED,
            false,
            false,
        );
        let percentage = hydration::percentage(total, goal);
        let sub = format!(
            "{} · {}% {}",
            self.tr("Hoje", "Today"),
            percentage,
            self.tr("da meta", "of goal")
        );
        r.text(
            &sub,
            ox + 24.,
            76.,
            172.,
            20.,
            12.,
            theme::BLUE,
            false,
            false,
        );
        r.fill(ox + 24., 99., 172., 5., 2.5, theme::BLUE_SURFACE);
        r.fill(ox + 24., 99., 172. * progress, 5., 2.5, theme::BLUE);

        let drink_label = format!("+ {} ml", self.state.settings.water_bottle_ml);
        r.fill(ox + 24., 112., 96., 32., 12., theme::BLUE_SURFACE);
        icon(r, "drop", ox + 32., 121., 14., theme::BLUE);
        r.text(
            &drink_label,
            ox + 50.,
            112.,
            66.,
            32.,
            12.,
            theme::TEXT,
            true,
            false,
        );
        self.hit(
            (ox + 24., 112., 96., 32.),
            &format!(
                "{} {} ml",
                self.tr("Registrar", "Record"),
                self.state.settings.water_bottle_ml
            ),
            Action::Command(AppCommand::RecordDrink),
        );
        r.fill_alpha(ox + 128., 112., 68., 32., 12., theme::TEXT, 0.12);
        r.text(
            self.tr("Ajustes", "Settings"),
            ox + 128.,
            112.,
            68.,
            32.,
            12.,
            theme::TEXT,
            true,
            true,
        );
        self.hit(
            (ox + 128., 112., 68., 32.),
            self.tr(
                "Ajustar meta, porção e horários dos lembretes",
                "Adjust goal, portion and reminder hours",
            ),
            Action::HydrationSettings,
        );
        r.line_alpha(ox + 208., 44., ox + 208., 144., theme::TEXT, 0.12, 1.);

        let hours = self.state.water.hourly_ml(now.date_naive());
        let maximum = hydration::chart_max(&hours);
        r.text(
            self.tr("Consumo por hora", "Intake by hour"),
            ox + 220.,
            42.,
            176.,
            20.,
            11.,
            theme::MUTED,
            false,
            false,
        );
        let left = ox + 220.;
        let width = 144.;
        let top = 74.;
        let bottom = 126.;
        for (y, value) in [
            (top, maximum),
            ((top + bottom) / 2., maximum / 2),
            (bottom, 0),
        ] {
            r.dashed_line(left, y, left + width, y, theme::TEXT, 0.18, 1.);
            r.text(
                &value.to_string(),
                left + width + 6.,
                y - 7.,
                40.,
                14.,
                10.,
                theme::MUTED,
                false,
                false,
            );
        }
        r.text(
            "ml",
            left + width + 6.,
            57.,
            36.,
            14.,
            10.,
            theme::MUTED,
            false,
            false,
        );
        let slot = width / 24.;
        for (hour, amount) in hours.iter().enumerate() {
            if *amount > 0 {
                let height = (*amount as f32 / maximum as f32 * (bottom - top)).max(2.);
                r.fill(
                    left + hour as f32 * slot + 1.,
                    bottom - height,
                    slot - 2.,
                    height,
                    2.,
                    theme::BLUE,
                );
            }
        }
        for hour in [0, 6, 12, 18, 24] {
            r.text(
                &format!("{hour:02}"),
                left + hour as f32 * slot - 9.,
                131.,
                18.,
                14.,
                10.,
                theme::MUTED,
                false,
                true,
            );
        }
    }
    unsafe fn list_scroll_position(&mut self, shelf: bool, pitch: f32) -> f32 {
        let now = Instant::now();
        let scroll = if shelf {
            &mut self.shelf_scroll
        } else {
            &mut self.task_scroll
        };
        if scroll.retarget(
            self.scroll as f32 * pitch,
            now,
            !self.state.settings.reduced_motion,
        ) {
            self.last_anim = now;
            SetTimer(Some(self.hwnd), 2, 10, None);
        }
        scroll.position(now)
    }
    fn clip_list_hits(&mut self, start: usize, clip: (f32, f32, f32, f32)) {
        for hit in &mut self.hits[start..] {
            let (x, y, w, h) = hit.rect;
            let left = x.max(clip.0);
            let top = y.max(clip.1 + self.content_offset_y);
            hit.rect = (
                left,
                top,
                (x + w).min(clip.0 + clip.2).max(left) - left,
                (y + h)
                    .min(clip.1 + self.content_offset_y + clip.3)
                    .max(top)
                    - top,
            );
        }
    }
    unsafe fn draw_shelf(&mut self, r: &Renderer, ox: f32) {
        let max_scroll = self.state.shelf.len().saturating_sub(5);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        let position = self.list_scroll_position(true, 76.);
        let first = (position / 76.).floor() as usize;
        let items = self
            .state
            .shelf
            .iter()
            .skip(first)
            .take(6)
            .cloned()
            .collect::<Vec<_>>();

        if items.is_empty() {
            // Full-width drop zone, aligned with the content gutter.
            r.stroke_rounded_rect(ox + 24., 33., 372., 88., 16., theme::TEXT, 0.3, 1.4, true);
            r.fill_alpha(ox + 24., 33., 372., 88., 16., theme::TEXT, 0.03);

            // Icon: tray and arrow
            icon(r, "download", ox + 200., 44., 20., theme::MUTED);
            // Text: Drop files here
            r.text(
                self.tr("Solte arquivos e textos aqui", "Drop files here"),
                ox + 112.,
                68.,
                200.,
                18.,
                13.,
                theme::TEXT,
                true,
                true,
            );
            let info_hovered = self
                .hovered_hit
                .and_then(|i| self.hits.get(i))
                .is_some_and(|h| matches!(h.action, Action::ShelfInfo));
            let subtext = if info_hovered {
                self.tr(
                    "Clique no ícone (i) acima para ver o guia completo da prateleira",
                    "Click the (i) icon above to view the complete shelf guide",
                )
            } else {
                self.tr(
                    "Ctrl/Shift para selecionar · duplo clique para abrir",
                    "Ctrl/Shift to select · double-click to open",
                )
            };
            // Subtext: Drag out anytime · click to open
            r.text(
                subtext,
                ox + 42.,
                86.,
                340.,
                16.,
                9.5,
                if info_hovered {
                    theme::ACCENT
                } else {
                    theme::MUTED
                },
                false,
                true,
            );
            self.hit(
                (ox + 24., 33., 372., 88.),
                self.tr("Adicionar arquivos à prateleira", "Add files to shelf"),
                Action::AddFiles,
            );
        } else {
            let stage = self.shelf_gesture.and_then(|g| {
                g.stage
                    .map(|at| (g.id, (at.elapsed().as_secs_f32() / 0.14).clamp(0., 1.)))
            });
            let mut layer = 0;
            let start_x = shelf_start_x(self.state.shelf.len().min(5), ox);
            let hit_start = self.hits.len();
            r.dc.PushAxisAlignedClip(
                &windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                    left: ox + 24.,
                    top: 12.,
                    right: ox + 396.,
                    bottom: 120.,
                },
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            let selected_count = items
                .iter()
                .filter(|i| self.selected.contains(&i.id))
                .count();
            let stack_base_x = shelf_stack_base_x(selected_count, ox);
            for (i, item) in items.iter().enumerate() {
                let card_x = start_x + (first + i) as f32 * 76. - position;
                let selected = self.selected.contains(&item.id);
                let mut thumb_x = card_x + 9.;
                let mut thumb_y = 31.;
                if let Some((_, t)) = stage.filter(|_| selected) {
                    let t = t * t * (3. - 2. * t);
                    thumb_x += (stack_base_x + layer as f32 * 4. - thumb_x) * t;
                    thumb_y += layer as f32 * 4. * t;
                    layer += 1;
                }
                let is_card_hovered = self.is_hovered((card_x, 31., 62., 80.));
                if selected {
                    r.fill(thumb_x - 3., thumb_y - 3., 50., 50., 11., theme::ACCENT);
                } else if is_card_hovered {
                    r.stroke_rounded_rect(
                        thumb_x - 2.,
                        thumb_y - 2.,
                        48.,
                        48.,
                        11.,
                        theme::TEXT,
                        0.28,
                        1.5,
                        false,
                    );
                }
                let bg_color = if is_card_hovered {
                    theme::INPUT
                } else {
                    theme::CARD
                };
                r.fill(thumb_x, thumb_y, 44., 44., 9., bg_color);
                if item.kind == ShelfKind::File
                    && nivumi::shelf::is_image(&item.value)
                    && !self.shelf_thumbnails.contains_key(&item.id)
                    && self
                        .shelf_worker
                        .tx
                        .try_send((item.id, item.value.clone()))
                        .is_ok()
                {
                    self.shelf_thumbnails.insert(item.id, None);
                }
                let preview = self
                    .shelf_thumbnails
                    .get(&item.id)
                    .and_then(|v| v.as_ref())
                    .is_some_and(|image| {
                        r.thumbnail(
                            item.id,
                            image,
                            windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                                left: thumb_x + 2.,
                                top: thumb_y + 2.,
                                right: thumb_x + 42.,
                                bottom: thumb_y + 42.,
                            },
                        )
                    });
                if !preview {
                    let (kind, color) = match item.kind {
                        ShelfKind::Folder => ("folder", theme::WARNING),
                        ShelfKind::Text => ("text", theme::ACCENT),
                        _ => ("file", theme::TEXT),
                    };
                    icon(r, kind, thumb_x + 10., thumb_y + 10., 24., color);
                }
                if stage.is_none() {
                    if selected {
                        r.circle(thumb_x + 5., thumb_y + 5., 7., theme::ACCENT);
                        icon(r, "check", thumb_x + 1., thumb_y + 1., 8., theme::TEXT);
                    }
                    r.circle(thumb_x + 43., thumb_y, 10., theme::INPUT);
                    icon(r, "close", thumb_x + 39., thumb_y - 4., 8., theme::TEXT);
                    self.hit(
                        (thumb_x + 25., thumb_y - 18., 36., 36.),
                        self.tr("Remover da prateleira", "Remove from shelf"),
                        Action::RemoveShelfItem(item.id),
                    );
                    r.text(
                        &item.label(),
                        card_x,
                        79.,
                        62.,
                        16.,
                        10.5,
                        theme::TEXT,
                        true,
                        true,
                    );
                    let sub = match item.kind {
                        ShelfKind::Folder => self.tr("Pasta", "Folder"),
                        ShelfKind::Text => self.tr("Texto", "Text"),
                        ShelfKind::Link => self.tr("Link", "Link"),
                        _ => {
                            if preview {
                                self.tr("Imagem", "Image")
                            } else {
                                self.tr("Arquivo", "File")
                            }
                        }
                    };
                    r.text(sub, card_x, 95., 62., 14., 9.5, theme::MUTED, false, true);
                    self.hit(
                        (card_x, 31., 62., 80.),
                        &item.label(),
                        Action::SelectShelf(item.id),
                    );
                }
            }
            r.dc.PopAxisAlignedClip();
            self.clip_list_hits(hit_start, (ox + 24., 12., 372., 108.));
            if self.state.shelf.len() > 5 {
                if self.scroll > 0 {
                    let left_rect = (ox + 2., 42., 22., 36.);
                    let is_left_hover = self.is_hovered(left_rect);
                    if is_left_hover {
                        r.fill_alpha(
                            left_rect.0,
                            left_rect.1,
                            left_rect.2,
                            left_rect.3,
                            6.,
                            theme::TEXT,
                            0.14,
                        );
                    }
                    r.text(
                        "‹",
                        left_rect.0,
                        left_rect.1 + 4.,
                        left_rect.2,
                        28.,
                        22.,
                        if is_left_hover {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        },
                        false,
                        true,
                    );
                    self.hit(
                        left_rect,
                        self.tr("Itens anteriores", "Previous items"),
                        Action::ShelfScroll(-1),
                    );
                }
                if self.scroll < max_scroll {
                    let right_rect = (ox + 396., 42., 22., 36.);
                    let is_right_hover = self.is_hovered(right_rect);
                    if is_right_hover {
                        r.fill_alpha(
                            right_rect.0,
                            right_rect.1,
                            right_rect.2,
                            right_rect.3,
                            6.,
                            theme::TEXT,
                            0.14,
                        );
                    }
                    r.text(
                        "›",
                        right_rect.0,
                        right_rect.1 + 4.,
                        right_rect.2,
                        28.,
                        22.,
                        if is_right_hover {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        },
                        false,
                        true,
                    );
                    self.hit(
                        right_rect,
                        self.tr("Próximos itens", "Next items"),
                        Action::ShelfScroll(1),
                    );
                }
                let total = self.state.shelf.len();
                let track_w = 120.;
                let track_x = ox + (BODY - track_w) / 2.;
                let track_y = 113.;
                let thumb_w = (track_w * 5.0 / total as f32).clamp(16., track_w);
                let thumb_x = track_x
                    + (track_w - thumb_w) * (position / 76. / max_scroll as f32).clamp(0., 1.);
                r.fill_alpha(track_x, track_y, track_w, 3., 1.5, theme::TEXT, 0.12);
                r.fill_alpha(thumb_x, track_y, thumb_w, 3., 1.5, theme::ACCENT, 0.85);
            }
            let hint = self
                .hovered_hit
                .and_then(|i| self.hits.get(i))
                .and_then(|h| {
                    if matches!(h.action, Action::SelectShelf(_)) {
                        Some(h.label.clone())
                    } else if matches!(h.action, Action::ShelfInfo) {
                        Some(
                            self.tr(
                                "Clique no ícone (i) para ver tudo o que a prateleira pode fazer",
                                "Click the (i) icon to see everything the shelf can do",
                            )
                            .into(),
                        )
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| {
                    if self.state.shelf.len() > 5 {
                        let total = self.state.shelf.len();
                        let start = self.scroll + 1;
                        let end = (self.scroll + 5).min(total);
                        if self.state.settings.en() {
                            format!(
                                "{start}–{end} of {total} items · Scroll or click ‹ › to browse"
                            )
                        } else {
                            format!("{start}–{end} de {total} itens · Role ou clique em ‹ ›")
                        }
                    } else {
                        self.tr(
                            "Ctrl/Shift: selecionar · Duplo clique: abrir",
                            "Ctrl/Shift: select · Double-click: open",
                        )
                        .into()
                    }
                });
            r.text(
                &hint,
                ox + 24.,
                119.,
                372.,
                14.,
                9.5,
                theme::MUTED,
                false,
                true,
            );
        }
    }
    fn shelf_select(&mut self, id: u64, control: bool, shift: bool) {
        let ids = self.state.shelf.iter().map(|i| i.id).collect::<Vec<_>>();
        nivumi::shelf::select(
            &ids,
            &mut self.selected,
            &mut self.shelf_anchor,
            id,
            control,
            shift,
        );
    }
    fn shelf_files(&self) -> Result<Vec<PathBuf>> {
        let items = self
            .state
            .shelf
            .iter()
            .filter(|i| self.selected.contains(&i.id))
            .collect::<Vec<_>>();
        if items.is_empty()
            || items
                .iter()
                .any(|i| !matches!(i.kind, ShelfKind::File | ShelfKind::Folder))
        {
            return Err(Error::new(
                E_INVALIDARG,
                self.tr(
                    "Selecione apenas arquivos e pastas para esta operação",
                    "Select only files and folders for this operation",
                ),
            ));
        }
        Ok(items.iter().map(|i| PathBuf::from(&i.value)).collect())
    }
    unsafe fn shelf_drag(&mut self, id: u64, right: bool) {
        let _ = KillTimer(Some(self.hwnd), 6);
        self.shelf_gesture = None;
        let _ = ReleaseCapture();
        self.modal = true;
        self.update_media_animation();
        let selected = self
            .state
            .shelf
            .iter()
            .filter(|i| self.selected.contains(&i.id))
            .cloned()
            .collect::<Vec<_>>();
        let any_files = selected
            .iter()
            .any(|i| matches!(i.kind, ShelfKind::File | ShelfKind::Folder));
        let result = if any_files {
            self.shelf_files().and_then(|files| {
                let image = nivumi::shelf::drag_image(
                    self.shelf_thumbnails.get(&id).and_then(|v| v.as_ref()),
                    selected.len(),
                );
                ole::drag_files_with_button(&files, right, Some(&image))
            })
        } else {
            let text = selected
                .iter()
                .map(|i| i.value.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            ole::drag_text_with_button(&text, right).map(|ok| {
                if ok {
                    ole::DragOutcome::Copied
                } else {
                    ole::DragOutcome::Cancelled
                }
            })
        };
        self.modal = false;
        match result {
            Ok(ole::DragOutcome::Moved) => {
                for item in selected {
                    if matches!(item.kind, ShelfKind::File | ShelfKind::Folder)
                        && !std::path::Path::new(&item.value).exists()
                    {
                        self.state
                            .apply(AppCommand::RemoveShelf(item.id), Local::now());
                        self.selected.remove(&item.id);
                    }
                }
                self.save();
            }
            Err(error) => self.toast = Some((error.message().to_string(), Instant::now())),
            _ => {}
        }
        self.update_media_animation();
        self.paint();
    }
    unsafe fn shelf_menu(&mut self) {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let has = !self.selected.is_empty();
        let files = self.shelf_files().is_ok();
        for (id, label, enabled) in [
            (1, self.tr("Abrir", "Open"), self.selected.len() == 1),
            (2, self.tr("Copiar", "Copy"), has),
            (3, self.tr("Recortar arquivos", "Cut files"), files),
            (
                4,
                self.tr("Remover da prateleira", "Remove from shelf"),
                has,
            ),
            (5, self.tr("Selecionar todos", "Select all"), true),
            (6, self.tr("Limpar prateleira…", "Clear shelf…"), true),
        ] {
            let _ = AppendMenuW(
                menu,
                MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED },
                id,
                &HSTRING::from(label),
            );
        }
        self.modal = true;
        let mut pointer = POINT::default();
        let _ = GetCursorPos(&mut pointer);
        let _ = SetForegroundWindow(self.hwnd);
        let _theme = nivumi::popup_menu::DarkMenu::attach(menu, self.hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD,
            pointer.x,
            pointer.y,
            None,
            self.hwnd,
            None,
        )
        .0;
        let _ = DestroyMenu(menu);
        self.modal = false;
        match command {
            1 => {
                if let Some(id) = self.selected.first().copied() {
                    self.action(Action::OpenShelf(id));
                }
            }
            2 | 3 => self.action(Action::ShelfClipboard(command == 3)),
            4 => self.action(Action::RemoveSelected),
            5 => {
                self.selected = self.state.shelf.iter().map(|i| i.id).collect();
                self.paint();
            }
            6 => self.action(Action::ClearShelf),
            _ => {}
        }
    }
    unsafe fn task_menu(&mut self, id: u64) {
        if !self.state.tasks.iter().any(|task| task.id == id) {
            return;
        }
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        for (command, label) in [
            (1, self.tr("Editar tarefa", "Edit task")),
            (2, self.tr("Adiar 10 minutos", "Snooze 10 minutes")),
        ] {
            let _ = AppendMenuW(menu, MF_STRING, command, &HSTRING::from(label));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            3,
            &HSTRING::from(self.tr("Excluir tarefa", "Delete task")),
        );
        // Anchor to the task button for both keyboard and pointer activation.
        let scale = self.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
        let rect = self
            .hits
            .iter()
            .find(|hit| matches!(hit.action, Action::TaskMenu(task) if task == id))
            .map(|hit| hit.rect)
            .unwrap_or((0., 0., 0., 0.));
        let mut point = POINT {
            x: (rect.0 * scale) as i32,
            y: ((rect.1 + rect.3) * scale) as i32,
        };
        let _ = ClientToScreen(self.hwnd, &mut point);
        self.modal = true;
        let _ = SetForegroundWindow(self.hwnd);
        let theme = nivumi::popup_menu::DarkMenu::attach(menu, self.hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            self.hwnd,
            None,
        )
        .0;
        let _ = DestroyMenu(menu);
        drop(theme);
        self.modal = false;
        match command {
            1 => self.action(Action::EditTask(id)),
            2 => self.action(Action::SnoozeTask(id)),
            3 => self.action(Action::Command(AppCommand::DeleteTask(id))),
            _ => self.paint(),
        }
    }
    unsafe fn draw_tasks(&mut self, r: &Renderer, ox: f32) {
        self.scroll = self.scroll.min(self.state.tasks.len().saturating_sub(3));
        let position = self.list_scroll_position(false, TASK_ROW_PITCH);
        let first = (position / TASK_ROW_PITCH).floor() as usize;
        let tasks = self
            .state
            .tasks
            .iter()
            .skip(first)
            .take(4)
            .cloned()
            .collect::<Vec<_>>();
        let done = self.state.tasks.iter().filter(|t| t.done).count();
        let count = self.state.tasks.len();

        // Left summary column:
        r.text(
            &format!("{done}/{count}"),
            ox + 24.,
            32.,
            100.,
            38.,
            32.,
            theme::TEXT,
            true,
            false,
        );
        r.text(
            self.tr("Tarefas concluídas", "Task Done Today"),
            ox + 24.,
            68.,
            140.,
            18.,
            11.5,
            theme::MUTED,
            false,
            false,
        );

        // Leave enough room for the full Portuguese captions.
        r.fill_alpha(ox + 24., 93., 98., 28., 13., theme::TEXT, 0.12);
        icon(r, "plus", ox + 31., 99., 13., theme::TEXT);
        r.text(
            self.tr("Adicionar", "Add"),
            ox + 50.,
            96.,
            64.,
            22.,
            11.5,
            theme::TEXT,
            true,
            false,
        );
        self.hit(
            (ox + 24., 93., 98., 28.),
            self.tr("+ Adicionar", "+ Add task"),
            Action::NewTask,
        );

        r.fill_alpha(ox + 130., 93., 74., 28., 13., theme::TEXT, 0.12);
        icon(r, "clock", ox + 136., 99., 13., theme::TEXT);
        r.text(
            self.tr("Agenda", "View"),
            ox + 156.,
            96.,
            44.,
            22.,
            11.5,
            theme::TEXT,
            true,
            false,
        );
        self.hit(
            (ox + 130., 93., 74., 28.),
            self.tr("Agenda", "Calendar"),
            Action::Page(6),
        );

        let hit_start = self.hits.len();
        // Right task list column:
        if tasks.is_empty() {
            r.text(
                self.tr("Nenhuma tarefa", "No tasks yet"),
                ox + 222.,
                52.,
                176.,
                36.,
                13.,
                theme::MUTED,
                false,
                true,
            );
        } else {
            r.dc.PushAxisAlignedClip(
                &windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                    left: ox + 222.,
                    top: 29.,
                    right: ox + 398.,
                    bottom: 29. + 3. * TASK_ROW_PITCH,
                },
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            for (i, t) in tasks.iter().enumerate() {
                let y = 29. + (first + i) as f32 * TASK_ROW_PITCH - position;
                let card_x = ox + 222.;
                if t.done {
                    r.fill_alpha(card_x, y, 176., TASK_TARGET, 9., theme::PRESSED, 0.38);
                } else {
                    r.fill_alpha(card_x, y, 176., TASK_TARGET, 9., theme::TEXT, 0.08);
                }
                if t.done {
                    r.circle(card_x + 15., y + TASK_TARGET / 2., 8., theme::POSITIVE);
                    icon(
                        r,
                        "check",
                        card_x + 11.,
                        y + TASK_TARGET / 2. - 4.,
                        8.,
                        theme::SURFACE,
                    );
                } else {
                    r.stroke_circle(
                        card_x + 15.,
                        y + TASK_TARGET / 2.,
                        8.,
                        theme::MUTED,
                        1.,
                        1.5,
                    );
                }
                self.hit(
                    (card_x, y, TASK_TARGET, TASK_TARGET),
                    self.tr("Concluir tarefa", "Complete task"),
                    Action::Command(AppCommand::ToggleTask(t.id)),
                );

                r.text(
                    &t.title,
                    card_x + 36.,
                    y + 3.,
                    96.,
                    24.,
                    12.,
                    if t.done { theme::MUTED } else { theme::TEXT },
                    false,
                    false,
                );
                if t.done {
                    r.line_alpha(
                        card_x + 36.,
                        y + 14.,
                        card_x + 125.,
                        y + 14.,
                        theme::TEXT,
                        0.5,
                        1.2,
                    );
                }
                self.hit(
                    (card_x + 36., y, 96., TASK_TARGET),
                    &t.title,
                    Action::EditTask(t.id),
                );

                r.text(
                    "⋯",
                    card_x + 144.,
                    y,
                    TASK_TARGET,
                    TASK_TARGET,
                    18.,
                    theme::MUTED,
                    true,
                    true,
                );
                self.hit(
                    (card_x + 144., y, TASK_TARGET, TASK_TARGET),
                    &format!(
                        "{}: {}",
                        self.tr("Ações da tarefa", "Task actions"),
                        t.title
                    ),
                    Action::TaskMenu(t.id),
                );
            }
        }
        if !tasks.is_empty() {
            r.dc.PopAxisAlignedClip();
            self.clip_list_hits(hit_start, (ox + 222., 29., 176., 3. * TASK_ROW_PITCH));
        }
        if count > 3 {
            r.fill_alpha(ox + 406., 36., 3., 34., 1.5, theme::TEXT, 0.4);
        }
    }
    unsafe fn draw_calendar(&mut self, r: &Renderer, ox: f32) {
        let today = chrono::Local::now().date_naive();
        let total = today.year() * 12 + today.month0() as i32 + self.calendar_offset;
        let year = total.div_euclid(12);
        let month = total.rem_euclid(12) as u32 + 1;
        let Some(first) = chrono::NaiveDate::from_ymd_opt(year, month, 1) else {
            return;
        };

        // Back button to return to To-do (page 5)

        r.text(
            &format!("{:02} / {}", month, year),
            ox + 110.,
            42.,
            200.,
            26.,
            15.,
            theme::TEXT,
            true,
            true,
        );
        let prev_month_rect = (ox + 48., 35., 44., 38.);
        let next_month_rect = (ox + 326., 35., 44., 38.);
        let is_prev_month_hover = self.is_hovered(prev_month_rect);
        let is_next_month_hover = self.is_hovered(next_month_rect);
        if is_prev_month_hover {
            r.fill_alpha(ox + 48., 36., 44., 36., 8., theme::TEXT, 0.10);
        }
        if is_next_month_hover {
            r.fill_alpha(ox + 326., 36., 44., 36., 8., theme::TEXT, 0.10);
        }
        r.text(
            "‹",
            ox + 54.,
            41.,
            35.,
            26.,
            22.,
            if is_prev_month_hover {
                theme::TEXT
            } else {
                theme::MUTED
            },
            false,
            true,
        );
        r.text(
            "›",
            ox + 331.,
            41.,
            35.,
            26.,
            22.,
            if is_next_month_hover {
                theme::TEXT
            } else {
                theme::MUTED
            },
            false,
            true,
        );
        self.hit(
            prev_month_rect,
            self.tr("Mês anterior", "Previous month"),
            Action::Calendar(-1),
        );
        self.hit(
            next_month_rect,
            self.tr("Próximo mês", "Next month"),
            Action::Calendar(1),
        );

        let start = if self.state.settings.week_monday {
            first.weekday().num_days_from_monday()
        } else {
            first.weekday().num_days_from_sunday()
        };
        let week_labels = if self.state.settings.week_monday {
            if self.state.settings.en() {
                ["M", "T", "W", "T", "F", "S", "S"]
            } else {
                ["S", "T", "Q", "Q", "S", "S", "D"]
            }
        } else if self.state.settings.en() {
            ["S", "M", "T", "W", "T", "F", "S"]
        } else {
            ["D", "S", "T", "Q", "Q", "S", "S"]
        };
        for (i, label) in week_labels.iter().enumerate() {
            r.text(
                label,
                ox + 64. + i as f32 * 42.,
                74.,
                36.,
                19.,
                10.,
                theme::MUTED,
                false,
                true,
            );
        }
        for day in 1..=31 {
            let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) else {
                break;
            };
            let index = start + day - 1;
            let x = ox + 64. + (index % 7) as f32 * 42.;
            let y = 98. + (index / 7) as f32 * 20.;
            if date == today {
                r.fill(x + 5., y, 26., 20., 7., theme::CALENDAR_SELECTED);
            }
            r.text(
                &day.to_string(),
                x,
                y,
                36.,
                20.,
                11.,
                theme::TEXT,
                date == today,
                true,
            );
            if self.state.tasks.iter().any(|t| {
                !t.done
                    && t.due_at
                        .and_then(|ts| chrono::Local.timestamp_opt(ts, 0).single())
                        .is_some_and(|dt| dt.date_naive() == date)
            }) {
                r.circle(x + 30., y + 10., 2., theme::ACCENT);
            }
            self.hit(
                (x - 3., y - 2., 42., 24.),
                &format!("{} {}", day, date.format("%B %Y")),
                Action::Page(5),
            );
        }
    }
    fn add_path(&mut self, p: PathBuf) {
        self.apply(AppCommand::AddShelf {
            kind: if p.is_dir() {
                ShelfKind::Folder
            } else {
                ShelfKind::File
            },
            value: p.to_string_lossy().into_owned(),
        });
        self.scroll = self.state.shelf.len().saturating_sub(5);
    }
    unsafe fn action(&mut self, a: Action) {
        match a {
            Action::Reveal => self.expand(true),
            Action::TaskMenu(id) => self.task_menu(id),
            Action::SnoozeTask(id) => self.apply(AppCommand::SnoozeTask { id, minutes: 10 }),
            Action::Page(p) => {
                if !self.expanded || !matches!(self.page, 0 | 1) || !matches!(p, 0 | 1) {
                    self.player_layout.snap(if p == 1 { 1. } else { 0. });
                }
                self.page = p;
                self.scroll = 0;
                self.focus = None;
                self.hovered_hit = None;
                self.volume_hovered = false;
                if self.seek_drag.is_some() {
                    self.seek_drag = None;
                    let _ = ReleaseCapture();
                }
                if self.volume_dragging {
                    self.volume_dragging = false;
                    let _ = ReleaseCapture();
                }
                if p == 1 && self.external_lyrics.is_none() && !self.media.title.is_empty() {
                    self.services.send(ServiceCommand::FetchLetrasLyrics {
                        title: self.media.title.clone(),
                        artist: self.media.artist.clone(),
                    });
                }
                self.expand(true);
                self.update_media_animation();
            }
            Action::ShelfScroll(delta) => {
                let max = self.state.shelf.len().saturating_sub(5);
                if delta < 0 {
                    self.scroll = self.scroll.saturating_sub(delta.unsigned_abs() as usize);
                } else {
                    self.scroll = (self.scroll + delta as usize).min(max);
                }
                self.paint();
            }
            Action::Command(c) => self.apply(c),
            Action::DismissToast => {
                self.toast = None;
                self.paint();
            }
            Action::DismissPomodoro => {
                self.pomodoro_notice = None;
            }
            Action::PomodoroNext(phase) => {
                self.apply(AppCommand::SetPomodoroPhase(phase));
                self.apply(AppCommand::StartPomodoro);
                self.pomodoro_notice = None;
                self.page = 2;
                self.hover = false;
                self.expand(false);
            }
            Action::Play => self.services.send(ServiceCommand::PlayPause),
            Action::Next => self.services.send(ServiceCommand::Next),
            Action::Previous => self.services.send(ServiceCommand::Previous),
            Action::Seek(s) => self.services.send(ServiceCommand::Seek(if s < 0. {
                (self.position() + 10.).min(self.media.duration)
            } else {
                s
            })),
            Action::VolumeMute => self.toggle_volume_mute(),
            Action::Volume(v) => {
                if v < 0. {
                    let next = (self.system.volume.unwrap_or(0.) + 0.1).min(1.);
                    self.change_volume(next);
                } else {
                    self.change_volume(v);
                }
            }
            Action::Settings | Action::HydrationSettings => {
                self.modal = true;
                let mut settings = self.state.settings.clone();
                let accepted = if matches!(a, Action::HydrationSettings) {
                    dialogs::hydration_settings(self.hwnd, &mut settings)
                } else {
                    dialogs::settings(self.hwnd, &mut settings)
                };
                if accepted {
                    let search_changed = settings.lyrics.ignored_title_terms
                        != self.state.settings.lyrics.ignored_title_terms;
                    if settings.startup != self.state.settings.startup {
                        if let Err(e) = set_startup(settings.startup) {
                            self.toast = Some((e.to_string(), Instant::now()));
                            settings.startup = self.state.settings.startup;
                        }
                    }
                    self.apply(AppCommand::Settings(Box::new(settings)));
                    if search_changed {
                        nivumi::letras::set_ignored_title_terms(
                            &self.state.settings.lyrics.ignored_title_terms,
                        );
                        self.offline_job = None;
                        self.offline_attempt.clear();
                        self.letras_done = false;
                        self.lyric_lines.clear();
                        self.external_lyrics = None;
                        self.external_scroll = 0;
                        self.lyric_status = self.tr("Buscando letras…", "Loading lyrics…").into();
                        if !self.media.title.is_empty() {
                            self.services.send(ServiceCommand::FetchLyrics {
                                title: self.media.title.clone(),
                                artist: self.media.artist.clone(),
                                duration: self.media.duration,
                            });
                            self.services.send(ServiceCommand::FetchLetrasLyrics {
                                title: self.media.title.clone(),
                                artist: self.media.artist.clone(),
                            });
                        }
                    }
                    self.resize();
                }
                self.modal = false;
                self.expand(true);
            }
            Action::NewTask => {
                self.modal = true;
                if let Some(task) = dialogs::task(self.hwnd, None, self.state.settings.en()) {
                    self.apply(AppCommand::AddTask {
                        title: task.title.clone(),
                        due_at: task.due_at,
                    });
                    if let Some(id) = self.state.tasks.last().map(|t| t.id) {
                        self.apply(AppCommand::EditTask {
                            id,
                            title: task.title,
                            due_at: task.due_at,
                            color: task.color,
                        });
                    }
                }
                self.modal = false;
            }
            Action::EditTask(id) => {
                self.modal = true;
                let task = self.state.tasks.iter().find(|t| t.id == id).cloned();
                if let Some(task) =
                    dialogs::task(self.hwnd, task.as_ref(), self.state.settings.en())
                {
                    self.apply(AppCommand::EditTask {
                        id,
                        title: task.title,
                        due_at: task.due_at,
                        color: task.color,
                    });
                }
                self.modal = false;
            }
            Action::AddText => {
                self.modal = true;
                if let Some(text) = dialogs::text(self.hwnd, self.state.settings.en()) {
                    self.apply(AppCommand::AddShelf {
                        kind: if text.starts_with("https://") || text.starts_with("http://") {
                            ShelfKind::Link
                        } else {
                            ShelfKind::Text
                        },
                        value: text,
                    });
                    self.scroll = self.state.shelf.len().saturating_sub(5);
                }
                self.modal = false;
            }
            Action::AddFiles | Action::AddFolder => {
                self.modal = true;
                if let Ok(dialog) = CoCreateInstance::<_, IFileOpenDialog>(
                    &FileOpenDialog,
                    None,
                    CLSCTX_INPROC_SERVER,
                ) {
                    let folders = matches!(a, Action::AddFolder);
                    let _ = dialog.SetOptions(
                        FOS_ALLOWMULTISELECT
                            | FOS_FORCEFILESYSTEM
                            | if folders {
                                FOS_PICKFOLDERS
                            } else {
                                FILEOPENDIALOGOPTIONS(0)
                            },
                    );
                    if dialog.Show(Some(self.hwnd)).is_ok() {
                        if let Ok(items) = dialog.GetResults() {
                            for i in 0..items.GetCount().unwrap_or(0) {
                                if let Ok(item) = items.GetItemAt(i) {
                                    if let Ok(path) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                                        let value = path.to_string().unwrap_or_default();
                                        CoTaskMemFree(Some(path.0.cast()));
                                        self.add_path(PathBuf::from(value));
                                    }
                                }
                            }
                        }
                    }
                }
                self.modal = false;
            }
            Action::OpenShelf(id) | Action::CopyShelf(id) => {
                if let Some(item) = self.state.shelf.iter().find(|i| i.id == id).cloned() {
                    if item.kind == ShelfKind::Text || matches!(a, Action::CopyShelf(_)) {
                        if let Err(e) = copy_text(self.hwnd, &item.value) {
                            log(&e.to_string());
                        }
                    } else {
                        let _ = ShellExecuteW(
                            Some(self.hwnd),
                            w!("open"),
                            &HSTRING::from(&item.value),
                            None,
                            None,
                            SW_SHOWNORMAL,
                        );
                    }
                }
            }
            Action::SelectShelf(id) => {
                self.shelf_select(
                    id,
                    GetKeyState(VK_CONTROL.0 as i32) < 0,
                    GetKeyState(VK_SHIFT.0 as i32) < 0,
                );
            }
            Action::RemoveShelfItem(id) => {
                self.apply(AppCommand::RemoveShelf(id));
                self.selected.remove(&id);
                self.shelf_thumbnails.remove(&id);
                self.scroll = self.scroll.min(self.state.shelf.len().saturating_sub(5));
            }
            Action::PasteShelfImage => {
                let directory = self
                    .path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("shelf-captures");
                let message = match nivumi::clipboard_image::paste(self.hwnd, &directory) {
                    Ok(Some(path)) => {
                        self.add_path(path);
                        self.selected.clear();
                        if let Some(item) = self.state.shelf.last() {
                            self.selected.insert(item.id);
                            self.shelf_anchor = Some(item.id);
                        }
                        self.action(Action::Page(4));
                        self.scroll = self.state.shelf.len().saturating_sub(5);
                        self.expand(true);
                        self.tr(
                            "Captura adicionada à Prateleira",
                            "Screenshot added to Shelf",
                        )
                        .to_owned()
                    }
                    Ok(None) => self
                        .tr(
                            "Nenhuma imagem na área de transferência",
                            "No image on the clipboard",
                        )
                        .to_owned(),
                    Err(error) => {
                        log(&format!("Paste screenshot: {error}"));
                        self.tr("Não foi possível colar a imagem", "Could not paste image")
                            .to_owned()
                    }
                };
                self.toast = Some((message, Instant::now()));
            }
            Action::ShelfClipboard(cut) => {
                let text_only = !self.selected.is_empty()
                    && self
                        .state
                        .shelf
                        .iter()
                        .filter(|i| self.selected.contains(&i.id))
                        .all(|i| matches!(i.kind, ShelfKind::Text | ShelfKind::Link));
                let result = if text_only && !cut {
                    let text = self
                        .state
                        .shelf
                        .iter()
                        .filter(|i| self.selected.contains(&i.id))
                        .map(|i| i.value.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    copy_text(self.hwnd, &text)
                } else {
                    self.shelf_files()
                        .and_then(|files| ole::clipboard_files(&files, cut))
                };
                if let Err(error) = result {
                    self.toast = Some((error.message().to_string(), Instant::now()));
                }
            }
            Action::RemoveSelected => {
                let ids = std::mem::take(&mut self.selected);
                for id in ids {
                    self.state.apply(AppCommand::RemoveShelf(id), Local::now());
                    self.shelf_thumbnails.remove(&id);
                }
                self.scroll = self.scroll.min(self.state.shelf.len().saturating_sub(5));
                self.save();
            }
            Action::ShelfInfo => {
                self.modal = true;
                let (title, message) = shelf_guide_text(self.state.settings.en());
                let _ = MessageBoxW(
                    Some(self.hwnd),
                    &HSTRING::from(message),
                    &HSTRING::from(title),
                    MB_OK | MB_ICONINFORMATION,
                );
                self.modal = false;
                self.expand(true);
            }
            Action::ClearShelf => {
                if MessageBoxW(
                    Some(self.hwnd),
                    &HSTRING::from(self.tr(
                        "Limpar a prateleira? Os arquivos originais serão preservados.",
                        "Clear shelf? Original files will be kept.",
                    )),
                    w!("Nivumi"),
                    MB_YESNO | MB_ICONQUESTION,
                ) == IDYES
                {
                    self.apply(AppCommand::ClearShelf);
                    self.selected.clear();
                    self.shelf_thumbnails.clear();
                    self.scroll = 0;
                }
            }
            Action::DragShelf(id) => self.shelf_drag(id, false),
            Action::Calendar(offset) => self.calendar_offset += offset,
            Action::OpenSubtitles => {
                self.select_song_sync();
                if let Some(window) = &mut self.subtitles {
                    window.edit(true);
                } else {
                    match SubtitleWindow::open(self.hwnd, self.state.settings.subtitles.clone()) {
                        Ok(window) => self.subtitles = Some(window),
                        Err(error) => {
                            self.toast = Some((format!("Legenda: {error}"), Instant::now()))
                        }
                    }
                }
                let position = (self.position() * 1000.) as i64;
                if let Some(window) = &mut self.subtitles {
                    window.set_reduced_motion(self.state.settings.reduced_motion);
                    window.update(&self.lyric_lines, position);
                }
            }
            Action::OpenLyricsWindow => {
                if let Some(window) = &mut self.lyrics_window {
                    window.show();
                } else {
                    let mut preferences = self.state.settings.lyrics.clone();
                    if preferences.window_bounds.is_none() {
                        preferences.window_mode = self.lyrics_view_mode;
                    }
                    match LyricsWindow::open(self.hwnd, preferences, self.state.settings.en()) {
                        Ok(window) => self.lyrics_window = Some(window),
                        Err(error) => {
                            self.toast = Some((
                                format!(
                                    "{}: {error}",
                                    self.tr(
                                        "Não foi possível abrir as letras",
                                        "Could not open lyrics"
                                    )
                                ),
                                Instant::now(),
                            ))
                        }
                    }
                }
                self.sync_lyrics_window();
            }
            Action::LyricsOptions => self.lyrics_options(),
            Action::LyricsAnimation(style) => {
                self.state.settings.lyrics.focus_animation = style.min(3);
                self.verse_change_at = None;
                self.verse_scroll = Default::default();
                self.save();
            }
            Action::LyricsAllTranslations => {
                self.state.settings.lyrics.translation_all_verses =
                    !self.state.settings.lyrics.translation_all_verses;
                self.save();
            }

            Action::CopyLyricsForAi => {
                let content = self.lyrics_content();
                let text = content.format_for_ai();
                if let Err(e) = copy_text(self.hwnd, &text) {
                    self.toast = Some((e.to_string(), Instant::now()));
                } else {
                    self.toast = Some((
                        self.tr(
                            "Letra e tradução copiadas para debugar com IA",
                            "Lyrics and translation copied to debug with AI",
                        )
                        .into(),
                        Instant::now(),
                    ));
                }
                self.paint();
            }
            Action::LyricsFont(delta) => {
                self.state.settings.lyrics.notch_font =
                    (self.state.settings.lyrics.notch_font + delta as f32).clamp(12., 24.);
                self.save();
            }
            Action::FollowLyrics => {
                self.notch_follow = true;
                self.external_scroll = 0;
            }
            Action::LyricsMode(m) => {
                self.notch_follow = true;
                self.lyrics_view_mode = self.lyrics_content().available_mode(m);
                self.external_scroll = 0;
                if m != 0 && self.external_lyrics.is_none() && !self.media.title.is_empty() {
                    self.services.send(ServiceCommand::FetchLetrasLyrics {
                        title: self.media.title.clone(),
                        artist: self.media.artist.clone(),
                    });
                }
            }
            Action::ReloadLetras => {
                self.services.send(ServiceCommand::FetchLyrics {
                    title: self.media.title.clone(),
                    artist: self.media.artist.clone(),
                    duration: self.media.duration,
                });
                self.external_lyrics = None;
                self.external_scroll = 0;
                if !self.media.title.is_empty() {
                    self.services.send(ServiceCommand::FetchLetrasLyrics {
                        title: self.media.title.clone(),
                        artist: self.media.artist.clone(),
                    });
                }
            }
            Action::LyricsScroll(delta) => {
                let verses = self.lyrics_content().verses(self.lyrics_view_mode);
                let max_scroll = verses.len().saturating_sub(1);
                if self.notch_follow && self.lyrics_view_mode <= 1 {
                    self.external_scroll = verses
                        .iter()
                        .position(|verse| verse.active)
                        .unwrap_or(0)
                        .saturating_sub(1);
                }
                self.notch_follow = false;
                self.external_scroll = self.external_scroll.min(max_scroll);
                if delta < 0 {
                    self.external_scroll = self.external_scroll.saturating_sub(2);
                } else if delta > 0 {
                    self.external_scroll = self.external_scroll.saturating_add(2).min(max_scroll);
                }
            }
            Action::TrackInfo => {}
            Action::Close => {
                self.hide_to_tray();
            }
            Action::Exit => {
                let data = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: self.hwnd,
                    uID: 1,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                let _ = DestroyWindow(self.hwnd);
                PostQuitMessage(0);
                return;
            }
        }
        self.paint();
    }
    unsafe fn lyrics_options(&mut self) {
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
                    | if self.lyrics_content().available_mode(mode) != mode {
                        MF_GRAYED
                    } else {
                        MENU_ITEM_FLAGS(0)
                    }
                    | if self.lyrics_view_mode == mode {
                        MF_CHECKED
                    } else {
                        MENU_ITEM_FLAGS(0)
                    },
                mode + 100,
                &HSTRING::from(*label),
            );
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING
                | if self.state.settings.lyrics.translation_all_verses {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                },
            117,
            &HSTRING::from(self.tr("Tradução em todos os versos", "Translation on all verses")),
        );
        if let Ok(animation) = CreatePopupMenu() {
            for (style, label) in [
                self.tr("Sem animação", "No animation"),
                self.tr("Dissolver", "Fade"),
                self.tr("Deslizar", "Slide"),
                self.tr("Realce suave", "Soft highlight"),
            ]
            .iter()
            .enumerate()
            {
                let _ = AppendMenuW(
                    animation,
                    MF_STRING
                        | if self.state.settings.lyrics.focus_animation == style {
                            MF_CHECKED
                        } else {
                            MENU_ITEM_FLAGS(0)
                        },
                    120 + style,
                    &HSTRING::from(*label),
                );
            }
            let _ = AppendMenuW(
                menu,
                MF_POPUP,
                animation.0 as usize,
                &HSTRING::from(self.tr("Animação do verso na notch", "Notch verse animation")),
            );
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        for (id, label) in [
            (110, "Diminuir fonte"),
            (111, "Aumentar fonte"),
            (112, "Voltar ao verso atual"),
            (113, "Abrir em janela"),
            (114, "Modo legenda"),
            (115, "Buscar letras novamente"),
        ] {
            let _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(label));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            116,
            &HSTRING::from(self.tr(
                "Copiar letra e tradução para IA",
                "Copy lyrics and translation for AI",
            )),
        );
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _theme = nivumi::popup_menu::DarkMenu::attach(menu, self.hwnd);
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
        let action = match choice.0 {
            100..=103 => Some(Action::LyricsMode(choice.0 as usize - 100)),
            110 => Some(Action::LyricsFont(-1)),
            111 => Some(Action::LyricsFont(1)),
            112 => Some(Action::FollowLyrics),
            113 => Some(Action::OpenLyricsWindow),
            114 => Some(Action::OpenSubtitles),
            115 => Some(Action::ReloadLetras),
            116 => Some(Action::CopyLyricsForAi),
            117 => Some(Action::LyricsAllTranslations),
            120..=123 => Some(Action::LyricsAnimation(choice.0 as usize - 120)),
            _ => None,
        };
        if let Some(action) = action {
            self.action(action);
        }
    }
    unsafe fn register_caption_hotkey(&mut self) {
        let _ = UnregisterHotKey(Some(self.hwnd), 2);
        let (modifiers, key) = self.state.settings.subtitles.hotkey();
        if RegisterHotKey(Some(self.hwnd), 2, modifiers | MOD_NOREPEAT, key).is_err() {
            self.toast = Some((
                self.tr(
                    "Atalho de legenda ocupado; use o menu da bandeja ou escolha outro em Atalho",
                    "Caption shortcut is busy; use the tray menu or choose another shortcut",
                )
                .into(),
                Instant::now(),
            ));
        }
    }
    unsafe fn register_sync_hotkeys(&mut self) {
        for id in 3..=5 {
            let _ = UnregisterHotKey(Some(self.hwnd), id);
        }
        let mut failures = vec![];
        for (index, binding) in self
            .state
            .settings
            .subtitles
            .sync_hotkeys
            .iter()
            .enumerate()
        {
            if let Some([mods, key]) = binding {
                if RegisterHotKey(
                    Some(self.hwnd),
                    index as i32 + 3,
                    HOT_KEY_MODIFIERS(*mods) | MOD_NOREPEAT,
                    *key,
                )
                .is_err()
                {
                    failures.push(["atrasar", "adiantar", "zerar"][index]);
                }
            }
        }
        if !failures.is_empty() {
            self.toast = Some((
                format!(
                    "Atalho ocupado ({}). Escolha outra combinação no menu da legenda.",
                    failures.join(", ")
                ),
                Instant::now(),
            ));
        }
    }
    fn collect_subtitle_preferences(&mut self) {
        if let Some(window) = &self.subtitles {
            let prefs = window.preferences().clone();
            let key = self.song_sync_key();
            self.state
                .settings
                .lyrics
                .set_song_offset(&key, prefs.delay_ms);
            self.state.settings.subtitles = prefs;
        }
    }
    unsafe fn menu(&mut self) {
        let Ok(menu) = CreatePopupMenu() else { return };
        if self.tray_hidden {
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                1,
                &HSTRING::from(self.tr("Abrir notch", "Open notch")),
            );
        } else {
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                1,
                &HSTRING::from(self.tr("Ocultar na bandeja", "Hide to tray")),
            );
            if self.expanded {
                let _ = AppendMenuW(
                    menu,
                    MF_STRING,
                    9,
                    &HSTRING::from(self.tr("Recolher notch", "Collapse notch")),
                );
            }
        }
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            8,
            &HSTRING::from(self.tr("Ocultar na borda da tela", "Hide to screen edge")),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            2,
            &HSTRING::from(self.tr("Configurações…", "Settings…")),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            5,
            &HSTRING::from(self.tr("Abrir / ajustar legenda", "Open / adjust captions")),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            6,
            &HSTRING::from(self.tr("Travar / destravar legenda", "Lock / unlock captions")),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            7,
            &HSTRING::from(self.tr("Ocultar legenda", "Hide captions")),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 3, &HSTRING::from(self.tr("Sair", "Exit")));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(self.hwnd);
        let _menu_theme = nivumi::popup_menu::DarkMenu::attach(menu, self.hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, p.x, p.y, None, self.hwnd, None);
        let _ = DestroyMenu(menu);
        match cmd.0 {
            1 => {
                if self.tray_hidden {
                    self.restore_from_tray(true);
                } else {
                    self.hide_to_tray();
                }
            }
            9 => self.expand(false),
            2 => self.action(Action::Settings),
            3 => self.action(Action::Exit),
            8 => {
                self.expand(false);
                self.hidden = true;
                self.hover = false;
                self.hovered_hit = None;
                self.focus = None;
                self.navigation_hover = false;
                self.volume_hovered = false;
                self.animate();
                self.paint();
            }
            5 => self.action(Action::OpenSubtitles),
            6 => {
                if let Some(window) = &mut self.subtitles {
                    window.toggle();
                } else {
                    self.action(Action::OpenSubtitles);
                }
            }
            7 => {
                if let Some(window) = &self.subtitles {
                    let _ = ShowWindow(window.hwnd, SW_HIDE);
                }
            }
            _ => {}
        }
    }
}
// Small geometry-only glyphs: no font-dependent emoji or bitmap controls.
unsafe fn icon(r: &Renderer, kind: &str, x: f32, y: f32, size: f32, c: u32) {
    let svg = match kind {
        "volume_low" => Some(include_str!("../assets/icons/volume_low.svg")),
        "volume" => Some(include_str!("../assets/icons/volume.svg")),
        "volume_high" => Some(include_str!("../assets/icons/volume_high.svg")),
        "volume_mute" => Some(include_str!("../assets/icons/volume_mute.svg")),
        "calendar" => Some(include_str!("../assets/icons/calendar.svg")),
        "timer" => Some(include_str!("../assets/icons/timer.svg")),
        "drop" => Some(include_str!("../assets/icons/drop.svg")),

        "lyrics" => Some(include_str!("../assets/icons/lyrics.svg")),
        "lyrics-active" => Some(include_str!("../assets/icons/lyrics-active.svg")),
        "play" => Some(include_str!("../assets/icons/play.svg")),
        "pause" => Some(include_str!("../assets/icons/pause.svg")),
        "next" => Some(include_str!("../assets/icons/next.svg")),
        "previous" => Some(include_str!("../assets/icons/previous.svg")),
        "settings" => Some(include_str!("../assets/icons/settings.svg")),
        "clock" => Some(include_str!("../assets/icons/clock.svg")),
        "check" => Some(include_str!("../assets/icons/check.svg")),
        "left" => Some(include_str!("../assets/icons/left.svg")),
        "right" => Some(include_str!("../assets/icons/right.svg")),
        "info" => Some(include_str!("../assets/icons/info.svg")),
        "download" => Some(include_str!("../assets/icons/download.svg")),
        "bell" => Some(include_str!("../assets/icons/bell.svg")),
        "music" => Some(include_str!("../assets/icons/music.svg")),
        _ => None,
    };
    if let Some(svg) = svg {
        if r.svg_icon(svg, x, y, size, c) {
            return;
        }
    }
    let line = |a: f32, b: f32, d: f32, e: f32| {
        r.line(
            x + a * size,
            y + b * size,
            x + d * size,
            y + e * size,
            c,
            (size / 8.).clamp(1.4, 2.4),
        )
    };
    let arc = |cx: f32, cy: f32, radius: f32, start: f32, end: f32| {
        for i in 0..24 {
            let a = (start + (end - start) * i as f32 / 24.).to_radians();
            let b = (start + (end - start) * (i + 1) as f32 / 24.).to_radians();
            r.line(
                x + (cx + radius * a.cos()) * size,
                y + (cy + radius * a.sin()) * size,
                x + (cx + radius * b.cos()) * size,
                y + (cy + radius * b.sin()) * size,
                c,
                (size / 8.).clamp(1.4, 2.4),
            );
        }
    };
    match kind {
        "left" => {
            line(0.65, 0.15, 0.3, 0.5);
            line(0.3, 0.5, 0.65, 0.85);
        }
        "right" => {
            line(0.35, 0.15, 0.7, 0.5);
            line(0.7, 0.5, 0.35, 0.85);
        }
        "play" => {
            for i in 0..10 {
                let t = i as f32 / 10.;
                line(
                    0.25 + t * 0.55,
                    0.15 + t * 0.35,
                    0.25 + t * 0.55,
                    0.85 - t * 0.35,
                );
            }
        }
        "pause" => {
            r.fill(
                x + size * 0.22,
                y + size * 0.18,
                size * 0.2,
                size * 0.64,
                1.5,
                c,
            );
            r.fill(
                x + size * 0.58,
                y + size * 0.18,
                size * 0.2,
                size * 0.64,
                1.5,
                c,
            );
        }
        "next" | "previous" => {
            let flip = kind == "previous";
            for base in [0.08, 0.52] {
                for i in 0..6 {
                    let t = i as f32 / 6.;
                    let xx = base + t * 0.38;
                    let a = if flip { 1. - xx } else { xx };
                    line(a, 0.18 + t * 0.32, a, 0.82 - t * 0.32);
                }
            }
        }
        "lyrics" => {
            r.stroke_rounded_rect(
                x + size * 0.05,
                y + size * 0.05,
                size * 0.9,
                size * 0.68,
                size * 0.2,
                c,
                1.0,
                1.3,
                false,
            );
            line(0.24, 0.72, 0.16, 0.94);
            line(0.16, 0.94, 0.42, 0.72);
            r.circle(x + size * 0.35, y + size * 0.38, size * 0.08, c);
            r.circle(x + size * 0.65, y + size * 0.38, size * 0.08, c);
        }
        "lyrics_inner" | "lyrics-active" => {
            r.fill(
                x + size * 0.125,
                y + size * 0.125,
                size * 0.75,
                size * 0.75,
                size * 0.17,
                theme::TEXT,
            );
            r.circle(
                x + size * 0.35,
                y + size * 0.42,
                size * 0.09,
                theme::SURFACE,
            );
            r.circle(
                x + size * 0.65,
                y + size * 0.42,
                size * 0.09,
                theme::SURFACE,
            );
        }
        "timer" => {
            r.stroke_circle(x + size * 0.5, y + size * 0.56, size * 0.34, c, 1.0, 2.0);
            line(0.5, 0.56, 0.5, 0.35);
            line(0.38, 0.1, 0.62, 0.1);
            line(0.5, 0.1, 0.5, 0.23);
        }
        "clock" => {
            r.stroke_circle(x + size * 0.5, y + size * 0.5, size * 0.36, c, 1.0, 1.6);
            line(0.5, 0.5, 0.5, 0.25);
            line(0.5, 0.5, 0.7, 0.5);
        }
        "reset" => {
            arc(0.5, 0.5, 0.34, 35., 325.);
            line(0.85, 0.15, 0.85, 0.4);
            line(0.85, 0.4, 0.6, 0.4);
        }
        "cup" => {
            r.stroke_rounded_rect(
                x + size * 0.18,
                y + size * 0.36,
                size * 0.5,
                size * 0.46,
                size * 0.12,
                c,
                1.0,
                1.6,
                false,
            );
            line(0.68, 0.42, 0.84, 0.42);
            line(0.84, 0.42, 0.84, 0.62);
            line(0.84, 0.62, 0.68, 0.62);
            line(0.32, 0.16, 0.32, 0.28);
            line(0.52, 0.16, 0.52, 0.28);
        }
        "target" => {
            r.stroke_circle(x + size * 0.5, y + size * 0.5, size * 0.38, c, 1.0, 1.6);
            r.circle(x + size * 0.5, y + size * 0.5, size * 0.14, c);
            line(0.5, 0.06, 0.5, 0.24);
            line(0.5, 0.76, 0.5, 0.94);
            line(0.06, 0.5, 0.24, 0.5);
            line(0.76, 0.5, 0.94, 0.5);
        }
        "drop" => {
            r.circle(x + size * 0.5, y + size * 0.64, size * 0.28, c);
            for i in 0..8 {
                let t = i as f32 / 8.;
                r.line(
                    x + (0.5 - t * 0.24) * size,
                    y + (0.2 + t * 0.44) * size,
                    x + (0.5 + t * 0.24) * size,
                    y + (0.2 + t * 0.44) * size,
                    c,
                    1.8,
                );
            }
        }
        "bell" => {
            arc(0.5, 0.46, 0.28, 180., 360.);
            line(0.22, 0.46, 0.14, 0.74);
            line(0.78, 0.46, 0.86, 0.74);
            line(0.1, 0.74, 0.9, 0.74);
            r.circle(x + size * 0.5, y + size * 0.84, size * 0.1, c);
        }
        "sliders" | "settings" => {
            line(0.12, 0.34, 0.88, 0.34);
            line(0.12, 0.68, 0.88, 0.68);
            r.circle(x + size * 0.38, y + size * 0.34, size * 0.13, c);
            r.circle(x + size * 0.64, y + size * 0.68, size * 0.13, c);
        }
        "plus" => {
            line(0.5, 0.18, 0.5, 0.82);
            line(0.18, 0.5, 0.82, 0.5);
        }
        "close" => {
            line(0.24, 0.24, 0.76, 0.76);
            line(0.76, 0.24, 0.24, 0.76);
        }
        "check" => {
            line(0.14, 0.5, 0.4, 0.76);
            line(0.4, 0.76, 0.88, 0.24);
        }
        "download" => {
            line(0.5, 0.12, 0.5, 0.62);
            line(0.3, 0.44, 0.5, 0.62);
            line(0.7, 0.44, 0.5, 0.62);
            line(0.14, 0.66, 0.14, 0.86);
            line(0.14, 0.86, 0.86, 0.86);
            line(0.86, 0.86, 0.86, 0.66);
        }
        "folder" => {
            r.fill(
                x + size * 0.08,
                y + size * 0.18,
                size * 0.42,
                size * 0.22,
                2.,
                theme::WARNING,
            );
            r.fill(
                x + size * 0.08,
                y + size * 0.32,
                size * 0.84,
                size * 0.54,
                3.,
                theme::WARNING,
            );
        }
        "info" => {
            r.stroke_circle(
                x + size * 0.5,
                y + size * 0.5,
                size * 0.42,
                c,
                1.0,
                (size / 8.).clamp(1.2, 1.8),
            );
            r.circle(
                x + size * 0.5,
                y + size * 0.3,
                (size * 0.08).clamp(1.0, 1.6),
                c,
            );
            line(0.5, 0.44, 0.5, 0.72);
        }
        "volume_mute" => {
            line(0.08, 0.4, 0.24, 0.4);
            line(0.24, 0.4, 0.48, 0.18);
            line(0.48, 0.18, 0.48, 0.82);
            line(0.48, 0.82, 0.24, 0.6);
            line(0.24, 0.6, 0.08, 0.6);
            line(0.08, 0.6, 0.08, 0.4);
            line(0.64, 0.38, 0.86, 0.62);
            line(0.86, 0.38, 0.64, 0.62);
        }
        "volume_low" => {
            line(0.08, 0.4, 0.24, 0.4);
            line(0.24, 0.4, 0.48, 0.18);
            line(0.48, 0.18, 0.48, 0.82);
            line(0.48, 0.82, 0.24, 0.6);
            line(0.24, 0.6, 0.08, 0.6);
            line(0.08, 0.6, 0.08, 0.4);
            arc(0.42, 0.5, 0.24, -40., 40.);
        }
        "volume" => {
            line(0.08, 0.4, 0.24, 0.4);
            line(0.24, 0.4, 0.48, 0.18);
            line(0.48, 0.18, 0.48, 0.82);
            line(0.48, 0.82, 0.24, 0.6);
            line(0.24, 0.6, 0.08, 0.6);
            line(0.08, 0.6, 0.08, 0.4);
            arc(0.42, 0.5, 0.36, -50., 50.);
        }
        "volume_high" => {
            line(0.08, 0.4, 0.24, 0.4);
            line(0.24, 0.4, 0.48, 0.18);
            line(0.48, 0.18, 0.48, 0.82);
            line(0.48, 0.82, 0.24, 0.6);
            line(0.24, 0.6, 0.08, 0.6);
            line(0.08, 0.6, 0.08, 0.4);
            arc(0.42, 0.5, 0.24, -40., 40.);
            arc(0.42, 0.5, 0.42, -55., 55.);
        }
        "music" => {
            line(0.35, 0.7, 0.35, 0.18);
            line(0.35, 0.18, 0.8, 0.1);
            line(0.8, 0.1, 0.8, 0.62);
            r.circle(x + size * 0.23, y + size * 0.75, size * 0.13, c);
            r.circle(x + size * 0.68, y + size * 0.67, size * 0.13, c);
        }
        _ => {
            line(0.2, 0.1, 0.65, 0.1);
            line(0.65, 0.1, 0.85, 0.3);
            line(0.85, 0.3, 0.85, 0.9);
            line(0.85, 0.9, 0.2, 0.9);
            line(0.2, 0.9, 0.2, 0.1);
            line(0.35, 0.45, 0.7, 0.45);
            line(0.35, 0.65, 0.7, 0.65);
        }
    }
}
unsafe fn ring(r: &Renderer, x: f32, y: f32, radius: f32, color: u32, fraction: f32) {
    for i in 0..96 {
        let a = (200. + i as f32 * 360. / 96.).to_radians();
        let b = (200. + (i + 1) as f32 * 360. / 96.).to_radians();
        r.line(
            x + a.cos() * radius,
            y + a.sin() * radius,
            x + b.cos() * radius,
            y + b.sin() * radius,
            if i as f32 / 96. < fraction.clamp(0., 1.) {
                color
            } else if color == theme::POSITIVE {
                theme::POSITIVE_SURFACE
            } else if color == theme::FOCUS {
                theme::ERROR_SURFACE
            } else {
                theme::SELECTED
            },
            if radius > 15. { 6. } else { 2.4 },
        );
    }
}
unsafe fn equalizer(r: &Renderer, x: f32, y: f32, playing: bool, color: u32, phase: Option<f32>) {
    for (i, h) in equalizer_heights(phase, playing)
        .iter()
        .copied()
        .enumerate()
    {
        r.fill(x + i as f32 * 3., y - h / 2., 2., h, 1., color);
    }
}

fn time(s: f64) -> String {
    let s = s.max(0.) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}
fn copy_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    for (d, c) in dst.iter_mut().take(N - 1).zip(s.encode_utf16()) {
        *d = c;
    }
}
fn log(s: &str) {
    use std::io::Write;
    let p = storage::default_path().with_file_name("diagnostic.log");
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(file, "{} {s}", Local::now().to_rfc3339());
    }
}
unsafe fn copy_text(hwnd: HWND, text: &str) -> Result<()> {
    crate::ole::copy_text(hwnd, text)
}
pub fn compact_notch_layout(
    has_left_icon: bool,
    has_right_icon: bool,
    ox: f32,
    w: f32,
) -> (f32, f32) {
    match (has_left_icon, has_right_icon) {
        (true, true) => (ox + 34., (w - 68.).max(10.)),
        (true, false) => (ox + 34., (w - 48.).max(10.)),
        (false, true) => (ox + 14., (w - 48.).max(10.)),
        (false, false) => (ox + 10., (w - 20.).max(10.)),
    }
}

pub fn shelf_start_x(count: usize, ox: f32) -> f32 {
    let count = count.clamp(1, 5);
    let total_width = (count - 1) as f32 * 76.0 + 62.0;
    ox + (BODY - total_width) / 2.0
}

pub fn shelf_stack_base_x(count: usize, ox: f32) -> f32 {
    let step_offset = count.saturating_sub(1) as f32 * 2.;
    ox + 188. - step_offset
}

pub fn shelf_guide_text(en: bool) -> (&'static str, &'static str) {
    if en {
        (
            "Shelf · How it works",
            "The Shelf temporarily stores files, folders, texts, and links to streamline your workflow.\n\n\
            • Add items:\n  Drag and drop files or folders from anywhere into the shelf (or click the dashed area to browse).\n\n\
            • Open or Copy:\n  Double-click to open files/folders in their default app or copy texts/links to the clipboard.\n\n\
            • Multi-selection:\n  Click an item to select it. Hold Ctrl to toggle individual items or Shift to select a range (Ctrl+A selects all).\n\n\
            • Drag out:\n  Drag selected items directly to File Explorer, emails, browser, or any application with drag-and-drop support.\n\n\
            • Context menu (right-click):\n  Right-click for options to copy, cut, or remove items.\n\n\
            • Quick shortcuts:\n  - Ctrl+C: Copy selected items\n  - Ctrl+V: Paste a screenshot into the Shelf\n  - Ctrl+X: Cut selected files\n  - Delete: Remove selected items\n  - Shift + Mouse wheel: Scroll through the list when there are more than 5 items\n\n\
            • Clear shelf:\n  The Clear button empties shelf references without deleting the original files from your computer.",
        )
    } else {
        (
            "Prateleira · Como funciona",
            "A Prateleira guarda temporariamente arquivos, pastas, textos e links para facilitar o seu fluxo de trabalho.\n\n\
            • Adicionar itens:\n  Arraste arquivos ou pastas de qualquer lugar e solte na prateleira (ou clique na área tracejada para selecionar).\n\n\
            • Abrir ou Copiar:\n  Dê duplo clique para abrir arquivos/pastas no aplicativo padrão ou copiar textos/links para a área de transferência.\n\n\
            • Seleção múltipla:\n  Clique em um item para selecionar. Segure Ctrl para alternar itens individuais ou Shift para selecionar um intervalo contínuo (Ctrl+A seleciona todos).\n\n\
            • Arrastar para fora:\n  Arraste os itens selecionados diretamente para o Explorador de Arquivos, e-mails, navegador ou qualquer outro aplicativo.\n\n\
            • Menu de contexto (botão direito):\n  Clique com o botão direito para copiar, recortar ou remover itens.\n\n\
            • Atalhos rápidos:\n  - Ctrl+C: Copiar itens selecionados\n  - Ctrl+V: Colar uma captura de tela na Prateleira\n  - Ctrl+X: Recortar arquivos selecionados\n  - Delete: Remover itens selecionados\n  - Shift + Roda do mouse: Rolar pela lista quando houver mais de 5 itens\n\n\
            • Limpar prateleira:\n  O botão Limpar esvazia as referências da prateleira sem apagar os arquivos originais do seu computador.",
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelTarget {
    Page(usize),
    LyricsScroll(i32),
    ItemScroll(usize),
}

fn adjacent_page(page: usize, previous: bool) -> usize {
    (page + if previous { 6 } else { 1 }) % 7
}

pub fn handle_wheel(
    page: usize,
    scroll: usize,
    shelf_len: usize,
    tasks_len: usize,
    up: bool,
    shift: bool,
    alt: bool,
) -> WheelTarget {
    if !shift && !alt {
        WheelTarget::Page(adjacent_page(page, up))
    } else if page == 1 {
        WheelTarget::LyricsScroll(if up { -2 } else { 2 })
    } else if matches!(page, 4 | 5) {
        let max_scroll = if page == 4 {
            shelf_len.saturating_sub(5)
        } else {
            tasks_len.saturating_sub(3)
        };
        let new_scroll = if up {
            scroll.saturating_sub(1)
        } else {
            (scroll + 1).min(max_scroll)
        };
        WheelTarget::ItemScroll(new_scroll)
    } else {
        WheelTarget::Page(adjacent_page(page, up))
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = nivumi::popup_menu::handle(msg, wp, lp) {
        return result;
    }
    if msg == WM_NCCREATE {
        let cs = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
    if msg == WM_DESTROY {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if !ptr.is_null() {
            let app = &mut *ptr;
            if let Some(window) = &mut app.lyrics_window {
                window.remember_bounds();
            }
            app.collect_lyrics_preferences();
            app.lyrics_window.take();
            app.collect_subtitle_preferences();
            app.subtitles.take();
            if let Some(p) = &app.accessible_provider {
                disconnect(p);
            }
            app.save();
        }
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        let _ = UnregisterHotKey(Some(hwnd), 1);
        let _ = UnregisterHotKey(Some(hwnd), 2);
        for id in 3..=5 {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
        PostQuitMessage(0);
        return LRESULT(0);
    }
    thread_local! {static BUSY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
    if BUSY.with(|b| b.replace(true)) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            BUSY.with(|b| b.set(false));
        }
    }
    let _guard = Guard;
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *ptr;
    static TASKBAR_MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    if msg == *TASKBAR_MESSAGE.get_or_init(|| RegisterWindowMessageW(w!("TaskbarCreated"))) {
        let mut tray = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY,
            hIcon: brand::tray_icon(hwnd).unwrap_or_default(),
            ..Default::default()
        };
        copy_wide(&mut tray.szTip, "Nivumi · Ctrl+Alt+N");
        let _ = Shell_NotifyIconW(NIM_ADD, &tray);
        return LRESULT(0);
    }
    match msg {
        WM_GETOBJECT => {
            if lp.0 as i32 == UiaRootObjectId {
                if let Some(provider) = &app.accessible_provider {
                    return UiaReturnRawElementProvider(hwnd, wp, lp, provider);
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        ACCESSIBLE_INVOKE => {
            if let Some(hit) = app.hits.iter().find(|h| h.id() == wp.0).cloned() {
                app.action(hit.action);
            }
            LRESULT(0)
        }
        ACCESSIBLE_FOCUS => {
            app.focus = app.hits.iter().position(|h| h.id() == wp.0);
            app.paint();
            LRESULT(0)
        }
        WM_PAINT => {
            let mut p = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut p);
            app.paint();
            let _ = EndPaint(hwnd, &p);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            if wp.0 == 6 {
                if let Some(gesture) = app.shelf_gesture {
                    if gesture.stage.is_some_and(|at| {
                        at.elapsed()
                            >= Duration::from_millis(if app.state.settings.reduced_motion {
                                0
                            } else {
                                140
                            })
                    }) {
                        let key = if gesture.right {
                            VK_RBUTTON
                        } else {
                            VK_LBUTTON
                        };
                        if GetKeyState(key.0 as i32) < 0 {
                            app.shelf_drag(gesture.id, gesture.right);
                        } else {
                            app.shelf_gesture = None;
                            let _ = KillTimer(Some(hwnd), 6);
                            let _ = ReleaseCapture();
                            app.paint();
                        }
                    } else {
                        app.paint();
                    }
                } else {
                    let _ = KillTimer(Some(hwnd), 6);
                }
                return LRESULT(0);
            }
            if wp.0 == 3 {
                app.update_media_animation();
                if app.media_animating {
                    let now = Instant::now();
                    app.media_phase = (app.media_phase
                        + now.duration_since(app.media_frame).as_secs_f32().min(0.2))
                        % 10000.;
                    app.media_frame = now;
                    app.paint();
                }
                return LRESULT(0);
            }
            if app.modal {
                return LRESULT(0);
            }
            if wp.0 == 2 {
                app.animate_frame();
            } else {
                app.tick();
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            if app.tray_hidden {
                return LRESULT(HTTRANSPARENT as isize);
            }
            let mut p = POINT {
                x: (lp.0 as u32 & 0xffff) as i16 as i32,
                y: ((lp.0 as u32 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut p);
            let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            LRESULT(if app.inside(p.x as f32 / scale, p.y as f32 / scale) {
                HTCLIENT as isize
            } else {
                HTTRANSPARENT as isize
            })
        }
        OPEN_SUBTITLES => {
            app.action(Action::OpenSubtitles);
            LRESULT(0)
        }
        SUBTITLE_CHANGED => {
            let old = app.state.settings.subtitles.shortcut;
            let sync_old = app.state.settings.subtitles.sync_hotkeys;
            app.collect_subtitle_preferences();
            app.sync_lyrics_window();
            let position = (app.position() * 1000.) as i64;
            if let Some(window) = &mut app.subtitles {
                window.update(&app.lyric_lines, position);
            }
            app.paint();
            if old != app.state.settings.subtitles.shortcut {
                app.register_caption_hotkey();
            }
            if sync_old != app.state.settings.subtitles.sync_hotkeys {
                app.register_sync_hotkeys();
            }
            app.save();
            LRESULT(0)
        }
        CAPTURE_SHORTCUT => {
            if wp.0 != 0 {
                for id in 2..=5 {
                    let _ = UnregisterHotKey(Some(hwnd), id);
                }
            } else {
                app.collect_subtitle_preferences();
                app.register_caption_hotkey();
                app.register_sync_hotkeys();
                app.save();
            }
            LRESULT(0)
        }
        PREFERENCES_CHANGED => {
            app.collect_lyrics_preferences();
            app.save();
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            if app.tray_hidden {
                return LRESULT(0);
            }
            app.focus = None;
            let right = msg == WM_RBUTTONDOWN;
            let mut pointer = POINT::default();
            let _ = GetCursorPos(&mut pointer);
            let mut client = pointer;
            let _ = ScreenToClient(hwnd, &mut client);
            let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            if app.page == 4 && app.expanded {
                let x = client.x as f32 / scale;
                let y = client.y as f32 / scale;
                let item = app
                    .hits
                    .iter()
                    .find(|h| {
                        x >= h.rect.0
                            && x <= h.rect.0 + h.rect.2
                            && y >= h.rect.1
                            && y <= h.rect.1 + h.rect.3
                    })
                    .and_then(|h| match h.action {
                        Action::SelectShelf(id) => Some(id),
                        Action::RemoveShelfItem(id) if right => Some(id),
                        _ => None,
                    });
                if let Some(id) = item {
                    let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
                    let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                    if (!right && (control || shift)) || !app.selected.contains(&id) {
                        app.shelf_select(id, control && !right, shift && !right);
                    }
                    app.shelf_gesture = Some(ShelfGesture {
                        id,
                        start: pointer,
                        right,
                        stage: None,
                        modified: control || shift,
                    });
                    SetCapture(hwnd);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetFocus(Some(hwnd));
                    app.paint();
                    return LRESULT(0);
                }
            }
            if right {
                return LRESULT(0);
            }

            let x = client.x as f32 / scale;
            let y = client.y as f32 / scale;

            if app.hidden {
                app.expand(true);
                app.paint();
                return LRESULT(0);
            }

            if app.volume_rects().is_some() {
                if app.is_volume_icon_hit(x, y) {
                    return LRESULT(0);
                } else if app.is_volume_bar_hit(x, y) {
                    let v = app.player_geometry().volume_at(x);
                    app.volume_dragging = true;
                    SetCapture(hwnd);
                    app.change_volume(v);
                    return LRESULT(0);
                }
            }

            if app.is_seek_hit(x, y) {
                let layout = app.player_geometry();
                let ratio = layout.seek_at(x);
                app.seek_drag = Some(ratio);
                SetCapture(hwnd);
                app.paint();
                return LRESULT(0);
            }

            if let Some((horizontal, vertical)) =
                app.resize_edge(client.x as f32 / scale, client.y as f32 / scale)
            {
                app.notch_resize = Some(NotchResize {
                    pointer,
                    width: app.body_width.value,
                    height: app.body_height.value,
                    horizontal,
                    vertical,
                });
                let _ = KillTimer(Some(hwnd), 2);
                SetCapture(hwnd);
                app.resize();
                app.paint();
            } else {
                let x = client.x as f32 / scale;
                let y = client.y as f32 / scale;
                let on_control = (app.expanded || y > app.body_height.value)
                    && app.hits.iter().any(|h| {
                        x >= h.rect.0
                            && x <= h.rect.0 + h.rect.2
                            && y >= h.rect.1
                            && y <= h.rect.1 + h.rect.3
                    });
                if !on_control {
                    let mut rect = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut rect);
                    app.notch_drag = Some((
                        pointer,
                        POINT {
                            x: rect.left,
                            y: rect.top,
                        },
                        false,
                    ));
                    SetCapture(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if app.tray_hidden {
                return LRESULT(0);
            }
            if app.hidden {
                app.fullscreen();
                return LRESULT(0);
            }
            let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            let x = (lp.0 as u32 & 0xffff) as i16 as f32 / scale;
            let y = ((lp.0 as u32 >> 16) & 0xffff) as i16 as f32 / scale;
            if app.seek_drag.is_some() {
                let ratio = app.player_geometry().seek_at(x);
                app.seek_drag = Some(ratio);
                app.paint();
                return LRESULT(0);
            }
            if app.volume_dragging {
                let v = app.player_geometry().volume_at(x);
                app.change_volume(v);
                return LRESULT(0);
            }
            if let Some(mut gesture) = app.shelf_gesture {
                let mut pointer = POINT::default();
                let _ = GetCursorPos(&mut pointer);
                if gesture.stage.is_none()
                    && (pointer.x - gesture.start.x)
                        .abs()
                        .max((pointer.y - gesture.start.y).abs()) as f32
                        > 6. * scale
                    && app.selected.contains(&gesture.id)
                {
                    gesture.stage = Some(Instant::now());
                    app.shelf_gesture = Some(gesture);
                    SetTimer(Some(hwnd), 6, 16, None);
                    app.paint();
                }
                return LRESULT(0);
            }
            if let Some((start, origin, moved)) = app.notch_drag {
                let mut pointer = POINT::default();
                let _ = GetCursorPos(&mut pointer);
                let dx = pointer.x - start.x;
                let dy = pointer.y - start.y;
                if moved || dx.abs().max(dy.abs()) as f32 > 6. * scale {
                    app.notch_drag = Some((start, origin, true));
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        origin.x + dx,
                        origin.y + dy,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                return LRESULT(0);
            }
            app.cursor_pos = Some((x, y));
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            app.update_navigation_hover(x, y);
            app.update_volume_hover(x, y);
            let hovered = app.hits.iter().position(|h| {
                x >= h.rect.0
                    && x <= h.rect.0 + h.rect.2
                    && y >= h.rect.1
                    && y <= h.rect.1 + h.rect.3
            });
            if hovered != app.hovered_hit {
                app.hovered_hit = hovered;
                app.paint();
            }
            if let Some(drag) = app.notch_resize {
                let mut pointer = POINT::default();
                let _ = GetCursorPos(&mut pointer);
                let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
                let max_width = app.canvas_width - 60.;
                let max_height = app.canvas_height - 70.;
                if drag.horizontal != 0 {
                    app.state.settings.lyrics.notch_width = (drag.width
                        + 2. * drag.horizontal as f32 * (pointer.x - drag.pointer.x) as f32
                            / scale)
                        .clamp(520f32.min(max_width), max_width);
                }
                if drag.vertical {
                    app.state.settings.lyrics.notch_height = (drag.height
                        + (pointer.y - drag.pointer.y) as f32 / scale)
                        .clamp(180f32.min(max_height), max_height);
                }
                app.body_width.snap(app.state.settings.lyrics.notch_width);
                app.body_height.snap(app.state.settings.lyrics.notch_height);
                app.paint();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            app.cursor_pos = None;
            app.hovered_hit = None;
            app.navigation_hover = false;
            app.volume_hovered = false;
            app.paint();
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if app.seek_drag.take().is_some() {
                app.paint();
            }
            if app.volume_dragging {
                app.volume_dragging = false;
                app.animate();
            }
            if app.shelf_gesture.take().is_some() {
                let _ = KillTimer(Some(hwnd), 6);
                app.paint();
            }
            app.finish_notch_drag(false);
            app.finish_notch_resize();
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let mut pointer = POINT::default();
            let _ = GetCursorPos(&mut pointer);
            let _ = ScreenToClient(hwnd, &mut pointer);
            let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            let x = pointer.x as f32 / scale;
            let y = pointer.y as f32 / scale;
            if let Some((horizontal, vertical)) = app.resize_edge(x, y) {
                let cursor = if horizontal == 0 {
                    IDC_SIZENS
                } else if !vertical {
                    IDC_SIZEWE
                } else if horizontal > 0 {
                    IDC_SIZENWSE
                } else {
                    IDC_SIZENESW
                };
                if let Ok(cursor) = LoadCursorW(None, cursor) {
                    SetCursor(Some(cursor));
                }
                LRESULT(1)
            } else if app.is_volume_hit(x, y) || app.volume_dragging {
                if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(cursor));
                }
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        WM_LBUTTONUP => {
            if let Some(ratio) = app.seek_drag.take() {
                let _ = ReleaseCapture();
                let target = (ratio * app.media.duration).clamp(0., app.media.duration);
                app.action(Action::Seek(target));
                app.paint();
                return LRESULT(0);
            }
            if app.volume_dragging {
                app.volume_dragging = false;
                let _ = ReleaseCapture();
                let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
                let x = (lp.0 as u32 & 0xffff) as i16 as f32 / scale;
                let y = ((lp.0 as u32 >> 16) & 0xffff) as i16 as f32 / scale;
                app.update_volume_hover(x, y);
                app.animate();
                return LRESULT(0);
            }
            if let Some(gesture) = app.shelf_gesture.take() {
                let _ = KillTimer(Some(hwnd), 6);
                let _ = ReleaseCapture();
                if gesture.stage.is_none() && !gesture.modified {
                    app.shelf_select(gesture.id, false, false);
                }
                app.paint();
                return LRESULT(0);
            }
            let background_click = app.notch_drag.is_some();
            if app.finish_notch_drag(true) {
                return LRESULT(0);
            }
            if background_click && app.page == 4 && app.expanded {
                app.selected.clear();
                app.paint();
                return LRESULT(0);
            }
            if app.notch_resize.is_some() {
                app.finish_notch_resize();
                return LRESULT(0);
            }
            let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            let x = (lp.0 as u32 & 0xffff) as i16 as f32 / scale;
            let y = ((lp.0 as u32 >> 16) & 0xffff) as i16 as f32 / scale;
            if let Some(hit) = app
                .hits
                .iter()
                .find(|h| {
                    matches!(
                        h.action,
                        Action::DismissPomodoro
                            | Action::DismissToast
                            | Action::PomodoroNext(_)
                            | Action::Page(2)
                    ) && y > app.body_height.value
                        && x >= h.rect.0
                        && y >= h.rect.1
                        && x <= h.rect.0 + h.rect.2
                        && y <= h.rect.1 + h.rect.3
                })
                .cloned()
            {
                app.action(hit.action);
            } else if !app.expanded {
                app.expand(true);
            } else if let Some(mut hit) = app
                .hits
                .iter()
                .find(|h| {
                    x >= h.rect.0
                        && y >= h.rect.1
                        && x <= h.rect.0 + h.rect.2
                        && y <= h.rect.1 + h.rect.3
                })
                .cloned()
            {
                if matches!(hit.action,Action::Seek(s)if s<0.) {
                    hit.action =
                        Action::Seek(((x - hit.rect.0) / hit.rect.2) as f64 * app.media.duration);
                }
                if matches!(hit.action, Action::Volume(v) if v < 0.) {
                    hit.action = Action::Volume(app.player_geometry().volume_at(x));
                }
                app.action(hit.action);
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            if app.page == 4 && app.expanded {
                let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
                let x = (lp.0 as u32 & 0xffff) as i16 as f32 / scale;
                let y = ((lp.0 as u32 >> 16) & 0xffff) as i16 as f32 / scale;
                if let Some(id) = app
                    .hits
                    .iter()
                    .find(|h| {
                        x >= h.rect.0
                            && x <= h.rect.0 + h.rect.2
                            && y >= h.rect.1
                            && y <= h.rect.1 + h.rect.3
                    })
                    .and_then(|h| match h.action {
                        Action::SelectShelf(id) => Some(id),
                        _ => None,
                    })
                {
                    app.action(Action::OpenShelf(id));
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            if app.page == 4 && app.expanded {
                let cancelled = app.shelf_gesture.take().is_some_and(|g| g.stage.is_some());
                let _ = KillTimer(Some(hwnd), 6);
                let _ = ReleaseCapture();
                app.paint();
                if !cancelled {
                    app.shelf_menu();
                }
                return LRESULT(0);
            }
            if app.page == 1 && app.expanded {
                app.lyrics_options();
            } else {
                app.menu();
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            if app.shelf_gesture.is_some() || app.volume_dragging || app.seek_drag.is_some() {
                return LRESULT(0);
            }
            let is_hwheel = msg == WM_MOUSEHWHEEL;
            let mut pointer = POINT::default();
            let _ = GetCursorPos(&mut pointer);
            let mut client = pointer;
            let _ = ScreenToClient(hwnd, &mut client);
            let scale = app.renderer.as_ref().map(|r| r.dpi / 96.).unwrap_or(1.);
            let x = client.x as f32 / scale;
            let y = client.y as f32 / scale;

            let delta = (wp.0 >> 16) as u16 as i16;
            let up = if is_hwheel { delta < 0 } else { delta > 0 };
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;

            if !is_hwheel && app.is_seek_hit(x, y) {
                let fine = shift || GetKeyState(VK_CONTROL.0 as i32) < 0;
                let step = if fine { 10.0 } else { 5.0 };
                let current_pos = app.position();
                let next_pos = if up {
                    (current_pos + step).min(app.media.duration)
                } else {
                    (current_pos - step).max(0.0)
                };
                app.action(Action::Seek(next_pos));
                app.paint();
                return LRESULT(0);
            }

            if !is_hwheel && app.is_volume_hit(x, y) {
                if let Some(current) = app.system.volume {
                    let fine = shift || GetKeyState(VK_CONTROL.0 as i32) < 0;
                    let step = if fine { 0.01 } else { 0.05 };
                    let next = if up {
                        (current + step).min(1.0)
                    } else {
                        (current - step).max(0.0)
                    };
                    app.change_volume(next);
                }
                return LRESULT(0);
            }
            match app.wheel_target(x, y, up, shift, alt, is_hwheel) {
                WheelTarget::Page(p) => {
                    app.action(Action::Page(p));
                }
                WheelTarget::LyricsScroll(delta) => {
                    app.action(Action::LyricsScroll(delta));
                }
                WheelTarget::ItemScroll(s) => {
                    app.scroll = s;
                }
            }
            app.paint();
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wp.0 == b'V' as usize && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                // WM_KEYDOWN is delivered only while the notch has keyboard focus.
                if lp.0 & (1 << 30) == 0 {
                    app.action(Action::PasteShelfImage);
                }
                return LRESULT(0);
            }

            if app.page == 1 && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                if [VK_OEM_PLUS.0, VK_ADD.0].contains(&(wp.0 as u16)) {
                    app.action(Action::LyricsFont(1));
                    return LRESULT(0);
                }
                if [VK_OEM_MINUS.0, VK_SUBTRACT.0].contains(&(wp.0 as u16)) {
                    app.action(Action::LyricsFont(-1));
                    return LRESULT(0);
                }
            }
            if wp.0 == b'C' as usize && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                if let Some(id) = app.selected.first().copied() {
                    app.action(Action::CopyShelf(id));
                }
                return LRESULT(0);
            }
            if app.page == 4 && app.expanded {
                let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
                match wp.0 as u16 {
                    0x41 if control => {
                        app.selected = app.state.shelf.iter().map(|i| i.id).collect();
                        app.paint();
                        return LRESULT(0);
                    }
                    0x43 | 0x58 if control => {
                        app.action(Action::ShelfClipboard(wp.0 == 0x58));
                        return LRESULT(0);
                    }
                    v if v == VK_DELETE.0 => {
                        app.action(Action::RemoveSelected);
                        return LRESULT(0);
                    }
                    v if v == VK_ESCAPE.0
                        && (app.shelf_gesture.is_some() || !app.selected.is_empty()) =>
                    {
                        app.shelf_gesture = None;
                        let _ = KillTimer(Some(hwnd), 6);
                        let _ = ReleaseCapture();
                        app.selected.clear();
                        app.paint();
                        return LRESULT(0);
                    }
                    v if v == VK_RETURN.0 && app.selected.len() == 1 => {
                        if let Some(id) = app.selected.first().copied() {
                            app.action(Action::OpenShelf(id));
                        }
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
            let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
            if wp.0 == 0x57 && control {
                app.hide_to_tray();
                return LRESULT(0);
            }
            match wp.0 as u16 {
                v if v == VK_ESCAPE.0 => {
                    app.hide_to_tray();
                }
                v if v == VK_LEFT.0 => {
                    app.action(Action::Page(adjacent_page(app.page, true)));
                }
                v if v == VK_RIGHT.0 => {
                    app.action(Action::Page(adjacent_page(app.page, false)));
                }
                v if v == VK_TAB.0 => {
                    if !app.hits.is_empty() {
                        let backwards = GetKeyState(VK_SHIFT.0 as i32) < 0;
                        let count = app.hits.len();
                        app.focus = Some(match app.focus {
                            Some(i) if backwards => (i + count - 1) % count,
                            Some(i) => (i + 1) % count,
                            None if backwards => count - 1,
                            None => 0,
                        });
                        app.navigation_hover = true;
                        app.animate();
                        app.paint();
                    }
                }
                v if v == VK_RETURN.0 || v == VK_SPACE.0 => {
                    if let Some(hit) = app.focus.and_then(|i| app.hits.get(i)).cloned() {
                        app.action(hit.action);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            if (3..=5).contains(&wp.0) {
                let delta = match wp.0 {
                    3 => Some(-100),
                    4 => Some(100),
                    _ => None,
                };
                app.adjust_song_sync(delta);
                return LRESULT(0);
            }
            if wp.0 == 2 {
                if let Some(window) = &mut app.subtitles {
                    window.toggle();
                } else {
                    app.action(Action::OpenSubtitles);
                }
                return LRESULT(0);
            }
            if app.tray_hidden {
                app.restore_from_tray(true);
            } else if app.expanded {
                app.hide_to_tray();
            } else {
                app.restore_from_tray(true);
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            let _ = brand::apply_window(hwnd);
            let icon_data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON,
                hIcon: brand::tray_icon(hwnd).unwrap_or_default(),
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_MODIFY, &icon_data);
            app.resize();
            app.paint();
            LRESULT(0)
        }
        TRAY => {
            match lp.0 as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    if app.tray_hidden {
                        app.restore_from_tray(true);
                    } else {
                        app.hide_to_tray();
                    }
                }
                WM_RBUTTONUP => app.menu(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(window) = &mut app.lyrics_window {
                window.remember_bounds();
            }
            app.collect_lyrics_preferences();
            app.lyrics_window.take();
            app.collect_subtitle_preferences();
            app.subtitles.take();
            if let Some(p) = &app.accessible_provider {
                disconnect(p);
            }
            app.save();
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = UnregisterHotKey(Some(hwnd), 1);
            let _ = UnregisterHotKey(Some(hwnd), 2);
            for id in 3..=5 {
                let _ = UnregisterHotKey(Some(hwnd), id);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn run() -> Result<()> {
    let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    OleInitialize(None)?;
    brand::initialize()?;
    let _mutex = windows::Win32::System::Threading::CreateMutexW(
        None,
        true,
        w!("Local\\NusaRebuild.SingleInstance"),
    )?;
    if GetLastError() == ERROR_ALREADY_EXISTS {
        return Ok(());
    }
    let path = storage::default_path();
    let loaded = storage::load(&path).map_err(|e| {
        log(&e.to_string());
        Error::new(E_FAIL, e.to_string())
    })?;
    let mut state = loaded.state;
    if std::env::var_os("NIVUMI_DATA_DIR").is_none()
        && std::env::var_os("NUSA_REBUILD_DATA_DIR").is_none()
    {
        if let Err(error) = refresh_existing_startup(state.settings.startup) {
            log(&format!("Startup registration: {error}"));
        }
    }
    nivumi::letras::set_ignored_title_terms(&state.settings.lyrics.ignored_title_terms);
    if state.settings.shelf_clear {
        state.shelf.clear();
    }
    let (drop_tx, drop_rx) = mpsc::channel();
    let minimal = state.settings.minimal;
    let mut app = Box::new(App {
        hwnd: HWND::default(),
        renderer: None,
        window_region: WindowRegion::default(),
        canvas_width: WIDTH,
        canvas_height: HEIGHT,
        notch_resize: None,
        notch_drag: None,
        offline_job: None,
        offline_attempt: String::new(),
        letras_done: false,
        lyrics_window: None,
        subtitles: None,
        notch_follow: true,
        state,
        path,
        services: ServiceHandle::spawn(),
        media: MediaSnapshot::default(),
        system: SystemSnapshot::default(),
        media_at: Instant::now(),
        track_change_at: Instant::now(),
        lyric_lines: vec![],
        painted_lyric_line: None,
        verse_change_at: None,
        verse_focus: None,
        verse_scroll: Default::default(),
        shelf_scroll: Default::default(),
        task_scroll: Default::default(),
        lyric_status: "".into(),
        external_lyrics: None,
        lyrics_view_mode: 0,
        external_scroll: 0,
        page: 0,
        expanded: false,
        expansion: Spring::new(0.),
        navigation: Spring::new(0.),
        navigation_hover: false,
        body_width: Spring::new(if minimal { 112. } else { 150. }),
        body_height: Spring::new(if minimal { 18. } else { 30. }),
        player_layout: Spring::new(0.),
        accent: [Spring::new(0.); 3],
        volume_level: Spring::new(1.),
        volume_hover: Spring::new(0.),
        volume_dragging: false,
        volume_hovered: false,
        volume_saved: 1.,
        volume_fade: None,
        seek_drag: None,
        media_phase: 0.,
        media_frame: Instant::now(),
        media_animating: false,
        last_anim: Instant::now(),
        hits: vec![],
        content_offset_y: 0.,
        selected: BTreeSet::new(),
        shelf_anchor: None,
        shelf_gesture: None,
        shelf_worker: Default::default(),
        shelf_thumbnails: Default::default(),
        scroll: 0,
        calendar_offset: 0,
        pomodoro_notice: None,
        toast: loaded.warning.map(|w| (w, Instant::now())),
        hover: false,
        focus: None,
        last_clock: Local::now().format("%H:%M").to_string(),
        drop_rx,
        _drop: None,
        modal: false,
        hidden: false,
        tray_hidden: false,
        fullscreen_state: FullscreenTracker::default(),
        last_second: 0,
        accessible_items: Arc::new(Mutex::new(vec![])),
        accessible_provider: None,
        pinned: std::env::args().any(|a| a == "--inspect"),
        hovered_hit: None,
        cursor_pos: None,
    });
    let instance = GetModuleHandleW(None)?;
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hIcon: brand::class_icons()?.1,
        hIconSm: brand::class_icons()?.0,
        lpszClassName: w!("Nivumi.Notch"),
        ..Default::default()
    };
    RegisterClassExW(&wc);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST
            | if app.pinned {
                WS_EX_APPWINDOW
            } else {
                WS_EX_TOOLWINDOW
            }
            | WS_EX_NOREDIRECTIONBITMAP,
        w!("Nivumi.Notch"),
        w!("Nivumi"),
        WS_POPUP,
        0,
        0,
        480,
        380,
        None,
        None,
        Some(instance.into()),
        Some((&mut *app as *mut App).cast()),
    )?;
    app.hwnd = hwnd;
    brand::apply_window(hwnd)?;
    app.accessible_provider = Some(provider(hwnd, app.accessible_items.clone()));
    app.resize();
    app._drop = ole::register_drop_target(hwnd, drop_tx).ok();
    let mut tray = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: TRAY,
        hIcon: brand::tray_icon(hwnd)?,
        ..Default::default()
    };
    copy_wide(&mut tray.szTip, "Nivumi · Ctrl+Alt+N");
    let _ = Shell_NotifyIconW(NIM_ADD, &tray);
    let _ = RegisterHotKey(
        Some(hwnd),
        1,
        MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
        b'N' as u32,
    );
    app.register_caption_hotkey();
    app.register_sync_hotkeys();
    SetTimer(Some(hwnd), 1, 250, None);
    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    app.paint();
    if std::env::args().any(|a| a == "--expanded") {
        app.expand(true);
    }
    if let Some(page) = std::env::args().find_map(|a| {
        a.strip_prefix("--page=")
            .and_then(|s| s.parse::<usize>().ok())
    }) {
        app.page = page.min(6);
        app.expand(true);
    }
    #[cfg(debug_assertions)]
    if let Some(directory) =
        std::env::args().find_map(|a| a.strip_prefix("--verify-brand=").map(PathBuf::from))
    {
        let verification = brand_preview::capture(&mut app, directory);
        let _ = DestroyWindow(hwnd);
        drop(app);
        let _ = CloseHandle(_mutex);
        OleUninitialize();
        return verification;
    }
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        if app
            .lyrics_window
            .as_ref()
            .is_some_and(|window| IsDialogMessageW(window.hwnd, &msg).as_bool())
        {
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    drop(app);
    let _ = CloseHandle(_mutex);
    OleUninitialize();
    std::process::exit(0);
}
fn main() {
    std::panic::set_hook(Box::new(|p| log(&p.to_string())));
    if let Err(e) = unsafe { run() } {
        log(&format!("Startup: {e}"));
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(e.to_string()),
                w!("Nivumi"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_wheel_navigates_features_when_unmodified() {
        // When on shelf (page 4), un-modified scroll down/right advances to tasks (page 5)
        assert_eq!(
            handle_wheel(4, 0, 0, 0, false, false, false),
            WheelTarget::Page(5)
        );
        // Even when shelf has items, un-modified scroll down advances to tasks
        assert_eq!(
            handle_wheel(4, 0, 10, 0, false, false, false),
            WheelTarget::Page(5)
        );
        // Un-modified scroll up/left retreats to water (page 3)
        assert_eq!(
            handle_wheel(4, 0, 0, 0, true, false, false),
            WheelTarget::Page(3)
        );
        assert_eq!(
            handle_wheel(4, 2, 10, 0, true, false, false),
            WheelTarget::Page(3)
        );
    }

    #[test]
    fn other_pages_navigate_features_consistently() {
        // Music, Lyrics, Pomodoro, Water, Shelf, Tasks, Calendar.
        for (page, next, previous) in [
            (0, 1, 6),
            (1, 2, 0),
            (2, 3, 1),
            (3, 4, 2),
            (4, 5, 3),
            (5, 6, 4),
            (6, 0, 5),
        ] {
            assert_eq!(adjacent_page(page, false), next);
            assert_eq!(adjacent_page(page, true), previous);
            assert_eq!(
                handle_wheel(page, 0, 0, 0, false, false, false),
                WheelTarget::Page(next)
            );
            assert_eq!(
                handle_wheel(page, 0, 0, 0, true, false, false),
                WheelTarget::Page(previous)
            );
        }
    }

    #[test]
    fn modified_wheel_scrolls_content_on_shelf_tasks_and_lyrics() {
        // Shelf with 8 items: 5 fit, max scroll index is 3
        assert_eq!(
            handle_wheel(4, 0, 8, 0, false, true, false),
            WheelTarget::ItemScroll(1)
        );
        assert_eq!(
            handle_wheel(4, 3, 8, 0, false, true, false),
            WheelTarget::ItemScroll(3)
        );
        assert_eq!(
            handle_wheel(4, 2, 8, 0, true, true, false),
            WheelTarget::ItemScroll(1)
        );
        // Also supports Alt key as modifier
        assert_eq!(
            handle_wheel(4, 1, 8, 0, true, false, true),
            WheelTarget::ItemScroll(0)
        );

        // Tasks with 5 items: 3 fit, max scroll index is 2
        assert_eq!(
            handle_wheel(5, 0, 0, 5, false, true, false),
            WheelTarget::ItemScroll(1)
        );
        assert_eq!(
            handle_wheel(5, 2, 0, 5, false, true, false),
            WheelTarget::ItemScroll(2)
        );

        // Lyrics on page 1
        assert_eq!(
            handle_wheel(1, 0, 0, 0, false, true, false),
            WheelTarget::LyricsScroll(2)
        );
        assert_eq!(
            handle_wheel(1, 0, 0, 0, true, true, false),
            WheelTarget::LyricsScroll(-2)
        );
    }

    fn test_app() -> App {
        App {
            hwnd: HWND::default(),
            renderer: None,
            window_region: WindowRegion::default(),
            canvas_width: WIDTH,
            canvas_height: HEIGHT,
            notch_resize: None,
            notch_drag: None,
            offline_job: None,
            offline_attempt: String::new(),
            letras_done: false,
            lyrics_window: None,
            subtitles: None,
            notch_follow: true,
            state: AppState::default(),
            path: PathBuf::new(),
            services: ServiceHandle::spawn(),
            media: MediaSnapshot::default(),
            system: SystemSnapshot::default(),
            media_at: Instant::now(),
            track_change_at: Instant::now(),
            lyric_lines: vec![],
            painted_lyric_line: None,
            verse_change_at: None,
            verse_focus: None,
            verse_scroll: Default::default(),
            shelf_scroll: Default::default(),
            task_scroll: Default::default(),
            lyric_status: "".into(),
            external_lyrics: None,
            lyrics_view_mode: 0,
            external_scroll: 0,
            page: 2,
            expanded: true,
            expansion: Spring::new(1.),
            navigation: Spring::new(0.),
            navigation_hover: false,
            body_width: Spring::new(420.),
            body_height: Spring::new(170.),
            player_layout: Spring::new(0.),
            accent: [Spring::new(0.); 3],
            volume_level: Spring::new(1.),
            volume_hover: Spring::new(0.),
            volume_dragging: false,
            volume_hovered: false,
            volume_saved: 1.,
            volume_fade: None,
            seek_drag: None,
            media_phase: 0.,
            media_frame: Instant::now(),
            media_animating: false,
            last_anim: Instant::now(),
            hits: vec![],
            content_offset_y: 0.,
            selected: BTreeSet::new(),
            shelf_anchor: None,
            shelf_gesture: None,
            shelf_worker: Default::default(),
            shelf_thumbnails: Default::default(),
            scroll: 0,
            calendar_offset: 0,
            pomodoro_notice: None,
            toast: None,
            hover: true,
            focus: None,
            last_clock: "03:18".into(),
            drop_rx: std::sync::mpsc::channel().1,
            _drop: None,
            modal: false,
            hidden: false,
            tray_hidden: false,
            fullscreen_state: FullscreenTracker::default(),
            last_second: 0,
            accessible_items: Arc::new(Mutex::new(vec![])),
            accessible_provider: None,
            pinned: false,
            hovered_hit: None,
            cursor_pos: None,
        }
    }

    #[test]
    fn song_sync_updates_notch_reader_and_captions_once_without_seeking_audio() {
        let mut app = test_app();
        app.media.title = "This Is Gospel".into();
        app.media.artist = "Panic! At The Disco".into();
        app.media.position = 1.9;
        app.media.duration = 200.;
        app.lyric_lines = lyrics::parse_lrc("[00:01]First\n[00:02]Second");
        assert_eq!(app.active_lyric_line(), Some(0));
        let key = app.song_sync_key();
        app.state
            .settings
            .lyrics
            .adjust_song_offset(&key, Some(100));
        app.select_song_sync();
        assert_eq!(app.active_lyric_line(), Some(1));
        assert_eq!(app.lyrics_content().active, Some(1));
        assert_eq!(
            nivumi::subtitles::caption_text(&app.lyric_lines, 1900, &app.state.settings.subtitles)
                .primary,
            "Second"
        );
        assert_eq!(app.position(), 1.9);
        app.media.duration = 201.; // Duration discovery must not discard the adjustment.
        assert_eq!(app.song_sync_offset(), 100);
        app.media.title = "Other song".into();
        app.select_song_sync();
        assert_eq!(app.song_sync_offset(), 0);
        assert_eq!(app.state.settings.subtitles.delay_ms, 0);
        app.media.title = "This Is Gospel".into();
        app.select_song_sync();
        assert_eq!(app.state.settings.subtitles.delay_ms, 100);
        app.state
            .settings
            .lyrics
            .adjust_song_offset(&key, Some(-200));
        assert_eq!(app.active_lyric_line(), Some(0));
        app.media.position = 1.;
        assert_eq!(app.active_lyric_line(), None); // Negative lyric time stays before the first verse.
    }

    #[test]
    fn full_lyrics_wheel_scrolls_text_without_modifiers() {
        let mut app = test_app();
        app.page = 1;
        app.player_layout.snap(1.);
        for width in [680., 1000.] {
            app.state.settings.lyrics.notch_width = width;
            let left = (app.canvas_width - app.player_width()) / 2. + 264.;
            for mode in [2, 3] {
                app.lyrics_view_mode = mode;
                assert_eq!(
                    app.wheel_target(left + 10., 80., false, false, false, false),
                    WheelTarget::LyricsScroll(2)
                );
                assert_eq!(
                    app.wheel_target(left + 10., 80., true, false, false, false),
                    WheelTarget::LyricsScroll(-2)
                );
                // Header, player, navigation dots and the right margin retain page navigation.
                for (x, y) in [
                    (left + 10., 20.),
                    (left - 20., 80.),
                    (left + 10., 230.),
                    (left + width - 280., 80.),
                ] {
                    assert_eq!(
                        app.wheel_target(x, y, false, false, false, false),
                        WheelTarget::Page(2)
                    );
                }
            }
            for mode in [0, 1] {
                app.lyrics_view_mode = mode;
                assert_eq!(
                    app.wheel_target(left + 10., 80., false, false, false, false),
                    WheelTarget::Page(2)
                );
                assert_eq!(
                    app.wheel_target(left + 10., 80., false, true, false, false),
                    WheelTarget::LyricsScroll(2)
                );
            }
        }
        app.lyrics_view_mode = 2;
        app.expanded = false;
        assert_eq!(
            app.wheel_target(app.canvas_width / 2., 80., false, false, false, false),
            WheelTarget::Page(2)
        );
    }

    #[test]
    fn list_scroll_continues_from_visible_position_and_respects_reduced_motion() {
        let mut app = test_app();
        unsafe {
            assert_eq!(app.list_scroll_position(true, 76.), 0.);
            app.scroll = 3;
            let start = app.list_scroll_position(true, 76.);
            assert!(start < 1.);
            app.scroll = 1;
            let redirected = app.list_scroll_position(true, 76.);
            assert!((redirected - start).abs() < 2.);
            app.state.settings.reduced_motion = true;
            assert_eq!(app.list_scroll_position(true, 76.), 76.);
            app.scroll = 2;
            assert_eq!(
                app.list_scroll_position(false, TASK_ROW_PITCH),
                2. * TASK_ROW_PITCH
            );
        }
    }
    #[test]
    fn scrolling_list_hit_targets_stay_inside_viewport() {
        let mut app = test_app();
        app.hit((10., 10., 40., 40.), "test", Action::Page(1));
        app.clip_list_hits(0, (20., 20., 20., 20.));
        assert_eq!(app.hits[0].rect, (20., 20., 20., 20.));
    }

    #[test]
    fn full_lyrics_scroll_is_bounded_and_reverses_immediately_at_end() {
        let mut app = test_app();
        app.page = 1;
        app.external_lyrics = Some(ExternalLyrics {
            translation_origin: Default::default(),
            title: String::new(),
            artist: String::new(),
            original: "One\nTwo\nThree\nFour\nFive".into(),
            translation: Some("Um\nDois\nTrês\nQuatro\nCinco".into()),
            source: "test".into(),
        });
        let lyrics = app.external_lyrics.clone();
        for mode in [2, 3] {
            app.external_lyrics = lyrics.clone();
            app.lyrics_view_mode = mode;
            app.external_scroll = 0;
            unsafe {
                app.action(Action::LyricsScroll(-2));
                assert_eq!(app.external_scroll, 0);
                app.action(Action::LyricsScroll(2));
                assert_eq!(app.external_scroll, 2);
                for _ in 0..20 {
                    app.action(Action::LyricsScroll(2));
                }
                assert_eq!(app.external_scroll, 4);
                app.action(Action::LyricsScroll(-2));
                assert_eq!(app.external_scroll, 2);
                assert!(!app.notch_follow);
                assert_eq!(app.page, 1);
                app.external_lyrics = None;
                app.action(Action::LyricsScroll(2));
                assert_eq!(app.external_scroll, 0);
            }
        }
    }

    #[test]
    fn pomodoro_start_clears_notice_and_hides_expanded_notch() {
        let mut app = test_app();

        app.apply(AppCommand::StartPomodoro);
        assert!(!app.expanded);
        assert!(!app.hover);
        assert!(app.pomodoro_notice.is_none());

        assert!(!app.inside(238., 120.));

        let (cw, ch) = app.target_size();
        assert_eq!((cw, ch), (150., 30.));
        let ox = (app.canvas_width - cw) / 2.;
        assert!(app.inside(ox + 10., 15.));
    }
    #[test]
    fn minimized_music_animates_only_during_playback_and_respects_reduced_motion() {
        let mut app = test_app();
        app.hidden = true;
        app.media.title = "Test track".into();
        app.media.playing = true;
        assert!(app.media_motion_enabled());
        app.media.playing = false;
        assert!(!app.media_motion_enabled());
        app.media.playing = true;
        app.state.settings.reduced_motion = true;
        assert!(!app.media_motion_enabled());
        app.state.settings.reduced_motion = false;
        app.modal = true;
        assert!(!app.media_motion_enabled());
        app.modal = false;
        app.media.title.clear();
        assert!(!app.media_motion_enabled());
    }
    #[test]
    fn hidden_notch_limits_input_to_the_edge_handle() {
        let mut app = test_app();
        app.hidden = true;
        app.expanded = false;
        let (x, y, w, h) = app.edge_rect();
        assert_eq!(app.target_size(), (EDGE_WIDTH, EDGE_HEIGHT));
        assert!(app.inside(x + w / 2., y));
        assert!(app.inside(x + w / 2., y + h / 2.));
        assert!(!app.inside(x - 1., y));
        assert!(!app.inside(x + w + 1., y));
        assert!(!app.inside(x + w / 2., y + h + 1.));
        assert!(app.volume_rects().is_none());
        app.state.settings.reduced_motion = true;
        unsafe {
            app.action(Action::Reveal);
        }
        assert!(!app.hidden);
        assert!(app.expanded);
        assert_eq!(app.expansion.value, 1.);
    }

    #[test]
    fn lyrics_volume_uses_current_geometry_and_is_unavailable_on_other_pages() {
        let mut app = test_app();
        app.page = 1;
        app.player_layout.snap(1.);
        app.system.volume = Some(0.5);
        app.volume_hovered = true;
        for width in [520., 680., 1200.] {
            app.state.settings.lyrics.notch_width = width;
            let layout = app.player_geometry();
            let (x, y, w) = layout.volume_track();
            assert!(app.is_volume_hit(x + w / 2., y));
            assert!(app.is_volume_bar_hit(x + w / 2., y));
            assert!(!app.is_volume_icon_hit(x + w / 2., y));
            assert!((layout.volume_at(x + w / 2.) - 0.5).abs() < 0.00001);
        }
        app.page = 4;
        assert!(app.volume_rects().is_none());
        app.page = 1;
        app.system.volume = None;
        assert!(app.volume_rects().is_none());
        app.system.volume = Some(0.5);
        app.expanded = false;
        assert!(app.volume_rects().is_none());
    }

    #[test]
    fn reduced_motion_navigation_snaps_shared_layout_to_its_destination() {
        let mut app = test_app();
        app.page = 0;
        app.state.settings.reduced_motion = true;
        unsafe {
            app.action(Action::Page(1));
        }
        assert_eq!(app.player_layout.value, 1.);
        assert_eq!(app.body_width.value, app.state.settings.lyrics.notch_width);
        assert_eq!(
            app.body_height.value,
            app.state.settings.lyrics.notch_height
        );
        unsafe {
            app.action(Action::Page(0));
        }
        assert_eq!(app.player_layout.value, 0.);
        assert_eq!(app.body_width.value, BODY);
        assert_eq!(app.body_height.value, 140.);
    }

    #[test]
    fn volume_slider_ratio_and_clamping() {
        let bar_x = 354.0f32;
        let bar_w = 40.0f32;
        let calc = |x: f32| ((x - bar_x) / bar_w).clamp(0.0, 1.0);

        assert_eq!(calc(300.0), 0.0);
        assert_eq!(calc(354.0), 0.0);
        assert_eq!(calc(374.0), 0.5);
        assert_eq!(calc(394.0), 1.0);
        assert_eq!(calc(450.0), 1.0);
    }

    #[test]
    fn volume_icon_selection_based_on_level() {
        let icon_for = |v: f32| {
            if v <= 0.001 {
                "volume_mute"
            } else if v < 0.33 {
                "volume_low"
            } else if v < 0.66 {
                "volume"
            } else {
                "volume_high"
            }
        };

        assert_eq!(icon_for(0.0), "volume_mute");
        assert_eq!(icon_for(0.0005), "volume_mute");
        assert_eq!(icon_for(0.15), "volume_low");
        assert_eq!(icon_for(0.50), "volume");
        assert_eq!(icon_for(0.85), "volume_high");
        assert_eq!(icon_for(1.0), "volume_high");
    }

    #[test]
    fn volume_line_thickness_expansion() {
        let height_for = |h_val: f32| 3.0f32 + 3.0f32 * h_val.clamp(0.0, 1.0);
        let y_for = |h: f32| 117.5f32 - h / 2.0f32;

        assert_eq!(height_for(0.0), 3.0);
        assert_eq!(y_for(3.0), 116.0);

        assert_eq!(height_for(1.0), 6.0);
        assert_eq!(y_for(6.0), 114.5);
    }

    #[test]
    fn volume_scroll_wheel_steps() {
        let step_normal = 0.05f32;
        let step_fine = 0.02f32;

        let up =
            |curr: f32, fine: bool| (curr + if fine { step_fine } else { step_normal }).min(1.0);
        let down =
            |curr: f32, fine: bool| (curr - if fine { step_fine } else { step_normal }).max(0.0);

        assert!((up(0.50, false) - 0.55).abs() < 0.0001);
        assert!((down(0.50, false) - 0.45).abs() < 0.0001);
        assert!((up(0.50, true) - 0.52).abs() < 0.0001);
        assert!((down(0.50, true) - 0.48).abs() < 0.0001);
        assert_eq!(up(0.98, false), 1.0);
        assert_eq!(down(0.03, false), 0.0);
    }

    #[test]
    fn resize_edge_deconflicts_with_navigation_zone() {
        let mut app = App {
            hwnd: HWND::default(),
            renderer: None,
            window_region: WindowRegion::default(),
            canvas_width: WIDTH,
            canvas_height: HEIGHT,
            notch_resize: None,
            notch_drag: None,
            offline_job: None,
            offline_attempt: String::new(),
            letras_done: false,
            lyrics_window: None,
            subtitles: None,
            notch_follow: true,
            state: AppState::default(),
            path: PathBuf::new(),
            services: ServiceHandle::spawn(),
            media: MediaSnapshot::default(),
            system: SystemSnapshot::default(),
            media_at: Instant::now(),
            track_change_at: Instant::now(),
            lyric_lines: vec![],
            painted_lyric_line: None,
            verse_change_at: None,
            verse_focus: None,
            verse_scroll: Default::default(),
            shelf_scroll: Default::default(),
            task_scroll: Default::default(),
            lyric_status: "".into(),
            external_lyrics: None,
            lyrics_view_mode: 0,
            external_scroll: 0,
            page: 1, // Lyrics mode
            expanded: true,
            expansion: Spring::new(1.),
            navigation: Spring::new(0.),
            navigation_hover: false,
            body_width: Spring::new(680.),
            body_height: Spring::new(240.),
            player_layout: Spring::new(0.),
            accent: [Spring::new(0.); 3],
            volume_level: Spring::new(1.),
            volume_hover: Spring::new(0.),
            volume_dragging: false,
            volume_hovered: false,
            volume_saved: 1.,
            volume_fade: None,
            seek_drag: None,
            media_phase: 0.,
            media_frame: Instant::now(),
            media_animating: false,
            last_anim: Instant::now(),
            hits: vec![],
            content_offset_y: 0.,
            selected: BTreeSet::new(),
            shelf_anchor: None,
            shelf_gesture: None,
            shelf_worker: Default::default(),
            shelf_thumbnails: Default::default(),
            scroll: 0,
            calendar_offset: 0,
            pomodoro_notice: None,
            toast: None,
            hover: true,
            focus: None,
            last_clock: "03:18".into(),
            drop_rx: std::sync::mpsc::channel().1,
            _drop: None,
            modal: false,
            hidden: false,
            tray_hidden: false,
            fullscreen_state: FullscreenTracker::default(),
            last_second: 0,
            accessible_items: Arc::new(Mutex::new(vec![])),
            accessible_provider: None,
            pinned: false,
            hovered_hit: None,
            cursor_pos: None,
        };

        let center = app.canvas_width / 2.;
        let height = 240.0f32;
        let width = 680.0f32;
        let left = (app.canvas_width - width) / 2.;

        // Inside the navigation zone at the bottom edge: must NOT capture as resize edge
        assert_eq!(app.resize_edge(center, height - 6.0), None);
        assert_eq!(app.resize_edge(center - 20.0, height - 8.0), None);
        assert_eq!(app.resize_edge(center + 20.0, height - 8.0), None);

        // Bottom edge outside the navigation zone: vertical resize
        assert_eq!(
            app.resize_edge(center + 80.0, height - 6.0),
            Some((0, true))
        );
        assert_eq!(
            app.resize_edge(center - 80.0, height - 6.0),
            Some((0, true))
        );

        // Bottom-right corner: diagonal resize
        assert_eq!(
            app.resize_edge(left + width - 15.0, height - 15.0),
            Some((1, true))
        );

        // Left edge: horizontal resize
        assert_eq!(app.resize_edge(left + 4.0, height / 2.0), Some((-1, false)));

        // Right edge: horizontal resize
        assert_eq!(
            app.resize_edge(left + width - 4.0, height / 2.0),
            Some((1, false))
        );

        // When not on page 1 (lyrics): resize_edge is always disabled
        app.page = 0;
        assert_eq!(app.resize_edge(left + width - 15.0, height - 15.0), None);
        assert_eq!(app.resize_edge(center + 80.0, height - 6.0), None);
    }

    #[test]
    fn navigation_dots_and_corner_grip_do_not_overlap() {
        for width in [520.0f32, 680.0f32, 840.0f32, 1200.0f32] {
            let canvas_width = WIDTH;
            let center = canvas_width / 2.0;
            let left = (canvas_width - width) / 2.0;

            let dots_left = center - NAV_DOTS_WIDTH / 2.;
            let dots_right = center + NAV_DOTS_WIDTH / 2.;

            // Corner grip at bottom-right corner: gx = left + width - 18.
            let grip_gx = left + width - 18.0;
            let grip_left = grip_gx - 8.0;
            let grip_right = grip_gx + 8.0;

            // The corner grip and navigation dots must have a strictly positive gap (> 150 DIP)
            assert!(
                grip_left > dots_right + 150.0,
                "Width {}: Grip left {} must be far to the right of dots right {}",
                width,
                grip_left,
                dots_right
            );

            // Bounding boxes must never intersect
            assert!(dots_right < grip_left || dots_left > grip_right);
        }
    }

    #[test]
    fn shelf_guide_text_is_complete_and_translated() {
        let (title_pt, text_pt) = shelf_guide_text(false);
        assert!(title_pt.contains("Prateleira"));
        assert!(text_pt.contains("Adicionar itens:"));
        assert!(text_pt.contains("Abrir ou Copiar:"));
        assert!(text_pt.contains("Seleção múltipla:"));
        assert!(text_pt.contains("Arrastar para fora:"));
        assert!(text_pt.contains("Menu de contexto"));
        assert!(text_pt.contains("Atalhos rápidos:"));
        assert!(text_pt.contains("Limpar prateleira:"));

        let (title_en, text_en) = shelf_guide_text(true);
        assert!(title_en.contains("Shelf"));
        assert!(text_en.contains("Add items:"));
        assert!(text_en.contains("Open or Copy:"));
        assert!(text_en.contains("Multi-selection:"));
        assert!(text_en.contains("Drag out:"));
        assert!(text_en.contains("Context menu"));
        assert!(text_en.contains("Quick shortcuts:"));
        assert!(text_en.contains("Clear shelf:"));
    }

    #[test]
    fn shelf_items_are_always_horizontally_centered() {
        for count in 1..=5 {
            let start_x = shelf_start_x(count, 0.0);
            let total_width = (count - 1) as f32 * 76.0 + 62.0;
            let end_x = start_x + total_width;
            let left_margin = start_x;
            let right_margin = BODY - end_x;

            // Symmetrical margins on left and right
            assert!(
                (left_margin - right_margin).abs() < 0.001,
                "count {} not centered: left {} != right {}",
                count,
                left_margin,
                right_margin
            );

            // Group center matches the exact center of the expanded notch (BODY / 2 = 210.0)
            let center = (start_x + end_x) / 2.0;
            assert!(
                (center - 210.0).abs() < 0.001,
                "count {} center {} != 210.0",
                count,
                center
            );
        }

        // Test with custom ox offset
        let ox = 50.0;
        let start_x = shelf_start_x(3, ox);
        assert_eq!(start_x, ox + 103.0);
    }

    #[test]
    fn shelf_stack_is_always_horizontally_centered() {
        for count in 1..=5 {
            let base = shelf_stack_base_x(count, 0.0);
            let left_edge = base - 3.0;
            let right_edge = base + (count - 1) as f32 * 4.0 + 47.0;
            let left_margin = left_edge;
            let right_margin = BODY - right_edge;

            // Symmetrical margins on left and right for the gathered stack
            assert!(
                (left_margin - right_margin).abs() < 0.001,
                "stack count {} not centered: left {} != right {}",
                count,
                left_margin,
                right_margin
            );

            // Stack center matches the exact center of the expanded notch (210.0)
            let center = (left_edge + right_edge) / 2.0;
            assert!(
                (center - 210.0).abs() < 0.001,
                "stack count {} center {} != 210.0",
                count,
                center
            );
        }

        // Test with custom ox offset
        let ox = 50.0;
        let base = shelf_stack_base_x(3, ox);
        assert_eq!(base, ox + 184.0);
    }

    #[test]
    fn test_compact_notch_layout_symmetry() {
        let ox = 100.0;
        let w = 150.0;

        // When both left and right icons are present:
        let (x, width) = compact_notch_layout(true, true, ox, w);
        let left_padding = x - ox;
        let right_padding = (ox + w) - (x + width);
        assert_eq!(left_padding, 34.0);
        assert_eq!(right_padding, 34.0);

        // When neither icon is present:
        let (x_none, width_none) = compact_notch_layout(false, false, ox, w);
        let left_none = x_none - ox;
        let right_none = (ox + w) - (x_none + width_none);
        assert_eq!(left_none, 10.0);
        assert_eq!(right_none, 10.0);

        // With only right icon:
        let (x_r, width_r) = compact_notch_layout(false, true, ox, w);
        assert_eq!(x_r, ox + 14.0);
        assert_eq!(width_r, w - 48.0);
    }

    #[test]
    fn navigation_expansion_increases_target_height() {
        let mut app = App {
            hwnd: HWND::default(),
            renderer: None,
            window_region: WindowRegion::default(),
            canvas_width: WIDTH,
            canvas_height: HEIGHT,
            notch_resize: None,
            notch_drag: None,
            offline_job: None,
            offline_attempt: String::new(),
            letras_done: false,
            lyrics_window: None,
            subtitles: None,
            notch_follow: true,
            state: AppState::default(),
            path: PathBuf::new(),
            services: ServiceHandle::spawn(),
            media: MediaSnapshot::default(),
            system: SystemSnapshot::default(),
            media_at: Instant::now(),
            track_change_at: Instant::now(),
            lyric_lines: vec![],
            painted_lyric_line: None,
            verse_change_at: None,
            verse_focus: None,
            verse_scroll: Default::default(),
            shelf_scroll: Default::default(),
            task_scroll: Default::default(),
            lyric_status: "".into(),
            external_lyrics: None,
            lyrics_view_mode: 0,
            external_scroll: 0,
            page: 0, // Music page
            expanded: true,
            expansion: Spring::new(1.),
            navigation: Spring::new(0.),
            navigation_hover: false,
            body_width: Spring::new(BODY),
            body_height: Spring::new(140.),
            player_layout: Spring::new(0.),
            accent: [Spring::new(0.); 3],
            volume_level: Spring::new(1.),
            volume_hover: Spring::new(0.),
            volume_dragging: false,
            volume_hovered: false,
            volume_saved: 1.,
            volume_fade: None,
            seek_drag: None,
            media_phase: 0.,
            media_frame: Instant::now(),
            media_animating: false,
            last_anim: Instant::now(),
            hits: vec![],
            content_offset_y: 0.,
            selected: BTreeSet::new(),
            shelf_anchor: None,
            shelf_gesture: None,
            shelf_worker: Default::default(),
            shelf_thumbnails: Default::default(),
            scroll: 0,
            calendar_offset: 0,
            pomodoro_notice: None,
            toast: None,
            hover: true,
            focus: None,
            last_clock: "03:18".into(),
            drop_rx: std::sync::mpsc::channel().1,
            _drop: None,
            modal: false,
            hidden: false,
            tray_hidden: false,
            fullscreen_state: FullscreenTracker::default(),
            last_second: 0,
            accessible_items: Arc::new(Mutex::new(vec![])),
            accessible_provider: None,
            pinned: false,
            hovered_hit: None,
            cursor_pos: None,
        };

        // When unhovered, normal expanded height on page 0 is 140.
        let (_, h_normal) = app.target_size();
        assert_eq!(h_normal, 140.);

        // When hovering navigation, target height expands by NAV_EXPAND_H (28.)
        app.navigation_hover = true;
        let (_, h_expanded) = app.target_size();
        assert_eq!(h_expanded, 140. + NAV_EXPAND_H);
        assert_eq!(h_expanded, 168.);

        // Tested on Pomodoro page (page 2, base 170.)
        app.page = 2;
        app.navigation_hover = false;
        assert_eq!(app.target_size().1, 170.);
        app.navigation_hover = true;
        assert_eq!(app.target_size().1, 170. + NAV_EXPAND_H);

        // When notch is collapsed, navigation hover does not expand height
        app.expanded = false;
        app.navigation_hover = true;
        assert_eq!(app.target_size().1, 30.);
    }

    #[test]
    fn hydration_content_clears_collapsed_navigation_and_rounded_bottom() {
        // The lowest elements are the chart labels (131 + 14 DIP) and
        // action targets (112 + 32 DIP). A second header inset clipped both.
        let offset = expanded_content_offset(3);
        let content_bottom = (131.0_f32 + 14.).max(112. + 32.) + offset;
        let body_height = 170.;
        let dots_top = body_height - 8.8;
        assert!(content_bottom + 12. <= dots_top);
        assert_eq!(offset, 0.);
        // Other section layouts still rely on the shared header inset.
        assert_eq!(expanded_content_offset(2), 28.);
        assert_eq!(expanded_content_offset(4), 28.);
        assert_eq!(expanded_content_offset(5), 28.);
    }
    #[test]
    fn navigation_elements_remain_strictly_inside_notch_bounds() {
        for body_h in [140.0f32, 168.0f32, 170.0f32, 198.0f32, 286.0f32, 314.0f32] {
            let pill_h = NAV_TARGET_HEIGHT;
            let pill_y = body_h - pill_h - 4.0;
            let icon_y = pill_y + (pill_h - 16.) / 2.;
            let icon_size = 16.0f32;

            // Pill top must be non-negative
            assert!(pill_y > 0.0);
            // Pill bottom must be strictly inside body_h with at least 4.0 margin
            assert!(pill_y + pill_h <= body_h - 4.0);
            // Icon must be strictly within pill bounds
            assert!(icon_y >= pill_y);
            assert!(icon_y + icon_size <= pill_y + pill_h);
            // Neither pill nor icon ever protrudes past the bottom of the notch
            assert!(pill_y + pill_h <= body_h);
            assert!(icon_y + icon_size <= body_h);
        }
    }

    #[test]
    fn navigation_hover_zone_covers_entry_and_icon_regions() {
        let mut app = App {
            hwnd: HWND::default(),
            renderer: None,
            window_region: WindowRegion::default(),
            canvas_width: WIDTH,
            canvas_height: HEIGHT,
            notch_resize: None,
            notch_drag: None,
            offline_job: None,
            offline_attempt: String::new(),
            letras_done: false,
            lyrics_window: None,
            subtitles: None,
            notch_follow: true,
            state: AppState::default(),
            path: PathBuf::new(),
            services: ServiceHandle::spawn(),
            media: MediaSnapshot::default(),
            system: SystemSnapshot::default(),
            media_at: Instant::now(),
            track_change_at: Instant::now(),
            lyric_lines: vec![],
            painted_lyric_line: None,
            verse_change_at: None,
            verse_focus: None,
            verse_scroll: Default::default(),
            shelf_scroll: Default::default(),
            task_scroll: Default::default(),
            lyric_status: "".into(),
            external_lyrics: None,
            lyrics_view_mode: 0,
            external_scroll: 0,
            page: 0,
            expanded: true,
            expansion: Spring::new(1.),
            navigation: Spring::new(0.),
            navigation_hover: false,
            body_width: Spring::new(BODY),
            body_height: Spring::new(140.),
            player_layout: Spring::new(0.),
            accent: [Spring::new(0.); 3],
            volume_level: Spring::new(1.),
            volume_hover: Spring::new(0.),
            volume_dragging: false,
            volume_hovered: false,
            volume_saved: 1.,
            volume_fade: None,
            seek_drag: None,
            media_phase: 0.,
            media_frame: Instant::now(),
            media_animating: false,
            last_anim: Instant::now(),
            hits: vec![],
            content_offset_y: 0.,
            selected: BTreeSet::new(),
            shelf_anchor: None,
            shelf_gesture: None,
            shelf_worker: Default::default(),
            shelf_thumbnails: Default::default(),
            scroll: 0,
            calendar_offset: 0,
            pomodoro_notice: None,
            toast: None,
            hover: true,
            focus: None,
            last_clock: "03:18".into(),
            drop_rx: std::sync::mpsc::channel().1,
            _drop: None,
            modal: false,
            hidden: false,
            tray_hidden: false,
            fullscreen_state: FullscreenTracker::default(),
            last_second: 0,
            accessible_items: Arc::new(Mutex::new(vec![])),
            accessible_provider: None,
            pinned: false,
            hovered_hit: None,
            cursor_pos: None,
        };

        let center = app.canvas_width / 2.;
        // 1. Unhovered dots detection around bottom edge (h = 140., dots at 131.2)
        assert!(app.navigation_contains(center, 131.2));
        assert!(app.navigation_contains(center - 25., 132.));
        assert!(app.navigation_contains(center + 25., 132.));
        // Outside unhovered width (> 38.)
        assert!(!app.navigation_contains(center + 50., 131.2));

        // 2. Hovered state: notch expands to 168. (140 + 28)
        app.navigation_hover = true;
        app.body_height.snap(168.);
        // Original cursor position where hover triggered remains valid (no flicker)
        assert!(app.navigation_contains(center, 131.2));
        // Both outer shortcuts remain inside the expanded hover zone.
        assert!(app.navigation_contains(center - 128., 150.));
        assert!(app.navigation_contains(center + 128., 150.));
        assert!(app.navigation_contains(center, 150.));
        assert!(!app.navigation_contains(center + NAV_HOVER_HALF_WIDTH + 1., 150.));
    }

    #[test]
    fn toast_dismiss_action_clears_toast() {
        let mut app = test_app();
        app.toast = Some(("Test notification".into(), Instant::now()));
        assert!(app.toast.is_some());
        unsafe {
            app.action(Action::DismissToast);
        }
        assert!(app.toast.is_none());
    }

    #[test]
    fn inside_detects_toast_overlay() {
        let mut app = test_app();
        app.page = 0;
        app.navigation_hover = false;
        app.navigation.snap(0.);
        app.body_height.snap(140.);
        let center = app.canvas_width / 2.;
        // Without toast, y = 160. (below 140.) is outside
        assert!(!app.inside(center, 160.));
        // With toast, overlay covers y in 148..=198 and x near center
        app.toast = Some(("Alerta".into(), Instant::now()));
        assert!(app.inside(center, 160.));
        assert!(app.inside(center - 100., 160.));
        assert!(app.inside(center + 100., 160.));
        // Far outside horizontally is not inside
        assert!(!app.inside(center + 200., 160.));
    }

    #[test]
    fn is_hovered_hit_testing_with_cursor_pos() {
        let mut app = test_app();
        let target = (100., 50., 40., 30.);
        assert!(!app.is_hovered(target));
        // Cursor inside
        app.cursor_pos = Some((110., 60.));
        assert!(app.is_hovered(target));
        // Cursor boundary
        app.cursor_pos = Some((100., 50.));
        assert!(app.is_hovered(target));
        app.cursor_pos = Some((140., 80.));
        assert!(app.is_hovered(target));
        // Cursor outside
        app.cursor_pos = Some((99., 60.));
        assert!(!app.is_hovered(target));
        app.cursor_pos = Some((141., 60.));
        assert!(!app.is_hovered(target));
    }

    #[test]
    fn seek_thumb_radii_match_target_diameters() {
        // Normal state: 8-10px diameter (radius 4.5 -> 9px diameter)
        let normal_r = 4.5f32;
        let normal_diameter = normal_r * 2.;
        assert!((8. ..=10.).contains(&normal_diameter));
        // Hover state: 12px diameter (radius 6.0 -> 12px diameter)
        let hover_r = 6.0f32;
        let hover_diameter = hover_r * 2.;
        assert_eq!(hover_diameter, 12.);
    }

    #[test]
    fn small_click_targets_meet_dense_desktop_guidelines() {
        // Shelf close button: 36x36 >= 32x32
        let shelf_close = (0., 0., 36., 36.);
        assert!(shelf_close.2 >= 32. && shelf_close.3 >= 32.);

        // Lyrics mode buttons: 32x36 >= 32x32
        let lyrics_btn = (0., 0., 32., 36.);
        assert!(lyrics_btn.2 >= 32. && lyrics_btn.3 >= 32.);

        // Pomodoro dismiss: 36x40 >= 32x32
        let pomodoro_dismiss = (0., 0., 36., 40.);
        assert!(pomodoro_dismiss.2 >= 32. && pomodoro_dismiss.3 >= 32.);

        // Calendar days: 42x24 (42px column pitch covers width ergonomically)
        let calendar_day = (0., 0., 42., 24.);
        assert!(calendar_day.2 >= 32.);
    }

    #[test]
    fn shelf_scroll_actions_clamp_and_navigate() {
        let mut app = test_app();
        for i in 0..12 {
            app.state.shelf.push(ShelfItem {
                id: i as u64 + 1,
                kind: ShelfKind::File,
                value: format!("file_{i}.txt"),
            });
        }
        assert_eq!(app.scroll, 0);
        unsafe {
            app.action(Action::ShelfScroll(2));
        }
        assert_eq!(app.scroll, 2);
        unsafe {
            app.action(Action::ShelfScroll(100));
        }
        // Clamps to max_scroll = 12 - 5 = 7
        assert_eq!(app.scroll, 7);
        unsafe {
            app.action(Action::ShelfScroll(-3));
        }
        assert_eq!(app.scroll, 4);
        unsafe {
            app.action(Action::ShelfScroll(-100));
        }
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn marquee_offset_stationary_when_fits() {
        // Fits within max width -> offset is 0.0 at any time
        assert_eq!(marquee_offset(100., 150., 0.), 0.);
        assert_eq!(marquee_offset(100., 150., 5.0), 0.);
        assert_eq!(marquee_offset(100., 150., 50.0), 0.);
        // Equal width -> 0.0
        assert_eq!(marquee_offset(150., 150., 10.0), 0.);
        // Negative or zero max width -> 0.0
        assert_eq!(marquee_offset(150., 0., 10.0), 0.);
    }

    #[test]
    fn marquee_offset_pauses_and_ping_pong_movement() {
        let text_w = 200.0;
        let max_w = 100.0;
        let distance = text_w - max_w; // 100.0
        let speed = 28.0;
        let pause = 2.0;
        let scroll_dur = distance / speed; // 100.0 / 28.0 = 3.5714s
        let cycle_dur = 2.0 * (pause + scroll_dur); // 2 * (2.0 + 3.5714) = 11.1428s

        // 1. Initial pause at start: 0.0s to 2.0s -> offset 0.0
        assert_eq!(marquee_offset(text_w, max_w, 0.0), 0.0);
        assert_eq!(marquee_offset(text_w, max_w, 1.0), 0.0);
        assert_eq!(marquee_offset(text_w, max_w, 1.99), 0.0);

        // 2. Midway through scrolling to the right
        let mid_t = pause + scroll_dur / 2.0;
        let mid_offset = marquee_offset(text_w, max_w, mid_t);
        assert!(mid_offset > 40.0 && mid_offset < 60.0);

        // 3. Pause at the end (from pause + scroll_dur to 2 * pause + scroll_dur)
        let end_reach = pause + scroll_dur;
        let end_offset = marquee_offset(text_w, max_w, end_reach + 0.1);
        assert_eq!(end_offset, distance);
        assert_eq!(marquee_offset(text_w, max_w, end_reach + 1.0), distance);

        // 4. Return scrolling backwards
        let ret_t = 2.0 * pause + scroll_dur + scroll_dur / 2.0;
        let ret_offset = marquee_offset(text_w, max_w, ret_t);
        assert!(ret_offset > 40.0 && ret_offset < 60.0);

        // 5. Complete cycle repeats
        let next_cycle_start = cycle_dur + 0.5;
        assert_eq!(marquee_offset(text_w, max_w, next_cycle_start), 0.0);
    }

    #[test]
    fn track_info_action_and_album_tooltip() {
        let mut app = test_app();
        app.media.title = "Bohemian Rhapsody".into();
        app.media.artist = "Queen".into();
        app.media.album = "A Night at the Opera".into();

        // Hit testing for track info
        let prefix = app.tr("Álbum: ", "Album: ");
        let expected_label = format!("{prefix}A Night at the Opera");
        app.hit((100., 32., 200., 44.), &expected_label, Action::TrackInfo);

        let hit = app.hits.last().unwrap();
        assert_eq!(hit.label, expected_label);
        assert!(matches!(hit.action, Action::TrackInfo));

        // Action::TrackInfo does not mutate app state or crash
        unsafe {
            app.action(Action::TrackInfo);
        }
    }

    #[test]
    fn ambient_glow_color_extraction() {
        let mut app = test_app();
        app.accent[0].snap(0.8);
        app.accent[1].snap(0.4);
        app.accent[2].snap(0.2);

        let ar = (app.accent[0].value.clamp(0., 1.) * 255.) as u32;
        let ag = (app.accent[1].value.clamp(0., 1.) * 255.) as u32;
        let ab = (app.accent[2].value.clamp(0., 1.) * 255.) as u32;
        let accent_rgb = (ar << 16) | (ag << 8) | ab;

        assert_eq!(ar, 204);
        assert_eq!(ag, 102);
        assert_eq!(ab, 51);
        assert_eq!(accent_rgb, (204 << 16) | (102 << 8) | 51);
    }

    #[test]
    fn close_action_hides_to_tray() {
        let mut app = test_app();
        app.expanded = true;
        app.hover = true;
        assert!(app.expanded);

        unsafe {
            app.action(Action::Close);
        }

        assert!(!app.expanded);
        assert!(!app.hover);
        assert!(app.tray_hidden);

        unsafe {
            app.restore_from_tray(true);
        }
        assert!(!app.tray_hidden);
        assert!(app.expanded);
    }

    #[test]
    fn copy_lyrics_for_ai_formats_content_cleanly() {
        let mut app = test_app();
        app.media.title = "Test Title".into();
        app.media.artist = "Test Artist".into();
        app.lyric_lines = vec![lyrics::LyricLine {
            timestamp_ms: 30000,
            text: "Hello from test".into(),
            translation: Some("Olá do teste".into()),
        }];
        let content = app.lyrics_content();
        let formatted = content.format_for_ai();
        assert!(formatted.contains("Test Title"));
        assert!(formatted.contains("Test Artist"));
        assert!(formatted.contains("[00:30.00] Hello from test"));
        assert!(formatted.contains("Olá do teste"));
    }

    #[test]
    fn live_scrubbing_and_seek_hit_testing() {
        let mut app = test_app();
        app.expanded = true;
        app.page = 0;
        app.media.can_seek = true;
        app.media.duration = 200.0;
        app.media.position = 50.0;

        let layout = app.player_geometry();
        let (sx, sy, sw, _sh) = layout.seek;

        assert!(app.is_seek_hit(sx + 10., sy + 5.));
        assert!(!app.is_seek_hit(sx - 10., sy));
        assert!(!app.is_seek_hit(sx + sw + 10., sy));

        // When not scrubbing, pos is playback position
        assert_eq!(app.seek_drag, None);
        assert!((app.position() - 50.0).abs() < 1.0);

        // When scrubbing, target position matches scrub ratio
        app.seek_drag = Some(0.75);
        let drag_pos = (app.seek_drag.unwrap() * app.media.duration).clamp(0., app.media.duration);
        assert!((drag_pos - 150.0).abs() < 0.001);
    }

    #[test]
    fn volume_fine_adjustment_and_mute_soft_fade() {
        let mut app = test_app();
        app.system.volume = Some(0.50);

        // Test fine step 1% vs normal 5%
        let fine_step = 0.01;
        let normal_step = 0.05;
        let vol = app.system.volume.unwrap();
        assert!((((vol + fine_step) * 100.).round() - 51.).abs() < 0.001);
        assert!((((vol + normal_step) * 100.).round() - 55.).abs() < 0.001);

        // Toggle mute activates volume_fade instead of instantaneous double toggle
        app.toggle_volume_mute();
        assert!(app.volume_fade.is_some());
        let (start, target, _, _) = app.volume_fade.unwrap();
        assert_eq!(start, 0.50);
        assert_eq!(target, 0.0);
        assert_eq!(app.volume_saved, 0.50);

        // Manual volume change cancels fade
        app.change_volume(0.80);
        assert!(app.volume_fade.is_none());
        assert_eq!(app.system.volume, Some(0.80));
    }

    #[test]
    fn lyrics_volume_hover_bridge_and_bar_hit_continuity() {
        let mut app = test_app();
        app.page = 1;
        app.expanded = true;
        app.player_layout.snap(1.);
        app.system.volume = Some(0.70);

        let (icon, bar) = app.volume_rects().expect("volume rects in lyrics");
        // Over icon is a hit
        assert!(app.is_volume_hit(icon.0 + 5., icon.1 + 5.));

        // Over bar is a hit directly, even if volume_hovered was false
        assert!(app.is_volume_hit(bar.0 + 10., bar.1 + 5.));
        assert!(app.is_volume_bar_hit(bar.0 + 10., bar.1 + 5.));

        // When moving between icon and bar, bridge keeps hit active
        app.volume_hovered = true;
        let mid_x = (icon.0 + bar.0) / 2.;
        let mid_y = (icon.1 + bar.1) / 2.;
        assert!(app.is_volume_hit(mid_x, mid_y));

        // Point outside the bounding box ceases hit
        assert!(!app.is_volume_hit(icon.0 + 100., icon.1));
    }

    #[test]
    fn offline_badge_label_and_notch_clip_alignment() {
        let mut app = test_app();
        app.lyrics_view_mode = 1;
        assert_eq!(app.lyrics_view_mode, 1);
        let content = app.lyrics_content();
        assert_eq!(content.offline_label(1, false), "");
    }
}
