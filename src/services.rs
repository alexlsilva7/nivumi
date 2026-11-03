//! Native Windows integrations. All blocking WinRT calls run away from the UI STA.
use std::{
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    core::Interface,
    Foundation::TypedEventHandler,
    Media::Control::*,
    Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation},
    Storage::Streams::DataReader,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        Media::Audio::{
            Endpoints::IAudioEndpointVolume, IAudioSessionControl2, IAudioSessionManager2,
            ISimpleAudioVolume, *,
        },
        System::{
            Com::*,
            Power::*,
            Registry::*,
            Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            WinRT::*,
        },
        UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaSnapshot {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub player: String,
    pub duration: f64,
    pub position: f64,
    pub playing: bool,
    pub rate: f64,
    pub can_toggle: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    pub cover: Vec<u8>,
    pub accent: Option<[f32; 3]>,
}
impl MediaSnapshot {
    pub fn key(&self) -> String {
        lyrics_key(&self.title, &self.artist, self.duration)
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemSnapshot {
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub online: Option<bool>,
    pub volume: Option<f32>,
    pub caps_lock: bool,
    pub keyboard_language: u16,
    pub brightness: Option<u8>,
    pub bluetooth_battery: Option<u8>,
    pub bluetooth_devices: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum ServiceCommand {
    PlayPause,
    Next,
    Previous,
    Seek(f64),
    Volume(f32),
    FetchLyrics {
        title: String,
        artist: String,
        duration: f64,
    },
    FetchLetrasLyrics {
        title: String,
        artist: String,
    },
    Refresh,
    RefreshArtwork,
    Stop,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalLyrics {
    pub translation_origin: crate::offline_translation::TranslationOrigin,
    pub title: String,
    pub artist: String,
    pub original: String,
    pub translation: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug)]
pub enum ServiceEvent {
    Media(MediaSnapshot),
    Lyrics {
        key: String,
        text: Option<crate::synced_lyrics::SyncedLyrics>,
        error: Option<String>,
    },
    LetrasFinished {
        key: String,
    },
    LetrasLyrics {
        key: String,
        lyrics: ExternalLyrics,
    },
    System(SystemSnapshot),
    Error(String),
}
pub struct ServiceHandle {
    pub tx: Sender<ServiceCommand>,
    pub rx: Receiver<ServiceEvent>,
}
impl ServiceHandle {
    pub fn spawn() -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let callback_tx = tx.clone();
        thread::spawn(move || {
            if let Err(e) = worker(commands, events.clone(), callback_tx) {
                let _ = events.send(ServiceEvent::Error(e.to_string()));
            }
        });
        Self { tx, rx }
    }
    pub fn send(&self, command: ServiceCommand) {
        let _ = self.tx.send(command);
    }
}
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(ServiceCommand::Stop);
    }
}
pub fn lyrics_key(title: &str, artist: &str, duration: f64) -> String {
    // Length prefixes keep literal separators in song metadata unambiguous.
    let title = title.trim().to_lowercase();
    let artist = artist.trim().to_lowercase();
    format!(
        "{}:{title}{}:{artist}:{}:{:016x}",
        title.len(),
        artist.len(),
        duration.round() as u64,
        crate::letras::title_filter_key()
    )
}

#[derive(Clone)]
struct LyricsRequest {
    title: String,
    artist: String,
    duration: f64,
}

#[derive(Default)]
struct PendingLyrics {
    latest: Option<LyricsRequest>,
    stopped: bool,
}

