//! Synchronized providers. Plain text is never accepted as synchronized lyrics.
use crate::{letras, lyrics::parse_lrc};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
    thread,
    time::{Duration, Instant},
};
use windows::{core::HSTRING, Foundation::Uri, Web::Http::HttpClient};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncedLyrics {
    pub text: String,
    pub source: String,
}

pub fn same_song(title: &str, artist: &str, other_title: &str, other_artist: &str) -> bool {
    let (title, artist) = letras::clean_song_metadata(title, artist);
    let (other_title, other_artist) = letras::clean_song_metadata(other_title, other_artist);
    if normalize(&title) != normalize(&other_title) {
        return false;
    }
    if artist.is_empty() || other_artist.is_empty() {
        return true;
    }
    let norm_a = normalize(&artist);
    let norm_b = normalize(&other_artist);
    if norm_a == norm_b
        || norm_b.starts_with(&norm_a)
        || norm_a.starts_with(&norm_b)
        || norm_b.contains(&norm_a)
        || norm_a.contains(&norm_b)
    {
        return true;
    }
    other_artist
        .split([',', '/', '&', ';'])
        .map(normalize)
        .any(|part| !part.is_empty() && (part == norm_a || norm_a.contains(&part)))
        || artist
            .split([',', '/', '&', ';'])
            .map(normalize)
            .any(|part| !part.is_empty() && (part == norm_b || norm_b.contains(&part)))
}

fn normalize(text: &str) -> String {
    // Keep version information (live/remix), while tolerating accents and punctuation.
    letras::slugify(&text.replace(['(', ')', '[', ']'], " ")).replace('-', "")
}

pub fn valid_lrc(text: &str) -> bool {
    parse_lrc(text).iter().any(|line| !line.text.is_empty())
}

pub fn is_challenge_page(html: &str) -> bool {
    html.contains("_cf_chl_opt") || html.contains("cf-chl-widget")
}

pub fn select_lrclib(
    entries: &[serde_json::Value],
    title: &str,
    artist: &str,
    duration: f64,
) -> Option<SyncedLyrics> {
    entries
        .iter()
        .filter(|entry| {
            let candidate_title = entry["trackName"].as_str().unwrap_or("");
            let candidate_artist = entry["artistName"].as_str().unwrap_or("");
            let difference = entry["duration"].as_f64().map(|d| (d - duration).abs());
            same_song(title, artist, candidate_title, candidate_artist)
                && (duration <= 0. || difference.is_some_and(|d| d <= 10.))
                && entry["syncedLyrics"].as_str().is_some_and(valid_lrc)
        })
        .min_by(|a, b| {
            let difference = |v: &serde_json::Value| {
                (v["duration"].as_f64().unwrap_or(duration) - duration).abs()
            };
            difference(a).total_cmp(&difference(b))
        })
        .map(|entry| SyncedLyrics {
            text: entry["syncedLyrics"].as_str().unwrap().into(),
            source: "lrclib.net".into(),
        })
}

pub fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn get(url: &str) -> Result<String, Error> {
    retry_temporary_failure(|| get_once(url), thread::sleep)
}

fn retry_temporary_failure<T>(
    mut request: impl FnMut() -> Result<T, Error>,
    mut wait: impl FnMut(Duration),
) -> Result<T, Error> {
    for attempt in 0..3 {
        match request() {
            Err(error) if attempt < 2 && temporary_http_error(error.as_ref()) => {
                wait(Duration::from_millis(300 * (attempt + 1)));
            }
            result => return result,
        }
    }
    unreachable!("the final attempt always returns")
}

