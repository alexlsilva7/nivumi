use crate::services::ExternalLyrics;
use std::{
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, Condvar, Mutex},
    thread,
};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Locate Google Chrome or Microsoft Edge executable on Windows
pub fn find_browser() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(prog_files) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(&prog_files).join(r"Google\Chrome\Application\chrome.exe"));
        candidates.push(PathBuf::from(&prog_files).join(r"Microsoft\Edge\Application\msedge.exe"));
    }
    if let Ok(prog_files_x86) = std::env::var("ProgramFiles(x86)") {
        candidates
            .push(PathBuf::from(&prog_files_x86).join(r"Google\Chrome\Application\chrome.exe"));
        candidates
            .push(PathBuf::from(&prog_files_x86).join(r"Microsoft\Edge\Application\msedge.exe"));
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        candidates
            .push(PathBuf::from(&local_app_data).join(r"Google\Chrome\Application\chrome.exe"));
    }

    candidates.into_iter().find(|p| p.is_file())
}

/// Convert artist or song title into a normalized Letras.mus.br URL slug
pub fn slugify(s: &str) -> String {
    let lower = s.to_lowercase();

    // Strip content in parentheses e.g. (feat. ...) or brackets [remastered]
    let mut cleaned = String::new();
    let mut depth = 0;
    for ch in lower.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ if depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }

    // Replace accented characters
    let mut result = String::new();
    for ch in cleaned.chars() {
        match ch {
            '.' | '\'' | '’' | '`' => {}
            'á' | 'à' | 'ã' | 'â' | 'ä' => result.push('a'),
            'é' | 'è' | 'ê' | 'ë' => result.push('e'),
            'í' | 'ì' | 'î' | 'ï' => result.push('i'),
            'ó' | 'ò' | 'õ' | 'ô' | 'ö' => result.push('o'),
            'ú' | 'ù' | 'û' | 'ü' => result.push('u'),
            'ç' => result.push('c'),
            'ñ' => result.push('n'),
            'a'..='z' | '0'..='9' => result.push(ch),
            _ => result.push('-'),
        }
    }

    // Collapse multiple hyphens and trim
    let mut final_slug = String::new();
    let mut prev_hyphen = false;
    for ch in result.chars() {
        if ch == '-' {
            if !prev_hyphen && !final_slug.is_empty() {
                final_slug.push('-');
                prev_hyphen = true;
            }
        } else {
            final_slug.push(ch);
            prev_hyphen = false;
        }
    }

    if final_slug.ends_with('-') {
        final_slug.pop();
    }

    final_slug
}

/// Fetch complete rendered DOM of a URL via background headless browser
pub fn fetch_dom(browser: &Path, url: &str, temp_dir: &Path) -> Result<String, String> {
    if LETRAS_DEADLINE
        .with(|d| d.get())
        .is_some_and(|d| std::time::Instant::now() >= d)
    {
        return Err("Letras timeout".into());
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // A pipe can fill before try_wait completes; write the DOM to a bounded-lifetime file.
    let output_path = temp_dir.join("nusa-dom.html");
    let output = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(browser)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", temp_dir.display()))
        .arg(format!("--user-agent={USER_AGENT}"))
        .arg("--dump-dom")
        .arg("--timeout=12000")
        .arg(url)
        .stdout(output)
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = LETRAS_DEADLINE
        .with(|d| d.get())
        .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(20));
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => break,
            Some(status) => {
                // Chrome can return a timeout status after successfully dumping the DOM.
                if let Ok(dom) = std::fs::read_to_string(&output_path) {
                    if !dom.trim().is_empty() {
                        return Ok(dom);
                    }
                }
                return Err(format!("Browser exited with status {status}"));
            }
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Browser lyrics request timed out".into());
            }
            None => thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    std::fs::read_to_string(output_path).map_err(|e| e.to_string())
}

/// Filter out ad and promo text from Letras.mus.br
fn is_ad_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("pague uma vez")
        || lower.contains("assinar o letras")
        || lower.contains("ano inteiro")
        || lower.contains("contextualplan")
        || lower.contains("fc-ab-")
        || lower.contains("funding choices")
        || lower.contains("bloqueador de anúncios")
        || lower.contains("player-media")
        || lower.contains("curta as suas músicas sem interrupções")
        || lower.contains("r$ /ano")
        || (lower.contains("r$") && lower.contains("/ano"))
        || lower.contains("experimente grátis")
        || lower.contains("sem anúncios")
        || lower.contains("letras premium")
}

/// Clean artist and title metadata from common YouTube/VEVO/Audio video patterns
pub fn clean_song_metadata(raw_title: &str, raw_artist: &str) -> (String, String) {
    clean_metadata(raw_title, raw_artist, true)
}

fn ignored_terms() -> &'static std::sync::RwLock<Vec<String>> {
    static TERMS: std::sync::OnceLock<std::sync::RwLock<Vec<String>>> = std::sync::OnceLock::new();
    TERMS.get_or_init(|| {
        std::sync::RwLock::new(crate::lyrics_view::LyricsPreferences::default().ignored_title_terms)
    })
}

/// Publish saved search preferences to all lyric providers and import matching.
pub fn set_ignored_title_terms(terms: &[String]) {
    *ignored_terms()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        normalize_ignored_terms(terms.iter().map(String::as_str));
}

pub fn title_filter_key() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    ignored_terms()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .hash(&mut hash);
    hash.finish()
}

pub fn normalize_ignored_terms<'a>(terms: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    terms
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .filter(|term| !term.is_empty() && seen.insert(term.to_lowercase()))
        .map(str::to_owned)
        .collect()
}

