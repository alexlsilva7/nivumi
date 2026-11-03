//! Shared lyrics content and preferences for the notch and detached reader.
use crate::lyrics::LyricLine;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LyricsPreferences {
    /// Per-song compensation shared by the notch, reader and caption overlay.
    pub song_offsets_ms: std::collections::BTreeMap<String, i64>,
    pub focus_animation: usize,
    pub translation_all_verses: bool,
    pub offline_translation: bool,
    pub ignored_title_terms: Vec<String>,
    pub notch_width: f32,
    pub notch_height: f32,
    pub notch_font: f32,
    pub window_font: f32,
    pub window_soft_background: bool,
    pub window_bounds: Option<[i32; 4]>,
    pub window_topmost: bool,
    pub window_mode: usize,
}
impl Default for LyricsPreferences {
    fn default() -> Self {
        Self {
            song_offsets_ms: Default::default(),
            focus_animation: 1,
            translation_all_verses: true,
            offline_translation: false,
            ignored_title_terms: vec!["(Live In Tampa 2022)".into()],
            notch_width: 680.,
            notch_height: 240.,
            notch_font: 15.,
            window_font: 20.,
            window_soft_background: false,
            window_bounds: None,
            window_topmost: false,
            window_mode: 1,
        }
    }
}
/// Identity deliberately excludes duration, player and search-filter settings.
pub fn song_sync_key(title: &str, artist: &str) -> String {
    let title = title.trim().to_lowercase();
    let artist = artist.trim().to_lowercase();
    if title.is_empty() {
        return String::new();
    }
    format!("{}:{title}{}:{artist}", title.len(), artist.len())
}
impl LyricsPreferences {
    pub fn song_offset(&self, key: &str) -> i64 {
        self.song_offsets_ms
            .get(key)
            .copied()
            .unwrap_or(0)
            .clamp(-10000, 10000)
    }
    pub fn set_song_offset(&mut self, key: &str, offset: i64) {
        if key.is_empty() {
            return;
        }
        let offset = offset.clamp(-10000, 10000);
        if offset == 0 {
            self.song_offsets_ms.remove(key);
        } else {
            self.song_offsets_ms.insert(key.into(), offset);
        }
    }
    pub fn adjust_song_offset(&mut self, key: &str, delta: Option<i64>) -> i64 {
        if key.is_empty() {
            return 0;
        }
        let offset = delta
            .map(|d| self.song_offset(key).saturating_add(d))
            .unwrap_or(0)
            .clamp(-10000, 10000);
        self.set_song_offset(key, offset);
        offset
    }