/// One dedicated HTTP worker and at most one queued, replaceable request.
struct LyricsWorker {
    queue: Arc<(Mutex<PendingLyrics>, Condvar)>,
}
impl LyricsWorker {
    fn spawn(events: Sender<ServiceEvent>) -> Self {
        let queue = Arc::new((Mutex::new(PendingLyrics::default()), Condvar::new()));
        let worker_queue = queue.clone();
        thread::spawn(move || {
            let (lock, ready) = &*worker_queue;
            loop {
                let request = {
                    let mut pending = lock
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while pending.latest.is_none() && !pending.stopped {
                        pending = ready
                            .wait(pending)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    if pending.stopped {
                        break;
                    }
                    pending.latest.take().expect("pending request checked")
                };
                let key = lyrics_key(&request.title, &request.artist, request.duration);
                let result = match Apartment::mta() {
                    Ok(_apartment) => {
                        fetch_lyrics(&request.title, &request.artist, request.duration)
                    }
                    Err(error) => Err(Box::new(error) as Box<dyn std::error::Error + Send + Sync>),
                };
                let (text, error) = match result {
                    Ok(text) => (text, None),
                    Err(error) => (None, Some(error.to_string())),
                };
                if events
                    .send(ServiceEvent::Lyrics { key, text, error })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self { queue }
    }
    fn submit(&self, request: LyricsRequest) {
        let (lock, ready) = &*self.queue;
        let mut pending = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !pending.stopped {
            pending.latest = Some(request);
            ready.notify_one();
        }
    }
}
impl Drop for LyricsWorker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.queue;
        let mut pending = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.stopped = true;
        pending.latest = None;
        ready.notify_one();
    }
}

fn lyrics_cache_root(state_path: &Path) -> PathBuf {
    state_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("lyrics")
}

struct Apartment;
impl Apartment {
    fn mta() -> windows::core::Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

fn worker(
    commands: Receiver<ServiceCommand>,
    events: Sender<ServiceEvent>,
    callback_tx: Sender<ServiceCommand>,
) -> windows::core::Result<()> {
    // Declared first so COM interface locals are dropped before RoUninitialize.
    let _apartment = Apartment::mta()?;
    let lyrics_worker = LyricsWorker::spawn(events.clone());
    let letras_worker = crate::letras::LetrasWorker::spawn(events.clone());
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
    let signal = callback_tx.clone();
    let manager_token = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = signal.send(ServiceCommand::Refresh);
        Ok(())
    }))?;
    let mut session: Option<GlobalSystemMediaTransportControlsSession> = None;
    let mut session_tokens = Vec::new();
    let mut previous = MediaSnapshot::default();
    let mut artwork = ArtworkRefresh::default();
    let mut last_system = SystemSnapshot::default();
    let mut next_system = Instant::now();
    let mut refresh = true;
    let brightness_provider = BrightnessProvider::new().ok();
    let mut bluetooth_devices = Vec::new();
    let mut next_bluetooth = Instant::now();
    loop {
        if refresh {
            let current = manager.GetCurrentSession().ok();
            if current != session {
                artwork.invalidate();
                if let Some(old) = session.take() {
                    for (kind, token) in session_tokens.drain(..) {
                        let _ = match kind {
                            0 => old.RemoveMediaPropertiesChanged(token),
                            1 => old.RemovePlaybackInfoChanged(token),
                            _ => old.RemoveTimelinePropertiesChanged(token),
                        };
                    }
                }
                session = current;
                if let Some(s) = &session {
                    for kind in 0..3 {
                        let signal = callback_tx.clone();
                        let result = match kind {
                            0 => s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                                let _ = signal.send(ServiceCommand::RefreshArtwork);
                                Ok(())
                            })),
                            1 => s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                                let _ = signal.send(ServiceCommand::Refresh);
                                Ok(())
                            })),
                            _ => {
                                s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                                    let _ = signal.send(ServiceCommand::Refresh);
                                    Ok(())
                                }))
                            }
                        };
                        if let Ok(token) = result {
                            session_tokens.push((kind, token));
                        }
                    }
                }
            }
            let snapshot = session
                .as_ref()
                .and_then(|s| read_media(s, &previous, &mut artwork).ok())
                .unwrap_or_default();
            if snapshot != previous {
                previous = snapshot.clone();
                if events.send(ServiceEvent::Media(snapshot)).is_err() {
                    break;
                }
            }
            refresh = false;
        }
        if Instant::now() >= next_system {
            let mut sys = system_snapshot();
            let app_id = session
                .as_ref()
                .and_then(|s| s.SourceAppUserModelId().ok())
                .map(|h| h.to_string())
                .unwrap_or_else(|| previous.player.clone());
            if !app_id.is_empty() {
                if let Some(pv) = get_player_volume(&app_id) {
                    sys.volume = Some(pv);
                }
            }
            sys.brightness = brightness_provider
                .as_ref()
                .and_then(|p| p.read().ok())
                .flatten();
            if Instant::now() >= next_bluetooth {
                bluetooth_devices = connected_bluetooth().unwrap_or_default();
                next_bluetooth = Instant::now() + Duration::from_secs(10);
            }
            sys.bluetooth_devices = bluetooth_devices.clone();
            if sys != last_system {
                last_system = sys.clone();
                let _ = events.send(ServiceEvent::System(sys));
            }
            next_system = Instant::now() + Duration::from_secs(2);
        }
        match commands.recv_timeout(Duration::from_secs(2)) {
            Ok(ServiceCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(ServiceCommand::FetchLyrics {
                title,
                artist,
                duration,
            }) => {
                lyrics_worker.submit(LyricsRequest {
                    title,
                    artist,
                    duration,
                });
            }
            Ok(ServiceCommand::FetchLetrasLyrics { title, artist }) => {
                letras_worker.submit(crate::letras::LetrasRequest { title, artist });
            }
            Ok(ServiceCommand::Volume(value)) => {
                let app_id = session
                    .as_ref()
                    .and_then(|s| s.SourceAppUserModelId().ok())
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| previous.player.clone());
                let res = if !app_id.is_empty() {
                    set_player_volume(&app_id, value)
                } else {
                    set_volume(value)
                };
                if let Err(e) = res {
                    let _ = events.send(ServiceEvent::Error(e.to_string()));
                }
                next_system = Instant::now();
            }
            Ok(ServiceCommand::RefreshArtwork) => {
                artwork.invalidate();
                refresh = true;
            }
            Ok(command) => {
                if let Some(s) = &session {
                    let result = match command {
                        ServiceCommand::PlayPause => {
                            s.TryTogglePlayPauseAsync().and_then(|a| a.join())
                        }
                        ServiceCommand::Next => s.TrySkipNextAsync().and_then(|a| a.join()),
                        ServiceCommand::Previous => s.TrySkipPreviousAsync().and_then(|a| a.join()),
                        ServiceCommand::Seek(seconds) => s
                            .TryChangePlaybackPositionAsync(
                                (seconds.max(0.0) * 10_000_000.0) as i64,
                            )
                            .and_then(|a| a.join()),
                        _ => Ok(true),
                    };
                    if let Err(e) = result {
                        let _ = events.send(ServiceEvent::Error(e.to_string()));
                    }
                }
                refresh = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                /* Events carry media changes; slow heartbeat repairs missed session transitions. */
                refresh = true;
            }
        }
    }
    if let Some(s) = session {
        for (kind, token) in session_tokens {
            let _ = match kind {
                0 => s.RemoveMediaPropertiesChanged(token),
                1 => s.RemovePlaybackInfoChanged(token),
                _ => s.RemoveTimelinePropertiesChanged(token),
            };
        }
    }
    let _ = manager.RemoveCurrentSessionChanged(manager_token);
    Ok(())
}
/// Metadata and thumbnails may arrive separately (especially in browser sessions).
/// Never keep a successful placeholder forever, even if no artwork event arrives.
#[derive(Default)]
struct ArtworkRefresh {
    next_check: Option<Instant>,
}

