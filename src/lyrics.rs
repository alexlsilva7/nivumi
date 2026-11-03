//! LRC text parsing without network or UI dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub timestamp_ms: i64,
    pub text: String,
    pub translation: Option<String>,
}

pub fn parse_lrc(input: &str) -> Vec<LyricLine> {
    let offset = input
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("[offset:")
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.trim().parse::<i64>().ok())
        })
        .next_back()
        .unwrap_or(0);
    let mut lines = vec![];
    for line in input.lines() {
        let mut rest = line.trim();
        let mut timestamps = vec![];
        while let Some(tail) = rest.strip_prefix('[') {
            let Some((tag, following)) = tail.split_once(']') else {
                break;
            };
            let Some(timestamp) = timestamp(tag) else {
                break;
            };
            timestamps.push(timestamp);
            rest = following;
        }
        for timestamp in timestamps {
            lines.push(LyricLine {
                timestamp_ms: timestamp.saturating_add(offset).max(0),
                text: rest.trim().to_string(),
                translation: None,
            });
        }
    }
    lines.sort_by_key(|line| line.timestamp_ms);
    lines
}

fn is_verse(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && !text.starts_with('[') && text.chars().any(char::is_alphanumeric)
}

/// Disregard multiplier suffixes such as (2x), (x2), (bis), [2x], etc. when comparing verses.
fn strip_multiplier_suffix(text: &str) -> &str {
    let mut s = text.trim();
    loop {
        let trimmed = s
            .trim()
            .trim_end_matches(['!', '?', '.', ',', ';', ':', '-', '–', '—'])
            .trim();

        // 1. Check for bracketed or parenthesized suffixes: (2x), [2x], (x2), [x2], (bis), [bis], etc.
        if let Some(last_open) = trimmed.rfind(['(', '[']) {
            let closing = if trimmed.as_bytes()[last_open] == b'(' {
                ')'
            } else {
                ']'
            };
            if trimmed.ends_with(closing) {
                let inner = &trimmed[last_open + 1..trimmed.len() - 1];
                let lower = inner.trim().to_lowercase();
                let is_multiplier = lower == "bis"
                    || lower.starts_with("refrão")
                    || lower.starts_with("refrao")
                    || lower.starts_with("chorus")
                    || (lower.ends_with('x')
                        && !lower.is_empty()
                        && lower[..lower.len() - 1]
                            .trim()
                            .chars()
                            .all(|c| c.is_ascii_digit()))
                    || (lower.starts_with('x')
                        && lower.len() > 1
                        && lower[1..].trim().chars().all(|c| c.is_ascii_digit()));
                if is_multiplier {
                    s = trimmed[..last_open].trim();
                    continue;
                }
            }
        }

        // 2. Check for bare suffix at the end, e.g. " 2x", " - 2x", " bis"
        let lower = trimmed.to_lowercase();
        if lower.ends_with("bis") {
            let before = &trimmed[..trimmed.len() - 3];
            if before.ends_with(|c: char| c.is_whitespace() || c == '-' || c == '–' || c == '—')
            {
                s = before.trim();
                continue;
            }
        } else if lower.ends_with('x') {
            let prefix = &lower[..lower.len() - 1];
            let num_digits = prefix
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .count();
            if num_digits > 0 && num_digits <= 2 && prefix.len() >= num_digits {
                let before = &trimmed[..trimmed.len() - 1 - num_digits];
                if before.ends_with(|c: char| c.is_whitespace() || c == '-' || c == '–' || c == '—')
                {
                    s = before.trim();
                    continue;
                }
            }
        } else if let Some(last_space) =
            trimmed.rfind(|c: char| c.is_whitespace() || c == '-' || c == '–' || c == '—')
        {
            let token = &lower[last_space + 1..];
            if token.starts_with('x')
                && token.len() > 1
                && token[1..].chars().all(|c| c.is_ascii_digit())
            {
                s = trimmed[..last_space].trim();
                continue;
            }
        }

        break;
    }
    s.trim()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut dp: Vec<usize> = (0..=b_bytes.len()).collect();
    for (i, &ca) in a_bytes.iter().enumerate() {
        let mut prev = dp[0];
        dp[0] = i + 1;
        for (j, &cb) in b_bytes.iter().enumerate() {
            let temp = dp[j + 1];
            dp[j + 1] = if ca == cb {
                prev
            } else {
                1 + prev.min(dp[j]).min(dp[j + 1])
            };
            prev = temp;
        }
    }
    dp[b_bytes.len()]
}

