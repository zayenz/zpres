use std::collections::BTreeMap;

use crate::deck::{ContentBlock, Deck, Slide};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpeakerNote {
    pub(crate) markdown: String,
    pub(crate) html: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SpeakerNotesCatalog {
    by_slide_id: BTreeMap<String, SpeakerNote>,
}

impl SpeakerNotesCatalog {
    pub(crate) fn from_deck(deck: &Deck) -> Self {
        let by_slide_id = deck
            .sections
            .iter()
            .flat_map(|section| {
                std::iter::once(&section.main_slide).chain(section.detail_slides.iter())
            })
            .filter_map(|slide| {
                slide_speaker_notes(slide).map(|markdown| {
                    let html = render_speaker_notes_markdown_body(&markdown);
                    (slide.id.clone(), SpeakerNote { markdown, html })
                })
            })
            .collect();
        Self { by_slide_id }
    }

    pub(crate) fn html_for(&self, slide_id: &str) -> &str {
        self.by_slide_id
            .get(slide_id)
            .map_or("", |note| note.html.as_str())
    }

    #[cfg(test)]
    pub(crate) fn get(&self, slide_id: &str) -> Option<&SpeakerNote> {
        self.by_slide_id.get(slide_id)
    }
}

pub(crate) fn slide_speaker_notes(slide: &Slide) -> Option<String> {
    let notes = slide
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::SpeakerNotes { markdown } if !markdown.trim().is_empty() => {
                Some(markdown.trim())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!notes.is_empty()).then_some(notes)
}

pub(crate) fn render_speaker_notes_body(markdown: &str) -> String {
    format!("<p>{}</p>", render_speaker_note_lines(markdown))
}

fn render_speaker_note_lines(markdown: &str) -> String {
    let mut rendered = String::new();
    let mut next_click = 1usize;
    for (line_index, line) in markdown.lines().enumerate() {
        if line_index > 0 {
            rendered.push_str("<br>");
        }
        if let Some((click, body)) = parse_speaker_note_click_marker(line.trim_start()) {
            let click_index = click.unwrap_or(next_click);
            next_click = next_click.max(click_index + 1);
            rendered.push_str(&format!(
                "<span class=\"zpres-speaker-note-click\" data-note-click-index=\"{}\">{}</span>",
                click_index,
                escape_html(body)
            ));
        } else {
            rendered.push_str(&escape_html(line));
        }
    }
    rendered
}

pub(crate) fn render_speaker_notes_markdown_body(markdown: &str) -> String {
    let mut rendered = String::new();
    let mut next_click = 1usize;
    for (line_index, line) in markdown.lines().enumerate() {
        if line_index > 0 {
            rendered.push_str("<br>");
        }
        if let Some((click, body)) = parse_speaker_note_click_marker(line.trim_start()) {
            let click_index = click.unwrap_or(next_click);
            next_click = next_click.max(click_index + 1);
            rendered.push_str(&format!(
                "<span class=\"zpres-speaker-note-click\" data-note-click-index=\"{}\">{}</span>",
                click_index,
                render_speaker_note_inline(body)
            ));
        } else {
            rendered.push_str(&render_speaker_note_inline(line));
        }
    }
    format!("<p>{rendered}</p>")
}

fn render_speaker_note_inline(markdown: &str) -> String {
    let mut rendered = String::new();
    let mut rest = markdown;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('\\')
            && let Some(character) = after.chars().next()
        {
            rendered.push_str(&escape_html(&character.to_string()));
            rest = &after[character.len_utf8()..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            rendered.push_str("<code>");
            rendered.push_str(&escape_html(&after[..end]));
            rendered.push_str("</code>");
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            rendered.push_str("<strong>");
            rendered.push_str(&render_speaker_note_inline(&after[..end]));
            rendered.push_str("</strong>");
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            rendered.push_str("<em>");
            rendered.push_str(&render_speaker_note_inline(&after[..end]));
            rendered.push_str("</em>");
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after_label_start) = rest.strip_prefix('[')
            && let Some(label_end) = after_label_start.find("](")
        {
            let after_url_start = &after_label_start[label_end + 2..];
            if let Some(url_end) = after_url_start.find(')') {
                let url = &after_url_start[..url_end];
                if is_safe_speaker_note_url(url) {
                    rendered.push_str("<a href=\"");
                    rendered.push_str(&escape_attr(url));
                    rendered.push_str("\">");
                    rendered.push_str(&render_speaker_note_inline(&after_label_start[..label_end]));
                    rendered.push_str("</a>");
                    rest = &after_url_start[url_end + 1..];
                    continue;
                }
            }
        }
        let next = rest
            .char_indices()
            .skip(1)
            .find(|(_, character)| matches!(character, '\\' | '`' | '*' | '['))
            .map(|(index, _)| index)
            .unwrap_or(rest.len());
        rendered.push_str(&escape_html(&rest[..next]));
        rest = &rest[next..];
    }
    rendered
}

fn is_safe_speaker_note_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
        || lower.starts_with('/')
        || lower.starts_with("./")
        || lower.starts_with("../")
        || !lower.contains(':')
}

fn parse_speaker_note_click_marker(line: &str) -> Option<(Option<usize>, &str)> {
    let rest = line.strip_prefix("[click")?;
    let (click, after_marker) = if let Some(after_marker) = rest.strip_prefix(']') {
        (None, after_marker)
    } else {
        let after_colon = rest.strip_prefix(':')?;
        let (raw_click, after_marker) = after_colon.split_once(']')?;
        let click = raw_click.trim().parse::<usize>().ok()?;
        (Some(click), after_marker)
    };
    Some((click, after_marker.trim_start()))
}

fn escape_attr(value: &str) -> String {
    escape_html(value).replace('"', "&quot;")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::parse_source_text;

    #[test]
    fn catalog_keeps_stable_ids_markers_markdown_and_sanitization() {
        let deck = parse_source_text(
            "::: slide\nid: stable\n:::\n# Title\n\n::: notes\n**Strong** and [safe](https://example.com).\n[click] `code`\n[unsafe](javascript:alert(1))\n:::\n",
            None,
        )
        .unwrap();
        let catalog = SpeakerNotesCatalog::from_deck(&deck);
        let note = catalog.get("stable").unwrap();
        assert!(note.markdown.contains("[click] `code`"));
        assert!(note.html.contains("<strong>Strong</strong>"));
        assert!(note.html.contains("data-note-click-index=\"1\""));
        assert!(note.html.contains("<code>code</code>"));
        assert!(!note.html.contains("href=\"javascript:"));
    }
}