impl ArtworkRefresh {
    fn invalidate(&mut self) {
        self.next_check = None;
    }

    fn update(
        &mut self,
        result: &mut MediaSnapshot,
        previous: &MediaSnapshot,
        now: Instant,
        read: impl FnOnce() -> Option<Vec<u8>>,
    ) {
        let same_track = result.title == previous.title
            && result.artist == previous.artist
            && result.album == previous.album
            && result.player == previous.player;
        result.cover.clear();
        result.accent = None;
        if same_track {
            result.cover = previous.cover.clone();
            result.accent = previous.accent;
            if self.next_check.is_some_and(|next| now < next) {
                return;
            }
        }
        self.next_check = Some(now + Duration::from_secs(5));
        if let Some(bytes) = read().filter(|bytes| !bytes.is_empty()) {
            result.accent = if bytes == previous.cover {
                previous.accent
            } else {
                crate::artwork::accent_from_encoded(&bytes)
            };
            result.cover = bytes;
        } else {
            // Keep same-track artwork through temporary stream errors, but never
            // carry the previous song's cover into a different track.
            self.next_check = Some(now + Duration::from_secs(2));
        }
    }
}

pub fn clean_media_title(raw: &str) -> String {
    let trimmed = raw.trim();
    let cleaned = trimmed.trim_start_matches(|c: char| {
        c == '·' || c == '•' || c == '▶' || c == '►' || c == '\u{00b7}' || c == '\u{2022}'
    });
    cleaned.trim().to_string()
}