fn verses_match(s1: &str, s2: &str) -> Option<f32> {
    if s1 == s2 {
        return Some(0.0);
    }
    let max_len = s1.len().max(s2.len());
    if max_len < 10 {
        return None;
    }
    let (shorter, longer) = if s1.len() <= s2.len() {
        (s1, s2)
    } else {
        (s2, s1)
    };
    for filler in [
        "the", "a", "an", "and", "oh", "ah", "ooh", "yeah", "i", "we", "you", "so", "but",
    ] {
        if longer.strip_prefix(filler) == Some(shorter)
            || longer.strip_suffix(filler) == Some(shorter)
        {
            return Some(0.2);
        }
    }
    let len_diff = longer.len() - shorter.len();
    if len_diff > 4 {
        return None;
    }
    let dist = edit_distance(s1, s2);
    if (dist == 1 && max_len >= 10)
        || (dist <= 2 && max_len >= 16)
        || (dist <= 3 && max_len >= 20)
        || (dist <= 4 && max_len >= 28)
    {
        Some(dist as f32 * 0.15)
    } else {
        None
    }
}

fn normalized_verse(text: &str) -> String {
    let stripped = strip_multiplier_suffix(text);
    let mut result = String::new();
    for word in stripped.split_whitespace() {
        let clean: String = word
            .chars()
            .flat_map(char::to_lowercase)
            .filter(|ch| ch.is_alphanumeric())
            .collect();
        match clean.as_str() {
            "nothin" => result.push_str("nothing"),
            "somethin" => result.push_str("something"),
            "anythin" => result.push_str("anything"),
            "everythin" => result.push_str("everything"),
            _ => result.push_str(&clean),
        }
    }
    result
}

fn text_verses(text: &str) -> Vec<String> {
    let parsed = parse_lrc(text);
    if parsed.is_empty() {
        text.lines()
            .map(str::trim)
            .filter(|line| is_verse(line))
            .map(str::to_owned)
            .collect()
    } else {
        parsed
            .into_iter()
            .filter(|line| is_verse(&line.text))
            .map(|line| line.text)
            .collect()
    }
}

/// Align and combine synchronized lyric lines with a multi-line Portuguese translation
pub fn combine_with_translation(lines: &mut [LyricLine], translation: &str) {
    let external = crate::services::ExternalLyrics {
        original: lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        translation: Some(translation.to_string()),
        ..Default::default()
    };
    align_external_translation(lines, &external);
}