    pub fn normalize(&mut self) {
        self.song_offsets_ms.retain(|key, offset| {
            *offset = (*offset).clamp(-10000, 10000);
            !key.is_empty() && *offset != 0
        });
        self.ignored_title_terms = crate::letras::normalize_ignored_terms(
            self.ignored_title_terms.iter().map(String::as_str),
        );
        fn bounded(value: f32, default: f32, min: f32, max: f32) -> f32 {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            }
        }
        self.notch_width = bounded(self.notch_width, 680., 520., 1800.);
        self.notch_height = bounded(self.notch_height, 240., 180., 1000.);
        self.notch_font = bounded(self.notch_font, 15., 12., 24.);
        self.window_font = bounded(self.window_font, 20., 12., 36.);
        self.window_mode = self.window_mode.min(3);
        self.focus_animation = self.focus_animation.min(3);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricsContent {
    pub translation_origin: crate::offline_translation::TranslationOrigin,
    pub key: String,
    pub title: String,
    pub artist: String,
    pub lines: Vec<LyricLine>,
    pub original: String,
    pub translation: Option<String>,
    pub status: String,
    pub active: Option<usize>,
    pub is_portuguese: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verse {
    pub text: String,
    pub translation: Option<String>,
    pub active: bool,
    pub line_index: Option<usize>,
}
impl Verse {
    pub fn visible_translation(&self, all: bool) -> Option<&str> {
        self.translation
            .as_deref()
            .filter(|text| (self.active || all) && !text.trim().is_empty())
    }
    /// Measure exactly what is displayed; hidden translations reserve no height.
    pub fn display_height(&self, all: bool, mut measure: impl FnMut(&str, bool) -> f32) -> f32 {
        measure(&self.text, false)
            + 14.
            + self
                .visible_translation(all)
                .map(|text| measure(text, true) + 4.)
                .unwrap_or(0.)
    }
}
pub fn is_lyrics_in_portuguese(text: &str) -> bool {
    let parsed = crate::lyrics::parse_lrc(text);
    let plain = if parsed.is_empty() {
        text.to_owned()
    } else {
        parsed
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let sample = plain
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .take(30)
        .collect::<Vec<_>>()
        .join(" ");
    sample.chars().count() >= 20
        && whatlang::detect(&sample)
            .is_some_and(|info| info.lang() == whatlang::Lang::Por && info.confidence() >= 0.7)
}

impl LyricsContent {
    pub fn available_mode(&self, mode: usize) -> usize {
        let has_translation = self
            .translation
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || self.lines.iter().any(|line| {
                line.translation
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty())
            });
        if self.is_portuguese && !has_translation && (mode == 1 || mode == 3) {
            if self.lines.is_empty() {
                2
            } else {
                0
            }
        } else {
            mode
        }
    }
    pub fn static_status(english: bool) -> &'static str {
        if english {
            "Full lyrics available (unsynced)"
        } else {
            "Letra completa disponível (sem sincronia)"
        }
    }
    pub fn compact_text(&self, english: bool) -> &str {
        if let Some(line) = self
            .active
            .and_then(|i| self.lines.get(i))
            .filter(|l| !l.text.is_empty())
        {
            &line.text
        } else if !self.lines.is_empty() {
            "♪"
        } else if !self.original.trim().is_empty() {
            if english {
                "Lyrics available · Click to open"
            } else {
                "Letra disponível · Clique para abrir"
            }
        } else if self.status.is_empty() {
            &self.title
        } else {
            &self.status
        }
    }

    pub fn offline_label(&self, mode: usize, en: bool) -> &str {
        if (mode == 1 || mode == 3)
            && self.translation_origin == crate::offline_translation::TranslationOrigin::Offline
        {
            if en {
                "Offline translation"
            } else {
                "Tradução offline"
            }
        } else {
            ""
        }
    }
    pub fn verses(&self, mode: usize) -> Vec<Verse> {
        if mode <= 1 && !self.lines.is_empty() {
            return self
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| !line.text.trim().is_empty())
                .map(|(index, line)| Verse {
                    text: line.text.clone(),
                    translation: if mode == 1 {
                        line.translation.clone()
                    } else {
                        None
                    },
                    active: self.active == Some(index),
                    line_index: Some(index),
                })
                .collect();
        }
        let text = if mode == 3 {
            self.translation.as_deref().unwrap_or("")
        } else if mode == 2 {
            &self.original
        } else {
            ""
        };
        let text = if mode == 2 && !crate::lyrics::parse_lrc(text).is_empty() {
            crate::lyrics::parse_lrc(text)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text.into()
        };
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| Verse {
                text: line.into(),
                translation: None,
                active: false,
                line_index: None,
            })
            .collect()
    }
    pub fn empty_message(&self, mode: usize, english: bool) -> &str {
        if mode == 3
            && self
                .translation
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
        {
            if english {
                "No translation available"
            } else {
                "Sem tradução disponível"
            }
        } else if self.status.is_empty() {
            if english {
                "Waiting for lyrics"
            } else {
                "Aguardando letras"
            }
        } else {
            &self.status
        }
    }

    /// Formats the synchronized lyrics and provider's full original/translation
    /// into a structured markdown report for debugging and analysis with AI.
    pub fn format_for_ai(&self) -> String {
        let mut out = String::new();
        out.push_str("# Diagnóstico de Letra e Tradução (Nivumi)\n\n");
        out.push_str("## Metadados\n");
        if self.translation_origin == crate::offline_translation::TranslationOrigin::Offline {
            out.push_str("- **Tradução:** offline (automática)\n");
        }
        out.push_str(&format!(
            "- **Título:** {}\n",
            if self.title.is_empty() {
                "(Desconhecido)"
            } else {
                &self.title
            }
        ));
        out.push_str(&format!(
            "- **Artista:** {}\n",
            if self.artist.is_empty() {
                "(Desconhecido)"
            } else {
                &self.artist
            }
        ));
        if !self.status.is_empty() {
            out.push_str(&format!("- **Status:** {}\n", self.status));
        }
        out.push('\n');

        if !self.lines.is_empty() {
            out.push_str("## Letra Sincronizada (LRC) & Pareamento Atual\n");
            for line in &self.lines {
                let total_secs = line.timestamp_ms / 1000;
                let ms = (line.timestamp_ms % 1000) / 10;
                let mins = total_secs / 60;
                let secs = total_secs % 60;
                let timestamp = format!("[{:02}:{:02}.{:02}]", mins, secs, ms);
                out.push_str(&format!("{} {}\n", timestamp, line.text));
                if let Some(trans) = &line.translation {
                    out.push_str(&format!("  -> Tradução: {}\n", trans.replace('\n', " / ")));
                } else {
                    out.push_str("  -> Tradução: (sem correspondência)\n");
                }
            }
            out.push('\n');
        } else {
            out.push_str(
                "## Letra Sincronizada (LRC)\n*(Nenhuma letra sincronizada disponível)*\n\n",
            );
        }

        if !self.original.trim().is_empty() {
            out.push_str("## Letra Original Completa (Provedor)\n```text\n");
            out.push_str(self.original.trim());
            out.push_str("\n```\n\n");
        }

        if let Some(trans) = &self.translation {
            if !trans.trim().is_empty() {
                out.push_str("## Tradução Completa (Provedor)\n```text\n");
                out.push_str(trans.trim());
                out.push_str("\n```\n");
            }
        } else {
            out.push_str(
                "## Tradução Completa (Provedor)\n*(Nenhuma tradução disponível no provedor)*\n",
            );
        }

        out
    }
}