fn read_media(
    session: &GlobalSystemMediaTransportControlsSession,
    previous: &MediaSnapshot,
    artwork: &mut ArtworkRefresh,
) -> windows::core::Result<MediaSnapshot> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let timeline = session.GetTimelineProperties()?;
    let info = session.GetPlaybackInfo()?;
    let controls = info.Controls()?;
    let raw_title = clean_media_title(&props.Title()?.to_string());
    let raw_artist = props.Artist()?.to_string();
    let (clean_title, clean_artist) = crate::letras::clean_song_metadata(&raw_title, &raw_artist);
    let title = if !clean_title.is_empty() {
        clean_title
    } else {
        raw_title
    };
    let artist = if !clean_artist.is_empty() {
        clean_artist
    } else if matches!(
        raw_artist.trim().to_ascii_lowercase().as_str(),
        "youtube" | "chrome" | "edge" | "msedge" | "browser" | "navegador"
    ) {
        String::new()
    } else {
        raw_artist
    };
    let mut result = MediaSnapshot {
        title,
        artist,
        album: props.AlbumTitle()?.to_string(),
        player: session.SourceAppUserModelId()?.to_string(),
        duration: timeline.EndTime()?.Duration as f64 / 10_000_000.0,
        position: timeline.Position()?.Duration as f64 / 10_000_000.0,
        playing: info.PlaybackStatus()?
            == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing,
        rate: info.PlaybackRate().and_then(|v| v.Value()).unwrap_or(1.0),
        can_toggle: controls.IsPlayPauseToggleEnabled().unwrap_or(false),
        can_next: controls.IsNextEnabled().unwrap_or(false),
        can_previous: controls.IsPreviousEnabled().unwrap_or(false),
        can_seek: controls.IsPlaybackPositionEnabled().unwrap_or(false),
        cover: Vec::new(),
        accent: None,
    };
    artwork.update(&mut result, previous, Instant::now(), || {
        read_thumbnail(&props).ok()
    });
    Ok(result)
}

fn read_thumbnail(
    props: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> windows::core::Result<Vec<u8>> {
    let image = props.Thumbnail()?.OpenReadAsync()?.join()?;
    let size = image.Size()?;
    if size == 0 || size > 4 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let reader = DataReader::CreateDataReader(&image)?;
    let loaded = reader.LoadAsync(size as u32)?.join()?;
    let mut bytes = vec![0; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}
fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)?
            .Activate(CLSCTX_ALL, None)
    }
}
pub fn set_volume(value: f32) -> windows::core::Result<()> {
    unsafe { endpoint()?.SetMasterVolumeLevelScalar(value.clamp(0.0, 1.0), std::ptr::null()) }
}

fn get_process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        if res.is_ok() && size > 0 {
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        } else {
            None
        }
    }
}

// Unlike volume matching, activation must not accept partial process names.
fn matches_player_window(process_name: &str, player_app: &str) -> bool {
    let process = process_name.to_lowercase();
    let process = process.strip_suffix(".exe").unwrap_or(&process);
    let target = player_app.trim().to_lowercase();
    let target = target.strip_suffix(".exe").unwrap_or(&target);
    !process.is_empty()
        && (process == target
            || (process == "chrome" && target.starts_with("chrome."))
            || (process == "msedge"
                && (target == "microsoftedge" || target.starts_with("msedge."))))
}

/// Open the displayed track's source on the COM-initialized UI thread, which
/// receives the user's click and can request foreground activation.
pub fn open_media_player(player_app: &str, track_title: &str) -> windows::core::Result<()> {
    let player_app = player_app.trim();
    if player_app.is_empty() || player_app.contains('\0') {
        return Err(windows::core::Error::from_hresult(E_INVALIDARG));
    }
    struct Search<'a> {
        player: &'a str,
        title: String,
        window: Option<HWND>,
        track_window: Option<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> windows::core::BOOL {
        let search = &mut *(param.0 as *mut Search<'_>);
        if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok() {
            return true.into();
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if get_process_name(pid).is_some_and(|name| matches_player_window(&name, search.player)) {
            search.window.get_or_insert(hwnd);
            // Prefer the playing browser window when several windows are open.
            let mut title = [0u16; 1024];
            let len = GetWindowTextW(hwnd, &mut title) as usize;
            if !search.title.is_empty()
                && String::from_utf16_lossy(&title[..len])
                    .to_lowercase()
                    .contains(&search.title)
            {
                search.track_window.get_or_insert(hwnd);
            }
        }
        true.into()
    }
    let mut search = Search {
        player: player_app,
        title: track_title.trim().to_lowercase(),
        window: None,
        track_window: None,
    };
    unsafe {
        EnumWindows(Some(visit), LPARAM(&mut search as *mut _ as isize))?;
        if let Some(hwnd) = search.track_window.or(search.window) {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
            }
            return if SetForegroundWindow(hwnd).as_bool() {
                Ok(())
            } else {
                Err(windows::core::Error::from_hresult(E_FAIL))
            };
        }
        // Store apps may be suspended or hosted by ApplicationFrameHost.
        let app_id = windows::core::HSTRING::from(player_app);
        if player_app.contains('!') {
            let activation: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_ALL)?;
            activation.ActivateApplication(&app_id, windows::core::PCWSTR::null(), AO_NOERRORUI)?;
            return Ok(());
        }
        // Registered desktop apps may keep playing with their window in the tray.
        let target = windows::core::HSTRING::from(format!("shell:AppsFolder\\{player_app}"));
        let result = ShellExecuteW(
            None,
            w!("open"),
            &target,
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(E_FAIL))
        }
    }
}

