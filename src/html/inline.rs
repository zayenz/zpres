//! The v1 inline subset: emphasis, strong, code, links, math, and citations.
//! Unsupported or incomplete syntax stays visible as escaped text.
use super::{RenderContext, escape_attr, escape_html, render_math};
use crate::deck::inline_code_span;

pub(super) fn render(source: &str, math: &[String], context: &RenderContext) -> String {
    render_inner(source, math, context, 0)
}

fn render_inner(source: &str, math: &[String], context: &RenderContext, depth: usize) -> String {
    if depth >= 16 {
        return escape_html(source);
    }
    let mut html = String::new();
    let mut offset = 0;
    while offset < source.len() {
        let rest = &source[offset..];
        if let Some((code, consumed)) = inline_code_span(rest) {
            html.push_str("<code>");
            html.push_str(&escape_html(code));
            html.push_str("</code>");
            offset += consumed;
            continue;
        }
        if let Some((latex, consumed)) = math_at(rest, math) {
            html.push_str(&render_math(latex, false));
            offset += consumed;
            continue;
        }
        if let Some(escaped) = rest.strip_prefix('\\')
            && let Some(character) = escaped.chars().next().filter(char::is_ascii_punctuation)
        {
            html.push_str(&escape_html(&character.to_string()));
            offset += 1 + character.len_utf8();
            continue;
        }
        if let Some(reference) = rest.strip_prefix("[^")
            && let Some(end) = reference.find(']')
            && let Some(number) = context.footnote_numbers.get(&reference[..end])
        {
            let label = &reference[..end];
            let target = context.footnote_target(label);
            let occurrence = context.footnote_occurrence.get() + 1;
            context.footnote_occurrence.set(occurrence);
            html.push_str(&format!(
                "<sup id=\"{}-ref-{occurrence}\" class=\"zpres-footnote-ref\" data-footnote-label=\"{}\" style=\"margin-inline:0.2em\"><a href=\"#{}\" data-zpres-footnote-target=\"{}\">[{number}]</a></sup>",
                escape_attr(&target), escape_attr(label), escape_attr(&target), escape_attr(&target),
            ));
            offset += end + 3;
            continue;
        }
        if let Some((label, target, consumed)) = link(rest) {
            // Explicitly permit ordinary document links; interpreter schemes stay text.
            if safe_link(target) {
                html.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_attr(target),
                    render_inner(label, math, context, depth + 1),
                ));
            } else {
                html.push_str(&escape_html(&rest[..consumed]));
            }
            offset += consumed;
            continue;
        }
        let marker = if rest.starts_with("**") || rest.starts_with("__") {
            Some((&rest[..2], "strong"))
        } else if rest.starts_with('*') || rest.starts_with('_') {
            Some((&rest[..1], "em"))
        } else {
            None
        };
        if let Some((marker, tag)) = marker {
            let body = &rest[marker.len()..];
            let intraword = marker.starts_with('_')
                && source[..offset]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            if !intraword
                && !body.starts_with(char::is_whitespace)
                && let Some(end) = closing_marker(body, marker, math)
                && end > 0
                && !body[..end].ends_with(char::is_whitespace)
            {
                html.push_str(&format!(
                    "<{tag}>{}</{tag}>",
                    render_inner(&body[..end], math, context, depth + 1)
                ));
                offset += marker.len() * 2 + end;
                continue;
            }
        }
        let character = rest.chars().next().unwrap();
        if character == '\n' {
            html.push_str("<br>");
        } else {
            html.push_str(&escape_html(&character.to_string()));
        }
        offset += character.len_utf8();
    }
    html
}

fn math_at<'a>(source: &str, math: &'a [String]) -> Option<(&'a str, usize)> {
    if !source.starts_with(['$', '\\']) {
        return None;
    }
    math.iter().find_map(|latex| {
        [format!(r"\({latex}\)"), format!("${latex}$")]
            .into_iter()
            .find(|spelling| source.starts_with(spelling))
            .map(|spelling| (latex.as_str(), spelling.len()))
    })
}

fn closing_marker(source: &str, marker: &str, math: &[String]) -> Option<usize> {
    let mut offset = 0;
    while offset < source.len() {
        let rest = &source[offset..];
        if let Some((_, consumed)) = inline_code_span(rest) {
            offset += consumed;
        } else if let Some((_, consumed)) = math_at(rest, math) {
            offset += consumed;
        } else if let Some(escaped) = rest.strip_prefix('\\') {
            offset += 1 + escaped.chars().next().map_or(0, char::len_utf8);
        } else if rest.starts_with(marker) {
            let run = rest
                .bytes()
                .take_while(|byte| *byte == marker.as_bytes()[0])
                .count();
            if marker.len() == 1 && run == 2 {
                offset += run;
            } else {
                return Some(offset + run.saturating_sub(marker.len()));
            }
        } else {
            offset += rest.chars().next().unwrap().len_utf8();
        }
    }
    None
}

fn link(source: &str) -> Option<(&str, &str, usize)> {
    let label = source.strip_prefix('[')?;
    let label_end = label.find("](")?;
    let target_start = label_end + 3;
    let mut nesting = 0usize;
    for (offset, character) in source[target_start..].char_indices() {
        match character {
            '(' => nesting += 1,
            ')' if nesting == 0 => {
                return Some((
                    &label[..label_end],
                    &source[target_start..target_start + offset],
                    target_start + offset + 1,
                ));
            }
            ')' => nesting -= 1,
            _ => {}
        }
    }
    None
}

fn safe_link(target: &str) -> bool {
    if target.is_empty()
        || target
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return false;
    }
    let head = target.split(['/', '?', '#']).next().unwrap_or_default();
    match head.split_once(':') {
        Some((scheme, _)) => matches!(
            scheme.to_ascii_lowercase().as_str(),
            "https" | "http" | "mailto"
        ),
        None => true,
    }
}
