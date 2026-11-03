//! Isolated visual check using synthetic text; never reads/writes the user's state.
use nivumi::{
    lyrics::{align_external_translation, parse_lrc},
    lyrics_view::{LyricsContent, LyricsPreferences},
    lyrics_window::LyricsWindow,
    services::ExternalLyrics,
};
use std::time::{Duration, Instant};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::Ole::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

fn main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
        let original = "[00:00]A long opening verse that should wrap naturally when this independent lyrics window is made narrower\n[00:05]The highlighted verse follows playback while the other lines remain available for reading\n[00:10]A final verse with accents: canção, coração, manhã — and a musical symbol ♪";
        let translation = "Um verso longo para conferir a quebra de linha e a leitura em uma janela menor\nO verso em destaque acompanha a reprodução e a tradução aparece logo abaixo\nUm verso final com acentos: canção, coração, manhã — e um símbolo musical ♪";
        let mut lines = parse_lrc(original);
        align_external_translation(
            &mut lines,
            &ExternalLyrics {
                original: original.into(),
                translation: Some(translation.into()),
                ..Default::default()
            },
        );
        let mut content = LyricsContent {
            key: "isolated-preview".into(),
            title: "Prévia isolada das letras".into(),
            artist: "Texto de teste".into(),
            lines,
            active: Some(0),
            original: original.into(),
            translation: Some(translation.into()),
            ..Default::default()
        };
        let mut window = LyricsWindow::open(HWND::default(), LyricsPreferences::default(), false)?;
        window.set_accent([0.32, 0.12, 0.16]);
        window.update(content.clone());
        let started = Instant::now();
        let mut msg = MSG::default();
        while started.elapsed() < Duration::from_secs(600) && IsWindowVisible(window.hwnd).as_bool()
        {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if !IsDialogMessageW(window.hwnd, &msg).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            content.active = Some((started.elapsed().as_secs() / 5 % 3) as usize);
            window.update(content.clone());
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(window);
        OleUninitialize();
        Ok(())
    }
}