pub fn matches_player_app(
    process_name: Option<&str>,
    session_id: Option<&str>,
    player_app: &str,
) -> bool {
    let target = player_app.trim().to_lowercase();
    if target.is_empty() {
        return false;
    }
    let target_stem = target.strip_suffix(".exe").unwrap_or(&target);

    if let Some(proc) = process_name {
        let proc_lower = proc.to_lowercase();
        let proc_stem = proc_lower.strip_suffix(".exe").unwrap_or(&proc_lower);
        if proc_lower == target || proc_stem == target_stem {
            return true;
        }
        if target.starts_with(proc_stem) || proc_stem.starts_with(target_stem) {
            return true;
        }
        if target.contains(proc_stem) || proc_stem.contains(target_stem) {
            return true;
        }
        if (target.contains("edge") || target.contains("msedge"))
            && (proc_stem.contains("edge") || proc_stem.contains("msedge"))
        {
            return true;
        }
    }

    if let Some(sid) = session_id {
        let sid_lower = sid.to_lowercase();
        if sid_lower.contains(&target)
            || (!target_stem.is_empty() && sid_lower.contains(target_stem))
        {
            return true;
        }
        if let Some(proc) = process_name {
            let proc_lower = proc.to_lowercase();
            let proc_stem = proc_lower.strip_suffix(".exe").unwrap_or(&proc_lower);
            if !proc_stem.is_empty() && sid_lower.contains(proc_stem) {
                return true;
            }
        }
    }

    false
}

pub fn find_player_sessions(player_app: &str) -> Vec<ISimpleAudioVolume> {
    let mut matching_sessions = Vec::new();
    let Ok(enumerator) = (unsafe {
        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }) else {
        return matching_sessions;
    };
    let Ok(device) = (unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }) else {
        return matching_sessions;
    };
    unsafe {
        let Ok(mgr): windows::core::Result<IAudioSessionManager2> =
            device.Activate(CLSCTX_ALL, None)
        else {
            return matching_sessions;
        };
        let Ok(session_enum) = mgr.GetSessionEnumerator() else {
            return matching_sessions;
        };
        let Ok(count) = session_enum.GetCount() else {
            return matching_sessions;
        };
        for i in 0..count {
            let Ok(session) = session_enum.GetSession(i) else {
                continue;
            };
            let Ok(ctrl2) = session.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if ctrl2.IsSystemSoundsSession().0 == 0 {
                continue;
            }
            let proc_name = if let Ok(pid) = ctrl2.GetProcessId() {
                if pid != 0 {
                    get_process_name(pid)
                } else {
                    None
                }
            } else {
                None
            };
            let sid_str = ctrl2
                .GetSessionIdentifier()
                .ok()
                .and_then(|p| p.to_string().ok());
            if matches_player_app(proc_name.as_deref(), sid_str.as_deref(), player_app) {
                if let Ok(simple) = session.cast::<ISimpleAudioVolume>() {
                    matching_sessions.push(simple);
                }
            }
        }
    }
    matching_sessions
}

pub fn get_player_volume(player_app: &str) -> Option<f32> {
    let sessions = find_player_sessions(player_app);
    for simple in sessions {
        unsafe {
            if let Ok(vol) = simple.GetMasterVolume() {
                return Some(vol);
            }
        }
    }
    None
}