/// Use the translation provider's original as a bridge to the timestamped text.
/// Ambiguous bilingual line counts are deliberately left without timed translation.
pub fn align_external_translation(
    lines: &mut [LyricLine],
    external: &crate::services::ExternalLyrics,
) {
    for line in lines.iter_mut() {
        line.translation = None;
    }
    let Some(translation) = external.translation.as_deref() else {
        return;
    };
    let original = text_verses(&external.original);
    let translated = text_verses(translation);
    if original.is_empty() || original.len() != translated.len() {
        return;
    }
    let timed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_verse(&line.text))
        .map(|(index, _)| index)
        .collect();
    // Bound the quadratic alignment for malformed or unusually large imports.
    if timed.is_empty() || timed.len() > 600 || original.len() > 600 {
        return;
    }
    let synced: Vec<String> = timed
        .iter()
        .map(|&index| normalized_verse(&lines[index].text))
        .collect();
    let source: Vec<String> = original.iter().map(|line| normalized_verse(line)).collect();
    if synced.iter().chain(&source).any(|line| line.len() > 2048) {
        return;
    }
    let groups = |verses: &[String]| -> Vec<Vec<String>> {
        (0..verses.len())
            .map(|start| {
                (1..=3.min(verses.len() - start))
                    .map(|count| verses[start..start + count].concat())
                    .collect()
            })
            .collect()
    };
    let synced_groups = groups(&synced);
    let source_groups = groups(&source);
    let width = source.len() + 1;
    let mut costs = vec![f32::INFINITY; (synced.len() + 1) * width];
    let mut steps = vec![(0_usize, 0_usize); costs.len()];
    costs[0] = 0.;
    for i in 0..=synced.len() {
        for j in 0..=source.len() {
            let cost = costs[i * width + j];
            let mut offer = |a: usize, b: usize, extra: f32| {
                let next = (i + a) * width + j + b;
                if cost + extra < costs[next] {
                    costs[next] = cost + extra;
                    steps[next] = (a, b);
                }
            };
            if i < synced.len() {
                offer(1, 0, 1.);
            }
            if j < source.len() {
                offer(0, 1, 1.);
            }
            if i == synced.len() || j == source.len() {
                continue;
            }
            // A verse may be split into up to three lines by either provider.
            // Require identical normalized text for groups; never guess their boundary.
            for &(a, b) in &[
                (1, 1),
                (1, 2),
                (1, 3),
                (2, 1),
                (2, 2),
                (2, 3),
                (3, 1),
                (3, 2),
            ] {
                if i + a <= synced.len() && j + b <= source.len() {
                    if let Some(extra) =
                        verses_match(&synced_groups[i][a - 1], &source_groups[j][b - 1])
                    {
                        offer(a, b, (a + b - 2) as f32 * 0.1 + extra);
                    }
                }
            }
        }
    }
    let (mut i, mut j) = (synced.len(), source.len());
    while i > 0 || j > 0 {
        let (a, b) = steps[i * width + j];
        if a > 0 && b > 0 {
            let text = translated[j - b..j].join("\n");
            for &index in &timed[i - a..i] {
                lines[index].translation = Some(text.clone());
            }
        }
        i -= a;
        j -= b;
    }

    // Propagação inteligente de versos idênticos:
    // Qualquer linha da letra sincronizada que tenha ficado sem tradução (None),
    // mas cujo texto normalizado seja idêntico ao de outra linha da mesma música
    // que já tenha tradução confirmada (ex: repetição de refrão), herda essa tradução.
    // Não afeta versos com traduções diferentes já associadas pelo DP.
    let normalized_lines: Vec<String> = lines.iter().map(|l| normalized_verse(&l.text)).collect();
    for idx in 0..lines.len() {
        if lines[idx].translation.is_some() || normalized_lines[idx].is_empty() {
            continue;
        }
        let target_norm = &normalized_lines[idx];
        let candidate = lines
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                *i != idx && l.translation.is_some() && &normalized_lines[*i] == target_norm
            })
            .min_by_key(|(i, _)| (*i as isize - idx as isize).abs());

        if let Some((_, source_line)) = candidate {
            lines[idx].translation = source_line.translation.clone();
        } else if let Some(src_idx) = source
            .iter()
            .position(|s| verses_match(s, target_norm).is_some())
        {
            if let Some(trans) = translated.get(src_idx) {
                lines[idx].translation = Some(trans.clone());
            }
        }
    }
}
fn timestamp(tag: &str) -> Option<i64> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes = minutes.parse::<i64>().ok()?;
    if minutes < 0 {
        return None;
    }
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let seconds = seconds.parse::<i64>().ok()?;
    if !(0..60).contains(&seconds) || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let fraction = fraction.chars().take(3).collect::<String>();
    let ms = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()? * 10_i64.pow(3 - fraction.len() as u32)
    };
    minutes
        .checked_mul(60000)?
        .checked_add(seconds * 1000)?
        .checked_add(ms)
}
pub fn active_line(lines: &[LyricLine], position_ms: i64) -> Option<usize> {
    lines
        .partition_point(|line| line.timestamp_ms <= position_ms)
        .checked_sub(1)
}