fn temporary_http_error(error: &(dyn std::error::Error + 'static)) -> bool {
    // WinRT maps HTTP status failures to HTTP_E_STATUS_* HRESULT values.
    let Some(error) = error.downcast_ref::<windows::core::Error>() else {
        return false;
    };
    let code = error.code().0 as u32;
    code & 0xFFFF0000 == 0x80190000 && matches!(code & 0xFFFF, 408 | 429 | 500 | 502 | 503 | 504)
}

fn get_once(url: &str) -> Result<String, Error> {
    let client = HttpClient::new()?;
    client.DefaultRequestHeaders()?.TryAppendWithoutValidation(
        &HSTRING::from("User-Agent"),
        &HSTRING::from("Nivumi/0.2.0 (Windows desktop lyrics client)"),
    )?;
    let operation = client.GetStringAsync(&Uri::CreateUri(&HSTRING::from(url))?)?;
    let deadline = Instant::now() + Duration::from_secs(12);
    while operation.Status()?.0 == 0 {
        if Instant::now() >= deadline {
            operation.Cancel()?;
            return Err("Lyrics request timed out".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(operation.GetResults()?.to_string())
}

pub fn parse_lyricsify(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("#lyrics_content, .lyrics_content, #lrc, .lrc, .lyrics-body, #lyrics, #lyrics_details, textarea, pre").unwrap();
    document.select(&selector).find_map(|element| {
        let text = letras::element_text(element);
        // Keep metadata/offset tags too, but never surrounding page content.
        let text = text
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('['))
            .collect::<Vec<_>>()
            .join("\n");
        valid_lrc(&text).then_some(text)
    })
}

fn fetch_lyricsify(title: &str, artist: &str) -> Result<Option<SyncedLyrics>, Error> {
    if title.is_empty() || artist.is_empty() {
        return Ok(None);
    }
    let path = format!(
        "/lyrics/{}/{}",
        letras::slugify(artist),
        letras::slugify(&title.replace(['(', ')', '[', ']'], " "))
    );
    // Lyricsify exposes public song pages rather than an LRCLIB-compatible JSON API.
    let url = format!("https://www.lyricsify.com{path}");
    let html = match get(&url) {
        Ok(html) => html,
        Err(http_error) => {
            let Some(browser) = letras::find_browser() else {
                return Err(http_error);
            };
            let profile =
                std::env::temp_dir().join(format!("nusa_lyricsify_profile_{}", std::process::id()));
            std::fs::create_dir_all(&profile)?;
            letras::fetch_dom(&browser, &url, &profile)
                .map_err(|error| format!("{http_error}; browser: {error}"))?
        }
    };
    if is_challenge_page(&html) {
        return Err(
            "Lyricsify requires browser verification; use the extension on the song page".into(),
        );
    }
    Ok(parse_lyricsify(&html).map(|text| SyncedLyrics {
        text,
        source: "lyricsify.com".into(),
    }))
}

pub fn fetch(title: &str, artist: &str, duration: f64) -> Result<Option<SyncedLyrics>, Error> {
    let (title, artist) = letras::clean_song_metadata(title, artist);
    if title.is_empty() {
        return Ok(None);
    }
    let url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        percent_encode(&title),
        percent_encode(&artist)
    );
    let lrclib =
        get(&url).and_then(|body| Ok(serde_json::from_str::<Vec<serde_json::Value>>(&body)?));
    if let Ok(entries) = &lrclib {
        if let Some(found) = select_lrclib(entries, &title, &artist, duration) {
            return Ok(Some(found));
        }
    }
    match fetch_lyricsify(&title, &artist) {
        Ok(Some(found)) => Ok(Some(found)),
        Ok(None) if lrclib.is_ok() => Ok(None),
        Ok(None) => Err(lrclib.unwrap_err()),
        Err(error) => Err(format!(
            "Lyricsify: {error}; LRCLIB: {}",
            match lrclib {
                Ok(_) => "no synchronized match".into(),
                Err(error) => error.to_string(),
            }
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn http_error(status: u32) -> Error {
        Box::new(windows::core::Error::from_hresult(windows::core::HRESULT(
            (0x80190000 | status) as i32,
        )))
    }

    #[test]
    fn temporary_provider_failure_recovers_without_losing_the_song() {
        let mut attempts = 0;
        let mut waits = vec![];
        let result = retry_temporary_failure(
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(http_error(503))
                } else {
                    Ok("[00:01]Recovered")
                }
            },
            |delay| waits.push(delay),
        );
        assert_eq!(result.unwrap(), "[00:01]Recovered");
        assert_eq!(attempts, 2);
        assert_eq!(waits, vec![Duration::from_millis(300)]);
    }

    #[test]
    fn retries_are_bounded_and_browser_verification_is_not_retried() {
        for (status, expected) in [(503, 3), (429, 3), (403, 1), (404, 1)] {
            let mut attempts = 0;
            let result: Result<(), Error> = retry_temporary_failure(
                || {
                    attempts += 1;
                    Err(http_error(status))
                },
                |_| {},
            );
            assert!(result.is_err());
            assert_eq!(attempts, expected);
        }
    }

    #[test]
    fn cleaned_youtube_metadata_matches_without_accepting_another_version() {
        assert!(same_song(
            "Cage The Elephant - Come A Little Closer (Official Video)",
            "CageTheElephantVEVO",
            "Come a Little Closer",
            "Cage The Elephant"
        ));
        assert!(same_song(
            "Mac DeMarco - Chamber Of Reflection (legendado/tradução)",
            "Riverside Traduções",
            "Chamber Of Reflection",
            "Mac DeMarco"
        ));
        assert!(same_song(
            "Chamber of Reflection",
            "Mac DeMarco",
            "Chamber Of Reflection",
            "Mac DeMarco, Mac DeMarco"
        ));
        assert!(!same_song("Song (Live)", "Artist", "Song", "Artist"));
    }

    #[test]
    fn selection_skips_empty_malformed_wrong_song_and_wrong_duration() {
        let entries = vec![
            json!({"trackName":"Song", "artistName":"Artist", "duration":120, "syncedLyrics":""}),
            json!({"trackName":"Song", "artistName":"Artist", "duration":120, "syncedLyrics":"plain text"}),
            json!({"trackName":"Song Remix", "artistName":"Artist", "duration":120, "syncedLyrics":"[00:01]Wrong"}),
            json!({"trackName":"Song", "artistName":"Artist", "duration":150, "syncedLyrics":"[00:01]Wrong version"}),
            json!({"trackName":"Song", "artistName":"Artist", "duration":123, "syncedLyrics":"[00:01]Correct"}),
        ];
        assert_eq!(
            select_lrclib(&entries, "Song", "Artist", 120.)
                .unwrap()
                .text,
            "[00:01]Correct"
        );
    }

    #[test]
    fn chrome_self_aware_selects_timed_entry_among_plain_lyrics() {
        let entries = vec![
            json!({"trackName":"Self Aware", "artistName":"Temper City", "duration":180, "plainLyrics":"Plain lyrics", "syncedLyrics":null}),
            json!({"trackName":"Self Aware", "artistName":"Temper City", "duration":181, "syncedLyrics":"[00:01]Timed verse"}),
        ];
        assert_eq!(
            select_lrclib(
                &entries,
                "Temper City - Self Aware (Official Video)",
                "Temper City",
                181.,
            )
            .unwrap()
            .text,
            "[00:01]Timed verse"
        );
    }

    #[test]
    fn lyricsify_preserves_breaks_entities_and_offset() {
        let html = r#"<div id='lyrics_details'>[ar:Demo]<br>[offset:100]<br>[00:01.00]One &amp; two<br>[00:02.00]Three</div><footer>Advertisement</footer>"#;
        let text = parse_lyricsify(html).unwrap();
        let lines = parse_lrc(&text);
        assert_eq!(lines[0].text, "One & two");
        assert_eq!(lines[0].timestamp_ms, 1100);
        assert_eq!(lines.len(), 2);
        assert!(parse_lyricsify("<pre>Plain text</pre>").is_none());
        assert!(is_challenge_page(
            "<script>window._cf_chl_opt = {};</script>"
        ));
    }
}

#[cfg(test)]
mod guest_match_tests {
    use super::*;
    #[test]
    fn guest_credits_match_but_version_and_duration_are_still_required() {
        let title = "Spitting Off the Edge of the World ft. Perfume Genius (Offici...";
        let base = "Spitting Off the Edge of the World";
        assert!(same_song(title, "Yeah Yeah Yeahs", base, "Yeah Yeah Yeahs"));
        assert!(!same_song(
            title,
            "Yeah Yeah Yeahs",
            &format!("{base} (Live)"),
            "Yeah Yeah Yeahs"
        ));
        let candidates = vec![
            serde_json::json!({"trackName":base,"artistName":"Yeah Yeah Yeahs","duration":257,"syncedLyrics":"[00:01]Album"}),
            serde_json::json!({"trackName":base,"artistName":"Yeah Yeah Yeahs","duration":270.192,"syncedLyrics":"[00:01]Video"}),
        ];
        assert_eq!(
            select_lrclib(&candidates, title, "Yeah Yeah Yeahs", 270.144875)
                .unwrap()
                .text,
            "[00:01]Video"
        );
    }
}