fn strip_ignored_title_terms(title: &str, terms: &[String]) -> String {
    let mut title = title.to_owned();
    for term in terms.iter().filter(|term| !term.trim().is_empty()) {
        let term = term.to_lowercase();
        loop {
            // Track original UTF-8 boundaries because lowercase can change byte lengths.
            let mut lowercase = String::new();
            let mut boundaries = Vec::new();
            for (index, ch) in title.char_indices() {
                boundaries.push((lowercase.len(), index));
                lowercase.extend(ch.to_lowercase());
            }
            boundaries.push((lowercase.len(), title.len()));
            let range = lowercase.match_indices(&term).find_map(|(start, _)| {
                let end = start + term.len();
                let from = boundaries
                    .binary_search_by_key(&start, |&(lower, _)| lower)
                    .ok()?;
                let to = boundaries
                    .binary_search_by_key(&end, |&(lower, _)| lower)
                    .ok()?;
                Some(boundaries[from].1..boundaries[to].1)
            });
            let Some(range) = range else { break };
            title.replace_range(range, " ");
        }
    }
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_metadata(raw_title: &str, raw_artist: &str, remove_guests: bool) -> (String, String) {
    let terms = ignored_terms()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clean_metadata_with_terms(raw_title, raw_artist, remove_guests, &terms)
}

fn clean_metadata_with_terms(
    raw_title: &str,
    raw_artist: &str,
    remove_guests: bool,
    terms: &[String],
) -> (String, String) {
    let mut artist = raw_artist.trim().to_string();
    let mut title = strip_ignored_title_terms(raw_title, terms);

    let lower_raw_artist = raw_artist.to_lowercase();
    let is_from_youtube = lower_raw_artist.contains("youtube");

    for suffix in [
        " on youtube music",
        " - youtube music",
        " on youtube",
        " - youtube",
    ] {
        if artist.to_lowercase().ends_with(suffix) {
            let len = artist.len() - suffix.len();
            artist = artist[..len].trim().to_string();
            break;
        }
    }

    // 1. Clean VEVO or - Topic suffixes from artist
    if artist.to_lowercase().ends_with("vevo") {
        let len = artist.len() - 4;
        artist = artist[..len].trim().to_string();
    }
    if artist.to_lowercase().ends_with("- topic") {
        let len = artist.len() - 7;
        artist = artist[..len].trim().to_string();
    }
    if [
        "youtube",
        "chrome",
        "edge",
        "spotify",
        "navegador",
        "browser",
        "msedge",
    ]
    .contains(&artist.to_lowercase().as_str())
    {
        artist.clear();
    }

    // 2. If title contains " - ", e.g. "Tame Impala - The Less I Know The Better"
    if let Some(idx) = title.find(" - ") {
        let left = title[..idx].trim();
        let right = title[idx + 3..].trim();
        let left_norm: String = left.chars().filter(|c| c.is_alphanumeric()).collect();
        let artist_norm: String = artist.chars().filter(|c| c.is_alphanumeric()).collect();
        let lower_artist = artist.to_lowercase();
        let lower_right = right.to_lowercase();
        let is_channel_or_label = is_from_youtube
            || lower_artist.ends_with(" records")
            || lower_artist.ends_with(" official")
            || lower_artist.ends_with(" oficial")
            || lower_artist.ends_with(" channel")
            || lower_artist.ends_with(" canal")
            || lower_artist.contains("traduç")
            || lower_artist.contains("traduc")
            || lower_artist.contains("legend")
            || lower_artist.contains("lyrics")
            || lower_artist.contains("letra")
            || lower_artist.contains(" vevo")
            || lower_artist.ends_with("vevo")
            || lower_artist.contains("sub")
            || lower_artist.contains("music")
            || lower_artist.contains("música")
            || lower_artist.contains("musica")
            || lower_artist.contains("clipe")
            || lower_artist.contains("audio")
            || lower_artist.contains("áudio")
            || lower_artist.contains("entertainment")
            || lower_artist.contains("ent.")
            || lower_artist.ends_with(" topic")
            || lower_artist.ends_with("- topic");
        let is_video_title = lower_right.contains("legendad")
            || lower_right.contains("traduç")
            || lower_right.contains("traduc")
            || lower_right.contains("legendas")
            || lower_right.contains("subtitul")
            || lower_right.contains("sub español")
            || lower_right.contains("sub espanol")
            || lower_right.contains("official video")
            || lower_right.contains("music video")
            || lower_right.contains("official audio")
            || lower_right.contains("clipe oficial")
            || lower_right.contains("vídeo oficial")
            || lower_right.contains("video oficial")
            || lower_right.contains("áudio oficial")
            || lower_right.contains("audio oficial")
            || lower_right.contains("lyric video")
            || lower_right.contains("visualizer")
            || lower_right.contains("(lyrics)")
            || lower_right.contains("[lyrics]")
            || lower_right.contains("(letra)")
            || lower_right.contains("[letra]");
        if artist.is_empty()
            || ((is_channel_or_label || is_video_title) && !left.is_empty() && !right.is_empty())
            || left_norm.eq_ignore_ascii_case(&artist_norm)
            || (!artist_norm.is_empty()
                && (left_norm
                    .to_lowercase()
                    .contains(&artist_norm.to_lowercase())
                    || artist_norm
                        .to_lowercase()
                        .contains(&left_norm.to_lowercase())))
        {
            artist = left.to_string();
            title = right.to_string();
        }
    }

    // 3. Strip bracketed/parenthesized video junk from title
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let Some((start, end)) =
            title
                .char_indices()
                .filter(|(_, c)| *c == open)
                .find_map(|(start, _)| {
                    let end = title[start..].find(close)? + start;
                    let inner = title[start + 1..end].trim().to_lowercase();
                    let is_junk = inner.contains("legendad")
                        || inner.contains("traduç")
                        || inner.contains("traduc")
                        || inner.contains("legendas")
                        || inner.contains("legenda")
                        || inner.contains("subtitul")
                        || inner.contains("sub español")
                        || inner.contains("sub espanol")
                        || inner.contains("sub english")
                        || inner.contains("sub pt")
                        || inner.contains("official video")
                        || inner.contains("official music video")
                        || inner.contains("official audio")
                        || inner.contains("official visualizer")
                        || inner.contains("official animated")
                        || inner.contains("music video")
                        || inner.contains("clipe oficial")
                        || inner.contains("vídeo oficial")
                        || inner.contains("video oficial")
                        || inner.contains("áudio oficial")
                        || inner.contains("audio oficial")
                        || inner.contains("videoclipe")
                        || inner.contains("lyric video")
                        || inner.contains("visualizer")
                        || inner.contains("full album")
                        || matches!(
                            inner.as_str(),
                            "official"
                                | "audio"
                                | "video"
                                | "clipe"
                                | "lyrics"
                                | "lyric"
                                | "letra"
                                | "letras"
                                | "4k"
                                | "hd"
                                | "hq"
                                | "sub"
                                | "subs"
                        );
                    if is_junk {
                        Some((start, end + close.len_utf8()))
                    } else {
                        None
                    }
                })
        {
            title.replace_range(start..end, " ");
            title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }

    // Strip trailing tags after separators (e.g. "Song | Legendado", "Song // Tradução", "Song - Legendado")
    for sep in [" | ", " // ", " - "] {
        if let Some(pos) = title.rfind(sep) {
            let tail = title[pos + sep.len()..].trim().to_lowercase();
            if tail.contains("legendad")
                || tail.contains("traduç")
                || tail.contains("traduc")
                || tail.contains("legendas")
                || tail.contains("subtitul")
                || tail.contains("official video")
                || tail.contains("clipe oficial")
                || tail.contains("video oficial")
                || tail.contains("vídeo oficial")
                || tail.contains("audio oficial")
                || tail.contains("áudio oficial")
                || tail.contains("music video")
                || tail.contains("lyric")
                || tail.contains("letra")
                || matches!(
                    tail.as_str(),
                    "official" | "clipe" | "video" | "audio" | "4k" | "hd"
                )
            {
                title.truncate(pos);
                title = title.trim().to_string();
            }
        }
    }

    // 4. Strip video junk from title: (Official Video), (Lyric Video), [Audio], etc.
    let junk_patterns = [
        "(official animated music video)",
        "[official animated music video]",
        "(legendado/tradução)",
        "[legendado/tradução]",
        "(legendado/traducao)",
        "[legendado/traducao]",
        "(tradução/legendado)",
        "[tradução/legendado]",
        "(traducao/legendado)",
        "[traducao/legendado]",
        "(legendado)",
        "[legendado]",
        "(legendas)",
        "[legendas]",
        "(tradução)",
        "[tradução]",
        "(traducao)",
        "[traducao]",
        "(subtitulado)",
        "[subtitulado]",
        "(sub español)",
        "[sub español]",
        "(sub espanol)",
        "[sub espanol]",
        "(sub pt-br)",
        "[sub pt-br]",
        "(sub pt)",
        "[sub pt]",
        "(letra)",
        "[letra]",
        "(lyrics)",
        "[lyrics]",
        "(official music video)",
        "(official video)",
        "(official audio)",
        "(clipe oficial)",
        "(vídeo oficial)",
        "(video oficial)",
        "(áudio oficial)",
        "(audio oficial)",
        "(lyric video)",
        "(visualizer)",
        "(official)",
        "(audio)",
        "(video)",
        "[official music video]",
        "[official video]",
        "[official audio]",
        "[clipe oficial]",
        "[vídeo oficial]",
        "[video oficial]",
        "[lyric video]",
        "[visualizer]",
        "[official]",
        "[audio]",
        "[video]",
    ];
    for junk in junk_patterns {
        let junk_lower = junk.to_lowercase();
        loop {
            let title_lower = title.to_lowercase();
            if let Some(pos) = title_lower.find(&junk_lower) {
                let char_idx = title_lower[..pos].chars().count();
                let junk_char_len = junk_lower.chars().count();
                let start_byte = title
                    .char_indices()
                    .nth(char_idx)
                    .map(|(i, _)| i)
                    .unwrap_or(pos);
                let end_byte = title
                    .char_indices()
                    .nth(char_idx + junk_char_len)
                    .map(|(i, _)| i)
                    .unwrap_or_else(|| title.len());
                title.replace_range(start_byte..end_byte, " ");
                title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            } else {
                break;
            }
        }
    }

    // GSMTC may already truncate the video's suffix before returning it to us.
    if title.ends_with("...") || title.ends_with('…') {
        if let Some(start) = title.rfind('(') {
            if title[start..].to_lowercase().starts_with("(offic") {
                title.truncate(start);
                title = title.trim().to_owned();
            }
        }
    }
    if remove_guests {
        title = without_featured_credit(&title);
    }
    (title, artist)
}

fn without_featured_credit(title: &str) -> String {
    let mut result = title.to_owned();
    // Only remove participation credits, retaining other version annotations.
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let Some((start, end)) =
            result
                .char_indices()
                .filter(|(_, c)| *c == open)
                .find_map(|(start, _)| {
                    let end = result[start..].find(close)? + start;
                    let content = result[start + 1..end].trim().to_lowercase();
                    ["feat.", "feat․", "ft.", "featuring "]
                        .iter()
                        .any(|p| content.starts_with(p))
                        .then_some((start, end))
                })
        {
            result.replace_range(start..=end, "");
        }
    }
    let lower = result.to_ascii_lowercase();
    if let Some(start) = [" feat. ", " feat․ ", " ft. ", " featuring "]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
    {
        let version = result[start..]
            .find('(')
            .map(|offset| result[start + offset..].trim().to_owned())
            .unwrap_or_default();
        result = format!("{} {}", result[..start].trim(), version);
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Read text in DOM order, preserving verse and paragraph boundaries.
pub fn element_text(element: scraper::ElementRef<'_>) -> String {
    fn visit(element: scraper::ElementRef<'_>, output: &mut String) {
        let name = element.value().name();
        if matches!(name, "script" | "style" | "button" | "iframe") {
            return;
        }
        let class = element.value().attr("class").unwrap_or("");
        if ["contextualPlan", "fc-ab-", "player-media", "cnt-ad"]
            .iter()
            .any(|marker| class.contains(marker))
        {
            return;
        }
        if name == "br" {
            output.push('\n');
            return;
        }
        for child in element.children() {
            if let Some(text) = child.value().as_text() {
                output.push_str(text);
            } else if let Some(child) = scraper::ElementRef::wrap(child) {
                visit(child, output);
            }
        }
        if matches!(name, "p" | "div" | "pre") || element.value().classes().any(|c| c == "verse") {
            output.push('\n');
        }
    }
    let mut text = String::new();
    visit(element, &mut text);
    text.replace('\u{a0}', " ")
        .lines()
        .map(str::trim)
        .filter(|line| !is_ad_text(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

pub fn clean_html(input: &str) -> String {
    element_text(scraper::Html::parse_fragment(input).root_element())
}

/// Parse only the original container; never collect paragraphs elsewhere on the page.
pub fn parse_original_lyrics(html: &str) -> Option<(String, String)> {
    use scraper::{Html, Selector};
    let document = Html::parse_document(html);
    let title = document
        .select(&Selector::parse("h1").unwrap())
        .next()
        .map(element_text)
        .unwrap_or_default();
    for selector in [".lyric-original", ".cnt-letra"] {
        if let Some(text) = document
            .select(&Selector::parse(selector).unwrap())
            .map(element_text)
            .find(|text| text.len() >= 20)
        {
            return Some((title, text));
        }
    }
    None
}

/// Translation is confined to the Portuguese container, including its verse spans.
pub fn parse_translation_lyrics(html: &str) -> Option<String> {
    use scraper::{Html, Selector};
    let document = Html::parse_document(html);
    for selector in [
        ".lyric-translation",
        ".lyric-traducao",
        ".cnt-trad",
        ".cnt-letra-trad",
    ] {
        for container in document.select(&Selector::parse(selector).unwrap()) {
            let verses = container
                .select(&Selector::parse(".verse").unwrap())
                .map(|verse| {
                    let first = verse
                        .children()
                        .filter_map(scraper::ElementRef::wrap)
                        .find(|child| !child.value().classes().any(|c| c == "romanization"));
                    element_text(first.unwrap_or(verse))
                })
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            let text = if verses.is_empty() {
                element_text(container)
            } else {
                verses.join("\n")
            };
            if text.len() >= 20 {
                return Some(text);
            }
        }
    }
    None
}

/// Extract first song link from search results page (backwards-compatible)
pub fn find_song_url_from_search(html: &str) -> Option<String> {
    find_matching_song_url_from_search(html, "", "")
}

/// Extract best matching song link from search results page based on title and artist
pub fn find_matching_song_url_from_search(
    html: &str,
    expected_title: &str,
    expected_artist: &str,
) -> Option<String> {
    use scraper::{Html, Selector};
    let document = Html::parse_document(html);
    let selector = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return None,
    };

    let exp_title_clean = expected_title.trim();
    let exp_artist_clean = expected_artist.trim();
    let exp_song_slug = slugify(exp_title_clean);
    let exp_artist_slug = slugify(exp_artist_clean);

    let mut best_url: Option<String> = None;
    let mut best_score: i32 = 0;

    for element in document.select(&selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };

        // Normalize URL to /artist/song/
        let relative = if let Some(stripped) = href.strip_prefix("https://www.letras.mus.br/") {
            stripped
        } else if let Some(stripped) = href.strip_prefix("http://www.letras.mus.br/") {
            stripped
        } else if let Some(stripped) = href.strip_prefix('/') {
            stripped
        } else {
            continue;
        };

        let parts: Vec<&str> = relative.trim_matches('/').split('/').collect();

        if parts.len() != 2 {
            continue;
        }

        let artist_part = parts[0];
        let song_part = parts[1];

        let is_invalid = [
            "fotos",
            "discografia",
            "biografia",
            "albuns",
            "top",
            "categoria",
            "estilos",
            "blog",
            "termos",
            "privacidade",
            "contato",
            "mais-acessadas",
            "player",
            "musica",
            "app",
            "login",
            "cadastro",
        ]
        .contains(&artist_part)
            || artist_part.contains('#')
            || artist_part.contains('?')
            || song_part.contains('#')
            || song_part.contains('?');

        if is_invalid {
            continue;
        }

        let full_url = format!("https://www.letras.mus.br/{artist_part}/{song_part}/");

        // If no title expected, return first valid song URL (backwards-compatible)
        if exp_song_slug.is_empty() {
            return Some(full_url);
        }

        let cand_song_slug = slugify(song_part);
        let cand_artist_slug = slugify(artist_part);
        let anchor_text = element_text(element);
        let anchor_slug = slugify(&anchor_text);

        // Score title match
        let mut title_score = 0;
        if cand_song_slug == exp_song_slug {
            title_score = 60;
        } else if !anchor_slug.is_empty() && anchor_slug == exp_song_slug {
            title_score = 55;
        } else if cand_song_slug.replace('-', "") == exp_song_slug.replace('-', "")
            || crate::synced_lyrics::same_song(
                exp_title_clean,
                exp_artist_clean,
                &anchor_text,
                artist_part,
            )
        {
            title_score = 50;
        } else if cand_song_slug.starts_with(&format!("{exp_song_slug}-")) {
            title_score = 40;
        } else if exp_song_slug.starts_with(&format!("{cand_song_slug}-")) {
            title_score = 35;
        }

        // If title does not match at all, this is an unrelated song (e.g. "oasis" vs "always-on-time")
        if title_score == 0 {
            continue;
        }

        // Score artist match
        let artist_score = if !exp_artist_slug.is_empty() {
            if cand_artist_slug == exp_artist_slug {
                40
            } else if cand_artist_slug.contains(&exp_artist_slug)
                || exp_artist_slug.contains(&cand_artist_slug)
            {
                30
            } else if cand_artist_slug.replace('-', "") == exp_artist_slug.replace('-', "") {
                35
            } else {
                -20
            }
        } else {
            10
        };

        let total_score = title_score + artist_score;
        if total_score > best_score {
            best_score = total_score;
            best_url = Some(full_url);
        }
    }

    best_url
}

fn is_matching_song(
    expected_title: &str,
    expected_artist: &str,
    parsed_title: &str,
    song_slug: &str,
) -> bool {
    let exp_title = expected_title.trim();
    if exp_title.is_empty() {
        return true;
    }
    let exp_slug = slugify(exp_title);
    let cand_slug = slugify(song_slug);
    let parsed_slug = slugify(parsed_title);

    // 1. Slug matches
    if !cand_slug.is_empty()
        && (cand_slug == exp_slug
            || cand_slug.starts_with(&format!("{exp_slug}-"))
            || exp_slug.starts_with(&format!("{cand_slug}-"))
            || cand_slug.replace('-', "") == exp_slug.replace('-', ""))
    {
        return true;
    }

    // 2. Parsed title from <h1> matches
    if !parsed_slug.is_empty()
        && (parsed_slug == exp_slug
            || parsed_slug.starts_with(&format!("{exp_slug}-"))
            || exp_slug.starts_with(&format!("{parsed_slug}-"))
            || parsed_slug.replace('-', "") == exp_slug.replace('-', ""))
    {
        return true;
    }

    // 3. same_song logic (handles features, casing, punctuation)
    if crate::synced_lyrics::same_song(exp_title, expected_artist, parsed_title, expected_artist) {
        return true;
    }

    false
}

/// Fetch lyrics and Portuguese translation from Letras.mus.br automatically in background
pub fn fetch_letras(raw_title: &str, raw_artist: &str) -> Option<ExternalLyrics> {
    let filtered_title = {
        let terms = ignored_terms()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        strip_ignored_title_terms(raw_title, &terms)
    };
    let raw_title = filtered_title.as_str();
    let browser = find_browser()?;
    let temp_dir = std::env::temp_dir().join(format!("nusa_letras_profile_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    let (clean_title, clean_artist) = clean_song_metadata(raw_title, raw_artist);

    let mut attempts = Vec::new();
    if !clean_artist.is_empty() && !clean_title.is_empty() {
        let clean_slug_a = slugify(&clean_artist);
        let clean_slug_t = slugify(&clean_title);
        attempts.push((clean_slug_a.clone(), clean_slug_t.clone()));
        let hyphen_a = slugify(&clean_artist.replace('.', "-"));
        if hyphen_a != clean_slug_a && !hyphen_a.is_empty() {
            attempts.push((hyphen_a, clean_slug_t));
        }
    }

    // Letras includes guest credits in some canonical URLs, unlike LRCLIB.
    let (credited_title, _) = clean_metadata(raw_title, raw_artist, false);
    let credited_title = credited_title
        .replace(" ft. ", " feat ")
        .replace(" feat. ", " feat ")
        .replace(['(', ')', '[', ']'], " ");
    let credited_slug = slugify(&credited_title);
    if !clean_artist.is_empty()
        && !credited_slug.is_empty()
        && !attempts.iter().any(|(_, title)| title == &credited_slug)
    {
        attempts.insert(0, (slugify(&clean_artist), credited_slug));
    }

    let raw_slug_a = slugify(raw_artist);
    let raw_slug_s = slugify(raw_title);
    if (!raw_slug_a.is_empty() && !raw_slug_s.is_empty())
        && (!attempts
            .iter()
            .any(|(a, s)| a == &raw_slug_a && s == &raw_slug_s))
    {
        attempts.push((raw_slug_a, raw_slug_s));
    }

    let mut target_song_url = None;
    let mut parsed_data = None;

    let query_title = if !clean_title.is_empty() {
        &clean_title
    } else {
        raw_title
    };
    let query_artist = if !clean_artist.is_empty() {
        &clean_artist
    } else {
        raw_artist
    };

    // 1. Try direct URL slug
    for (slug_a, slug_s) in attempts {
        let direct_url = format!("https://www.letras.mus.br/{slug_a}/{slug_s}/");
        if let Ok(dom) = fetch_dom(&browser, &direct_url, &temp_dir) {
            if let Some((p_title, lyrics)) = parse_original_lyrics(&dom) {
                if is_matching_song(query_title, query_artist, &p_title, &slug_s) {
                    target_song_url = Some(direct_url);
                    parsed_data = Some((p_title, lyrics));
                    break;
                }
            }
        }
    }

    // 2. Fallback to search query if direct slug wasn't found
    if parsed_data.is_none() {
        let mut search_queries = Vec::new();
        if !clean_artist.is_empty() || !clean_title.is_empty() {
            let q = format!("{clean_artist} {clean_title}").trim().to_string();
            if !q.is_empty() {
                search_queries.push(q);
            }
        }
        let raw_q = format!("{raw_artist} {raw_title}").trim().to_string();
        if !raw_q.is_empty() && !search_queries.contains(&raw_q) {
            search_queries.push(raw_q);
        }

        for query in search_queries {
            let query_enc = urlencoding_simple(&query);
            let search_url = format!("https://www.letras.mus.br/?q={query_enc}");
            if let Ok(search_dom) = fetch_dom(&browser, &search_url, &temp_dir) {
                if let Some(song_url) =
                    find_matching_song_url_from_search(&search_dom, query_title, query_artist)
                {
                    if let Ok(dom) = fetch_dom(&browser, &song_url, &temp_dir) {
                        if let Some((p_title, lyrics)) = parse_original_lyrics(&dom) {
                            let song_part = song_url
                                .trim_end_matches('/')
                                .rsplit('/')
                                .next()
                                .unwrap_or("");
                            if is_matching_song(query_title, query_artist, &p_title, song_part) {
                                target_song_url = Some(song_url);
                                parsed_data = Some((p_title, lyrics));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    let (mut song_title, mut lyrics) = parsed_data?;
    if song_title.is_empty() {
        song_title = if !clean_title.is_empty() {
            clean_title
        } else {
            raw_title.to_string()
        };
    }

    let actual_url = target_song_url.unwrap_or_default();

    // 3. Try to fetch Portuguese translation
    let mut translation = None;
    if !actual_url.is_empty() {
        let base_url = actual_url.trim_end_matches('/');
        let trans_url = format!("{base_url}/traducao.html");
        if let Ok(trans_dom) = fetch_dom(&browser, &trans_url, &temp_dir) {
            translation = parse_translation_lyrics(&trans_dom);
            // Keep both sides from the same edition and verse layout when possible.
            if translation.is_some() {
                if let Some((_, original)) = parse_original_lyrics(&trans_dom) {
                    lyrics = original;
                }
            }
        }
    }

    let final_artist = if !clean_artist.is_empty() {
        clean_artist
    } else {
        raw_artist.to_string()
    };

    Some(ExternalLyrics {
        translation_origin: Default::default(),
        title: song_title,
        artist: final_artist,
        original: lyrics,
        translation,
        source: "letras.mus.br".into(),
    })
}

fn urlencoding_simple(s: &str) -> String {
    let mut enc = String::new();
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                enc.push(b as char)
            }
            b' ' => enc.push('+'),
            _ => enc.push_str(&format!("%{b:02X}")),
        }
    }
    enc
}

#[derive(Clone)]
pub struct LetrasRequest {
    pub title: String,
    pub artist: String,
}

#[derive(Default)]
struct PendingLetras {
    latest: Option<LetrasRequest>,
    stopped: bool,
}

// Background worker with single-slot replaceable queue for Letras.mus.br
thread_local! { static LETRAS_DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) }; }
pub struct LetrasWorker {
    queue: Arc<(Mutex<PendingLetras>, Condvar)>,
}

impl LetrasWorker {
    pub fn spawn(events: Sender<crate::services::ServiceEvent>) -> Self {
        let queue = Arc::new((Mutex::new(PendingLetras::default()), Condvar::new()));
        let worker_queue = queue.clone();

        thread::spawn(move || {
            let initialized = unsafe {
                windows::Win32::System::WinRT::RoInitialize(
                    windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
                )
            }
            .is_ok();
            let (lock, ready) = &*worker_queue;
            loop {
                let req = {
                    let mut pending = lock.lock().unwrap();
                    while !pending.stopped && pending.latest.is_none() {
                        pending = ready.wait(pending).unwrap();
                    }
                    if pending.stopped {
                        break;
                    }
                    pending.latest.take()
                };

                if let Some(req) = req {
                    let key = crate::services::lyrics_key(&req.title, &req.artist, 0.);
                    LETRAS_DEADLINE.with(|d| {
                        d.set(Some(
                            std::time::Instant::now() + std::time::Duration::from_secs(20),
                        ))
                    });
                    let online=windows::Networking::Connectivity::NetworkInformation::GetInternetConnectionProfile().ok().and_then(|p|p.GetNetworkConnectivityLevel().ok()).is_some_and(|l|l==windows::Networking::Connectivity::NetworkConnectivityLevel::InternetAccess || l==windows::Networking::Connectivity::NetworkConnectivityLevel::ConstrainedInternetAccess);
                    if let Some(ext) = online
                        .then(|| fetch_letras(&req.title, &req.artist))
                        .flatten()
                    {
                        let _ = events.send(crate::services::ServiceEvent::LetrasLyrics {
                            key: key.clone(),
                            lyrics: ext,
                        });
                    }
                    let _ = events.send(crate::services::ServiceEvent::LetrasFinished { key });
                    LETRAS_DEADLINE.with(|d| d.set(None));
                }
            }
            if initialized {
                unsafe {
                    windows::Win32::System::WinRT::RoUninitialize();
                }
            }
        });

        Self { queue }
    }

    pub fn submit(&self, req: LetrasRequest) {
        let (lock, ready) = &*self.queue;
        if let Ok(mut pending) = lock.lock() {
            pending.latest = Some(req);
            ready.notify_one();
        }
    }
}

impl Drop for LetrasWorker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.queue;
        if let Ok(mut pending) = lock.lock() {
            pending.stopped = true;
            ready.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("SIAMÉS"), "siames");
        assert_eq!(slugify("Mr. FEAR"), "mr-fear");
        assert_eq!(slugify("Mr. FEAR (feat. Someone)"), "mr-fear");
        assert_eq!(slugify("Água e Café [Remastered]"), "agua-e-cafe");
        assert_eq!(slugify("Fontaines D.C."), "fontaines-dc");
        assert_eq!(slugify("R.E.M."), "rem");
        assert_eq!(
            slugify("Don't Look Back In Anger"),
            "dont-look-back-in-anger"
        );
    }

    #[test]
    fn test_clean_html() {
        let raw = "<p>Hello&nbsp;world!<br>How are you? &quot;fine&quot;</p>";
        assert_eq!(clean_html(raw), "Hello world!\nHow are you? \"fine\"");
    }

    #[test]
    fn test_parse_original_lyrics() {
        let html = r#"
            <h1>Mr. Fear</h1>
            <div class="cnt-letra">
                <p>Line 1<br>Line 2</p>
                <p>Line 3<br>Line 4</p>
            </div>
        "#;
        let res = parse_original_lyrics(html);
        assert!(res.is_some());
        let (title, lyrics) = res.unwrap();
        assert_eq!(title, "Mr. Fear");
        assert!(lyrics.contains("Line 1\nLine 2"));
        assert!(lyrics.contains("Line 3\nLine 4"));
    }

    #[test]
    fn test_parse_ignores_adblock_modal() {
        let html = r#"
            <h1>Mr. Fear</h1>
            <div class="cnt-letra">
                <p class="fc-ab-root">Desative seu bloqueador de anúncios para continuar lendo</p>
                <p>Hello, my name is Mr. Fear<br>I wish I had a faster therapy</p>
                <p class="player-media">Video ad player</p>
            </div>
        "#;
        let res = parse_original_lyrics(html);
        assert!(res.is_some());
        let (title, lyrics) = res.unwrap();
        assert_eq!(title, "Mr. Fear");
        assert!(lyrics.contains("Hello, my name is Mr. Fear"));
        assert!(!lyrics.contains("Desative seu bloqueador"));
        assert!(!lyrics.contains("Video ad player"));
    }

    #[test]
    fn test_parse_translation_lyrics() {
        let html = r#"
            <div class="lyric-translation"><span class="verse"><span>Olá mundo</span><span class="romanization">Hello world</span></span>
            <span class="verse"><span>Como vai você?</span><span class="romanization">How are you?</span></span></div>
        "#;
        let trans = parse_translation_lyrics(html);
        assert!(trans.is_some());
        let lines = trans.unwrap();
        assert!(lines.contains("Olá mundo"));
        assert!(lines.contains("Como vai você?"));
    }

    #[test]
    fn translation_never_uses_original_verses_or_footer_paragraphs() {
        let html = r#"<div class='lyric-original'><span class='verse'><span>Original text in English</span></span></div>
            <div class='other lyric-translation'><span class='verse'><span>Primeiro verso traduzido</span></span><span class='verse'><span>Segundo verso traduzido</span></span></div><footer><p>Footer paragraph</p></footer>"#;
        assert_eq!(
            parse_translation_lyrics(html).as_deref(),
            Some("Primeiro verso traduzido\nSegundo verso traduzido")
        );
        assert!(parse_translation_lyrics("<div class='lyric-original'><span class='verse'><span>Only original text in English</span></span></div>").is_none());
        assert!(!parse_original_lyrics(html).unwrap().1.contains("Footer"));
    }

    #[test]
    fn test_clean_song_metadata() {
        let (title, artist) =
            clean_song_metadata("Tame Impala - The Less I Know The Better", "tameimpalaVEVO");
        assert_eq!(artist, "Tame Impala");
        assert_eq!(title, "The Less I Know The Better");

        let (title, artist) = clean_song_metadata("Mr. Fear (Official Video)", "SIAMÉS - Topic");
        assert_eq!(artist, "SIAMÉS");
        assert_eq!(title, "Mr. Fear");

        let (title, artist) = clean_song_metadata(
            "Queen - Bohemian Rhapsody (Official Music Video)",
            "YouTube",
        );
        assert_eq!(artist, "Queen");
        assert_eq!(title, "Bohemian Rhapsody");

        let (title, artist) = clean_song_metadata(
            "Wye Oak - Civilian (Official Video)",
            "Merge Records on YouTube",
        );
        assert_eq!(artist, "Wye Oak");
        assert_eq!(title, "Civilian");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection (legendado/tradução)",
            "Riverside Traduções",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber Of Reflection (legendado/tradução)",
            "Riverside Traduções",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber Of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection (LEGENDADO/TRADUÇÃO)",
            "Riverside Traduções",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection (legendado / tradução)",
            "Riverside Traduções",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection (Legendado PT-BR)",
            "Riverside Traduções",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection [Tradução]",
            "Riverside Traduções",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection | Legendado",
            "Canal Qualquer",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) = clean_song_metadata(
            "Mac DeMarco - Chamber of Reflection // Tradução",
            "Canal de Música",
        );
        assert_eq!(artist, "Mac DeMarco");
        assert_eq!(title, "Chamber of Reflection");

        let (title, artist) =
            clean_song_metadata("Civilian (Official Video)", "Wye Oak on YouTube");
        assert_eq!(artist, "Wye Oak");
        assert_eq!(title, "Civilian");
    }

    #[test]
    fn search_metadata_removes_video_and_lyrics_markers() {
        for marker in [
            "[Official Animated Music Video]",
            "(Official Video)",
            "(Lyrics)",
            "[OFFICIAL ANIMATED MUSIC VIDEO] (lyrics) (Lyrics)",
        ] {
            let raw = format!("Mr. FEAR {marker}");
            let (title, artist) = clean_song_metadata(&raw, "SIAMÉS");
            assert_eq!(title, "Mr. FEAR");
            assert_eq!(artist, "SIAMÉS");
        }
        assert_eq!(clean_song_metadata("İ Song (Lyrics)", "Artist").0, "İ Song");
        assert_eq!(
            clean_song_metadata("Song (Live) (Lyrics)", "Artist").0,
            "Song (Live)"
        );
    }

    #[test]
    fn custom_title_terms_are_literal_case_insensitive_and_unicode_safe() {
        let terms = normalize_ignored_terms([
            " (Live In Tampa 2022)\r\n",
            "(Letra)",
            "(LETRA)",
            "",
            "ÁUDIO",
            "İ",
        ]);
        assert_eq!(terms.len(), 4);
        assert_eq!(
            clean_metadata_with_terms(
                "Ghost - Mary On A Cross (LIVE IN TAMPA 2022) (Letra) (letra)",
                "GhostBCVEVO",
                true,
                &terms,
            ),
            ("Mary On A Cross".into(), "Ghost".into())
        );
        assert_eq!(strip_ignored_title_terms("İ Song áUdIo", &terms), "Song");
        assert_eq!(
            strip_ignored_title_terms("Song (Live) [Remix]", &terms),
            "Song (Live) [Remix]"
        );
        assert_eq!(
            strip_ignored_title_terms("Song (Live In Tampa 2022)", &[]),
            "Song (Live In Tampa 2022)"
        );
    }

    #[test]
    fn test_parse_ignores_subscription_banner() {
        let html = r#"
            <h1>The Less I Know The Better</h1>
            <div class="cnt-letra">
                <p>Someone said they left together<br>I ran out the door to get her</p>
                <p>She was with another man</p>
            </div>
            <div class="contextualPlanModal">
                <p>Pague uma vez e use por um ano inteiro</p>
            </div>
        "#;
        let res = parse_original_lyrics(html);
        assert!(res.is_some());
        let (title, lyrics) = res.unwrap();
        assert_eq!(title, "The Less I Know The Better");
        assert!(lyrics.contains("Someone said they left together"));
        assert!(!lyrics.contains("Pague uma vez"));
    }

    #[test]
    fn test_find_song_url_from_search() {
        let html = r#"
            <a href="https://www.letras.mus.br/top/semana/">Top</a>
            <a href="https://www.letras.mus.br/siames/mr-fear/">Mr Fear</a>
        "#;
        let url = find_song_url_from_search(html);
        assert_eq!(
            url.as_deref(),
            Some("https://www.letras.mus.br/siames/mr-fear/")
        );
    }

    #[test]
    fn test_find_matching_song_url_rejects_unrelated_song_by_same_artist() {
        let html = r#"
            <a href="https://www.letras.mus.br/top/semana/">Top</a>
            <a href="https://www.letras.mus.br/friend-of-a-friend/oasis/">Oasis</a>
        "#;
        let url = find_matching_song_url_from_search(html, "Always On Time", "Friend of a Friend");
        assert!(
            url.is_none(),
            "Unrelated song 'oasis' must not match 'Always On Time'"
        );
    }

    #[test]
    fn test_find_matching_song_url_selects_correct_song_among_multiple() {
        let html = r#"
            <a href="/siames/the-wolf/">The Wolf</a>
            <a href="/siames/mr-fear/">Mr Fear</a>
            <a href="/siames/summer-nights/">Summer Nights</a>
        "#;
        let url = find_matching_song_url_from_search(html, "Mr. Fear", "SIAMÉS");
        assert_eq!(
            url.as_deref(),
            Some("https://www.letras.mus.br/siames/mr-fear/")
        );
    }

    #[test]
    fn test_is_matching_song() {
        assert!(!is_matching_song(
            "Always On Time",
            "Friend of a Friend",
            "Oasis",
            "oasis"
        ));
        assert!(is_matching_song(
            "Always On Time",
            "Friend of a Friend",
            "Always On Time",
            "always-on-time"
        ));
        assert!(is_matching_song(
            "Mr. FEAR (feat. Someone)",
            "SIAMÉS",
            "Mr. Fear",
            "mr-fear"
        ));
        assert!(is_matching_song("Song (Live)", "Artist", "Song", "song"));
    }
}

#[cfg(test)]
mod guest_credit_regressions {
    use super::*;
    #[test]
    fn youtube_truncated_guest_title_is_searchable_without_losing_live_version() {
        let raw = "Spitting Off the Edge of the World ft. Perfume Genius (Offici...";
        assert_eq!(
            clean_song_metadata(raw, "Yeah Yeah Yeahs").0,
            "Spitting Off the Edge of the World"
        );
        let credited = clean_metadata(raw, "Yeah Yeah Yeahs", false)
            .0
            .replace(" ft. ", " feat ");
        assert_eq!(
            slugify(&credited),
            "spitting-off-the-edge-of-the-world-feat-perfume-genius"
        );
        assert_eq!(
            clean_song_metadata("Song (feat. Guest) (Live)", "Artist").0,
            "Song (Live)"
        );
        assert_eq!(
            clean_song_metadata("Song ft. Guest (Remix)", "Artist").0,
            "Song (Remix)"
        );
    }
}