/// A late provider miss must not erase synchronized lyrics already imported.
pub fn apply_synced(lines: &mut Vec<LyricLine>, text: Option<&str>) -> bool {
    let parsed = parse_lrc(text.unwrap_or_default());
    if !parsed.iter().any(|line| !line.text.is_empty()) {
        return false;
    }
    *lines = parsed;
    true
}

/// Merge independent providers without losing the Letras original or translation.
pub fn merge_external(
    current: &mut Option<crate::services::ExternalLyrics>,
    lines: &mut Vec<LyricLine>,
    mut incoming: crate::services::ExternalLyrics,
) -> bool {
    let synchronized = apply_synced(lines, Some(&incoming.original));
    if let Some(previous) = current.as_mut() {
        if incoming
            .translation
            .as_ref()
            .is_none_or(|text| text.trim().is_empty())
        {
            if previous.translation_origin == crate::offline_translation::TranslationOrigin::Offline
            {
                incoming.original = previous.original.clone();
            }
            incoming.translation = previous.translation.clone();
            incoming.translation_origin = previous.translation_origin.clone();
        }
        if synchronized && previous.source == "letras.mus.br" {
            previous.translation = incoming.translation;
            previous.translation_origin = incoming.translation_origin;
        } else {
            *previous = incoming;
        }
    } else {
        *current = Some(incoming);
    }
    if let Some(external) = current.as_ref() {
        align_external_translation(lines, external);
    }
    synchronized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_offline_provenance_until_human_translation_arrives() {
        use crate::{offline_translation::TranslationOrigin, services::ExternalLyrics};
        let mut current = Some(ExternalLyrics {
            original: "Hello".into(),
            translation: Some("Olá".into()),
            translation_origin: TranslationOrigin::Offline,
            ..Default::default()
        });
        let mut lines = Vec::new();
        merge_external(
            &mut current,
            &mut lines,
            ExternalLyrics {
                original: "Hello".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            current.as_ref().unwrap().translation_origin,
            TranslationOrigin::Offline
        );
        merge_external(
            &mut current,
            &mut lines,
            ExternalLyrics {
                original: "Hello".into(),
                translation: Some("Oi".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            current.as_ref().unwrap().translation_origin,
            TranslationOrigin::Human
        );
        assert_eq!(current.unwrap().translation.as_deref(), Some("Oi"));
    }
    fn aligned(lrc: &str, original: &str, translation: &str) -> Vec<Option<String>> {
        let mut lines = parse_lrc(lrc);
        align_external_translation(
            &mut lines,
            &crate::services::ExternalLyrics {
                original: original.into(),
                translation: Some(translation.into()),
                ..Default::default()
            },
        );
        lines.into_iter().map(|line| line.translation).collect()
    }

    #[test]
    fn extra_timed_verse_does_not_shift_following_translations() {
        assert_eq!(
            aligned(
                "[00:01]Opening\n[00:02]Extra ad-lib\n[00:03]Ending",
                "Opening\nEnding",
                "Abertura\nFim",
            ),
            [Some("Abertura".into()), None, Some("Fim".into())]
        );
    }

    #[test]
    fn missing_timed_verse_does_not_shift_following_translations() {
        assert_eq!(
            aligned(
                "[00:01]Opening\n[00:03]Ending",
                "Opening\nMiddle\nEnding",
                "Abertura\nMeio\nFim",
            ),
            [Some("Abertura".into()), Some("Fim".into())]
        );
    }

    #[test]
    fn joined_source_verse_follows_its_timed_parts() {
        assert_eq!(
            aligned(
                "[00:01]First part\n[00:02]Second part\n[00:03]Third part\n[00:04]Ending",
                "First part, second part, third part\nEnding",
                "Três partes juntas\nFim",
            ),
            [
                Some("Três partes juntas".into()),
                Some("Três partes juntas".into()),
                Some("Três partes juntas".into()),
                Some("Fim".into()),
            ]
        );
    }

    #[test]
    fn joined_timed_verse_keeps_both_translated_parts() {
        assert_eq!(
            aligned(
                "[00:01]First part, second part\n[00:03]Ending",
                "First part\nSecond part\nEnding",
                "Primeira parte\nSegunda parte\nFim",
            ),
            [
                Some("Primeira parte\nSegunda parte".into()),
                Some("Fim".into())
            ]
        );
    }

    #[test]
    fn punctuation_case_annotations_and_blank_lines_do_not_shift_alignment() {
        assert_eq!(
            aligned(
                "[00:00]♪\n[00:01]DON'T stop!\n[00:02]\n[00:03]Café?",
                "[Verse 1]\nDon't stop\n\nCafé",
                "[Verso 1]\nNão pare\n\nCafé",
            ),
            [None, Some("Não pare".into()), None, Some("Café".into())]
        );
    }

    #[test]
    fn repeated_verses_respect_the_surrounding_sequence() {
        assert_eq!(
            aligned(
                "[00:01]Refrain\n[00:02]Bridge\n[00:03]Refrain\n[00:04]Outro",
                "Refrain\nBridge\nExtra\nRefrain\nOutro",
                "Refrão inicial\nPonte\nExtra\nRefrão final\nFim",
            ),
            [
                Some("Refrão inicial".into()),
                Some("Ponte".into()),
                Some("Refrão final".into()),
                Some("Fim".into()),
            ]
        );
    }

    #[test]
    fn equal_line_counts_do_not_justify_matching_different_words() {
        assert_eq!(
            aligned(
                "[00:01]Different opening\n[00:02]Shared ending",
                "Original opening\nShared ending",
                "Abertura original\nFim em comum",
            ),
            [None, Some("Fim em comum".into())]
        );
    }

    #[test]
    fn ambiguous_bilingual_structure_clears_old_matches() {
        let mut lines = parse_lrc("[00:01]Opening\n[00:02]Ending");
        let mut external = crate::services::ExternalLyrics {
            original: "Opening\nEnding".into(),
            translation: Some("Abertura\nFim".into()),
            ..Default::default()
        };
        align_external_translation(&mut lines, &external);
        assert!(lines.iter().all(|line| line.translation.is_some()));
        external.translation = Some("Versos unidos sem correspondência explícita".into());
        align_external_translation(&mut lines, &external);
        assert!(lines.iter().all(|line| line.translation.is_none()));
        external.translation = None;
        align_external_translation(&mut lines, &external);
        assert!(lines.iter().all(|line| line.translation.is_none()));
    }

    #[test]
    fn timestamped_original_and_translation_can_be_used_as_the_bridge() {
        assert_eq!(
            aligned(
                "[00:01]Opening\n[00:02]Ending",
                "[00:01]Opening\n[00:02]Ending",
                "[00:01]Abertura\n[00:02]Fim",
            ),
            [Some("Abertura".into()), Some("Fim".into())]
        );
    }
    #[test]
    fn imports_and_automatic_results_can_arrive_in_either_order() {
        use crate::services::ExternalLyrics;
        let letras = ExternalLyrics {
            original: "Original verse".into(),
            translation: Some("Verso traduzido".into()),
            source: "letras.mus.br".into(),
            ..Default::default()
        };
        let lrc = ExternalLyrics {
            original: "[00:01]Original verse".into(),
            source: "lyricsify.com".into(),
            ..Default::default()
        };
        for order in [[letras.clone(), lrc.clone()], [lrc.clone(), letras.clone()]] {
            let mut current = None;
            let mut lines = Vec::new();
            for incoming in order {
                merge_external(&mut current, &mut lines, incoming);
            }
            assert!(!apply_synced(&mut lines, None));
            assert!(!apply_synced(&mut lines, Some("")));
            assert_eq!(lines[0].translation.as_deref(), Some("Verso traduzido"));
            let ext = current.unwrap();
            assert_eq!(ext.original, "Original verse");
            assert_eq!(ext.translation.as_deref(), Some("Verso traduzido"));
            assert_eq!(ext.source, "letras.mus.br");
        }
    }
    #[test]
    fn multiple_stamps_offset_and_precision() {
        let lines =
            parse_lrc("[ar:Artist]\n[offset:-200]\n[00:01.5][00:03.125]Olá\n[00:02.05]Second");
        assert_eq!(
            lines.iter().map(|l| l.timestamp_ms).collect::<Vec<_>>(),
            [1300, 1850, 2925]
        );
        assert_eq!(lines[2].text, "Olá");
        assert_eq!(active_line(&lines, 1299), None);
        assert_eq!(active_line(&lines, 1850), Some(1));
    }
    #[test]
    fn malformed_and_overflow_ignored() {
        assert!(
            parse_lrc("[99:99]bad\n[-1:02]bad\n[999999999999999999999:01]bad\n[00:02.x]bad")
                .is_empty()
        );
    }
    #[test]
    fn instrumental_empty_and_negative_offset() {
        assert!(parse_lrc("").is_empty());
        assert_eq!(parse_lrc("[offset:-5000]\n[00:01]start")[0].timestamp_ms, 0);
    }

    #[test]
    fn test_combine_with_translation() {
        let lrc = "[00:10.00]Someone said they left together\n[00:15.00]I ran out the door to get her\n[00:20.00]♪\n[00:25.00]She was with another man";
        let mut lines = parse_lrc(lrc);
        let translation = "Alguém disse que eles saíram juntos\nEu saí correndo pela porta\nEla estava com outro homem";
        align_external_translation(
            &mut lines,
            &crate::services::ExternalLyrics {
                original: "Someone said they left together\nI ran out the door to get her\nShe was with another man".into(),
                translation: Some(translation.into()),
                ..Default::default()
            },
        );

        assert_eq!(
            lines[0].translation.as_deref(),
            Some("Alguém disse que eles saíram juntos")
        );
        assert_eq!(
            lines[1].translation.as_deref(),
            Some("Eu saí correndo pela porta")
        );
        assert_eq!(lines[2].translation, None); // "♪" skipped
        assert_eq!(
            lines[3].translation.as_deref(),
            Some("Ela estava com outro homem")
        );
    }

    #[test]
    fn multiplier_suffixes_are_ignored_during_matching() {
        assert_eq!(
            aligned(
                "[00:01]Chorus\n[00:02]Outro",
                "Chorus (2x)\nOutro",
                "Refrão\nFim",
            ),
            [Some("Refrão".into()), Some("Fim".into())]
        );
        assert_eq!(
            aligned(
                "[00:01]Verse (x2)\n[00:02]Chorus [bis]",
                "Verse\nChorus",
                "Verso\nRefrão",
            ),
            [Some("Verso".into()), Some("Refrão".into())]
        );
    }

    #[test]
    fn repeated_verses_inherit_confirmed_translation_when_source_has_fewer_repetitions() {
        assert_eq!(
            aligned(
                "[00:01]Refrain\n[00:02]Verse\n[00:03]Refrain\n[00:04]Outro",
                "Refrain\nVerse\nOutro",
                "Refrão\nVerso\nFim",
            ),
            [
                Some("Refrão".into()),
                Some("Verso".into()),
                Some("Refrão".into()),
                Some("Fim".into()),
            ]
        );
    }

    #[test]
    fn multiple_repeated_verses_all_receive_translation() {
        assert_eq!(
            aligned(
                "[00:01]Chorus\n[00:02]Verse 1\n[00:03]Chorus\n[00:04]Verse 2\n[00:05]Chorus",
                "Chorus\nVerse 1\nVerse 2",
                "Refrão\nVerso 1\nVerso 2",
            ),
            [
                Some("Refrão".into()),
                Some("Verso 1".into()),
                Some("Refrão".into()),
                Some("Verso 2".into()),
                Some("Refrão".into()),
            ]
        );
    }

    #[test]
    fn multiplier_with_repetition_propagates_to_all_occurrences() {
        assert_eq!(
            aligned(
                "[00:01]I love you\n[00:02]I love you\n[00:03]Ending",
                "I love you (2x)\nEnding",
                "Eu te amo\nFim",
            ),
            [
                Some("Eu te amo".into()),
                Some("Eu te amo".into()),
                Some("Fim".into()),
            ]
        );
    }

    #[test]
    fn alignment_tolerates_slang_cross_split_and_minor_typos() {
        assert_eq!(
            aligned(
                "[00:01]I am nothin' without pretend\n[00:02]Line one part\n[00:03]Line two part\n[00:04]I wanted to give you everything",
                "I am nothing without pretend\nLine one\nPart line two\nPart\nI wanted yo give you everything",
                "Eu não sou nada sem fingir\nParte 1\nParte 2\nParte 3\nEu queria te dar tudo",
            ),
            [
                Some("Eu não sou nada sem fingir".into()),
                Some("Parte 1\nParte 2\nParte 3".into()),
                Some("Parte 1\nParte 2\nParte 3".into()),
                Some("Eu queria te dar tudo".into()),
            ]
        );
    }

    #[test]
    fn civilian_wye_oak_full_alignment() {
        let lrc = r#"[00:05.84] I am nothin' without pretend I know my thoughts
[00:12.36] Can't live with them
[00:16.67] I am nothin' without a man
[00:20.69] I know my faults, but I can't hide them
[00:37.98] I still keep my baby teeth
[00:42.12] In the bedside table with my jewelry
[00:48.52] You still sleep in the bed with me
[00:52.38] My jewelry and my baby teeth
[01:25.08] I don't need another friend
[01:29.30] When most of them I can barely keep up with
[01:34.60] Them perfectly able to hold my own hand
[01:39.96] But I still can't kiss my own neck
[01:56.73] I wanted to give you everything
[02:00.86] But I still stand in awe of superficial things
[02:07.52] I wanted to love you like my mother's, mother's, mother's did
[02:14.65] Civilian
[02:25.05] Civilian"#;

        let orig = r#"I am nothing without pretend
I know my thoughts
Can't live with them
I am nothing without a man
I know my faults
But I can't hide them
I still keep my baby teeth
In the bedside table with my jewelry
You still sleep in the bed with me
My jewelry, and my baby teeth
I don't need another friend
When most of them
I can barely keep up with them
Perfectly able to hold my own hand
But I still can't kiss my own neck
I wanted yo give you everything
But I still stand in awe of superficial things
I wanted to love you like my mother's mother's mother did
Civilian, civilian"#;

        let trans = r#"Eu não sou nada sem fingir
Eu conheço os meus pensamentos
Não consigo viver com eles
Eu não sou nada sem um homem
Eu conheço as minhas falhas
Mas eu não posso escondê-las
Eu ainda mantenho meus dentes de leite
Na mesa de cabeceira com minhas jóias
Você ainda dorme na cama comigo
Minhas jóias, e meus dentes de leite
Eu não preciso de outro amigo
Quando a maioria deles
Eu mal consigo acompanhar
Perfeitamente capaz de segurar minha própria mão,
Mas eu ainda não consigo beijar meu próprio pescoço
Eu queria te dar tudo
Mas eu ainda fico admirado com coisas superficiais
Eu queria amar você como a mãe da mãe da minha mãe fez
Civil, civil"#;

        let res = aligned(lrc, orig, trans);
        for (i, line) in res.iter().enumerate() {
            assert!(
                line.is_some(),
                "Line {i} should be matched with translation"
            );
        }
    }

    #[test]
    fn intro_m83_full_alignment_with_omitted_articles_and_misheard_words() {
        let lrc = r#"[00:29.00] We didn't need a story, we didn't need a real world
[00:36.70] We just had to keep walking
[00:39.02] And we became the stories, we became the places
[00:44.58] We were the lights, the deserts, the faraway worlds
[00:50.85] We were you before you even existed
[00:55.82] I carry on, carry on, carry on
[01:02.81] And after us the flood
[01:06.76] 
[01:13.82] Carry on, carry on, carry on
[01:21.09] Our silver horn it leads the way
[01:24.99] 
[01:32.57] Banners of gold shine
[01:36.95] In the cold, in the cold, in the cold
[01:41.74] 
[01:50.54] Footprints of snow
[01:53.21] Blind from the road
[01:56.75] Hail!
[02:05.02] 
[02:09.10] We carry on, carry on
[02:13.78] Follow us, we are one
[02:18.61] 
[02:25.86] Battle's fought, the deed is done
[02:30.58] The silver hum runs deep and strong
[02:37.13] 
[02:44.64] Hand to the heart, lips to the horn
[02:50.08] We can stand, we can be reborn
[02:58.60] 
[03:02.62] Head on my breast, I'll keep you warm
[03:09.75] Hail!
[03:15.16] "#;

        let orig = r#"We didn't need a story, we didn't need a real world
We just had to keep walking
And we became the stories, we became the places
We were the lights, the deserts, the faraway worlds
We were you before you even existed
Carry on, carry on, carry on
And after us the flood
Carry on, carry on, carry on
Our silver horn it leads the way
Banners of gold shine
In the cold, in the cold, in the cold
Footprints of snow, blind from the road
Hail!
We carry on, carry on
Follow us, we are one
The battle's fought, the deed is done
Our silver hum runs deep and strong
Hand to the heart, lips to the horn
We can save, we can be reborn
Head on my breast, I'll keep you warm
Hail!"#;

        let trans = r#"Não precisávamos de uma história, nós não precisávamos de um mundo real
Nós só tínhamos que continuar caminhando
E nós nos tornamos as histórias, nos tornamos os lugares
Nós éramos as luzes, os desertos, os mundos distantes
Nós éramos você antes mesmo de você existir
Siga em frente, siga em frente, siga em frente
E atrás de nós dilúvio
Siga em frente, siga em frente, siga em frente
Nosso chifre prateado lidera o caminho
Estandarte de ouro brilha
No frio, no frio, no frio
Pegadas de neve, cego pela neve
Saudação!
Nós seguimos em frente, em frente
Venha conosco, somos um só
A batalha foi travada, o ato está feito
Nosso zumbido de prata é mais profundo e forte
Mão no coração, lábios no chifre
Podemos salvar, podemos renascer
Cabeça no meu peito, eu vou te manter aquecido
Saudação!"#;

        let mut lines = parse_lrc(lrc);
        let ext = crate::services::ExternalLyrics {
            original: orig.to_string(),
            translation: Some(trans.to_string()),
            ..Default::default()
        };
        align_external_translation(&mut lines, &ext);

        let non_empty: Vec<_> = lines.iter().filter(|l| is_verse(&l.text)).collect();

        for line in &non_empty {
            assert!(
                line.translation.is_some(),
                "Line {:?} should have translation",
                line.text
            );
        }

        let find_trans = |prefix: &str| {
            lines
                .iter()
                .find(|l| l.text.contains(prefix))
                .and_then(|l| l.translation.as_deref())
                .unwrap_or("")
        };
        assert_eq!(
            find_trans("Battle's fought"),
            "A batalha foi travada, o ato está feito"
        );
        assert_eq!(
            find_trans("The silver hum"),
            "Nosso zumbido de prata é mais profundo e forte"
        );
        assert_eq!(
            find_trans("We can stand"),
            "Podemos salvar, podemos renascer"
        );
    }
}