/// Keep a remembered outer window rect reachable on its current/nearest monitor.
pub fn fit_bounds(rect: [i32; 4], work: [i32; 4], min_size: [i32; 2]) -> [i32; 4] {
    let available_w = (work[2] - work[0]).max(1);
    let available_h = (work[3] - work[1]).max(1);
    let width = rect[2].clamp(min_size[0].min(available_w).max(1), available_w);
    let height = rect[3].clamp(min_size[1].min(available_h).max(1), available_h);
    [
        rect[0].clamp(work[0], work[2] - width),
        rect[1].clamp(work[1], work[3] - height),
        width,
        height,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_settings_load_and_reader_preferences_round_trip() {
        let old: crate::model::Settings =
            serde_json::from_str(r#"{"language":"en","scale":1.25}"#).unwrap();
        assert_eq!(old.lyrics, LyricsPreferences::default());
        let prefs = LyricsPreferences {
            notch_width: 840.,
            notch_height: 360.,
            window_font: 26.,
            window_bounds: Some([-1400, 120, 640, 720]),
            window_topmost: true,
            window_mode: 3,
            ..Default::default()
        };
        let loaded: LyricsPreferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(loaded, prefs);
    }
    #[test]
    fn title_terms_load_defaults_and_preserve_an_explicitly_empty_list() {
        let mut prefs: LyricsPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.ignored_title_terms, vec!["(Live In Tampa 2022)"]);
        prefs.ignored_title_terms = vec!["  (Custom)  ".into(), "(CUSTOM)".into(), " ".into()];
        prefs.normalize();
        assert_eq!(prefs.ignored_title_terms, vec!["(Custom)"]);
        assert_eq!(
            serde_json::from_str::<LyricsPreferences>(&serde_json::to_string(&prefs).unwrap())
                .unwrap(),
            prefs
        );
        let empty: LyricsPreferences =
            serde_json::from_str(r#"{"ignored_title_terms":[]}"#).unwrap();
        assert!(empty.ignored_title_terms.is_empty());
    }
    #[test]
    fn disconnected_and_negative_coordinate_monitors_remain_reachable() {
        assert_eq!(
            fit_bounds([3000, 2000, 900, 700], [0, 0, 1280, 720], [400, 260]),
            [380, 20, 900, 700]
        );
        assert_eq!(
            fit_bounds([-1500, -20, 9000, -10], [-1920, 0, 0, 1080], [400, 260]),
            [-1920, 0, 1920, 260]
        );
    }
    #[test]
    fn timed_and_full_text_views_share_content_without_showing_lrc_tags() {
        let content = LyricsContent {
            lines: crate::lyrics::parse_lrc("[00:01]First\n[00:02]\n[00:03]Next"),
            active: Some(2),
            original: "[00:01]First\n[00:03]Next".into(),
            translation: Some("Primeiro\nPróximo".into()),
            ..Default::default()
        };
        let verses = content.verses(0);
        assert_eq!(verses.len(), 2);
        assert_eq!(verses[1].line_index, Some(2));
        assert!(verses[1].active);
        assert_eq!(content.verses(2)[0].text, "First");
        assert_eq!(content.verses(3)[1].text, "Próximo");
    }
    #[test]
    fn bilingual_view_keeps_original_verses_without_a_translation_match() {
        let mut lines = crate::lyrics::parse_lrc("[00:01]Opening\n[00:02]Unmatched\n[00:03]Ending");
        crate::lyrics::align_external_translation(
            &mut lines,
            &crate::services::ExternalLyrics {
                original: "Opening\nEnding".into(),
                translation: Some("Abertura\nFim".into()),
                ..Default::default()
            },
        );
        let content = LyricsContent {
            lines,
            ..Default::default()
        };
        let verses = content.verses(1);
        assert_eq!(verses.len(), 3);
        assert_eq!(verses[0].translation.as_deref(), Some("Abertura"));
        assert_eq!(verses[1].text, "Unmatched");
        assert_eq!(verses[1].line_index, Some(1));
        assert_eq!(verses[1].translation, None);
        assert_eq!(verses[2].translation.as_deref(), Some("Fim"));
    }

    #[test]
    fn malformed_preferences_do_not_create_invalid_geometry_or_fonts() {
        let mut prefs = LyricsPreferences {
            notch_width: f32::NAN,
            notch_height: -1.,
            window_font: f32::INFINITY,
            window_mode: 99,
            ..Default::default()
        };
        prefs.normalize();
        assert_eq!(prefs.notch_width, 680.);
        assert_eq!(prefs.notch_height, 180.);
        assert_eq!(prefs.window_font, 20.);
        assert_eq!(prefs.window_mode, 3);
    }

    #[test]
    fn format_for_ai_includes_all_sections_and_translations() {
        let content = LyricsContent {
            title: "Song Title".into(),
            artist: "Artist Name".into(),
            status: "● lyricsify.com".into(),
            lines: vec![
                LyricLine {
                    timestamp_ms: 65430,
                    text: "Original line 1".into(),
                    translation: Some("Linha traduzida 1".into()),
                },
                LyricLine {
                    timestamp_ms: 70000,
                    text: "Original line 2".into(),
                    translation: None,
                },
            ],
            original: "Original line 1\nOriginal line 2".into(),
            translation: Some("Linha traduzida 1\nLinha traduzida 2".into()),
            ..Default::default()
        };
        let formatted = content.format_for_ai();
        assert!(formatted.contains("**Título:** Song Title"));
        assert!(formatted.contains("**Artista:** Artist Name"));
        assert!(formatted.contains("[01:05.43] Original line 1"));
        assert!(formatted.contains("-> Tradução: Linha traduzida 1"));
        assert!(formatted.contains("-> Tradução: (sem correspondência)"));
        assert!(formatted.contains("## Letra Original Completa (Provedor)"));
        assert!(formatted.contains("## Tradução Completa (Provedor)"));
    }
}

/// Opacity, vertical offset, and highlight progress during a bounded focus transition.
pub fn focus_effect(style: usize, elapsed: f32, reduced: bool) -> (f32, f32, f32) {
    if reduced || style == 0 || !elapsed.is_finite() {
        return (1., 0., 1.);
    }
    let t = (elapsed / 0.28).clamp(0., 1.);
    let eased = 1. - (1. - t).powi(3);
    match style {
        1 => (0.45 + 0.55 * eased, 0., 1.),
        2 => (1., 0., eased),
        3 => (1., 0., eased),
        _ => (1., 0., 1.),
    }
}

#[cfg(test)]
mod focus_effect_tests {
    use super::*;
    #[test]
    fn reduced_motion_and_disabled_styles_are_static() {
        for style in 0..4 {
            assert_eq!(focus_effect(style, 0., true), (1., 0., 1.));
        }
        assert_eq!(focus_effect(0, 0., false), (1., 0., 1.));
    }
    #[test]
    fn transitions_finish_and_never_overshoot() {
        for style in 1..4 {
            assert_eq!(focus_effect(style, 1., false), (1., 0., 1.));
            for step in 0..30 {
                let (alpha, y, progress) = focus_effect(style, step as f32 / 100., false);
                assert!((0.0..=1.0).contains(&alpha));
                assert!((0.0..=8.0).contains(&y));
                assert!((0.0..=1.0).contains(&progress));
            }
        }
    }
    #[test]
    fn old_preferences_get_defaults_and_new_preferences_round_trip() {
        let mut p: LyricsPreferences = serde_json::from_str("{}").unwrap();
        assert!(p.translation_all_verses);
        assert_eq!(p.focus_animation, 1);
        p.translation_all_verses = false;
        p.focus_animation = 2;
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(p, serde_json::from_str(&text).unwrap());
    }
}

/// Interruptible viewport scrolling. Retargeting starts at the currently displayed offset.
#[derive(Default)]
pub struct FocusScroll {
    from: f32,
    target: f32,
    started: Option<std::time::Instant>,
    initialized: bool,
}
impl FocusScroll {
    pub fn position(&self, now: std::time::Instant) -> f32 {
        let t = self
            .started
            .map(|start| now.saturating_duration_since(start).as_secs_f32() / 0.28)
            .unwrap_or(1.)
            .clamp(0., 1.);
        self.from + (self.target - self.from) * (1. - (1. - t).powi(3))
    }
    pub fn retarget(&mut self, target: f32, now: std::time::Instant, animate: bool) -> bool {
        let target = if target.is_finite() {
            target.max(0.)
        } else {
            0.
        };
        if !self.initialized || !animate {
            self.from = target;
            self.target = target;
            self.started = None;
            self.initialized = true;
            return false;
        }
        if (target - self.target).abs() < 0.01 {
            return false;
        }
        self.from = self.position(now);
        self.target = target;
        self.started = Some(now);
        true
    }
    pub fn is_animating(&self, now: std::time::Instant) -> bool {
        self.started
            .is_some_and(|t| now.saturating_duration_since(t).as_secs_f32() < 0.28)
            && (self.from - self.target).abs() > 0.01
    }
}
#[cfg(test)]
mod focus_scroll_tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn scrolls_the_viewport_and_finishes_at_the_target() {
        let now = Instant::now();
        let mut s = FocusScroll::default();
        assert!(!s.retarget(0., now, true));
        assert!(s.retarget(100., now, true));
        assert_eq!(s.position(now), 0.);
        let halfway = s.position(now + Duration::from_millis(140));
        assert!(halfway > 0. && halfway < 100.);
        assert_eq!(s.position(now + Duration::from_millis(280)), 100.);
        assert!(!s.is_animating(now + Duration::from_millis(280)));
    }
    #[test]
    fn rapid_focus_changes_continue_from_the_visible_position() {
        let now = Instant::now();
        let mut s = FocusScroll::default();
        s.retarget(0., now, true);
        s.retarget(100., now, true);
        let mid = now + Duration::from_millis(100);
        let visible = s.position(mid);
        s.retarget(250., mid, true);
        assert_eq!(s.position(mid), visible);
        assert_eq!(s.position(mid + Duration::from_millis(280)), 250.);
    }
    #[test]
    fn backwards_seek_and_reduced_motion_are_bounded() {
        let now = Instant::now();
        let mut s = FocusScroll::default();
        s.retarget(400., now, true);
        s.retarget(20., now, true);
        let x = s.position(now + Duration::from_millis(100));
        assert!((20.0..=400.0).contains(&x));
        s.retarget(150., now, false);
        assert_eq!(s.position(now), 150.);
        assert!(!s.is_animating(now));
    }
}

#[cfg(test)]
mod verse_spacing_tests {
    use super::*;
    #[test]
    fn hidden_translation_reserves_no_space_even_if_it_wraps() {
        let verse = Verse {
            text: "Original".into(),
            translation: Some("A long translation".into()),
            active: false,
            line_index: Some(0),
        };
        assert_eq!(
            verse.display_height(false, |_, translated| if translated { 100. } else { 20. }),
            34.
        );
        assert_eq!(
            verse.display_height(true, |_, translated| if translated { 100. } else { 20. }),
            138.
        );
        let active = Verse {
            active: true,
            ..verse
        };
        assert_eq!(
            active.display_height(false, |_, translated| if translated { 100. } else { 22. }),
            140.
        );
    }
    #[test]
    fn missing_and_empty_translations_do_not_add_a_blank_line() {
        for translation in [None, Some(String::new()), Some("  ".into())] {
            let verse = Verse {
                text: "Original".into(),
                translation,
                active: true,
                line_index: Some(0),
            };
            assert_eq!(verse.display_height(true, |_, _| 20.), 34.);
            assert!(verse.visible_translation(true).is_none());
        }
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    #[test]
    fn static_lyrics_override_compact_error_and_portuguese_modes() {
        let content = LyricsContent {
            original: "Uma letra completa".into(),
            status: "Não foi possível buscar letras".into(),
            is_portuguese: true,
            ..Default::default()
        };
        assert_eq!(
            content.compact_text(false),
            "Letra disponível · Clique para abrir"
        );
        assert_eq!(content.available_mode(1), 2);
        assert_eq!(content.available_mode(3), 2);
        let translated = LyricsContent {
            translation: Some("Translation".into()),
            ..content
        };
        assert_eq!(translated.available_mode(3), 3);
    }
    #[test]
    fn detects_portuguese_in_plain_and_timestamped_lyrics() {
        let text = "Eu quero encontrar você e dizer que o meu coração está cheio de saudade\nA nossa vida tem amor e esperança, eu nunca vou esquecer você";
        assert!(is_lyrics_in_portuguese(text));
        assert!(is_lyrics_in_portuguese(&format!(
            "[00:01.00]{}",
            text.replace('\n', "\n[00:05.00]")
        )));
        assert!(!is_lyrics_in_portuguese("amor"));
        assert!(!is_lyrics_in_portuguese(
            "I remember all the days we spent together and I will always love you"
        ));
    }
    #[test]
    fn compact_keeps_real_errors_when_no_lyrics_exist() {
        let content = LyricsContent {
            status: "Could not load lyrics".into(),
            ..Default::default()
        };
        assert_eq!(content.compact_text(true), content.status);
    }
}

#[cfg(test)]
mod song_sync_tests {
    use super::*;
    #[test]
    fn offsets_are_per_song_persisted_and_reset_independently() {
        let a = song_sync_key(" This Is Gospel ", "Panic! At The Disco");
        assert_eq!(a, song_sync_key("this is gospel", "panic! at the disco"));
        let b = song_sync_key("Other song", "Panic! At The Disco");
        let mut prefs = LyricsPreferences::default();
        assert_eq!(prefs.adjust_song_offset(&a, Some(100)), 100);
        assert_eq!(prefs.adjust_song_offset(&a, Some(100)), 200);
        assert_eq!(prefs.song_offset(&b), 0);
        prefs.adjust_song_offset(&b, Some(-100));
        let mut restored: LyricsPreferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored.song_offset(&a), 200);
        assert_eq!(restored.song_offset(&b), -100);
        assert_eq!(restored.adjust_song_offset(&a, None), 0);
        assert_eq!(restored.song_offset(&b), -100);
        assert!(!restored.song_offsets_ms.contains_key(&a));
    }
    #[test]
    fn empty_tracks_and_overflow_never_create_invalid_compensation() {
        let mut prefs: LyricsPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(
            prefs.adjust_song_offset(&song_sync_key(" ", "Artist"), Some(100)),
            0
        );
        assert!(prefs.song_offsets_ms.is_empty());
        let key = song_sync_key("Song", "Artist");
        assert_eq!(prefs.adjust_song_offset(&key, Some(i64::MAX)), 10000);
        assert_eq!(prefs.adjust_song_offset(&key, Some(i64::MIN)), -10000);
        assert_ne!(song_sync_key("a:b", "c"), song_sync_key("a", "b:c"));
        assert_ne!(
            song_sync_key("Song", "Artist"),
            song_sync_key("Song", "Other artist")
        );
    }
}