pub fn set_player_volume(player_app: &str, value: f32) -> windows::core::Result<()> {
    let sessions = find_player_sessions(player_app);
    if sessions.is_empty() {
        return set_volume(value);
    }
    let clamped = value.clamp(0.0, 1.0);
    for simple in &sessions {
        unsafe {
            let _ = simple.SetMasterVolume(clamped, std::ptr::null());
            if clamped > 0.0 {
                if let Ok(muted) = simple.GetMute() {
                    if muted.as_bool() {
                        let _ = simple.SetMute(false, std::ptr::null());
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn is_laptop() -> bool {
    unsafe {
        let role = PowerDeterminePlatformRole();
        if role == PlatformRoleMobile || role == PlatformRoleSlate {
            return true;
        }
        if role == PlatformRoleDesktop
            || role == PlatformRoleWorkstation
            || role == PlatformRoleEnterpriseServer
            || role == PlatformRoleSOHOServer
            || role == PlatformRoleAppliancePC
            || role == PlatformRolePerformanceServer
        {
            return false;
        }
        let mut power = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut power).is_ok() {
            (power.BatteryFlag & 128 == 0)
                && power.BatteryFlag != 255
                && power.BatteryLifePercent <= 100
        } else {
            false
        }
    }
}
pub fn system_snapshot() -> SystemSnapshot {
    unsafe {
        let mut power = SYSTEM_POWER_STATUS::default();
        let power_ok = GetSystemPowerStatus(&mut power).is_ok();
        let language = GetKeyboardLayout(GetWindowThreadProcessId(GetForegroundWindow(), None)).0
            as usize as u16;
        let has_battery = power_ok
            && power.BatteryLifePercent <= 100
            && (power.BatteryFlag & 128 == 0)
            && power.BatteryFlag != 255;
        SystemSnapshot {
            battery_percent: if has_battery {
                Some(power.BatteryLifePercent)
            } else {
                None
            },
            charging: if has_battery && power.ACLineStatus != 255 {
                Some(power.ACLineStatus == 1)
            } else {
                None
            },
            online: NetworkInformation::GetInternetConnectionProfile()
                .and_then(|p| p.GetNetworkConnectivityLevel())
                .map(|s| s == NetworkConnectivityLevel::InternetAccess)
                .ok(),
            volume: endpoint().and_then(|e| e.GetMasterVolumeLevelScalar()).ok(),
            caps_lock: GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0,
            keyboard_language: language,
            brightness: None,
            bluetooth_battery: None,
            bluetooth_devices: Vec::new(),
        }
    }
}
/// Refresh an already-enabled registration after replacing or moving the executable.
/// An absent value or a disabled preference is never turned on by startup.
pub fn refresh_existing_startup(enabled: bool) -> windows::core::Result<()> {
    if !enabled {
        return Ok(());
    }
    unsafe {
        let mut bytes = 0u32;
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            w!("NusaRebuild"),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        );
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        result.ok()?;
        set_startup(true)
    }
}
pub fn set_startup(enabled: bool) -> windows::core::Result<()> {
    unsafe {
        let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        if enabled {
            let exe = std::env::current_exe().map_err(|_| windows::core::Error::from_thread())?;
            let wide: Vec<u16> = format!("\"{}\"", exe.display())
                .encode_utf16()
                .chain(Some(0))
                .collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key,
                w!("NusaRebuild"),
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                (wide.len() * 2) as u32,
            )
            .ok()
        } else {
            let result = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, w!("NusaRebuild"));
            if result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                result.ok()
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub primary: bool,
}
pub fn enum_monitors() -> Vec<Monitor> {
    unsafe extern "system" fn callback(
        m: HMONITOR,
        _: HDC,
        _: *mut RECT,
        l: LPARAM,
    ) -> windows::core::BOOL {
        unsafe {
            let monitors = &mut *(l.0 as *mut Vec<Monitor>);
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(m, &mut info.monitorInfo).as_bool() {
                let r = info.monitorInfo.rcMonitor;
                monitors.push(Monitor {
                    name: String::from_utf16_lossy(
                        &info.szDevice[..info.szDevice.iter().position(|v| *v == 0).unwrap_or(32)],
                    ),
                    left: r.left,
                    top: r.top,
                    right: r.right,
                    bottom: r.bottom,
                    primary: info.monitorInfo.dwFlags & 1 != 0,
                });
            }
            true.into()
        }
    }
    let mut result = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut result as *mut _ as isize),
        );
    }
    result
}

fn connected_bluetooth() -> windows::core::Result<Vec<String>> {
    use windows::Devices::{
        Bluetooth::{BluetoothConnectionStatus, BluetoothDevice},
        Enumeration::DeviceInformation,
    };
    let selector = BluetoothDevice::GetDeviceSelectorFromConnectionStatus(
        BluetoothConnectionStatus::Connected,
    )?;
    let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
    let mut names = Vec::new();
    for i in 0..devices.Size()? {
        if let Ok(name) = devices.GetAt(i).and_then(|v| v.Name()) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}
struct BrightnessProvider(windows::Win32::System::Wmi::IWbemServices);
impl BrightnessProvider {
    fn new() -> windows::core::Result<Self> {
        use windows::{core::BSTR, Win32::System::Wmi::*};
        unsafe {
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
            let empty = BSTR::new();
            let service = locator.ConnectServer(
                &BSTR::from("ROOT\\WMI"),
                &empty,
                &empty,
                &empty,
                0,
                &empty,
                None,
            )?;
            CoSetProxyBlanket(
                &service,
                10,
                0,
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?;
            Ok(Self(service))
        }
    }
    fn read(&self) -> windows::core::Result<Option<u8>> {
        use windows::{
            core::BSTR,
            Win32::System::{Variant::*, Wmi::*},
        };
        unsafe {
            let results = self.0.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE"),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )?;
            let mut items = [None];
            let mut count = 0;
            results.Next(500, &mut items, &mut count).ok()?;
            if count == 0 {
                return Ok(None);
            }
            let Some(item) = items[0].take() else {
                return Ok(None);
            };
            let mut value = VARIANT::default();
            item.Get(w!("CurrentBrightness"), 0, &mut value, None, None)?;
            let inner = &value.Anonymous.Anonymous;
            let result = if inner.vt == VT_UI1 {
                Some(inner.Anonymous.bVal)
            } else if inner.vt == VT_I4 {
                u8::try_from(inner.Anonymous.lVal).ok()
            } else {
                None
            };
            let _ = VariantClear(&mut value);
            Ok(result.filter(|v| *v <= 100))
        }
    }
}

fn fetch_lyrics(
    title: &str,
    artist: &str,
    duration: f64,
) -> Result<Option<crate::synced_lyrics::SyncedLyrics>, Box<dyn std::error::Error + Send + Sync>> {
    let cache_root = lyrics_cache_root(&crate::storage::default_path());
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    lyrics_key(title, artist, duration).hash(&mut hash);
    // New schema avoids reusing old unvalidated provider responses.
    let cache = cache_root.join(format!("v3-{:016x}.json", hash.finish()));
    if let Ok(data) = std::fs::read_to_string(&cache) {
        if let Ok(value) = serde_json::from_str::<crate::synced_lyrics::SyncedLyrics>(&data) {
            if crate::synced_lyrics::valid_lrc(&value.text) {
                return Ok(Some(value));
            }
        }
    }
    let result = crate::synced_lyrics::fetch(title, artist, duration)?;
    if let Some(value) = &result {
        let _ = std::fs::create_dir_all(cache_root);
        let _ = std::fs::write(cache, serde_json::to_vec(value)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_window_matching_is_strict_and_supports_browser_profile_ids() {
        for (process, source) in [
            ("Spotify.exe", "spotify.EXE"),
            ("vlc.exe", "VLC"),
            ("chrome.exe", "Chrome"),
            ("chrome.exe", "Chrome.Profile 1"),
            ("msedge.exe", "MSEdge.Profile 1"),
        ] {
            assert!(
                matches_player_window(process, source),
                "{process}: {source}"
            );
        }
        for (process, source) in [
            ("spotify-helper.exe", "Spotify.exe"),
            ("chromedriver.exe", "Chrome"),
            ("chrome.exe", "ChromeOther"),
            ("vlc.exe", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            ("Spotify.exe", ""),
            ("", "Spotify.exe"),
        ] {
            assert!(
                !matches_player_window(process, source),
                "{process}: {source}"
            );
        }
    }

    #[test]
    fn missing_player_identity_is_rejected_before_activation() {
        for source in ["", "   ", "Spotify\0.exe"] {
            assert_eq!(
                open_media_player(source, "Song").unwrap_err().code(),
                E_INVALIDARG
            );
        }
    }

    fn media_with_cover(title: &str, cover: &[u8]) -> MediaSnapshot {
        MediaSnapshot {
            title: title.into(),
            artist: "Artist".into(),
            player: "Chrome".into(),
            cover: cover.into(),
            ..Default::default()
        }
    }

    #[test]
    fn delayed_artwork_replaces_browser_icon_without_metadata_change() {
        let mut artwork = ArtworkRefresh::default();
        let now = Instant::now();
        let mut first = media_with_cover("Song", &[]);
        artwork.update(&mut first, &MediaSnapshot::default(), now, || Some(vec![1]));
        let mut pending = first.clone();
        artwork.update(&mut pending, &first, now + Duration::from_secs(1), || {
            panic!("timeline updates must not repeatedly open the thumbnail stream")
        });
        assert_eq!(pending.cover, vec![1]);
        let mut resolved = pending.clone();
        artwork.update(
            &mut resolved,
            &pending,
            now + Duration::from_secs(6),
            || Some(vec![2]),
        );
        assert_eq!(resolved.title, first.title);
        assert_eq!(resolved.cover, vec![2]);
    }

    #[test]
    fn artwork_event_refreshes_same_song_before_periodic_deadline() {
        let mut artwork = ArtworkRefresh::default();
        let now = Instant::now();
        let previous = media_with_cover("Song", &[1]);
        let mut result = previous.clone();
        artwork.update(&mut result, &previous, now, || Some(vec![1]));
        artwork.invalidate();
        let mut updated = result.clone();
        artwork.update(&mut updated, &result, now, || Some(vec![2]));
        assert_eq!(updated.cover, vec![2]);
    }

    #[test]
    fn failed_stream_keeps_current_cover_but_not_previous_tracks_cover() {
        let mut artwork = ArtworkRefresh::default();
        let now = Instant::now();
        let mut previous = media_with_cover("Song A", &[1]);
        previous.accent = Some([0.5, 0.1, 0.2]);
        let mut same = previous.clone();
        artwork.update(&mut same, &previous, now, || None);
        assert_eq!(same.cover, previous.cover);
        assert_eq!(same.accent, previous.accent);
        let mut next = media_with_cover("Song B", &[]);
        artwork.update(&mut next, &same, now, || None);
        assert!(next.cover.is_empty());
        assert_eq!(next.accent, None);
        let mut recovered = next.clone();
        artwork.update(&mut recovered, &next, now + Duration::from_secs(2), || {
            Some(vec![2])
        });
        assert_eq!(recovered.cover, vec![2]);
    }

    #[test]
    fn changing_player_refreshes_artwork_even_with_identical_song_metadata() {
        let mut artwork = ArtworkRefresh::default();
        let now = Instant::now();
        let previous = media_with_cover("Song", &[1]);
        let mut same = previous.clone();
        artwork.update(&mut same, &previous, now, || Some(vec![1]));
        let mut other = same.clone();
        other.player = "Spotify".into();
        artwork.update(&mut other, &same, now, || Some(vec![2]));
        assert_eq!(other.cover, vec![2]);
    }

    #[test]
    fn cache_is_a_sibling_of_the_selected_state_file() {
        let isolated = PathBuf::from("C:/isolated-profile/state.json");
        assert_eq!(
            lyrics_cache_root(&isolated),
            PathBuf::from("C:/isolated-profile/lyrics")
        );
        let regular = PathBuf::from("C:/Users/user/AppData/Local/NusaRebuild/state.json");
        assert_eq!(
            lyrics_cache_root(&regular),
            PathBuf::from("C:/Users/user/AppData/Local/NusaRebuild/lyrics")
        );
    }
    #[test]
    fn lyric_keys_normalize_case_but_do_not_alias_metadata_separators() {
        assert_eq!(
            lyrics_key(" Song ", "ARTIST", 120.1),
            lyrics_key("song", "artist", 120.4)
        );
        assert_ne!(lyrics_key("a|b", "c", 120.), lyrics_key("a", "b|c", 120.));
        assert_ne!(
            lyrics_key("song", "artist", 120.),
            lyrics_key("song", "artist", 121.)
        );
    }
    #[test]
    fn pending_requests_keep_only_the_latest_song() {
        let queue = LyricsWorker {
            queue: Arc::new((Mutex::new(PendingLyrics::default()), Condvar::new())),
        };
        for title in ["first", "intermediate", "latest"] {
            queue.submit(LyricsRequest {
                title: title.into(),
                artist: String::new(),
                duration: 0.,
            });
        }
        let pending = queue.queue.0.lock().unwrap().latest.take().unwrap();
        assert_eq!(pending.title, "latest");
        assert!(queue.queue.0.lock().unwrap().latest.is_none());
    }

    #[test]
    fn test_power_status() {
        let is_lap = super::is_laptop();
        println!("is_laptop: {}", is_lap);
    }

    #[test]
    fn test_clean_media_title() {
        assert_eq!(
            clean_media_title("· SKELETONS (Audio)"),
            "SKELETONS (Audio)"
        );
        assert_eq!(clean_media_title("• Song Name"), "Song Name");
        assert_eq!(clean_media_title("▶ Another Track"), "Another Track");
        assert_eq!(clean_media_title("Normal Title"), "Normal Title");
    }

    #[test]
    fn test_matches_player_app() {
        assert!(matches_player_app(Some("Spotify.exe"), None, "Spotify.exe"));
        assert!(matches_player_app(
            Some("Spotify.exe"),
            None,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"
        ));
        assert!(matches_player_app(Some("chrome.exe"), None, "Chrome"));
        assert!(matches_player_app(
            Some("msedge.exe"),
            None,
            "Microsoft.Edge.Desktop"
        ));
        assert!(matches_player_app(Some("vlc.exe"), None, "vlc"));
        assert!(matches_player_app(
            None,
            Some("C:\\Program Files\\WindowsApps\\SpotifyAB.SpotifyMusic_1.2.3\\Spotify.exe"),
            "spotify"
        ));
        assert!(!matches_player_app(
            Some("Discord.exe"),
            None,
            "Spotify.exe"
        ));
    }
}
