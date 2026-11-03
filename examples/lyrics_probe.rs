//! Manual network check. Prints counts and provider errors, never lyric contents.
use nivumi::{letras, lyrics, synced_lyrics};
use serde_json::json;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 {
        return Err("Usage: lyrics_probe <title> <artist> <seconds> [--pages]".into());
    }
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let duration: f64 = args[2].parse()?;
    let result = synced_lyrics::fetch(&args[0], &args[1], duration);
    let mut report = json!({"sync": match result {
        Ok(Some(found)) => {
            let lines = lyrics::parse_lrc(&found.text);
            json!({"source":found.source, "lines":lines.len(), "last_timestamp_ms":lines.last().map(|line| line.timestamp_ms)})
        }
        Ok(None) => json!({"status":"unavailable"}),
        Err(error) => json!({"error":error.to_string()}),
    }});
    if args.iter().any(|arg| arg == "--pages") {
        let (title, artist) = letras::clean_song_metadata(&args[0], &args[1]);
        if let Some(browser) = letras::find_browser() {
            let profile = std::env::temp_dir().join(format!("nusa_probe_{}", std::process::id()));
            std::fs::create_dir_all(&profile)?;
            for (provider, url) in [
                (
                    "lyricsify",
                    format!(
                        "https://www.lyricsify.com/lyrics/{}/{}",
                        letras::slugify(&artist),
                        letras::slugify(&title)
                    ),
                ),
                (
                    "letras",
                    format!(
                        "https://www.letras.mus.br/{}/{}/traducao.html",
                        letras::slugify(&artist),
                        letras::slugify(&title)
                    ),
                ),
            ] {
                report[provider] = match letras::fetch_dom(&browser, &url, &profile) {
                    Ok(html) => {
                        std::fs::write(profile.join(format!("{provider}.html")), &html)?;
                        json!({"dom_bytes":html.len(), "lrc_lines":synced_lyrics::parse_lyricsify(&html).map(|text| lyrics::parse_lrc(&text).len()), "translation_lines":letras::parse_translation_lyrics(&html).map(|text| text.lines().count()), "challenge":synced_lyrics::is_challenge_page(&html)})
                    }
                    Err(error) => json!({"error":error}),
                };
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    unsafe {
        RoUninitialize();
    }
    Ok(())
}
