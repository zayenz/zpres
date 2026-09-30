use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::chart::ChartPoint;
use crate::deck::{
    BackgroundImageFit, BackgroundImageSplitSide, CalloutKind, ChartData, CodeReveal, ContentBlock,
    Deck, DeckBackgroundImage, DiagnosticSeverity, DiagramLanguage, FigureAlign, FigureFit,
    FigureOptions, Footnote, GalleryItem, LayoutAlign, LayoutKind, LayoutRegion, LayoutSize,
    LayoutValues, ListItem, MediaKind, MermaidDirection, MermaidFlowchart, PdfStepState, Slide,
    SlideVariant, Step, StepPdfPolicy, TableAlignment, inline_math_segments,
    parse_mermaid_flowchart,
};
use crate::file_url::file_url;
use crate::publication;
use crate::theme::{self, RenderedTheme};

mod inline;
mod theme_api_v1;

#[derive(Debug, Error)]
pub enum HtmlError {
    #[error("cannot render chart on slide '{slide_id}': {reason}")]
    UnsupportedChart { slide_id: String, reason: String },
    #[error("failed to write HTML bundle at {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("HTML bundle path '{path}' is claimed by both {first_owner} and {second_owner}")]
    BundlePathCollision {
        path: PathBuf,
        first_owner: String,
        second_owner: String,
    },
    #[error("failed to copy HTML dependency from {source_path} to {destination}: {source}")]
    Copy {
        source_path: PathBuf,
        destination: PathBuf,
        source: io::Error,
    },
    #[error("cannot publish HTML bundle at {path}: {source}")]
    Publication {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(transparent)]
    ThemeContract(#[from] theme::ThemeDeckContractError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlBundlePublicationReport {
    pub output_root: PathBuf,
    pub index_path: PathBuf,
    pub generation: String,
    pub generation_path: PathBuf,
    pub presentation_index: PathBuf,
    pub warnings: Vec<String>,
}

/// Controls audience-visible content in a live HTML presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveHtmlOptions {
    pub include_speaker_notes: bool,
}

impl Default for LiveHtmlOptions {
    fn default() -> Self {
        Self {
            include_speaker_notes: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentHtmlBundlePublication {
    pub generation: String,
    pub generation_path: PathBuf,
    pub presentation_index: PathBuf,
}

/// Resolve the complete HTML generation selected by the root `index.html`.
///
/// This is read-only. It returns `None` when no generation-qualified HTML has
/// been published yet.
pub fn current_html_bundle_publication(
    output_dir: &Path,
) -> Result<Option<CurrentHtmlBundlePublication>, HtmlError> {
    publication::current_html_publication(output_dir)
        .map(|publication| {
            publication.map(|publication| CurrentHtmlBundlePublication {
                generation: publication.generation,
                generation_path: publication.generation_path,
                presentation_index: publication.presentation_index,
            })
        })
        .map_err(|source| HtmlError::Publication {
            path: output_dir.to_path_buf(),
            source: Box::new(source),
        })
}

pub fn write_debug_html_bundle(
    deck: &Deck,
    theme: &RenderedTheme,
    output_dir: &Path,
) -> Result<HtmlBundlePublicationReport, HtmlError> {
    write_debug_html_bundle_with_options(deck, theme, output_dir, LiveHtmlOptions::default())
}

pub fn write_debug_html_bundle_with_options(
    deck: &Deck,
    theme: &RenderedTheme,
    output_dir: &Path,
    options: LiveHtmlOptions,
) -> Result<HtmlBundlePublicationReport, HtmlError> {
    theme::validate_deck_for_theme_contract(deck, &theme.manifest)?;
    for slide in deck.pdf_slide_order() {
        validate_chart_blocks(&slide.blocks, &slide.id, deck.deck_root())?;
    }
    validate_html_bundle_inventory(deck, theme)?;
    let publication = publication::publish_html_bundle(output_dir, |stage| {
        theme_api_v1::populate_html_bundle(deck, theme, stage, options)
    })
    .map_err(|source| HtmlError::Publication {
        path: output_dir.to_path_buf(),
        source: Box::new(source),
    })?;
    Ok(HtmlBundlePublicationReport {
        output_root: publication.output_root,
        index_path: publication.pointer_index,
        generation: publication.generation,
        generation_path: publication.generation_path,
        presentation_index: publication.presentation_index,
        warnings: publication.warnings,
    })
}

fn validate_chart_blocks(
    blocks: &[ContentBlock],
    slide_id: &str,
    deck_root: Option<&Path>,
) -> Result<(), HtmlError> {
    for block in blocks {
        match block {
            ContentBlock::Chart { spec, data, .. } => {
                crate::chart::load(spec, data.as_ref(), deck_root).map_err(|reason| {
                    HtmlError::UnsupportedChart {
                        slide_id: slide_id.to_string(),
                        reason,
                    }
                })?;
            }
            ContentBlock::Layout { regions, .. } => {
                for region in regions {
                    validate_chart_blocks(&region.blocks, slide_id, deck_root)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_html_bundle_inventory(deck: &Deck, theme: &RenderedTheme) -> Result<(), HtmlError> {
    let mut claims = BTreeMap::<PathBuf, String>::new();
    for path in ["index.html", "assets/theme.css", "assets/reveal.js"] {
        claim_html_bundle_path(&mut claims, PathBuf::from(path), "the renderer")?;
    }
    for path in [
        publication::HTML_STAGE_MARKER_FILE,
        publication::HTML_GENERATION_MARKER_FILE,
        publication::HTML_PRESENTATION_FILE,
    ] {
        claim_html_bundle_path(
            &mut claims,
            PathBuf::from(path),
            "HTML publication metadata",
        )?;
    }
    claim_html_bundle_path(
        &mut claims,
        PathBuf::from(theme_api_v1::FOUNDATION_ASSET_PATH),
        "the Theme API renderer",
    )?;
    for dependency in theme::theme_dependency_paths(&theme.manifest) {
        claim_html_bundle_path(
            &mut claims,
            Path::new("assets").join(dependency),
            &format!("Theme dependency '{dependency}'"),
        )?;
    }
    for reference in deck.local_asset_references() {
        claim_html_bundle_path(
            &mut claims,
            PathBuf::from(reference),
            &format!("Deck dependency '{reference}'"),
        )?;
    }
    Ok(())
}

fn claim_html_bundle_path(
    claims: &mut BTreeMap<PathBuf, String>,
    path: PathBuf,
    owner: &str,
) -> Result<(), HtmlError> {
    let portable_path = portable_bundle_path_components(&path);
    for (claimed_path, claimed_owner) in claims.iter() {
        let portable_claimed_path = portable_bundle_path_components(claimed_path);
        if portable_path == portable_claimed_path
            || portable_path.starts_with(&portable_claimed_path)
            || portable_claimed_path.starts_with(&portable_path)
        {
            return Err(HtmlError::BundlePathCollision {
                path,
                first_owner: claimed_owner.clone(),
                second_owner: owner.to_string(),
            });
        }
    }
    claims.insert(path, owner.to_string());
    Ok(())
}

fn portable_bundle_path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component.to_string_lossy().to_lowercase()),
            Component::CurDir => None,
            _ => Some(component.as_os_str().to_string_lossy().to_lowercase()),
        })
        .collect()
}

/// Renders an already prepared Deck.
///
/// Published Output targets should use [`write_debug_html_bundle`], which
/// enforces the selected Theme contract before it writes any files.
pub fn render_debug_html(deck: &Deck, theme: &RenderedTheme) -> String {
    render_debug_html_with_options(deck, theme, LiveHtmlOptions::default())
}

pub fn render_debug_html_with_options(
    deck: &Deck,
    theme: &RenderedTheme,
    options: LiveHtmlOptions,
) -> String {
    theme_api_v1::render_html(deck, theme, options)
}

/// Renders print HTML for an already prepared Deck.
///
/// Published PDF, PNG, and JPEG Output targets should use the fallible PDF
/// renderer APIs, which enforce the selected Theme contract before writing.
pub fn render_debug_print_html(deck: &Deck, theme: &RenderedTheme) -> String {
    render_debug_print_html_with_options(deck, theme, StaticExportOptions::default())
}

pub fn render_debug_print_html_with_options(
    deck: &Deck,
    theme: &RenderedTheme,
    options: StaticExportOptions,
) -> String {
    render_debug_print_html_filtered(deck, theme, options, None)
}

pub fn print_page_count_for_theme_with_options(
    deck: &Deck,
    _theme: &RenderedTheme,
    options: StaticExportOptions,
) -> usize {
    crate::presentation_plan::PresentationPlan::for_theme_api_v1(deck)
        .print_pages(options.include_speaker_notes)
        .len()
}

/// Renders one print page for an already prepared Deck.
pub fn render_debug_print_page_html(
    deck: &Deck,
    theme: &RenderedTheme,
    page_index: usize,
) -> Option<String> {
    render_debug_print_page_html_with_options(
        deck,
        theme,
        page_index,
        StaticExportOptions::default(),
    )
}

pub fn render_debug_print_page_html_with_options(
    deck: &Deck,
    theme: &RenderedTheme,
    page_index: usize,
    options: StaticExportOptions,
) -> Option<String> {
    (1..=print_page_count_for_theme_with_options(deck, theme, options))
        .contains(&page_index)
        .then(|| render_debug_print_html_filtered(deck, theme, options, Some(page_index)))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticExportOptions {
    pub include_speaker_notes: bool,
}

fn render_debug_print_html_filtered(
    deck: &Deck,
    theme: &RenderedTheme,
    options: StaticExportOptions,
    selected_page_index: Option<usize>,
) -> String {
    theme_api_v1::render_print_html_filtered(deck, theme, options, selected_page_index)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackgroundPhase {
    Title,
    Splash,
    Content,
}

fn background_phase_attr(phase: Option<BackgroundPhase>) -> String {
    let Some(phase) = phase else {
        return String::new();
    };
    let value = match phase {
        BackgroundPhase::Title => "title",
        BackgroundPhase::Splash => "splash",
        BackgroundPhase::Content => "content",
    };
    format!(" data-background-phase=\"{value}\"")
}

fn background_split_attr(background_image: Option<&DeckBackgroundImage>) -> String {
    let Some(split) = background_image.and_then(|image| image.split.as_ref()) else {
        return String::new();
    };
    let value = match split.side {
        BackgroundImageSplitSide::Left => "left",
        BackgroundImageSplitSide::Right => "right",
    };
    format!(" data-background-split=\"{value}\"")
}

#[derive(Clone, Copy)]
struct SlideNumberContext {
    current: usize,
    total: usize,
}

fn slide_footer_control_attrs(slide: &Slide) -> String {
    let mut attrs = String::new();
    if slide.footer.hidden {
        attrs.push_str(" data-footer-hidden=\"true\"");
    }
    if slide.footer.content.is_some() {
        attrs.push_str(" data-footer-content=\"true\"");
    }
    if let Some(show) = slide.footer.slide_numbers {
        attrs.push_str(&format!(" data-slide-numbers=\"{show}\""));
    }
    attrs
}

fn slide_autoscale_attr(slide: &Slide, deck: &Deck) -> String {
    if let Some(value) = slide.autoscale {
        format!(" data-autoscale=\"{value}\"")
    } else if deck.metadata.autoscale == Some(true) {
        " data-autoscale=\"true\"".to_string()
    } else {
        String::new()
    }
}

fn slide_transition_attr(slide: &Slide, deck: &Deck) -> String {
    let transition = slide.transition.or(deck.metadata.transition);
    transition.map_or_else(String::new, |transition| {
        format!(" data-transition=\"{}\"", transition.as_str())
    })
}

fn render_slide_footer(
    slide: &Slide,
    deck: &Deck,
    theme: &RenderedTheme,
    section_title: Option<&str>,
    slide_number: SlideNumberContext,
) -> String {
    if slide.footer.hidden {
        return String::new();
    }

    let footer_mode = theme
        .params
        .get("footer")
        .map(String::as_str)
        .unwrap_or("slide-number");
    let show_slide_number = slide.footer.slide_numbers.unwrap_or_else(|| {
        deck.metadata
            .slide_numbers
            .unwrap_or(matches!(footer_mode, "slide-number" | "section-progress"))
    });
    let content = slide
        .footer
        .content
        .as_deref()
        .or(deck.metadata.footer.as_deref())
        .or_else(|| {
            (footer_mode == "section-title")
                .then_some(section_title)
                .flatten()
        });

    if content.is_none() && !show_slide_number {
        return String::new();
    }

    let mut html = format!(
        "          <footer class=\"zpres-slide-footer\" data-footer-mode=\"{}\" data-slide-number=\"{}\" data-slide-count=\"{}\">\n",
        escape_attr(footer_mode),
        slide_number.current,
        slide_number.total
    );
    if let Some(content) = content {
        html.push_str(&format!(
            "            <span class=\"zpres-slide-footer-content\">{}</span>\n",
            escape_multiline(content)
        ));
    }
    if show_slide_number {
        html.push_str(&format!(
            "            <span class=\"zpres-slide-footer-number\">{} / {}</span>\n",
            slide_number.current, slide_number.total
        ));
    }
    html.push_str("          </footer>\n");
    html
}

fn slide_variant_attr(slide: &Slide) -> String {
    slide.variant.map_or_else(String::new, |variant| {
        format!(" data-slide-variant=\"{}\"", slide_variant_name(variant))
    })
}

fn slide_preset_attr(slide: &Slide) -> String {
    slide.preset.as_ref().map_or_else(String::new, |preset| {
        format!(" data-slide-preset=\"{}\"", escape_attr(preset))
    })
}

fn slide_classes_attr(slide: &Slide) -> String {
    if slide.classes.is_empty() {
        String::new()
    } else {
        format!(
            " data-slide-classes=\"{}\"",
            escape_attr(&slide.classes.join(" "))
        )
    }
}

fn slide_theme_attrs(slide: &Slide) -> String {
    if slide.theme_params.is_empty() {
        return String::new();
    }
    let mut attrs = format!(
        " data-theme-params=\"{}\"",
        escape_attr(
            &slide
                .theme_params
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        )
    );
    for (name, value) in &slide.theme_params {
        attrs.push_str(&format!(
            " data-theme-param-{}=\"{}\"",
            theme::theme_param_data_name(name),
            escape_attr(value)
        ));
    }
    attrs
}

fn slide_theme_style_attr(slide: &Slide, theme: &RenderedTheme) -> String {
    slide_theme_style_attr_with_extra(slide, theme, &[])
}

fn slide_theme_style_attr_with_extra(
    slide: &Slide,
    theme: &RenderedTheme,
    extra: &[(&str, &str)],
) -> String {
    let params = slide_theme_style_params(slide, theme);
    let mut declarations = theme::theme_param_css_declarations(&theme.manifest, &params)
        .into_iter()
        .map(|declaration| format!("{}: {};", declaration.name, declaration.value))
        .collect::<Vec<_>>();
    declarations.extend(
        extra
            .iter()
            .map(|(name, value)| format!("{name}: {value};")),
    );
    if declarations.is_empty() {
        return String::new();
    }
    let style = declarations.join(" ");
    format!(" style=\"{}\"", escape_attr(&style))
}

fn slide_theme_style_params(slide: &Slide, theme: &RenderedTheme) -> BTreeMap<String, String> {
    if slide.theme_params.is_empty() {
        return BTreeMap::new();
    }
    let resolved = theme::validate_theme_params_best_effort(&theme.manifest, &slide.theme_params);
    let mut names = slide.theme_params.keys().cloned().collect::<BTreeSet<_>>();
    if let Some(variant_name) = slide.theme_params.get(&theme.manifest.palette_parameter)
        && let Some(variant) = theme.manifest.color_variants.get(variant_name)
    {
        names.extend(variant.colors.keys().cloned());
    }
    names
        .into_iter()
        .filter_map(|name| resolved.get(&name).cloned().map(|value| (name, value)))
        .collect()
}

fn slide_section_classes(slide: &Slide, base: &str) -> String {
    if slide.classes.is_empty() && slide.preset.is_none() {
        base.to_string()
    } else {
        let mut class_hooks = slide
            .classes
            .iter()
            .map(|class| format!("zpres-slide-class-{class}"))
            .collect::<Vec<_>>();
        if let Some(preset) = &slide.preset {
            class_hooks.push(format!("zpres-slide-preset-{preset}"));
        }
        let class_hooks = class_hooks.join(" ");
        format!("{base} {class_hooks}")
    }
}

fn slide_variant_name(variant: SlideVariant) -> &'static str {
    variant.as_str()
}

fn pdf_step_state_attr(step_state: PdfStepState) -> String {
    match step_state {
        PdfStepState::Final => " data-pdf-step-state=\"final\"".to_string(),
        PdfStepState::UpTo { step } => {
            format!(" data-pdf-step-state=\"up-to\" data-pdf-step=\"{step}\"")
        }
    }
}

#[derive(Clone, Copy)]
enum AssetRenderMode<'a> {
    Bundle {
        deck_root: Option<&'a Path>,
    },
    Print {
        deck_root: Option<&'a Path>,
        step_state: PdfStepState,
    },
}

#[derive(Debug, Default)]
struct RenderContext {
    footnote_numbers: BTreeMap<String, usize>,
    footnote_scope: Option<String>,
    footnote_occurrence: Cell<usize>,
    slide_variant: Option<crate::deck::SlideVariant>,
}

impl RenderContext {
    fn for_page(
        slide: &Slide,
        _theme_api: theme::ThemeApiVersion,
        state: Option<PdfStepState>,
    ) -> Self {
        let mut footnote_numbers = BTreeMap::new();
        for block in &slide.blocks {
            if let ContentBlock::Footnotes { notes } = block {
                for note in notes {
                    footnote_numbers.insert(note.label.clone(), note.number);
                }
            }
        }
        Self {
            footnote_numbers,
            footnote_scope: Some({
                let page = match state {
                    None => "live".to_string(),
                    Some(PdfStepState::Final) => "print-final".to_string(),
                    Some(PdfStepState::UpTo { step }) => format!("print-step-{step}"),
                };
                format!("{}-{page}", slide.id)
            }),
            footnote_occurrence: Cell::new(0),
            slide_variant: slide.variant,
        }
    }
    fn footnote_target(&self, label: &str) -> String {
        match (&self.footnote_scope, self.footnote_numbers.get(label)) {
            (Some(scope), Some(number)) => format!("zpres-footnote-{scope}-{number}"),
            _ => footnote_dom_id(label),
        }
    }
}

impl<'a> AssetRenderMode<'a> {
    fn deck_root(self) -> Option<&'a Path> {
        match self {
            AssetRenderMode::Bundle { deck_root } | AssetRenderMode::Print { deck_root, .. } => {
                deck_root
            }
        }
    }

    fn step_state(self) -> Option<PdfStepState> {
        match self {
            AssetRenderMode::Bundle { .. } => None,
            AssetRenderMode::Print { step_state, .. } => Some(step_state),
        }
    }
}

fn render_block(
    block: &ContentBlock,
    asset_mode: AssetRenderMode<'_>,
    context: &RenderContext,
) -> String {
    match block {
        ContentBlock::Heading { level, text } => format!(
            "            <div class=\"debug-block zpres-block zpres-block-heading\" data-block-type=\"heading\"><span class=\"debug-block-label zpres-block-label\">heading h{}</span><h{}>{}</h{}></div>\n",
            level,
            level,
            render_paragraph(text, &inline_math_segments(text), context),
            level
        ),
        ContentBlock::Paragraph {
            markdown,
            inline_math,
        } => format!(
            "            <div class=\"debug-block zpres-block zpres-block-paragraph\" data-block-type=\"paragraph\"><span class=\"debug-block-label zpres-block-label\">paragraph</span><p>{}</p></div>\n",
            render_paragraph(markdown, inline_math, context)
        ),
        ContentBlock::FitText {
            markdown,
            inline_math,
        } => render_fit_text(markdown, inline_math, context),
        ContentBlock::Quote {
            markdown,
            inline_math,
        } => render_quote(markdown, inline_math, context),
        ContentBlock::Callout {
            kind,
            title,
            markdown,
            inline_math,
        } => render_callout(*kind, title.as_deref(), markdown, inline_math, context),
        ContentBlock::List {
            ordered,
            reveal,
            reveal_skip_first,
            items,
        } => render_list(
            *ordered,
            *reveal,
            *reveal_skip_first,
            items,
            asset_mode.step_state().is_some(),
            context,
        ),
        ContentBlock::Math { display, latex } => format!(
            "            <div class=\"debug-block zpres-block debug-math zpres-block-math\" data-block-type=\"math\" data-zpres-type-role=\"technical\" data-zpres-content-role=\"technical\"><span class=\"debug-block-label zpres-block-label\">{} math</span>{}</div>\n",
            if *display { "display" } else { "inline" },
            render_math(latex, *display)
        ),
        ContentBlock::Code {
            language,
            code,
            reveal,
        } => render_code_block(
            language.as_deref(),
            code,
            reveal.as_ref(),
            asset_mode.step_state(),
        ),
        ContentBlock::Table {
            headers,
            alignments,
            rows,
        } => render_table(headers, alignments, rows, context),
        ContentBlock::Figure {
            src,
            alt,
            caption,
            static_src,
            options,
        } => render_figure(
            src,
            static_src.as_deref(),
            alt,
            caption,
            options,
            asset_mode,
            context,
        ),
        ContentBlock::Footnotes { notes } => render_footnotes(notes, context),
        ContentBlock::Gallery { items, columns } => {
            render_gallery(items, *columns, asset_mode, context)
        }
        ContentBlock::Media {
            kind,
            src,
            title,
            caption,
            poster,
            alt,
            start_time,
            options,
            autoplay,
            controls,
            loop_playback,
            muted,
            autoadvance,
            visual_hidden,
        } => render_media(MediaRender {
            kind: *kind,
            src,
            title: title.as_deref(),
            caption: caption.as_deref(),
            poster: poster.as_deref(),
            alt,
            start_time: *start_time,
            options,
            autoplay: *autoplay,
            controls: *controls,
            loop_playback: *loop_playback,
            muted: *muted,
            autoadvance: *autoadvance,
            visual_hidden: *visual_hidden,
            asset_mode,
            context,
        }),
        ContentBlock::Diagram { language, source } => render_diagram(*language, source),
        ContentBlock::Chart {
            format: _,
            spec,
            data,
        } => render_chart(spec, data.as_ref(), asset_mode),
        ContentBlock::Steps { pdf_policy, steps } => {
            render_steps(steps, *pdf_policy, asset_mode.step_state(), context)
        }
        ContentBlock::Layout {
            kind,
            values,
            regions,
        } => render_layout(*kind, values, regions, asset_mode, context),
        ContentBlock::SpeakerNotes { markdown } => format!(
            "            <aside class=\"debug-block zpres-block debug-notes zpres-block-speaker-notes zpres-speaker-notes-source\" data-block-type=\"speaker-notes\" hidden><span class=\"debug-block-label zpres-block-label\">speaker notes</span><div class=\"debug-notes-body zpres-speaker-notes-body\">{}</div></aside>\n",
            render_speaker_notes_body(markdown)
        ),
        ContentBlock::HtmlOnly { html } => format!(
            "            <div class=\"debug-block zpres-block debug-html-only zpres-block-html-only\" data-block-type=\"html-only\"><span class=\"debug-block-label zpres-block-label\">html-only</span><div class=\"debug-html-frame zpres-html-frame\">{}</div></div>\n",
            html
        ),
        ContentBlock::UnsupportedDirective { name, body } => format!(
            "            <div class=\"debug-block zpres-block debug-placeholder zpres-block-unsupported\" data-block-type=\"unsupported-directive\"><span class=\"debug-block-label zpres-block-label\">unsupported directive: {}</span><pre>{}</pre></div>\n",
            escape_html(name),
            escape_html(body)
        ),
    }
}

fn render_speaker_notes_panel() -> String {
    "  <aside id=\"zpres-speaker-notes-panel\" class=\"zpres-speaker-notes-panel\" aria-label=\"speaker notes\" hidden>\n    <strong>speaker notes</strong>\n    <div class=\"zpres-speaker-notes-panel-body\" data-speaker-notes-content>No speaker notes.</div>\n  </aside>\n".to_string()
}

fn render_table(
    headers: &[String],
    alignments: &[TableAlignment],
    rows: &[Vec<String>],
    context: &RenderContext,
) -> String {
    let mut html = String::new();
    html.push_str("            <div class=\"debug-block zpres-block debug-table zpres-block-table\" data-block-type=\"table\" data-zpres-type-role=\"technical\" data-zpres-content-role=\"technical\"><span class=\"debug-block-label zpres-block-label\">table</span><table><thead data-table-role=\"header\"><tr>");
    for (index, header) in headers.iter().enumerate() {
        html.push_str(&format!(
            "<th scope=\"col\" data-align=\"{}\">{}</th>",
            table_alignment_attr(alignments.get(index).copied()),
            render_table_cell(header, context)
        ));
    }
    html.push_str("</tr></thead><tbody data-table-role=\"body\">");
    for row in rows {
        html.push_str("<tr>");
        for (index, cell) in row.iter().enumerate() {
            html.push_str(&format!(
                "<td data-align=\"{}\">{}</td>",
                table_alignment_attr(alignments.get(index).copied()),
                render_table_cell(cell, context)
            ));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table></div>\n");
    html
}

fn render_table_cell(cell: &str, context: &RenderContext) -> String {
    let trimmed = cell.trim();
    if let Some(strong) = trimmed
        .strip_prefix("**")
        .and_then(|value| value.strip_suffix("**"))
    {
        format!(
            "<strong>{}</strong>",
            render_paragraph(strong, &inline_math_segments(strong), context)
        )
    } else {
        render_paragraph(cell, &inline_math_segments(cell), context)
    }
}

fn render_quote(markdown: &str, inline_math: &[String], context: &RenderContext) -> String {
    format!(
        "            <figure class=\"debug-block zpres-block debug-quote zpres-block-quote\" data-block-type=\"quote\"><span class=\"debug-block-label zpres-block-label\">quote</span><blockquote><p>{}</p></blockquote></figure>\n",
        render_paragraph(markdown, inline_math, context)
    )
}

fn render_fit_text(markdown: &str, inline_math: &[String], context: &RenderContext) -> String {
    format!(
        "            <div class=\"debug-block zpres-block debug-fit-text zpres-block-fit-text\" data-block-type=\"fit-text\" data-zpres-type-role=\"display\"><span class=\"debug-block-label zpres-block-label\">fit text</span><p>{}</p></div>\n",
        render_paragraph(markdown, inline_math, context)
    )
}

fn render_callout(
    kind: CalloutKind,
    title: Option<&str>,
    markdown: &str,
    inline_math: &[String],
    context: &RenderContext,
) -> String {
    let kind_name = callout_kind_name(kind);
    let title = title.unwrap_or_else(|| callout_kind_title(kind));
    format!(
        "            <aside class=\"debug-block zpres-block debug-callout zpres-block-callout\" data-block-type=\"callout\" data-callout-kind=\"{}\"><span class=\"debug-block-label zpres-block-label\">callout</span><strong class=\"zpres-callout-title\">{}</strong><p>{}</p></aside>\n",
        kind_name,
        render_paragraph(title, &inline_math_segments(title), context),
        render_paragraph(markdown, inline_math, context)
    )
}

fn render_list(
    ordered: bool,
    reveal: bool,
    reveal_skip_first: bool,
    items: &[ListItem],
    is_static_print: bool,
    context: &RenderContext,
) -> String {
    let tag = if ordered { "ol" } else { "ul" };
    let kind = if ordered { "ordered" } else { "unordered" };
    let reveal_attr = reveal.then_some(" data-list-reveal=\"fragments\"");
    let mut html = format!(
        "            <div class=\"debug-block zpres-block debug-list zpres-block-list\" data-block-type=\"list\" data-list-kind=\"{kind}\"{}><span class=\"debug-block-label zpres-block-label\">{} list</span><{tag}>",
        reveal_attr.unwrap_or_default(),
        if reveal { "fragmented" } else { kind }
    );
    for (index, item) in items.iter().enumerate() {
        let fragment_index = if reveal_skip_first {
            index.checked_sub(1)
        } else {
            Some(index)
        };
        let fragment_attrs = if reveal && let Some(fragment_index) = fragment_index {
            let fragment_class = if is_static_print {
                ""
            } else {
                " class=\"fragment\""
            };
            format!(
                "{fragment_class} data-step-index=\"{}\" data-fragment-index=\"{}\"",
                fragment_index + 1,
                fragment_index + 1,
            )
        } else {
            String::new()
        };
        html.push_str(&format!(
            "<li{}>{}</li>",
            fragment_attrs,
            render_paragraph(&item.markdown, &item.inline_math, context)
        ));
    }
    html.push_str(&format!("</{tag}></div>\n"));
    html
}

fn render_footnotes(notes: &[Footnote], context: &RenderContext) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut html = "            <aside class=\"debug-block zpres-block zpres-block-footnotes\" data-block-type=\"footnotes\"><span class=\"debug-block-label zpres-block-label\">footnotes</span><ol class=\"zpres-footnote-list\">\n".to_string();
    for note in notes {
        html.push_str(&format!(
            "              <li id=\"{}\" class=\"zpres-footnote\" data-footnote-label=\"{}\" value=\"{}\">{}</li>\n",
            escape_attr(&context.footnote_target(&note.label)),
            escape_attr(&note.label),
            note.number,
            render_paragraph(&note.markdown, &note.inline_math, context)
        ));
    }
    html.push_str("            </ol></aside>\n");
    html
}

fn callout_kind_name(kind: CalloutKind) -> &'static str {
    kind.as_str()
}

fn callout_kind_title(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note => "Note",
        CalloutKind::Tip => "Tip",
        CalloutKind::Important => "Important",
        CalloutKind::Warning => "Warning",
        CalloutKind::Caution => "Caution",
    }
}

fn table_alignment_attr(alignment: Option<TableAlignment>) -> &'static str {
    match alignment.unwrap_or(TableAlignment::Default) {
        TableAlignment::Default => "default",
        TableAlignment::Left => "left",
        TableAlignment::Center => "center",
        TableAlignment::Right => "right",
    }
}

fn render_figure(
    src: &str,
    static_src: Option<&str>,
    alt: &str,
    caption: &Option<String>,
    options: &FigureOptions,
    asset_mode: AssetRenderMode<'_>,
    context: &RenderContext,
) -> String {
    let src = static_export_image_src(src, static_src, asset_mode);
    let rendered_src = render_asset_src(src, asset_mode);
    let image_loading_attr = image_loading_attr(asset_mode);
    let caption_html = caption.as_ref().map_or_else(String::new, |caption| {
        format!(
            "<figcaption data-zpres-type-role=\"micro\">{}</figcaption>",
            render_paragraph(caption, &inline_math_segments(caption), context)
        )
    });
    let style_attr = figure_style_attr(options);
    let fit_attr = options.fit.map_or_else(String::new, |fit| {
        format!(" data-figure-fit=\"{}\"", figure_fit_name(fit))
    });
    let align_attr = options.align.map_or_else(String::new, |align| {
        format!(" data-figure-align=\"{}\"", figure_align_name(align))
    });
    let radius_attr = options.radius.as_ref().map_or_else(String::new, |radius| {
        format!(" data-figure-radius=\"{}\"", escape_attr(radius))
    });
    let treatment_attr = figure_has_treatment(options).then_some(" data-figure-treatment=\"true\"");
    format!(
        "            <figure class=\"debug-block zpres-block debug-figure zpres-block-figure\" data-block-type=\"figure\" data-zpres-content-role=\"evidence\"{}{}{}{}{}><span class=\"debug-block-label zpres-block-label\">figure</span><img src=\"{}\" alt=\"{}\"{}>{}</figure>\n",
        fit_attr,
        align_attr,
        radius_attr,
        treatment_attr.unwrap_or_default(),
        style_attr,
        escape_attr(&rendered_src),
        escape_attr(alt),
        image_loading_attr,
        caption_html
    )
}

fn render_gallery(
    items: &[GalleryItem],
    columns: Option<u8>,
    asset_mode: AssetRenderMode<'_>,
    context: &RenderContext,
) -> String {
    let resolved_columns = columns.unwrap_or_else(|| gallery_auto_columns(items.len()));
    let columns_attr = format!(" data-gallery-columns=\"{resolved_columns}\"");
    let style_attr = format!(" style=\"--zpres-gallery-columns: {resolved_columns};\"");
    let mut html = format!(
        "            <figure class=\"debug-block zpres-block debug-gallery zpres-block-gallery\" data-block-type=\"gallery\" data-zpres-content-role=\"evidence\" data-gallery-count=\"{}\"{}{}><span class=\"debug-block-label zpres-block-label\">gallery</span><div class=\"zpres-gallery-items\">\n",
        items.len(),
        columns_attr,
        style_attr
    );
    for item in items {
        let src = static_export_image_src(&item.src, item.static_src.as_deref(), asset_mode);
        let rendered_src = render_asset_src(src, asset_mode);
        let image_loading_attr = image_loading_attr(asset_mode);
        let fit_attr = item.options.fit.map_or_else(String::new, |fit| {
            format!(" data-figure-fit=\"{}\"", figure_fit_name(fit))
        });
        let align_attr = item.options.align.map_or_else(String::new, |align| {
            format!(" data-figure-align=\"{}\"", figure_align_name(align))
        });
        let radius_attr = item
            .options
            .radius
            .as_ref()
            .map_or_else(String::new, |radius| {
                format!(" data-figure-radius=\"{}\"", escape_attr(radius))
            });
        let treatment_attr =
            figure_has_treatment(&item.options).then_some(" data-figure-treatment=\"true\"");
        let style_attr = figure_style_attr(&item.options);
        let caption_html = item.caption.as_ref().map_or_else(String::new, |caption| {
            format!(
                "<figcaption data-zpres-type-role=\"micro\">{}</figcaption>",
                render_paragraph(caption, &inline_math_segments(caption), context)
            )
        });
        html.push_str(&format!(
            "              <figure class=\"zpres-gallery-item\"{}{}{}{}{}><img src=\"{}\" alt=\"{}\"{}>{}</figure>\n",
            fit_attr,
            align_attr,
            radius_attr,
            treatment_attr.unwrap_or_default(),
            style_attr,
            escape_attr(&rendered_src),
            escape_attr(&item.alt),
            image_loading_attr,
            caption_html
        ));
    }
    html.push_str("            </div></figure>\n");
    html
}

fn gallery_auto_columns(count: usize) -> u8 {
    match count {
        0 | 1 => 1,
        2 => 2,
        _ => 3,
    }
}

fn image_loading_attr(asset_mode: AssetRenderMode<'_>) -> &'static str {
    match asset_mode {
        AssetRenderMode::Bundle { .. } => " loading=\"lazy\" decoding=\"async\"",
        AssetRenderMode::Print { .. } => "",
    }
}

fn figure_style_attr(options: &FigureOptions) -> String {
    if options.is_default() {
        return String::new();
    }
    let mut declarations = Vec::new();
    if let Some(width) = &options.width {
        declarations.push(format!("--zpres-figure-width: {};", css_size(width)));
    }
    if let Some(height) = &options.height {
        declarations.push(format!("--zpres-figure-height: {};", css_size(height)));
    }
    if let Some(fit) = options.fit {
        declarations.push(format!("--zpres-figure-fit: {};", figure_fit_css(fit)));
    }
    if let Some(align) = options.align {
        declarations.push(format!(
            "--zpres-figure-align: {};",
            figure_align_css(align)
        ));
    }
    if let Some(dim) = options.dim {
        let dim = dim.min(100);
        declarations.push(format!(
            "--zpres-figure-dim: {:.2};",
            f32::from(dim) / 100.0
        ));
        declarations.push(format!(
            "--zpres-figure-brightness: {:.2};",
            f32::from(100 - dim) / 100.0
        ));
    }
    if let Some(grayscale) = options.grayscale {
        declarations.push(format!(
            "--zpres-figure-gray: {:.2};",
            f32::from(grayscale.min(100)) / 100.0
        ));
    }
    if let Some(saturate) = options.saturate {
        declarations.push(format!(
            "--zpres-figure-saturate: {:.2};",
            f32::from(saturate.min(100)) / 100.0
        ));
    }
    if let Some(blur) = options.blur {
        declarations.push(format!("--zpres-figure-blur: {}px;", blur.min(24)));
    }
    if let Some(radius) = &options.radius {
        declarations.push(format!("--zpres-figure-radius: {};", css_radius(radius)));
    }
    format!(" style=\"{}\"", escape_attr(&declarations.join(" ")))
}

fn figure_has_treatment(options: &FigureOptions) -> bool {
    options.dim.is_some()
        || options.grayscale.is_some()
        || options.saturate.is_some()
        || options.blur.is_some()
}

fn figure_fit_name(fit: FigureFit) -> &'static str {
    match fit {
        FigureFit::Contain => "contain",
        FigureFit::Cover => "cover",
        FigureFit::Fill => "fill",
    }
}

fn figure_fit_css(fit: FigureFit) -> &'static str {
    match fit {
        FigureFit::Contain => "contain",
        FigureFit::Cover => "cover",
        FigureFit::Fill => "fill",
    }
}

fn figure_align_name(align: FigureAlign) -> &'static str {
    match align {
        FigureAlign::Start => "start",
        FigureAlign::Center => "center",
        FigureAlign::End => "end",
        FigureAlign::Stretch => "stretch",
    }
}

fn figure_align_css(align: FigureAlign) -> &'static str {
    match align {
        FigureAlign::Start => "start",
        FigureAlign::Center => "center",
        FigureAlign::End => "end",
        FigureAlign::Stretch => "stretch",
    }
}

fn css_size(value: &str) -> String {
    value.trim().to_string()
}

fn css_radius(value: &str) -> String {
    let value = value.trim();
    if value
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
    {
        format!("{value}px")
    } else {
        value.to_string()
    }
}

struct MediaRender<'a> {
    kind: MediaKind,
    src: &'a str,
    title: Option<&'a str>,
    caption: Option<&'a str>,
    poster: Option<&'a str>,
    alt: &'a str,
    start_time: Option<u32>,
    options: &'a FigureOptions,
    autoplay: bool,
    controls: bool,
    loop_playback: bool,
    muted: bool,
    autoadvance: bool,
    visual_hidden: bool,
    asset_mode: AssetRenderMode<'a>,
    context: &'a RenderContext,
}

fn render_media(media: MediaRender<'_>) -> String {
    match media.asset_mode {
        AssetRenderMode::Print { .. } => render_print_media(media),
        AssetRenderMode::Bundle { .. } => render_html_media(media),
    }
}

fn render_html_media(media: MediaRender<'_>) -> String {
    let src = media_src_with_start_time(
        &render_asset_src(media.src, media.asset_mode),
        media.kind,
        media.start_time,
    );
    let poster = media
        .poster
        .map(|poster| render_asset_src(poster, media.asset_mode));
    let caption_html = media.caption.map_or_else(String::new, |caption| {
        format!(
            "<figcaption data-zpres-type-role=\"micro\">{}</figcaption>",
            render_paragraph(caption, &inline_math_segments(caption), media.context)
        )
    });
    let title_attr = media.title.map_or_else(String::new, |title| {
        format!(" title=\"{}\"", escape_attr(title))
    });
    let media_html = match media.kind {
        MediaKind::Video => format!(
            "<video src=\"{}\"{}{}{}{}{}{} playsinline preload=\"metadata\" aria-label=\"{}\"></video>",
            escape_attr(&src),
            poster.as_ref().map_or_else(String::new, |poster| format!(
                " poster=\"{}\"",
                escape_attr(poster)
            )),
            title_attr,
            bool_attr("controls", media.controls),
            bool_attr("autoplay", media.autoplay),
            bool_attr("loop", media.loop_playback),
            bool_attr("muted", media.muted),
            escape_attr(media.alt)
        ),
        MediaKind::Audio => format!(
            "<audio src=\"{}\"{}{}{}{}{} preload=\"metadata\" aria-label=\"{}\"></audio>",
            escape_attr(&src),
            title_attr,
            bool_attr("controls", media.controls),
            bool_attr("autoplay", media.autoplay),
            bool_attr("loop", media.loop_playback),
            bool_attr("muted", media.muted),
            escape_attr(media.alt)
        ),
        MediaKind::Iframe => format!(
            "<iframe src=\"{}\"{} loading=\"lazy\" referrerpolicy=\"no-referrer\" allowfullscreen></iframe>",
            escape_attr(&src),
            title_attr
        ),
    };
    let start_attr = media.start_time.map_or_else(String::new, |seconds| {
        format!(" data-media-start=\"{seconds}\"")
    });
    let style_attr = media_style_attr(media.options);
    let fit_attr = media.options.fit.map_or_else(String::new, |fit| {
        format!(" data-media-fit=\"{}\"", figure_fit_name(fit))
    });
    let align_attr = media.options.align.map_or_else(String::new, |align| {
        format!(" data-media-align=\"{}\"", figure_align_name(align))
    });
    let hidden_attr = if media.visual_hidden {
        " data-media-hidden=\"true\""
    } else {
        ""
    };
    let autoadvance_attr = if media.autoadvance {
        " data-media-autoadvance=\"true\""
    } else {
        ""
    };
    format!(
        "            <figure class=\"debug-block zpres-block debug-media zpres-block-media\" data-block-type=\"media\" data-zpres-content-role=\"evidence\" data-media-kind=\"{}\"{}{}{}{}{}{}><span class=\"debug-block-label zpres-block-label\">{}</span>{}{}</figure>\n",
        media_kind_name(media.kind),
        start_attr,
        fit_attr,
        align_attr,
        hidden_attr,
        autoadvance_attr,
        style_attr,
        media_kind_name(media.kind),
        media_html,
        caption_html
    )
}

fn render_print_media(media: MediaRender<'_>) -> String {
    let caption = media.caption.or(media.title).unwrap_or(media.src);
    let body = if let Some(poster) = media.poster {
        let poster = render_asset_src(poster, media.asset_mode);
        format!(
            "<img src=\"{}\" alt=\"{}\"><figcaption data-zpres-type-role=\"micro\">{}</figcaption>",
            escape_attr(&poster),
            escape_attr(media.alt),
            if media.context.footnote_scope.is_some() && media.caption.is_some() {
                render_paragraph(caption, &inline_math_segments(caption), media.context)
            } else {
                escape_html(caption)
            }
        )
    } else {
        format!(
            "<div class=\"debug-media-fallback zpres-media-fallback\"><strong>{}</strong><span>{}</span></div><figcaption data-zpres-type-role=\"micro\">{}</figcaption>",
            media_kind_name(media.kind),
            escape_html(media.title.unwrap_or(media.src)),
            if media.context.footnote_scope.is_some() && media.caption.is_some() {
                render_paragraph(caption, &inline_math_segments(caption), media.context)
            } else {
                escape_html(caption)
            }
        )
    };
    let start_attr = media.start_time.map_or_else(String::new, |seconds| {
        format!(" data-media-start=\"{seconds}\"")
    });
    let style_attr = media_style_attr(media.options);
    let fit_attr = media.options.fit.map_or_else(String::new, |fit| {
        format!(" data-media-fit=\"{}\"", figure_fit_name(fit))
    });
    let align_attr = media.options.align.map_or_else(String::new, |align| {
        format!(" data-media-align=\"{}\"", figure_align_name(align))
    });
    let hidden_attr = if media.visual_hidden {
        " data-media-hidden=\"true\""
    } else {
        ""
    };
    let autoadvance_attr = if media.autoadvance {
        " data-media-autoadvance=\"true\""
    } else {
        ""
    };
    format!(
        "            <figure class=\"debug-block zpres-block debug-media zpres-block-media\" data-block-type=\"media\" data-zpres-content-role=\"evidence\" data-media-kind=\"{}\"{}{}{}{}{}{}><span class=\"debug-block-label zpres-block-label\">{} media</span>{}</figure>\n",
        media_kind_name(media.kind),
        start_attr,
        fit_attr,
        align_attr,
        hidden_attr,
        autoadvance_attr,
        style_attr,
        media_kind_name(media.kind),
        body
    )
}

fn media_style_attr(options: &FigureOptions) -> String {
    if options.is_default() {
        return String::new();
    }
    let mut declarations = Vec::new();
    if let Some(width) = &options.width {
        declarations.push(format!("--zpres-media-width: {};", css_size(width)));
    }
    if let Some(height) = &options.height {
        declarations.push(format!("--zpres-media-height: {};", css_size(height)));
    }
    if let Some(fit) = options.fit {
        declarations.push(format!("--zpres-media-fit: {};", figure_fit_css(fit)));
    }
    if let Some(align) = options.align {
        declarations.push(format!("--zpres-media-align: {};", figure_align_css(align)));
    }
    format!(" style=\"{}\"", escape_attr(&declarations.join(" ")))
}

fn media_src_with_start_time(src: &str, kind: MediaKind, start_time: Option<u32>) -> String {
    let Some(seconds) = start_time else {
        return src.to_string();
    };
    if matches!(kind, MediaKind::Iframe) {
        if src.starts_with("https://www.youtube.com/embed/") {
            let separator = if src.contains('?') { '&' } else { '?' };
            return format!("{src}{separator}start={seconds}");
        }
        return src.to_string();
    }
    if src.contains('#') {
        format!("{src}&t={seconds}")
    } else {
        format!("{src}#t={seconds}")
    }
}

fn bool_attr(name: &str, enabled: bool) -> String {
    if enabled {
        format!(" {name}")
    } else {
        String::new()
    }
}

fn media_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Iframe => "iframe",
    }
}

fn render_diagram(language: DiagramLanguage, source: &str) -> String {
    match language {
        DiagramLanguage::Mermaid => {
            let svg = parse_mermaid_flowchart(source)
                .map(|flowchart| {
                    render_mermaid_flowchart_svg(&flowchart)
                })
                .unwrap_or_else(|| {
                    "<div class=\"debug-diagram-unresolved zpres-diagram-unresolved\" data-zpres-unresolved=\"diagram\">diagram could not be rendered</div>".to_string()
                });
            format!(
                "            <div class=\"debug-block zpres-block debug-diagram zpres-block-diagram\" data-block-type=\"diagram\" data-zpres-type-role=\"technical\" data-zpres-content-role=\"evidence\" data-diagram-language=\"mermaid\"><span class=\"debug-block-label zpres-block-label\">mermaid diagram</span>{}</div>\n",
                svg
            )
        }
    }
}

const MERMAID_FONT_SIZE: f32 = 24.0;
const MERMAID_LINE_HEIGHT: f32 = 32.0;
const MERMAID_LAYOUT_MARGIN: f32 = 32.0;
const MERMAID_NODE_GAP: f32 = 48.0;
const MERMAID_HORIZONTAL_LAYER_GAP: f32 = 96.0;
const MERMAID_VERTICAL_LAYER_GAP: f32 = 96.0;
const MERMAID_LABEL_CLEARANCE: f32 = 14.0;

fn render_mermaid_flowchart_svg(flowchart: &MermaidFlowchart) -> String {
    let nodes = mermaid_flowchart_nodes(flowchart);
    let accessible_title = format!(
        "Mermaid flowchart, {}",
        mermaid_direction_description(flowchart.direction)
    );
    let accessible_description = mermaid_accessible_description(flowchart, &nodes);
    let horizontal = matches!(
        flowchart.direction,
        MermaidDirection::LeftRight | MermaidDirection::RightLeft
    );
    let unwrapped_node_layouts = mermaid_node_layouts(&nodes, false);
    let unwrapped_edge_layouts = mermaid_edge_layouts(flowchart, false);
    let unwrapped_layout = mermaid_positioned_layout(
        flowchart,
        &nodes,
        &unwrapped_node_layouts,
        &unwrapped_edge_layouts,
    );
    let wrap_labels = if horizontal {
        unwrapped_layout.width > 1_136.0
    } else {
        unwrapped_layout.height > 640.0
    };
    let node_layouts = if wrap_labels {
        mermaid_node_layouts(&nodes, true)
    } else {
        unwrapped_node_layouts
    };
    let edge_layouts = if wrap_labels {
        mermaid_edge_layouts(flowchart, true)
    } else {
        unwrapped_edge_layouts
    };
    let layout = mermaid_positioned_layout(flowchart, &nodes, &node_layouts, &edge_layouts);
    let width = layout.width;
    let height = layout.height;
    let mut svg = format!(
        "<svg class=\"debug-diagram-svg zpres-diagram-svg\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\"><title>{}</title><desc>{}</desc><rect x=\"0\" y=\"0\" width=\"{width:.0}\" height=\"{height:.0}\" rx=\"10\" class=\"zpres-diagram-surface\"/>",
        escape_html(&accessible_title),
        escape_html(&accessible_description)
    );
    for (index, edge) in flowchart.edges.iter().enumerate() {
        let Some(geometry) = layout.edges.get(index) else {
            continue;
        };
        let path = geometry
            .points
            .iter()
            .enumerate()
            .map(|(point_index, (x, y))| {
                format!("{} {x:.1} {y:.1}", if point_index == 0 { "M" } else { "L" })
            })
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            "<path d=\"{path}\" fill=\"none\" class=\"zpres-diagram-edge\" data-zpres-mermaid-edge=\"{}:{}\"/>",
            escape_attr(&edge.from.id),
            escape_attr(&edge.to.id)
        ));
        if let Some(arrow) = mermaid_arrow_path(&geometry.points) {
            svg.push_str(&format!(
                "<path d=\"{arrow}\" class=\"zpres-diagram-arrow\" data-zpres-mermaid-arrow=\"{}:{}\" aria-hidden=\"true\"/>",
                escape_attr(&edge.from.id),
                escape_attr(&edge.to.id)
            ));
        }
        if let (Some(label_layout), Some((x, y))) =
            (edge_layouts[index].as_ref(), geometry.label_center)
        {
            svg.push_str(&render_mermaid_text(
                x,
                y,
                "zpres-diagram-edge-label",
                label_layout,
            ));
        }
    }
    for node in nodes {
        let Some((x, y)) = layout.positions.get(&node.id).copied() else {
            continue;
        };
        let Some(text_layout) = node_layouts.get(&node.id) else {
            continue;
        };
        svg.push_str(&format!(
            "<g class=\"zpres-diagram-node\" data-zpres-mermaid-node=\"{}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\"/>{}</g>",
            escape_attr(&node.id),
            x - text_layout.box_width / 2.0,
            y - text_layout.box_height / 2.0,
            text_layout.box_width,
            text_layout.box_height,
            render_mermaid_text(x, y, "", text_layout)
        ));
    }
    svg.push_str("</svg>");
    svg
}

fn mermaid_arrow_path(points: &[(f32, f32)]) -> Option<String> {
    let [(from_x, from_y), (to_x, to_y)] = points.get(points.len().checked_sub(2)?..)? else {
        return None;
    };
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        return None;
    }
    let unit_x = dx / length;
    let unit_y = dy / length;
    let normal_x = -unit_y;
    let normal_y = unit_x;
    let tip_x = to_x + unit_x;
    let tip_y = to_y + unit_y;
    let base_x = to_x - unit_x * 8.0;
    let base_y = to_y - unit_y * 8.0;
    let first_x = base_x + normal_x * 3.0;
    let first_y = base_y + normal_y * 3.0;
    let second_x = base_x - normal_x * 3.0;
    let second_y = base_y - normal_y * 3.0;
    Some(format!(
        "M{first_x:.1} {first_y:.1} L{second_x:.1} {second_y:.1} L{tip_x:.1} {tip_y:.1} z"
    ))
}

fn mermaid_direction_description(direction: MermaidDirection) -> &'static str {
    match direction {
        MermaidDirection::TopDown => "top to bottom",
        MermaidDirection::BottomTop => "bottom to top",
        MermaidDirection::LeftRight => "left to right",
        MermaidDirection::RightLeft => "right to left",
    }
}

fn mermaid_accessible_description(
    flowchart: &MermaidFlowchart,
    nodes: &[crate::deck::MermaidNode],
) -> String {
    let nodes = nodes
        .iter()
        .map(|node| format!("Node {}: ‘{}’.", node.id, node.label))
        .collect::<Vec<_>>()
        .join(" ");
    let edges = flowchart
        .edges
        .iter()
        .map(|edge| match edge.label.as_deref() {
            Some(label) => format!(
                "Edge {} to {}, label: ‘{}’.",
                edge.from.id, edge.to.id, label
            ),
            None => format!("Edge {} to {}.", edge.from.id, edge.to.id),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("Nodes in source order: {nodes} Edges in source order: {edges}")
}

#[derive(Clone, Debug)]
struct MermaidTextLayout {
    authored: String,
    lines: Vec<String>,
    text_width: f32,
    box_width: f32,
    box_height: f32,
}

fn mermaid_text_layout(label: &str, wrap: bool, node: bool) -> MermaidTextLayout {
    let lines = if wrap {
        wrap_mermaid_label(label)
    } else {
        vec![label.to_string()]
    };
    let text_width = lines
        .iter()
        .map(|line| estimated_mermaid_text_width(line))
        .fold(0.0f32, f32::max);
    MermaidTextLayout {
        authored: label.to_string(),
        box_width: if node {
            128.0f32.max(text_width + 24.0)
        } else {
            text_width
        },
        box_height: if node {
            48.0f32.max(lines.len() as f32 * MERMAID_LINE_HEIGHT + 24.0)
        } else {
            lines.len() as f32 * MERMAID_LINE_HEIGHT
        },
        lines,
        text_width,
    }
}

fn estimated_mermaid_text_width(label: &str) -> f32 {
    label
        .chars()
        .map(|character| {
            let em = match character {
                '\u{200d}' | '\u{fe0e}' | '\u{fe0f}' => 0.0,
                character if character.is_whitespace() => 0.5,
                'W' | 'M' | '@' | '%' => 0.96,
                'w' | 'm' | 'Q' | 'O' | 'G' => 0.84,
                'I' | 'i' | 'l' | '!' | '|' | ':' | ';' | '.' | ',' | '\'' => 0.42,
                'A'..='Z' => 0.76,
                '0'..='9' => 0.66,
                character if character.is_ascii() => 0.64,
                character if ('\u{0300}'..='\u{036f}').contains(&character) => 0.0,
                _ => 1.05,
            };
            em * MERMAID_FONT_SIZE
        })
        .sum()
}

fn wrap_mermaid_label(label: &str) -> Vec<String> {
    if label.trim().is_empty() {
        return vec![String::new()];
    }
    if estimated_mermaid_text_width(label) <= 10.0 * MERMAID_FONT_SIZE * 0.68 {
        return vec![label.to_string()];
    }

    let mut candidates = Vec::new();
    let mut in_whitespace = false;
    for (index, character) in label.char_indices() {
        if character.is_whitespace() {
            if !in_whitespace {
                candidates.push(index);
            }
            in_whitespace = true;
        } else {
            in_whitespace = false;
        }
    }
    let Some((_, left, right)) = candidates
        .into_iter()
        .filter_map(|split| {
            let left = label[..split].trim_end();
            let right = label[split..].trim_start();
            (!left.is_empty() && !right.is_empty()).then(|| {
                (
                    estimated_mermaid_text_width(left).max(estimated_mermaid_text_width(right)),
                    left,
                    right,
                )
            })
        })
        .min_by(|first, second| first.0.total_cmp(&second.0))
    else {
        // Never split an unspaced word: doing so by scalar value can break a grapheme cluster.
        return vec![label.to_string()];
    };
    vec![left.to_string(), right.to_string()]
}

fn mermaid_node_layouts(
    nodes: &[crate::deck::MermaidNode],
    wrap: bool,
) -> BTreeMap<String, MermaidTextLayout> {
    nodes
        .iter()
        .map(|node| {
            (
                node.id.clone(),
                mermaid_text_layout(&node.label, wrap, true),
            )
        })
        .collect()
}

fn mermaid_edge_layouts(
    flowchart: &MermaidFlowchart,
    wrap: bool,
) -> Vec<Option<MermaidTextLayout>> {
    flowchart
        .edges
        .iter()
        .map(|edge| {
            edge.label
                .as_deref()
                .map(|label| mermaid_text_layout(label, wrap, false))
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct MermaidRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl MermaidRect {
    fn from_center(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            left: x - width / 2.0,
            top: y - height / 2.0,
            right: x + width / 2.0,
            bottom: y + height / 2.0,
        }
    }

    fn intersects(self, other: Self, clearance: f32) -> bool {
        self.left < other.right + clearance
            && self.right > other.left - clearance
            && self.top < other.bottom + clearance
            && self.bottom > other.top - clearance
    }

    fn include(&mut self, other: Self) {
        self.left = self.left.min(other.left);
        self.top = self.top.min(other.top);
        self.right = self.right.max(other.right);
        self.bottom = self.bottom.max(other.bottom);
    }
}

#[derive(Clone, Debug)]
struct MermaidEdgeGeometry {
    points: Vec<(f32, f32)>,
    label_center: Option<(f32, f32)>,
}

#[derive(Debug)]
struct MermaidPositionedLayout {
    positions: BTreeMap<String, (f32, f32)>,
    edges: Vec<MermaidEdgeGeometry>,
    width: f32,
    height: f32,
}

fn mermaid_positioned_layout(
    flowchart: &MermaidFlowchart,
    nodes: &[crate::deck::MermaidNode],
    node_layouts: &BTreeMap<String, MermaidTextLayout>,
    edge_layouts: &[Option<MermaidTextLayout>],
) -> MermaidPositionedLayout {
    let horizontal = matches!(
        flowchart.direction,
        MermaidDirection::LeftRight | MermaidDirection::RightLeft
    );
    let (layers, ranks) = mermaid_node_layers(flowchart, nodes);
    let layer_gaps = mermaid_layer_gaps(flowchart, &ranks, edge_layouts, horizontal, layers.len());
    let mut positions =
        mermaid_initial_node_positions(flowchart.direction, &layers, node_layouts, &layer_gaps);
    let node_rects = mermaid_node_rects(&positions, node_layouts);
    let layer_rects = mermaid_layer_rects(&layers, &node_rects);
    let node_extent = node_rects
        .values()
        .copied()
        .reduce(|mut extent, rect| {
            extent.include(rect);
            extent
        })
        .unwrap_or(MermaidRect {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        });

    let mut edges = Vec::with_capacity(flowchart.edges.len());
    let mut placed_label_rects = Vec::new();
    let mut horizontal_route_cursor = node_extent.top - 48.0;
    let mut vertical_route_cursor = node_extent.right + 48.0;
    for (index, edge) in flowchart.edges.iter().enumerate() {
        let Some(&(from_x, from_y)) = positions.get(&edge.from.id) else {
            continue;
        };
        let Some(&(to_x, to_y)) = positions.get(&edge.to.id) else {
            continue;
        };
        let Some(from_layout) = node_layouts.get(&edge.from.id) else {
            continue;
        };
        let Some(to_layout) = node_layouts.get(&edge.to.id) else {
            continue;
        };
        let from_rank = ranks[&edge.from.id];
        let to_rank = ranks[&edge.to.id];
        let label_layout = edge_layouts.get(index).and_then(Option::as_ref);
        let direct = to_rank == from_rank + 1;
        let geometry = if direct {
            let (x1, y1, x2, y2) = edge_line_points(
                horizontal,
                from_x,
                from_y,
                to_x,
                to_y,
                from_layout.box_width,
                from_layout.box_height,
                to_layout.box_width,
                to_layout.box_height,
            );
            let label_center = label_layout.map(|label| {
                mermaid_clear_label_center(
                    x1,
                    y1,
                    x2,
                    y2,
                    label,
                    horizontal,
                    &node_rects,
                    &placed_label_rects,
                )
            });
            if let (Some(label), Some(center)) = (label_layout, label_center) {
                placed_label_rects.push(MermaidRect::from_center(
                    center.0,
                    center.1,
                    label.text_width,
                    label.box_height,
                ));
            }
            MermaidEdgeGeometry {
                points: vec![(x1, y1), (x2, y2)],
                label_center,
            }
        } else if horizontal {
            let from_layer = layer_rects[from_rank];
            let to_layer = layer_rects[to_rank];
            let left_to_right = flowchart.direction == MermaidDirection::LeftRight;
            let (start, from_escape_x, end, to_escape_x) = if left_to_right {
                (
                    (from_x + from_layout.box_width / 2.0, from_y),
                    from_layer.right + 28.0,
                    (to_x - to_layout.box_width / 2.0, to_y),
                    to_layer.left - 28.0,
                )
            } else {
                (
                    (from_x - from_layout.box_width / 2.0, from_y),
                    from_layer.left - 28.0,
                    (to_x + to_layout.box_width / 2.0, to_y),
                    to_layer.right + 28.0,
                )
            };
            let lane_y = horizontal_route_cursor;
            let label_center = label_layout.map(|label| {
                (
                    (from_escape_x + to_escape_x) / 2.0,
                    lane_y - label.box_height / 2.0 - MERMAID_LABEL_CLEARANCE,
                )
            });
            horizontal_route_cursor = label_center.map_or(lane_y - 48.0, |center| {
                center.1 - label_layout.unwrap().box_height / 2.0 - 48.0
            });
            if let (Some(label), Some(center)) = (label_layout, label_center) {
                placed_label_rects.push(MermaidRect::from_center(
                    center.0,
                    center.1,
                    label.text_width,
                    label.box_height,
                ));
            }
            MermaidEdgeGeometry {
                points: vec![
                    start,
                    (from_escape_x, from_y),
                    (from_escape_x, lane_y),
                    (to_escape_x, lane_y),
                    (to_escape_x, to_y),
                    end,
                ],
                label_center,
            }
        } else {
            let from_layer = layer_rects[from_rank];
            let to_layer = layer_rects[to_rank];
            let top_down = flowchart.direction == MermaidDirection::TopDown;
            let (start, from_escape_y, end, to_escape_y) = if top_down {
                (
                    (from_x, from_y + from_layout.box_height / 2.0),
                    from_layer.bottom + 28.0,
                    (to_x, to_y - to_layout.box_height / 2.0),
                    to_layer.top - 28.0,
                )
            } else {
                (
                    (from_x, from_y - from_layout.box_height / 2.0),
                    from_layer.top - 28.0,
                    (to_x, to_y + to_layout.box_height / 2.0),
                    to_layer.bottom + 28.0,
                )
            };
            let lane_x = vertical_route_cursor;
            let label_center = label_layout.map(|label| {
                (
                    lane_x + label.text_width / 2.0 + MERMAID_LABEL_CLEARANCE,
                    (from_escape_y + to_escape_y) / 2.0,
                )
            });
            vertical_route_cursor = label_center.map_or(lane_x + 48.0, |center| {
                center.0 + label_layout.unwrap().text_width / 2.0 + 48.0
            });
            if let (Some(label), Some(center)) = (label_layout, label_center) {
                placed_label_rects.push(MermaidRect::from_center(
                    center.0,
                    center.1,
                    label.text_width,
                    label.box_height,
                ));
            }
            MermaidEdgeGeometry {
                points: vec![
                    start,
                    (from_x, from_escape_y),
                    (lane_x, from_escape_y),
                    (lane_x, to_escape_y),
                    (to_x, to_escape_y),
                    end,
                ],
                label_center,
            }
        };
        edges.push(geometry);
    }
    mermaid_reposition_edge_labels(&mut edges, edge_layouts, &node_rects, horizontal);

    let mut extent = node_extent;
    for (index, geometry) in edges.iter().enumerate() {
        for &(x, y) in &geometry.points {
            extent.include(MermaidRect::from_center(x, y, 8.0, 8.0));
        }
        if let (Some(label), Some((x, y))) = (
            edge_layouts.get(index).and_then(Option::as_ref),
            geometry.label_center,
        ) {
            extent.include(MermaidRect::from_center(
                x,
                y,
                label.text_width,
                label.box_height,
            ));
        }
    }
    let raw_width = extent.right - extent.left + MERMAID_LAYOUT_MARGIN * 2.0;
    let raw_height = extent.bottom - extent.top + MERMAID_LAYOUT_MARGIN * 2.0;
    let width = raw_width.max(520.0).ceil();
    let height = raw_height
        .max(if horizontal { 220.0 } else { 260.0 })
        .ceil();
    let shift_x = MERMAID_LAYOUT_MARGIN - extent.left + (width - raw_width) / 2.0;
    let shift_y = MERMAID_LAYOUT_MARGIN - extent.top + (height - raw_height) / 2.0;
    for position in positions.values_mut() {
        position.0 += shift_x;
        position.1 += shift_y;
    }
    for geometry in &mut edges {
        for point in &mut geometry.points {
            point.0 += shift_x;
            point.1 += shift_y;
        }
        if let Some(center) = &mut geometry.label_center {
            center.0 += shift_x;
            center.1 += shift_y;
        }
    }
    MermaidPositionedLayout {
        positions,
        edges,
        width,
        height,
    }
}

fn mermaid_reposition_edge_labels(
    edges: &mut [MermaidEdgeGeometry],
    edge_layouts: &[Option<MermaidTextLayout>],
    node_rects: &BTreeMap<String, MermaidRect>,
    horizontal: bool,
) {
    for edge_index in 0..edges.len() {
        let Some(label) = edge_layouts.get(edge_index).and_then(Option::as_ref) else {
            continue;
        };
        let Some(origin) = edges[edge_index].label_center else {
            continue;
        };
        let mut accepted = None;
        for distance in [0.0, 24.0, 48.0, 80.0, 120.0, 168.0, 224.0, 288.0] {
            let offsets = if horizontal {
                [
                    (0.0, -distance),
                    (0.0, distance),
                    (-distance, 0.0),
                    (distance, 0.0),
                ]
            } else {
                [
                    (distance, 0.0),
                    (-distance, 0.0),
                    (0.0, -distance),
                    (0.0, distance),
                ]
            };
            for offset in offsets {
                let candidate = (origin.0 + offset.0, origin.1 + offset.1);
                let candidate_rect = MermaidRect::from_center(
                    candidate.0,
                    candidate.1,
                    label.text_width,
                    label.box_height,
                );
                let collides_with_node = node_rects
                    .values()
                    .any(|rect| candidate_rect.intersects(*rect, 8.0));
                let collides_with_label = edges.iter().enumerate().any(|(other_index, edge)| {
                    if other_index == edge_index {
                        return false;
                    }
                    let Some(other_label) = edge_layouts.get(other_index).and_then(Option::as_ref)
                    else {
                        return false;
                    };
                    edge.label_center.is_some_and(|other_center| {
                        candidate_rect.intersects(
                            MermaidRect::from_center(
                                other_center.0,
                                other_center.1,
                                other_label.text_width,
                                other_label.box_height,
                            ),
                            8.0,
                        )
                    })
                });
                let collides_with_edge = edges.iter().any(|edge| {
                    edge.points.windows(2).any(|segment| {
                        mermaid_segment_intersects_rect(
                            segment[0],
                            segment[1],
                            candidate_rect,
                            MERMAID_LABEL_CLEARANCE,
                        )
                    })
                });
                if !collides_with_node && !collides_with_label && !collides_with_edge {
                    accepted = Some(candidate);
                    break;
                }
            }
            if accepted.is_some() {
                break;
            }
        }
        if let Some(center) = accepted {
            edges[edge_index].label_center = Some(center);
        }
    }
}

fn mermaid_segment_intersects_rect(
    start: (f32, f32),
    end: (f32, f32),
    rect: MermaidRect,
    clearance: f32,
) -> bool {
    let left = rect.left - clearance;
    let right = rect.right + clearance;
    let top = rect.top - clearance;
    let bottom = rect.bottom + clearance;
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let mut minimum = 0.0f32;
    let mut maximum = 1.0f32;
    for (coefficient, distance) in [
        (-dx, start.0 - left),
        (dx, right - start.0),
        (-dy, start.1 - top),
        (dy, bottom - start.1),
    ] {
        if coefficient.abs() < f32::EPSILON {
            if distance < 0.0 {
                return false;
            }
            continue;
        }
        let ratio = distance / coefficient;
        if coefficient < 0.0 {
            minimum = minimum.max(ratio);
        } else {
            maximum = maximum.min(ratio);
        }
        if minimum > maximum {
            return false;
        }
    }
    true
}

fn mermaid_node_layers(
    flowchart: &MermaidFlowchart,
    nodes: &[crate::deck::MermaidNode],
) -> (Vec<Vec<String>>, BTreeMap<String, usize>) {
    let mut outgoing = nodes
        .iter()
        .map(|node| (node.id.clone(), Vec::<String>::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = nodes
        .iter()
        .map(|node| (node.id.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for edge in &flowchart.edges {
        if edge.from.id == edge.to.id {
            continue;
        }
        let successors = outgoing.entry(edge.from.id.clone()).or_default();
        if !successors.contains(&edge.to.id) {
            successors.push(edge.to.id.clone());
            *indegree.entry(edge.to.id.clone()).or_default() += 1;
        }
    }

    let mut queue = nodes
        .iter()
        .filter(|node| indegree[&node.id] == 0)
        .map(|node| node.id.clone())
        .collect::<VecDeque<_>>();
    let mut ranks = nodes
        .iter()
        .map(|node| (node.id.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut processed = BTreeSet::new();
    while let Some(node_id) = queue.pop_front() {
        if !processed.insert(node_id.clone()) {
            continue;
        }
        let next_rank = ranks[&node_id] + 1;
        for successor in outgoing.get(&node_id).into_iter().flatten() {
            let successor_rank = ranks.entry(successor.clone()).or_default();
            *successor_rank = (*successor_rank).max(next_rank);
            let degree = indegree.get_mut(successor).unwrap();
            *degree = degree.saturating_sub(1);
            if *degree == 0 {
                queue.push_back(successor.clone());
            }
        }
    }

    let mut next_cycle_rank = processed
        .iter()
        .filter_map(|node| ranks.get(node))
        .copied()
        .max()
        .unwrap_or(0)
        + usize::from(!processed.is_empty());
    for node in nodes {
        if !processed.contains(&node.id) {
            ranks.insert(node.id.clone(), next_cycle_rank);
            next_cycle_rank += 1;
        }
    }
    let layer_count = ranks.values().copied().max().unwrap_or(0) + 1;
    let mut layers = vec![Vec::new(); layer_count];
    for node in nodes {
        layers[ranks[&node.id]].push(node.id.clone());
    }
    (layers, ranks)
}

fn mermaid_layer_gaps(
    flowchart: &MermaidFlowchart,
    ranks: &BTreeMap<String, usize>,
    edge_layouts: &[Option<MermaidTextLayout>],
    horizontal: bool,
    layer_count: usize,
) -> Vec<f32> {
    let base = if horizontal {
        MERMAID_HORIZONTAL_LAYER_GAP
    } else {
        MERMAID_VERTICAL_LAYER_GAP
    };
    let mut gaps = vec![base; layer_count.saturating_sub(1)];
    for (edge, label) in flowchart.edges.iter().zip(edge_layouts) {
        let Some(label) = label else {
            continue;
        };
        let from_rank = ranks[&edge.from.id];
        let to_rank = ranks[&edge.to.id];
        if from_rank.abs_diff(to_rank) != 1 {
            continue;
        }
        let boundary = from_rank.min(to_rank);
        let required = if horizontal {
            base
        } else {
            label.box_height + MERMAID_LABEL_CLEARANCE * 2.0
        };
        gaps[boundary] = gaps[boundary].max(required);
    }
    gaps
}

fn mermaid_initial_node_positions(
    direction: MermaidDirection,
    layers: &[Vec<String>],
    node_layouts: &BTreeMap<String, MermaidTextLayout>,
    layer_gaps: &[f32],
) -> BTreeMap<String, (f32, f32)> {
    let horizontal = matches!(
        direction,
        MermaidDirection::LeftRight | MermaidDirection::RightLeft
    );
    let primary_sizes = layers
        .iter()
        .map(|layer| {
            layer
                .iter()
                .filter_map(|node| node_layouts.get(node))
                .map(|layout| {
                    if horizontal {
                        layout.box_width
                    } else {
                        layout.box_height
                    }
                })
                .fold(0.0f32, f32::max)
        })
        .collect::<Vec<_>>();
    let mut primary_centers = Vec::with_capacity(layers.len());
    let mut cursor = 0.0;
    for (index, size) in primary_sizes.iter().enumerate() {
        primary_centers.push(cursor + size / 2.0);
        cursor += size + layer_gaps.get(index).copied().unwrap_or_default();
    }
    let total_primary = cursor;

    let reverse = matches!(
        direction,
        MermaidDirection::RightLeft | MermaidDirection::BottomTop
    );
    let mut positions = BTreeMap::new();
    for (layer_index, layer) in layers.iter().enumerate() {
        let cross_size = layer
            .iter()
            .filter_map(|node| node_layouts.get(node))
            .map(|layout| {
                if horizontal {
                    layout.box_height
                } else {
                    layout.box_width
                }
            })
            .sum::<f32>()
            + layer.len().saturating_sub(1) as f32 * MERMAID_NODE_GAP;
        let mut cross_cursor = -cross_size / 2.0;
        for node in layer {
            let layout = &node_layouts[node];
            let node_cross = if horizontal {
                layout.box_height
            } else {
                layout.box_width
            };
            let cross_center = cross_cursor + node_cross / 2.0;
            cross_cursor += node_cross + MERMAID_NODE_GAP;
            let mut primary_center = primary_centers[layer_index];
            if reverse {
                primary_center = total_primary - primary_center;
            }
            positions.insert(
                node.clone(),
                if horizontal {
                    (primary_center, cross_center)
                } else {
                    (cross_center, primary_center)
                },
            );
        }
    }
    positions
}

fn mermaid_node_rects(
    positions: &BTreeMap<String, (f32, f32)>,
    node_layouts: &BTreeMap<String, MermaidTextLayout>,
) -> BTreeMap<String, MermaidRect> {
    positions
        .iter()
        .filter_map(|(node, &(x, y))| {
            node_layouts.get(node).map(|layout| {
                (
                    node.clone(),
                    MermaidRect::from_center(x, y, layout.box_width, layout.box_height),
                )
            })
        })
        .collect()
}

fn mermaid_layer_rects(
    layers: &[Vec<String>],
    node_rects: &BTreeMap<String, MermaidRect>,
) -> Vec<MermaidRect> {
    layers
        .iter()
        .map(|layer| {
            layer
                .iter()
                .filter_map(|node| node_rects.get(node).copied())
                .reduce(|mut extent, rect| {
                    extent.include(rect);
                    extent
                })
                .unwrap_or(MermaidRect {
                    left: 0.0,
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                })
        })
        .collect()
}

fn mermaid_clear_label_center(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    label: &MermaidTextLayout,
    horizontal: bool,
    node_rects: &BTreeMap<String, MermaidRect>,
    placed_label_rects: &[MermaidRect],
) -> (f32, f32) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let length = dx.hypot(dy).max(1.0);
    let mut normal = (-dy / length, dx / length);
    if (horizontal && normal.1 > 0.0) || (!horizontal && normal.0 < 0.0) {
        normal = (-normal.0, -normal.1);
    }
    let projected_half_extent =
        normal.0.abs() * label.text_width / 2.0 + normal.1.abs() * label.box_height / 2.0;
    let midpoint = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    for extra in [0.0, 24.0, 48.0, 80.0, 120.0] {
        for sign in [1.0, -1.0] {
            let distance = projected_half_extent + MERMAID_LABEL_CLEARANCE + extra;
            let candidate = (
                midpoint.0 + normal.0 * distance * sign,
                midpoint.1 + normal.1 * distance * sign,
            );
            let candidate_rect = MermaidRect::from_center(
                candidate.0,
                candidate.1,
                label.text_width,
                label.box_height,
            );
            let node_collision = node_rects
                .values()
                .any(|rect| candidate_rect.intersects(*rect, 8.0));
            let label_collision = placed_label_rects
                .iter()
                .any(|rect| candidate_rect.intersects(*rect, 8.0));
            if !node_collision && !label_collision {
                return candidate;
            }
        }
    }
    (
        midpoint.0 + normal.0 * (projected_half_extent + 160.0),
        midpoint.1 + normal.1 * (projected_half_extent + 160.0),
    )
}

fn render_mermaid_text(
    x: f32,
    center_y: f32,
    class_name: &str,
    layout: &MermaidTextLayout,
) -> String {
    let class_attr = if class_name.is_empty() {
        String::new()
    } else {
        format!(" class=\"{class_name}\"")
    };
    let y = center_y - layout.lines.len().saturating_sub(1) as f32 * MERMAID_LINE_HEIGHT / 2.0
        + MERMAID_FONT_SIZE * 0.34;
    let semantics = format!(
        " aria-hidden=\"true\" data-zpres-authored-label=\"{}\"",
        escape_attr(&layout.authored)
    );
    if layout.lines.len() == 1 {
        return format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\"{class_attr}{semantics} text-anchor=\"middle\">{}</text>",
            escape_html(&layout.lines[0])
        );
    }
    let mut text =
        format!("<text x=\"{x:.1}\" y=\"{y:.1}\"{class_attr}{semantics} text-anchor=\"middle\">");
    for (index, line) in layout.lines.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&format!(
            "<tspan x=\"{x:.1}\" dy=\"{}\">{}</tspan>",
            if index == 0 { 0.0 } else { MERMAID_LINE_HEIGHT },
            escape_html(line)
        ));
    }
    text.push_str("</text>");
    text
}

fn mermaid_flowchart_nodes(flowchart: &MermaidFlowchart) -> Vec<crate::deck::MermaidNode> {
    let mut nodes = Vec::new();
    for edge in &flowchart.edges {
        for node in [&edge.from, &edge.to] {
            if !nodes
                .iter()
                .any(|existing: &crate::deck::MermaidNode| existing.id == node.id)
            {
                nodes.push(node.clone());
            }
        }
    }
    nodes
}

fn edge_line_points(
    horizontal: bool,
    from_x: f32,
    from_y: f32,
    to_x: f32,
    to_y: f32,
    from_width: f32,
    from_height: f32,
    to_width: f32,
    to_height: f32,
) -> (f32, f32, f32, f32) {
    if horizontal {
        let sign = if to_x >= from_x { 1.0 } else { -1.0 };
        (
            from_x + sign * from_width / 2.0,
            from_y,
            to_x - sign * to_width / 2.0,
            to_y,
        )
    } else {
        let sign = if to_y >= from_y { 1.0 } else { -1.0 };
        (
            from_x,
            from_y + sign * from_height / 2.0,
            to_x,
            to_y - sign * to_height / 2.0,
        )
    }
}

fn render_chart(
    spec: &serde_json::Value,
    data: Option<&ChartData>,
    asset_mode: AssetRenderMode<'_>,
) -> String {
    let svg = render_chart_svg(spec, data, asset_mode.deck_root()).unwrap_or_else(|| {
        "<div class=\"debug-chart-unresolved zpres-chart-unresolved\" data-zpres-unresolved=\"chart\">chart could not be rendered</div>".to_string()
    });
    format!(
        "            <div class=\"debug-block zpres-block debug-chart zpres-block-chart\" data-block-type=\"chart\" data-zpres-type-role=\"technical\" data-zpres-content-role=\"evidence\"><span class=\"debug-block-label zpres-block-label\">vega-lite chart</span>{}</div>\n",
        svg
    )
}

fn render_steps(
    steps: &[Step],
    pdf_policy: StepPdfPolicy,
    print_state: Option<PdfStepState>,
    context: &RenderContext,
) -> String {
    let policy = step_pdf_policy_name(pdf_policy);
    let mut html = format!(
        "            <div class=\"debug-block zpres-block debug-steps zpres-block-steps\" data-block-type=\"steps\" data-step-count=\"{}\" data-step-pdf-policy=\"{policy}\"><span class=\"debug-block-label zpres-block-label\">steps</span><ol>",
        steps.len()
    );
    for step in steps {
        let (classes, visible, state) = match print_state {
            Some(PdfStepState::Final) => ("debug-step zpres-step is-visible", true, "complete"),
            Some(PdfStepState::UpTo { step: max_step }) if step.index <= max_step => {
                let state = if step.index == max_step {
                    "active"
                } else {
                    "complete"
                };
                ("debug-step zpres-step is-visible", true, state)
            }
            Some(PdfStepState::UpTo { .. }) => (
                "debug-step zpres-step zpres-print-step-hidden",
                false,
                "future",
            ),
            None => ("debug-step zpres-step fragment", false, "future"),
        };
        let hidden_attr = if visible || print_state.is_none() {
            ""
        } else {
            " aria-hidden=\"true\""
        };
        let state_attribute = if print_state.is_some() {
            "data-zpres-print-step-state"
        } else {
            "data-step-state"
        };
        html.push_str(&format!(
            "<li class=\"{classes}\" data-step-index=\"{}\" data-fragment-index=\"{}\" {state_attribute}=\"{state}\"{}>{}</li>",
            step.index,
            step.index,
            hidden_attr,
            if context.footnote_scope.is_some() { render_paragraph(&step.markdown, &inline_math_segments(&step.markdown), context) } else { escape_html(&step.markdown) }
        ));
    }
    html.push_str("</ol></div>\n");
    html
}

fn step_pdf_policy_name(policy: StepPdfPolicy) -> &'static str {
    match policy {
        StepPdfPolicy::FinalState => "final-state",
        StepPdfPolicy::OnePagePerStep => "one-page-per-step",
    }
}

fn render_code_block(
    language: Option<&str>,
    code: &str,
    reveal: Option<&CodeReveal>,
    print_state: Option<PdfStepState>,
) -> String {
    let language_label = language.map_or_else(String::new, |language| {
        format!(": {}", escape_html(language))
    });
    let reveal_attrs = reveal.map_or_else(String::new, |reveal| {
        format!(" data-code-reveal-steps=\"{}\"", reveal.groups.len())
    });
    let line_number_policy = if reveal.is_some() { "true" } else { "false" };
    format!(
        "            <div class=\"debug-block zpres-block debug-code zpres-block-code\" data-block-type=\"code\" data-zpres-type-role=\"technical\" data-zpres-content-role=\"technical\" data-code-line-numbers=\"{line_number_policy}\"{}><span class=\"debug-block-label zpres-block-label\">code{}</span><pre><code>{}</code></pre></div>\n",
        reveal_attrs,
        language_label,
        render_code_lines(code, language, reveal, print_state)
    )
}

fn render_code_lines(
    code: &str,
    language: Option<&str>,
    reveal: Option<&CodeReveal>,
    print_state: Option<PdfStepState>,
) -> String {
    code.split('\n')
        .enumerate()
        .map(|(offset, line)| {
            let line_number = offset + 1;
            let step_index = reveal_step_for_line(reveal, line_number);
            let (classes, hidden) = match (step_index, print_state) {
                (Some(_), None) => ("debug-code-line zpres-code-line fragment", false),
                (Some(step), Some(PdfStepState::UpTo { step: max_step })) if step > max_step => (
                    "debug-code-line zpres-code-line zpres-print-step-hidden",
                    true,
                ),
                _ => ("debug-code-line zpres-code-line is-visible", false),
            };
            let step_attrs = step_index.map_or_else(String::new, |step| {
                format!(" data-step-index=\"{step}\" data-fragment-index=\"{step}\"")
            });
            let hidden_attrs = if hidden {
                " aria-hidden=\"true\" data-code-line-hidden=\"true\""
            } else {
                ""
            };
            format!(
                "<span class=\"{classes}\" data-line=\"{line_number}\"{step_attrs}{hidden_attrs}><span class=\"debug-code-line-no zpres-code-line-number\">{line_number}</span><span class=\"debug-code-line-text zpres-code-line-text\">{}</span></span>",
                highlight_code(line, language)
            )
        })
        .collect()
}

fn reveal_step_for_line(reveal: Option<&CodeReveal>, line_number: usize) -> Option<usize> {
    let reveal = reveal?;
    reveal.groups.iter().find_map(|group| {
        group
            .ranges
            .iter()
            .any(|range| (range.start..=range.end).contains(&line_number))
            .then_some(group.index)
    })
}

fn render_layout(
    kind: LayoutKind,
    values: &LayoutValues,
    regions: &[LayoutRegion],
    asset_mode: AssetRenderMode<'_>,
    context: &RenderContext,
) -> String {
    let kind_name = layout_kind_name(kind);
    let style = layout_style(kind, values);
    let widths = values
        .widths
        .as_ref()
        .map(|widths| {
            widths
                .iter()
                .map(layout_size_token)
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| "auto".to_string());
    let tracks = values
        .tracks
        .as_ref()
        .map(|tracks| {
            tracks
                .iter()
                .map(layout_size_token)
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| "auto".to_string());
    let safety_attributes = if kind == LayoutKind::Overlay {
        " data-overlay-policy=\"edge-only\" data-overlay-forbidden-boundaries=\"title footer caption safe-area\""
    } else {
        ""
    };
    let derivation_attributes = values.step_pdf_policy.map_or_else(String::new, |policy| {
        let step_count = regions
            .iter()
            .filter(|region| region.derivation_step.is_some())
            .count();
        format!(
            " data-derivation=\"true\" data-step-count=\"{step_count}\" data-step-pdf-policy=\"{}\"",
            step_pdf_policy_name(policy)
        )
    });
    let mut html = format!(
        "            <div class=\"debug-block zpres-block debug-layout zpres-block-layout\" data-block-type=\"layout\" data-layout-kind=\"{kind_name}\" data-layout-widths=\"{}\" data-layout-tracks=\"{}\"{safety_attributes}{derivation_attributes}><span class=\"debug-block-label zpres-block-label\">layout: {kind_name}</span><div class=\"debug-layout-regions zpres-layout-regions\" style=\"{}\">",
        escape_attr(&widths),
        escape_attr(&tracks),
        escape_attr(&style)
    );
    for (index, region) in regions.iter().enumerate() {
        let mut region_classes = "debug-layout-region zpres-layout-region".to_string();
        let mut region_attributes = String::new();
        let mut region_style = String::new();
        if let Some(role) = region.role {
            region_attributes.push_str(&format!(
                " data-region-role=\"{}\"",
                layout_region_role_name(role)
            ));
            if context.slide_variant == Some(crate::deck::SlideVariant::Comparison)
                && matches!(
                    role,
                    crate::deck::LayoutRegionRole::Primary
                        | crate::deck::LayoutRegionRole::Supporting
                )
            {
                let comparison_role = if role == crate::deck::LayoutRegionRole::Primary {
                    "primary"
                } else {
                    "supporting"
                };
                let cue = if role == crate::deck::LayoutRegionRole::Primary {
                    "circle"
                } else {
                    "square"
                };
                region_attributes.push_str(&format!(
                    " data-comparison-role=\"{comparison_role}\" data-comparison-cue=\"{cue}\" data-comparison-index=\"{}\"",
                    index + 1
                ));
            }
            if matches!(
                role,
                crate::deck::LayoutRegionRole::Stable | crate::deck::LayoutRegionRole::Change
            ) {
                let derivation_role = if role == crate::deck::LayoutRegionRole::Stable {
                    "context"
                } else {
                    "stage"
                };
                region_attributes.push_str(&format!(" data-derivation-role=\"{derivation_role}\""));
            }
        }
        if let Some(step) = region.derivation_step {
            region_attributes.push_str(&format!(
                " data-step-index=\"{step}\" data-fragment-index=\"{step}\""
            ));
            match asset_mode.step_state() {
                None => {
                    region_classes.push_str(" fragment");
                    region_attributes.push_str(" data-step-state=\"future\"");
                }
                Some(PdfStepState::Final) => {
                    region_classes.push_str(" is-visible");
                    region_attributes.push_str(" data-step-state=\"complete\"");
                }
                Some(PdfStepState::UpTo { step: max_step }) if step <= max_step => {
                    region_classes.push_str(" is-visible");
                    let state = if step == max_step {
                        "active"
                    } else {
                        "complete"
                    };
                    region_attributes.push_str(&format!(" data-step-state=\"{state}\""));
                }
                Some(PdfStepState::UpTo { .. }) => {
                    region_classes.push_str(" zpres-print-step-hidden");
                    region_attributes.push_str(" data-step-state=\"future\" aria-hidden=\"true\"");
                }
            }
        }
        if let Some(placement) = region.grid_placement.as_ref() {
            let column = placement
                .column
                .map_or_else(|| "auto".to_string(), |value| value.to_string());
            let row = placement
                .row
                .map_or_else(|| "auto".to_string(), |value| value.to_string());
            region_attributes.push_str(&format!(
                    " data-grid-column=\"{column}\" data-grid-column-span=\"{}\" data-grid-row=\"{row}\" data-grid-row-span=\"{}\"",
                    placement.column_span,
                    placement.row_span
                ));
            region_style.push_str(&grid_region_style(placement));
        }
        if let Some(placement) = region.overlay_placement.as_ref() {
            region_attributes.push_str(&format!(
                " data-overlay-anchor=\"{}\" data-overlay-width=\"{}\"",
                overlay_anchor_name(placement.anchor),
                overlay_width_name(placement.width)
            ));
            region_style.push_str(&overlay_region_style(placement, index));
        } else if region.role == Some(crate::deck::LayoutRegionRole::Base) {
            region_style.push_str("grid-area:1/1;z-index:0;");
        }
        if !region_style.is_empty() {
            region_attributes.push_str(&format!(" style=\"{}\"", escape_attr(&region_style)));
        }
        html.push_str(&format!(
            "<section class=\"{}\" data-region-index=\"{}\"{}>",
            region_classes,
            index + 1,
            region_attributes
        ));
        if let Some(name) = &region.name {
            let step_attribute = region
                .derivation_step
                .map_or_else(String::new, |step| format!(" data-step-index=\"{step}\""));
            html.push_str(&format!(
                "<h3 class=\"debug-layout-region-title zpres-layout-region-title\" data-zpres-type-role=\"technical\"{step_attribute}>{}</h3>",
                escape_html(name)
            ));
        }
        let comparison_region = context.slide_variant
            == Some(crate::deck::SlideVariant::Comparison)
            && matches!(
                region.role,
                Some(
                    crate::deck::LayoutRegionRole::Primary
                        | crate::deck::LayoutRegionRole::Supporting
                )
            );
        if comparison_region {
            html.push_str("<div class=\"zpres-layout-region-content\">");
        }
        for block in &region.blocks {
            html.push_str(&render_block(block, asset_mode, context));
        }
        if comparison_region {
            html.push_str("</div>");
        }
        html.push_str("</section>");
    }
    html.push_str("</div></div>\n");
    html
}

fn layout_kind_name(kind: LayoutKind) -> &'static str {
    match kind {
        LayoutKind::Columns => "columns",
        LayoutKind::Grid => "grid",
        LayoutKind::Stack => "stack",
        LayoutKind::Overlay => "overlay",
        LayoutKind::Aside => "aside",
    }
}

fn layout_style(kind: LayoutKind, values: &LayoutValues) -> String {
    let gap = values
        .gap
        .as_ref()
        .map(layout_size_css)
        .unwrap_or_else(|| "1rem".to_string());
    let align = values.align.map(layout_align_css).unwrap_or("stretch");
    match kind {
        LayoutKind::Columns => {
            let columns = values
                .widths
                .as_ref()
                .map_or_else(|| "1fr 1fr".to_string(), |widths| column_tracks_css(widths));
            format!("display:grid;grid-template-columns:{columns};gap:{gap};align-items:{align};")
        }
        LayoutKind::Grid => {
            let tracks = values.tracks.as_ref().map_or_else(
                || "repeat(2,minmax(0,1fr))".to_string(),
                |tracks| {
                    tracks
                        .iter()
                        .map(|track| format!("minmax(0,{})", layout_size_css(track)))
                        .collect::<Vec<_>>()
                        .join(" ")
                },
            );
            format!(
                "display:grid;grid-template-columns:{tracks};grid-auto-flow:row;gap:{gap};align-items:{align};"
            )
        }
        LayoutKind::Stack => {
            format!("display:flex;flex-direction:column;gap:{gap};align-items:{align};")
        }
        LayoutKind::Overlay => {
            format!("display:grid;grid-template:1fr/1fr;gap:0;align-items:{align};")
        }
        LayoutKind::Aside => {
            let supporting = match values.aside_width {
                Some(crate::deck::AsideWidth::Compact) => "1fr",
                Some(crate::deck::AsideWidth::Standard) | None => "minmax(0,1.4fr)",
            };
            format!(
                "display:grid;grid-template-columns:minmax(0,3fr) {supporting};gap:{gap};align-items:{align};"
            )
        }
    }
}

fn column_tracks_css(widths: &[LayoutSize]) -> String {
    let percentages = widths
        .iter()
        .map(|width| match width {
            LayoutSize::Arbitrary { value } => value
                .trim()
                .strip_suffix('%')
                .and_then(|percentage| percentage.trim().parse::<f64>().ok())
                .filter(|percentage| percentage.is_finite() && *percentage > 0.0),
            LayoutSize::Scale { .. } | LayoutSize::Fraction { .. } => None,
        })
        .collect::<Option<Vec<_>>>();

    if let Some(percentages) = percentages
        && (percentages.iter().sum::<f64>() - 100.0).abs() <= 0.001
    {
        // CSS Grid resolves percentage tracks before adding `gap`. Imported
        // Pandoc/Quarto Columns commonly declare widths that total 100%, so
        // emitting those percentages verbatim makes the tracks plus the gap
        // wider than their content slot. Equivalent fractional tracks divide
        // the gap-free remainder while retaining the authored proportions.
        return percentages
            .iter()
            .map(|percentage| format!("minmax(0,{percentage}fr)"))
            .collect::<Vec<_>>()
            .join(" ");
    }

    widths
        .iter()
        .map(layout_size_css)
        .collect::<Vec<_>>()
        .join(" ")
}

fn layout_region_role_name(role: crate::deck::LayoutRegionRole) -> &'static str {
    match role {
        crate::deck::LayoutRegionRole::Base => "base",
        crate::deck::LayoutRegionRole::Annotation => "annotation",
        crate::deck::LayoutRegionRole::Primary => "primary",
        crate::deck::LayoutRegionRole::Supporting => "supporting",
        crate::deck::LayoutRegionRole::Stable => "stable",
        crate::deck::LayoutRegionRole::Change => "change",
    }
}

fn overlay_anchor_name(anchor: crate::deck::OverlayAnchor) -> &'static str {
    match anchor {
        crate::deck::OverlayAnchor::TopStart => "top-start",
        crate::deck::OverlayAnchor::TopEnd => "top-end",
        crate::deck::OverlayAnchor::BottomStart => "bottom-start",
        crate::deck::OverlayAnchor::BottomEnd => "bottom-end",
    }
}

fn overlay_width_name(width: crate::deck::OverlayWidth) -> &'static str {
    match width {
        crate::deck::OverlayWidth::Compact => "compact",
        crate::deck::OverlayWidth::Standard => "standard",
    }
}

fn overlay_region_style(placement: &crate::deck::OverlayPlacement, index: usize) -> String {
    let place = match placement.anchor {
        crate::deck::OverlayAnchor::TopStart => "start start",
        crate::deck::OverlayAnchor::TopEnd => "start end",
        crate::deck::OverlayAnchor::BottomStart => "end start",
        crate::deck::OverlayAnchor::BottomEnd => "end end",
    };
    let width = match placement.width {
        crate::deck::OverlayWidth::Compact => "16rem",
        crate::deck::OverlayWidth::Standard => "22rem",
    };
    format!(
        "grid-area:1/1;place-self:{place};z-index:{};max-width:{width};margin:1rem;",
        index + 1
    )
}

fn grid_region_style(placement: &crate::deck::GridPlacement) -> String {
    let mut style = String::new();
    if let Some(column) = placement.column {
        style.push_str(&format!(
            "grid-column:{column}/span {};",
            placement.column_span
        ));
    } else if placement.column_span > 1 {
        style.push_str(&format!("grid-column:span {};", placement.column_span));
    }
    if let Some(row) = placement.row {
        style.push_str(&format!("grid-row:{row}/span {};", placement.row_span));
    } else if placement.row_span > 1 {
        style.push_str(&format!("grid-row:span {};", placement.row_span));
    }
    style
}

fn layout_size_css(size: &LayoutSize) -> String {
    match size {
        LayoutSize::Scale { step } => match step {
            0 => "0".to_string(),
            1 => "0.25rem".to_string(),
            2 => "0.5rem".to_string(),
            3 => "0.75rem".to_string(),
            4 => "1rem".to_string(),
            6 => "1.5rem".to_string(),
            8 => "2rem".to_string(),
            12 => "3rem".to_string(),
            _ => "1rem".to_string(),
        },
        LayoutSize::Fraction { units } => format!("{units}fr"),
        LayoutSize::Arbitrary { value } => value.clone(),
    }
}

fn layout_size_token(size: &LayoutSize) -> String {
    match size {
        LayoutSize::Scale { step } => step.to_string(),
        LayoutSize::Fraction { units } => units.to_string(),
        LayoutSize::Arbitrary { value } => format!("[{value}]"),
    }
}

fn layout_align_css(align: LayoutAlign) -> &'static str {
    match align {
        LayoutAlign::Start => "start",
        LayoutAlign::Center => "center",
        LayoutAlign::End => "end",
        LayoutAlign::Stretch => "stretch",
    }
}

fn render_chart_svg(
    spec: &serde_json::Value,
    data: Option<&ChartData>,
    deck_root: Option<&Path>,
) -> Option<String> {
    let chart = crate::chart::load(spec, data, deck_root).ok()?;
    Some(line_chart_svg(
        &chart.points,
        &chart.x_title,
        &chart.y_title,
        chart.color_title.as_deref(),
    ))
}

fn line_chart_svg(
    points: &[ChartPoint],
    x_field: &str,
    y_field: &str,
    color_field: Option<&str>,
) -> String {
    let width = 820.0;
    let height = 330.0;
    // Keep the legend outside the data rectangle.
    let right = if color_field.is_some() { 216.0 } else { 26.0 };
    let bottom = 90.0;
    let (min_x, max_x) = extent(points.iter().map(|point| point.x));
    let (min_y, max_y) = extent(points.iter().flat_map(|point| {
        [Some(point.y), point.lower, point.upper]
            .into_iter()
            .flatten()
    }));
    // Narrow ranges far from zero need an additive offset, so repeated leading
    // digits do not consume the plotting area or obscure differences.
    let x_offset = chart_axis_offset(min_x, max_x);
    let y_offset = chart_axis_offset(min_y, max_y);
    let top = if y_offset != 0.0 { 64.0 } else { 24.0 };
    let x_ticks: Vec<_> = (0..=4)
        .map(|tick| {
            chart_tick_label(
                (min_x - x_offset) + (max_x - min_x) * (tick as f64 / 4.0),
                (max_x - min_x) / 4.0,
            )
        })
        .collect();
    let y_ticks: Vec<_> = (0..=4)
        .map(|tick| {
            chart_tick_label(
                (max_y - y_offset) - (max_y - min_y) * (tick as f64 / 4.0),
                (max_y - min_y) / 4.0,
            )
        })
        .collect();
    // Reserve a separate register for numbers and the rotated axis title.
    let left = (64.0 + y_ticks.iter().map(String::len).max().unwrap_or(0) as f64 * 14.0).max(128.0);
    let plot_width = width - left - right;
    let plot_height = height - top - bottom;
    let mut series: BTreeMap<String, Vec<&ChartPoint>> = BTreeMap::new();
    for point in points {
        series.entry(point.series.clone()).or_default().push(point);
    }

    let colors = ["#0f766e", "#2563eb", "#dc2626", "#7c3aed", "#ea580c"];
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg class=\"debug-chart-svg zpres-chart-svg\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"Vega-Lite line chart\"><rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" rx=\"8\" fill=\"#ffffff\"/><line x1=\"{left}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#475569\" stroke-width=\"2\"/><line x1=\"{left}\" y1=\"{top}\" x2=\"{left}\" y2=\"{}\" stroke=\"#475569\" stroke-width=\"2\"/>",
        height - bottom,
        width - right,
        height - bottom,
        height - bottom
    ));
    for tick in 0..=4 {
        let ratio = tick as f64 / 4.0;
        let y = top + plot_height * ratio;
        svg.push_str(&format!(
            "<line x1=\"{left}\" y1=\"{y:.2}\" x2=\"{}\" y2=\"{y:.2}\" stroke=\"#e2e8f0\"/>",
            width - right
        ));
    }
    {
        let widest_x_tick = x_ticks.iter().map(String::len).max().unwrap_or(0) as f64 * 14.0;
        for (tick, label) in y_ticks.iter().enumerate() {
            svg.push_str(&format!(
                "<text x=\"{}\" y=\"{:.2}\" class=\"zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"axis-tick\" data-chart-axis=\"y\" text-anchor=\"end\">{label}</text>",
                left - 16.0,
                top + plot_height * tick as f64 / 4.0 + 8.0,
            ));
        }
        for (tick, label) in x_ticks.iter().enumerate() {
            // Endpoint labels extend inward by their full width; middle labels
            // extend by half. Reduce density using that combined clearance.
            let stride = if widest_x_tick * 1.5 + 16.0 <= plot_width / 4.0 {
                1
            } else if widest_x_tick * 1.5 + 16.0 <= plot_width / 2.0 {
                2
            } else {
                4
            };
            if tick % stride != 0 {
                continue;
            }
            let x = left + plot_width * tick as f64 / 4.0;
            let anchor = match tick {
                0 => "start",
                4 => "end",
                _ => "middle",
            };
            svg.push_str(&format!(
                "<line x1=\"{x:.2}\" y1=\"{}\" x2=\"{x:.2}\" y2=\"{}\" stroke=\"#475569\"/><text x=\"{x:.2}\" y=\"{}\" class=\"zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"axis-tick\" data-chart-axis=\"x\" text-anchor=\"{anchor}\">{label}</text>",
                height - bottom,
                height - bottom + 6.0,
                height - bottom + 38.0,
            ));
        }
    }
    let mut legend = String::new();
    for (index, (name, values)) in series.iter_mut().enumerate() {
        values.sort_by(|left, right| left.x.total_cmp(&right.x));
        let color = colors[index % colors.len()];
        let path = values
            .iter()
            .enumerate()
            .map(|(point_index, point)| {
                let x = scale(point.x, min_x, max_x, left, left + plot_width);
                let y = scale(point.y, min_y, max_y, top + plot_height, top);
                if point_index == 0 {
                    format!("M {x:.2} {y:.2}")
                } else {
                    format!("L {x:.2} {y:.2}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            "<path d=\"{path}\" class=\"zpres-chart-line\" data-chart-role=\"line\" data-chart-series-index=\"{index}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"4\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
        ));
        for point in values.iter() {
            let x = scale(point.x, min_x, max_x, left, left + plot_width);
            let y = scale(point.y, min_y, max_y, top + plot_height, top);
            if let (Some(lower), Some(upper)) = (point.lower, point.upper) {
                let lower_y = scale(lower, min_y, max_y, top + plot_height, top);
                let upper_y = scale(upper, min_y, max_y, top + plot_height, top);
                svg.push_str(&format!(
                    "<line x1=\"{x:.2}\" y1=\"{lower_y:.2}\" x2=\"{x:.2}\" y2=\"{upper_y:.2}\" class=\"zpres-chart-mark zpres-chart-uncertainty\" data-chart-role=\"uncertainty\" data-chart-series-index=\"{index}\" stroke=\"{color}\" stroke-width=\"3\"/><line x1=\"{}\" y1=\"{lower_y:.2}\" x2=\"{}\" y2=\"{lower_y:.2}\" class=\"zpres-chart-mark\" data-chart-role=\"uncertainty-cap\" data-chart-series-index=\"{index}\" stroke=\"{color}\" stroke-width=\"3\"/><line x1=\"{}\" y1=\"{upper_y:.2}\" x2=\"{}\" y2=\"{upper_y:.2}\" class=\"zpres-chart-mark\" data-chart-role=\"uncertainty-cap\" data-chart-series-index=\"{index}\" stroke=\"{color}\" stroke-width=\"3\"/>",
                    x - 6.0,
                    x + 6.0,
                    x - 6.0,
                    x + 6.0,
                ));
            }
            let (point_class, point_role) = ("zpres-chart-point", "point");
            svg.push_str(&format!(
                "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"5\" class=\"{point_class}\" data-chart-role=\"{point_role}\" data-chart-series-index=\"{index}\" fill=\"{color}\" stroke=\"#ffffff\" stroke-width=\"2\"/>"
            ));
        }
        if color_field.is_some() {
            let legend_y = top + 58.0 + index as f64 * 32.0;
            legend.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{legend_y:.2}\" r=\"5\" class=\"zpres-chart-mark\" data-chart-role=\"legend-mark\" data-chart-series-index=\"{index}\" fill=\"{color}\"/><text x=\"{}\" y=\"{:.2}\" class=\"debug-chart-legend zpres-chart-legend\" data-zpres-type-role=\"technical\" data-chart-role=\"legend\">{}</text>",
                width - right + 28.0,
                width - right + 46.0,
                legend_y + 5.0,
                escape_html(name)
            ));
        }
    }
    svg.push_str(&legend);
    if y_offset != 0.0 {
        svg.push_str(&format!(
            "<text x=\"{left}\" y=\"28\" class=\"zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"axis-offset\" data-chart-axis=\"y\">{}</text>",
            chart_offset_label(y_offset),
        ));
    }
    let x_title = if x_offset != 0.0 {
        format!(
            "{} ({})",
            escape_html(x_field),
            chart_offset_label(x_offset)
        )
    } else {
        escape_html(x_field)
    };
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"debug-chart-axis-label zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"axis-label\" text-anchor=\"middle\">{}</text><text x=\"{}\" y=\"{}\" class=\"debug-chart-axis-label zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"axis-label\" transform=\"rotate(-90)\" text-anchor=\"middle\">{}</text>",
        left + plot_width / 2.0,
        height - 16.0,
        x_title,
        -top - plot_height / 2.0,
        32.0,
        escape_html(y_field)
    ));
    if let Some(color_field) = color_field {
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" class=\"debug-chart-axis-label zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"legend-title\">{}</text>",
            width - right + 28.0,
            top + 20.0,
            escape_html(color_field)
        ));
    }
    svg.push_str("</svg>");
    svg
}

fn chart_axis_offset(min: f64, max: f64) -> f64 {
    if min.abs() / (max - min) >= 1_000.0 {
        min
    } else {
        0.0
    }
}

fn chart_offset_label(offset: f64) -> String {
    if (0.001..1_000_000_000_000.0).contains(&offset.abs()) {
        format!("{offset:+}")
    } else {
        format!("{offset:+e}")
    }
}

fn chart_tick_label(value: f64, step: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let step_exponent = step.log10().floor();
    let scientific = !(0.001..1_000_000.0).contains(&value.abs());
    let precision = if scientific {
        value.abs().log10().floor() - step_exponent + 2.0
    } else {
        2.0 - step_exponent
    }
    .clamp(0.0, 16.0) as usize;
    let formatted = if scientific {
        format!("{value:.precision$e}")
    } else {
        format!("{value:.precision$}")
    };
    let (number, exponent) = formatted.split_once('e').unwrap_or((&formatted, ""));
    let number = if number.contains('.') {
        number.trim_end_matches('0').trim_end_matches('.')
    } else {
        number
    };
    if scientific {
        format!("{number}e{exponent}")
    } else {
        number.to_string()
    }
}

fn extent(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in values {
        min = min.min(value);
        max = max.max(value);
    }
    if min == max {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    }
}

fn scale(value: f64, domain_min: f64, domain_max: f64, range_min: f64, range_max: f64) -> f64 {
    let ratio = (value - domain_min) / (domain_max - domain_min);
    range_min + ratio * (range_max - range_min)
}

fn render_asset_src(src: &str, asset_mode: AssetRenderMode<'_>) -> String {
    match asset_mode {
        AssetRenderMode::Bundle { .. } => src.to_string(),
        AssetRenderMode::Print { deck_root, .. } => {
            if looks_remote_or_fragment(src) {
                return src.to_string();
            }
            deck_root.map_or_else(
                || src.to_string(),
                |root| file_url(&absolute_path(&root.join(src))),
            )
        }
    }
}

fn static_export_image_src<'a>(
    src: &'a str,
    static_src: Option<&'a str>,
    asset_mode: AssetRenderMode<'_>,
) -> &'a str {
    if matches!(asset_mode, AssetRenderMode::Print { .. }) {
        static_src.unwrap_or(src)
    } else {
        src
    }
}

fn render_v1_background_layer(
    background_image: &DeckBackgroundImage,
    asset_mode: AssetRenderMode<'_>,
    phase: BackgroundPhase,
) -> String {
    render_background_layer_markup(background_image, asset_mode, phase, true)
}

fn render_background_layer_markup(
    background_image: &DeckBackgroundImage,
    asset_mode: AssetRenderMode<'_>,
    phase: BackgroundPhase,
    expose_authored_source: bool,
) -> String {
    let src = render_asset_src(&background_image.src, asset_mode);
    let source_attr = expose_authored_source.then(|| {
        format!(
            " data-zpres-background-source=\"{}\"",
            escape_attr(&background_image.src)
        )
    });
    if expose_authored_source
        && background_image.intent != crate::deck::BackgroundImageIntent::Decorative
    {
        let description =
            background_image
                .description
                .as_deref()
                .map_or_else(String::new, |description| {
                    format!(
                        "<figcaption class=\"zpres-background-description\">{}</figcaption>",
                        escape_html(description)
                    )
                });
        return format!(
            "          <figure class=\"zpres-background-semantic\" data-zpres-background-intent=\"{}\"><img src=\"{}\" alt=\"{}\">{}</figure>\n          <div class=\"zpres-slide-background\"{} aria-hidden=\"true\" style=\"{}\"></div>\n",
            background_intent_name(background_image.intent),
            escape_attr(&src),
            escape_attr(&background_image.alt),
            description,
            source_attr.as_deref().unwrap_or_default(),
            escape_attr(&background_image_style(background_image, &src))
        );
    }
    let aria = if phase == BackgroundPhase::Splash && !background_image.alt.is_empty() {
        format!(
            " role=\"img\" aria-label=\"{}\"",
            escape_attr(&background_image.alt)
        )
    } else {
        " aria-hidden=\"true\"".to_string()
    };
    format!(
        "          <div class=\"zpres-slide-background\"{}{} style=\"{}\"></div>\n",
        source_attr.as_deref().unwrap_or_default(),
        aria,
        escape_attr(&background_image_style(background_image, &src))
    )
}

fn background_intent_name(intent: crate::deck::BackgroundImageIntent) -> &'static str {
    match intent {
        crate::deck::BackgroundImageIntent::Decorative => "decorative",
        crate::deck::BackgroundImageIntent::Contextual => "contextual",
        crate::deck::BackgroundImageIntent::Evidence => "evidence",
    }
}

fn background_image_style(background_image: &DeckBackgroundImage, src: &str) -> String {
    let split_size = background_image
        .split
        .as_ref()
        .map_or("50%", |split| split.size.as_str());
    format!(
        "background-image: url(\"{}\"); --zpres-background-image: url(\"{}\"); --zpres-background-position: {}; --zpres-background-fit: {}; --zpres-background-split-size: {}; --zpres-background-dim: {:.2}; --zpres-background-gray: {:.2}; --zpres-background-saturate: {:.2}; --zpres-background-blur: {}px;",
        css_string(src),
        css_string(src),
        sanitize_css_token_list(&background_image.position),
        background_image_fit_css(background_image.fit),
        split_size,
        f32::from(background_image.dim.min(100)) / 100.0,
        f32::from(background_image.grayscale.min(100)) / 100.0,
        f32::from(background_image.saturate.min(100)) / 100.0,
        background_image.blur.min(24)
    )
}

fn background_image_fit_css(fit: BackgroundImageFit) -> &'static str {
    match fit {
        BackgroundImageFit::Cover => "cover",
        BackgroundImageFit::Contain => "contain",
    }
}

fn sanitize_css_token_list(value: &str) -> String {
    if is_safe_css_token_list(value) {
        value.to_string()
    } else {
        "center center".to_string()
    }
}

fn is_safe_css_token_list(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character.is_ascii_whitespace()
                || matches!(character, '-' | '.' | '%')
        })
}

fn css_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

fn render_paragraph(markdown: &str, inline_math: &[String], context: &RenderContext) -> String {
    if context.footnote_scope.is_some() {
        return inline::render(markdown, inline_math, context);
    }
    let html = render_inline_math(markdown, inline_math);
    render_footnote_references(&html, context)
}

fn render_inline_math(markdown: &str, inline_math: &[String]) -> String {
    let mut html = escape_multiline(markdown);
    for latex in inline_math {
        let rendered = render_math(latex, false);
        for source in [format!(r"\({latex}\)"), format!("${latex}$")] {
            html = html.replace(&escape_html(&source), &rendered);
        }
    }
    html
}

fn render_footnote_references(html: &str, context: &RenderContext) -> String {
    let mut rendered = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("[^") {
        rendered.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find(']') else {
            rendered.push_str(&rest[start..]);
            return rendered;
        };
        let label = &after_start[..end];
        if let Some(number) = context.footnote_numbers.get(label) {
            rendered.push_str(&format!(
                "<sup id=\"{}\" class=\"zpres-footnote-ref\" data-footnote-label=\"{}\"><a href=\"#{}\">{}</a></sup>",
                footnote_ref_dom_id(label),
                escape_attr(label),
                footnote_dom_id(label),
                number
            ));
        } else {
            rendered.push_str("[^");
            rendered.push_str(label);
            rendered.push(']');
        }
        rest = &after_start[end + 1..];
    }
    rendered.push_str(rest);
    rendered
}

fn footnote_dom_id(label: &str) -> String {
    format!("zpres-footnote-{}", dom_id_suffix(label))
}

fn footnote_ref_dom_id(label: &str) -> String {
    format!("zpres-footnote-ref-{}", dom_id_suffix(label))
}

fn dom_id_suffix(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn render_math(latex: &str, display: bool) -> String {
    let Ok(opts) = katex::Opts::builder()
        .display_mode(display)
        .output_type(katex::OutputType::Mathml)
        .throw_on_error(true)
        .build()
    else {
        return format!(
            "<code class=\"debug-unresolved-math zpres-unresolved-math\">{}</code>",
            escape_html(latex)
        );
    };
    match katex::render_with_opts(latex, &opts) {
        Ok(mathml) => {
            if display {
                format!("<div class=\"debug-math-display zpres-math-display\">{mathml}</div>")
            } else {
                format!("<span class=\"debug-math-inline zpres-math-inline\">{mathml}</span>")
            }
        }
        Err(_) => format!(
            "<code class=\"debug-unresolved-math zpres-unresolved-math\">{}</code>",
            escape_html(latex)
        ),
    }
}

fn highlight_code(code: &str, language: Option<&str>) -> String {
    let escaped = escape_html(code);
    match language {
        Some("rust" | "rs") => highlight_rust(&escaped),
        Some("json") => highlight_json(&escaped),
        Some("pseudo" | "pseudocode" | "algorithm") => highlight_pseudocode(&escaped),
        _ => escaped,
    }
}

fn highlight_rust(code: &str) -> String {
    let keywords = [
        "fn", "let", "mut", "struct", "enum", "impl", "for", "in", "if", "else", "match", "pub",
        "use", "mod", "return",
    ];
    highlight_words(code, &keywords)
}

fn highlight_json(code: &str) -> String {
    code.replace("&quot;", "<span class=\"syntax-string\">&quot;</span>")
}

fn highlight_pseudocode(code: &str) -> String {
    let keywords = [
        "Input", "Output", "for", "each", "in", "if", "then", "else", "return", "build", "compute",
        "update", "set", "to",
    ];
    highlight_words(code, &keywords)
}

fn highlight_words(code: &str, keywords: &[&str]) -> String {
    let mut highlighted = String::new();
    let mut word = String::new();
    for character in code.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            word.push(character);
            continue;
        }
        flush_highlighted_word(&mut highlighted, &mut word, keywords);
        highlighted.push(character);
    }
    flush_highlighted_word(&mut highlighted, &mut word, keywords);
    highlighted
}

fn flush_highlighted_word(output: &mut String, word: &mut String, keywords: &[&str]) {
    if word.is_empty() {
        return;
    }
    if keywords.contains(&word.as_str()) {
        output.push_str(&format!("<span class=\"syntax-keyword\">{word}</span>"));
    } else if word.chars().all(|character| character.is_ascii_digit()) {
        output.push_str(&format!("<span class=\"syntax-number\">{word}</span>"));
    } else {
        output.push_str(word);
    }
    word.clear();
}

fn render_diagnostics(deck: &Deck) -> String {
    render_diagnostics_list(&deck.diagnostics)
}

pub fn render_diagnostics_list(diagnostics: &[crate::deck::Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return "  <aside class=\"debug-diagnostics\" aria-label=\"diagnostics\"><strong>diagnostics</strong><span>none</span></aside>\n".to_string();
    }

    let mut html = String::new();
    html.push_str("  <aside class=\"debug-diagnostics\" aria-label=\"diagnostics\">\n");
    html.push_str("    <strong>diagnostics</strong>\n");
    html.push_str("    <ol>\n");
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warning",
        };
        html.push_str(&format!(
            "      <li data-severity=\"{}\"><span>{}</span> {}: {}</li>\n",
            severity,
            severity,
            render_diagnostic_location(diagnostic),
            escape_html(&diagnostic.message)
        ));
    }
    html.push_str("    </ol>\n");
    html.push_str("  </aside>\n");
    html
}

pub fn core_runtime_asset(
    path: &str,
    api: theme::ThemeApiVersion,
) -> Option<(&'static str, &'static str)> {
    match path {
        theme_api_v1::FOUNDATION_ASSET_PATH => {
            Some(("text/css; charset=utf-8", theme_api_v1::FOUNDATION_CSS))
        }
        "assets/reveal.js" => Some(("text/javascript; charset=utf-8", theme_runtime_js(api))),
        _ => None,
    }
}

fn render_diagnostic_location(diagnostic: &crate::deck::Diagnostic) -> String {
    match &diagnostic.span {
        Some(span) => {
            let path = span
                .source_path
                .as_ref()
                .map_or_else(|| "<source>".to_string(), |path| path.display().to_string());
            format!("{}:{}:{}", escape_html(&path), span.line, span.column)
        }
        None => "<unknown>".to_string(),
    }
}

fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), HtmlError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| HtmlError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(contents.as_ref())
        .map_err(|source| HtmlError::Write {
            path: path.to_path_buf(),
            source,
        })
}

fn copy_local_assets(deck: &Deck, output_dir: &Path) -> Result<(), HtmlError> {
    let Some(deck_root) = deck.deck_root() else {
        return Ok(());
    };
    for reference in deck.local_asset_references() {
        let source_path = deck_root.join(reference);
        let output_path = output_dir.join(reference);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|source| HtmlError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        copy_file_create_new(&source_path, &output_path)?;
    }
    Ok(())
}

fn copy_theme_dependencies(theme: &RenderedTheme, output_dir: &Path) -> Result<(), HtmlError> {
    let theme_root = theme
        .manifest
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    for dependency in theme::theme_dependency_paths(&theme.manifest) {
        let source_path = theme_root.join(dependency);
        let output_path = output_dir.join("assets").join(dependency);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|source| HtmlError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        copy_file_create_new(&source_path, &output_path)?;
    }
    Ok(())
}

fn copy_file_create_new(source_path: &Path, destination: &Path) -> Result<(), HtmlError> {
    let mut source = fs::File::open(source_path).map_err(|source| HtmlError::Copy {
        source_path: source_path.to_path_buf(),
        destination: destination.to_path_buf(),
        source,
    })?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|source| HtmlError::Copy {
            source_path: source_path.to_path_buf(),
            destination: destination.to_path_buf(),
            source,
        })?;
    io::copy(&mut source, &mut output).map_err(|source| HtmlError::Copy {
        source_path: source_path.to_path_buf(),
        destination: destination.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|current_dir| current_dir.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn looks_remote_or_fragment(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with('#')
}

fn escape_multiline(value: &str) -> String {
    escape_html(value).replace('\n', "<br>")
}

fn render_speaker_notes_body(markdown: &str) -> String {
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

pub(crate) const REVEAL_JS: &str = include_str!("html/runtime-v1.js");
pub(crate) fn theme_runtime_js(_api: theme::ThemeApiVersion) -> &'static str {
    REVEAL_JS
}

const AUTOSCALE_JS: &str = r##"(function () {
  const MIN_SCALE = 0.62;
  const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));

  function slideId(slide) {
    return slide?.getAttribute("data-slide-id") || slide?.id || "unknown-slide";
  }

  function isMeasurable(element) {
    const style = getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
      return false;
    }
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function isFitVisible(element, content) {
    if (!element || element.closest(".zpres-debug-boundary-label, .zpres-block-label, .zpres-slide-meta, script, style")) {
      return false;
    }
    for (let current = element; current; current = current.parentElement) {
      const style = getComputedStyle(current);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
        return false;
      }
      if (current === content) break;
    }
    return true;
  }

  function authoredFitRects(content) {
    const rects = [];
    const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      if (!node.nodeValue.trim() || !isFitVisible(node.parentElement, content)) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      const blockBounds = node.parentElement.getBoundingClientRect();
      for (const rect of Array.from(range.getClientRects())) {
        if (rect.width > 0 && rect.height > 0) {
          rects.push({ bounds: rect, blockBounds, text: true });
        }
      }
    }

    const evidenceSelector = [
      "img", "video", "audio", "iframe", "canvas", "math", "table", "pre", "hr",
      "svg :is(path, line, polyline, polygon, circle, ellipse, rect, image, foreignObject, use)",
      ".zpres-media-fallback", "[data-zpres-unresolved]", ".zpres-block-html-only > *",
    ].join(",");
    for (const element of Array.from(new Set(content.querySelectorAll(evidenceSelector)))) {
      if (!isFitVisible(element, content) || !isMeasurable(element)) continue;
      // An inline SVG's viewport-filling rectangle is its painted surface,
      // analogous to a Theme-owned card. Measure the authored marks within
      // that viewport instead, so an inset chart or diagram can remain safe
      // while its surrounding treatment intentionally enters the canvas.
      const svg = element.ownerSVGElement;
      if (svg && element.localName === "rect") {
        const bounds = element.getBoundingClientRect();
        const viewport = svg.getBoundingClientRect();
        if (bounds.left <= viewport.left + 2 && bounds.top <= viewport.top + 2
          && bounds.right >= viewport.right - 2 && bounds.bottom >= viewport.bottom - 2) {
          continue;
        }
      }
      rects.push({ bounds: element.getBoundingClientRect(), text: false });
    }
    return rects;
  }

  function contentFits(content, available) {
    // The safe content slot constrains authored glyphs and semantic evidence.
    // Theme-owned surfaces such as a staggered card may intentionally enter
    // the surrounding canvas treatment; their box alone is not clipping.
    for (const subject of authoredFitRects(content)) {
      const rect = subject.bounds;
      // DOM Range rectangles follow the font line box on the block axis, so
      // their top/bottom can extend beyond the actual glyph paint. Use the
      // owning text element for block containment and the Range for inline
      // containment, where it distinguishes safe glyphs from a side-bleeding
      // Theme surface. Evidence boxes retain strict geometry on every side.
      const blockRect = subject.text ? subject.blockBounds : rect;
      const inlineTolerance = 2;
      if (rect.left < available.left - inlineTolerance
        || blockRect.top < available.top - 2
        || rect.right > available.right + inlineTolerance
        || blockRect.bottom > available.bottom + 2) {
        return false;
      }
    }
    return true;
  }

  async function applyScale(content, factor) {
    content.style.setProperty("--zpres-autoscale-factor", String(factor));
    if (factor < 1) {
      content.style.width = (100 / factor) + "%";
      content.style.height = (100 / factor) + "%";
    } else {
      content.style.removeProperty("width");
      content.style.removeProperty("height");
    }
    await nextFrame();
  }

  async function autoscaleSlide(slide) {
    if (!slide || slide.getAttribute("data-autoscale") !== "true") return null;
    const canvas = slide.querySelector(".zpres-slide-canvas");
    const content = canvas?.querySelector(".zpres-slide-content");
    if (!canvas || !content) {
      return { slide_id: slideId(slide), factor: null, overflow: false, skipped: "missing-canvas" };
    }
    const available = content.getBoundingClientRect();
    if (!available.width || !available.height) {
      return { slide_id: slideId(slide), factor: null, overflow: false, skipped: "empty-canvas" };
    }

    content.style.removeProperty("--zpres-autoscale-factor");
    content.style.removeProperty("width");
    content.style.removeProperty("height");
    slide.removeAttribute("data-zpres-overflow");
    slide.removeAttribute("data-autoscale-factor");

    await applyScale(content, 1);
    let factor = 1;
    let fits = contentFits(content, available);
    if (!fits) {
      await applyScale(content, MIN_SCALE);
      fits = contentFits(content, available);
      if (fits) {
        let lower = MIN_SCALE;
        let upper = 1;
        for (let iteration = 0; iteration < 12; iteration += 1) {
          const candidate = (lower + upper) / 2;
          await applyScale(content, candidate);
          if (contentFits(content, available)) lower = candidate;
          else upper = candidate;
        }
        factor = Math.floor(lower * 1000) / 1000;
        await applyScale(content, factor);
        fits = contentFits(content, available);
      } else {
        factor = MIN_SCALE;
      }
    }

    if (factor < 1) slide.setAttribute("data-autoscale-factor", factor.toFixed(3));
    const overflow = !fits;
    if (overflow) slide.setAttribute("data-zpres-overflow", "clipped");
    return { slide_id: slideId(slide), factor, overflow };
  }

  async function autoscaleAll() {
    const body = document.body;
    body?.setAttribute("data-zpres-autoscale-ready", "pending");
    try {
      const slides = Array.from(document.querySelectorAll(".zpres-slide[data-autoscale=\"true\"]"));
      const results = (await Promise.all(slides.map(autoscaleSlide))).filter(Boolean);
      await nextFrame();
      body?.setAttribute("data-zpres-autoscale-ready", "true");
      return { status: "ready", slides: results };
    } catch (error) {
      body?.setAttribute("data-zpres-autoscale-ready", "failed");
      throw error;
    }
  }

  function scheduleAutoscale() {
    requestAnimationFrame(() => {
      autoscaleAll().catch(() => {});
    });
  }

  window.zpresAutoscaleAll = autoscaleAll;
  window.addEventListener("resize", scheduleAutoscale);
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", scheduleAutoscale, { once: true });
  } else {
    scheduleAutoscale();
  }
})();
"##;

const STATIC_READINESS_JS: &str = r##"(function () {
  const TIMEOUT_MS = 5000;
  const body = document.body;
  const renderPromises = window.zpresStaticRenderPromises == null
    ? []
    : Array.isArray(window.zpresStaticRenderPromises)
      ? window.zpresStaticRenderPromises
      : [window.zpresStaticRenderPromises];
  window.zpresStaticRenderPromises = renderPromises;

  function timeoutError(stage, label) {
    const error = new Error(label + " timed out after " + TIMEOUT_MS + "ms");
    error.name = "ZpresStaticReadinessTimeout";
    error.stage = stage;
    return error;
  }

  function withTimeout(promise, stage, label) {
    return Promise.race([
      Promise.resolve(promise),
      new Promise((_, reject) => {
        setTimeout(() => reject(timeoutError(stage, label)), TIMEOUT_MS);
      }),
    ]);
  }

  function errorMessage(error) {
    if (error instanceof Error) return error.message;
    return String(error);
  }

  function failure(stage, kind, error, details) {
    return Object.assign({
      stage,
      kind,
      message: errorMessage(error),
    }, details || {});
  }

  function eventPromise(target, eventName, failureEvent) {
    return new Promise((resolve, reject) => {
      target.addEventListener(eventName, resolve, { once: true });
      if (failureEvent) {
        target.addEventListener(failureEvent, () => reject(new Error(failureEvent + " event")), { once: true });
      }
    });
  }

  async function waitForDocument() {
    if (document.readyState === "loading") {
      await eventPromise(document, "DOMContentLoaded");
    }
    if (document.readyState !== "complete") {
      await eventPromise(window, "load");
    }
    return document.readyState;
  }

  async function waitForImage(image) {
    if ("loading" in image) image.loading = "eager";
    const source = image.currentSrc || image.src || "";
    if (!source) throw new Error("image has no source");
    if (!image.complete) await eventPromise(image, "load", "error");
    if (!image.naturalWidth || !image.naturalHeight) {
      throw new Error("image has no natural size");
    }
    if (typeof image.decode === "function") await image.decode();
    return {
      kind: "image",
      source,
      natural_width: image.naturalWidth,
      natural_height: image.naturalHeight,
    };
  }

  function backgroundSources() {
    const sources = new Set();
    const collect = (element, pseudo) => {
      let value = "";
      try {
        value = getComputedStyle(element, pseudo).backgroundImage || "";
      } catch (_) {
        return;
      }
      const pattern = /url\((?:"([^"]*)"|'([^']*)'|([^)]*))\)/g;
      let match;
      while ((match = pattern.exec(value)) !== null) {
        const raw = (match[1] || match[2] || match[3] || "").trim();
        if (!raw) continue;
        try {
          sources.add(new URL(raw, document.baseURI).href);
        } catch (_) {
          sources.add(raw);
        }
      }
    };
    for (const element of Array.from(document.querySelectorAll("*"))) {
      collect(element, null);
      collect(element, "::before");
      collect(element, "::after");
    }
    return Array.from(sources);
  }

  async function waitForBackground(source) {
    const image = new Image();
    const loaded = eventPromise(image, "load", "error");
    image.src = source;
    if (!image.complete || !image.naturalWidth) await loaded;
    if (!image.naturalWidth || !image.naturalHeight) {
      throw new Error("background image has no natural size");
    }
    if (typeof image.decode === "function") await image.decode();
    return {
      kind: "background-image",
      source,
      natural_width: image.naturalWidth,
      natural_height: image.naturalHeight,
    };
  }

  async function settleResources(values, stage, describe, wait) {
    const settled = await Promise.allSettled(values.map((value, index) => withTimeout(
      wait(value),
      stage,
      describe(value, index),
    )));
    const ready = [];
    const errors = [];
    settled.forEach((result, index) => {
      if (result.status === "fulfilled") {
        ready.push(result.value);
      } else {
        errors.push(failure(stage, stage === "images" ? "image" : "background-image", result.reason, {
          source: describe(values[index], index),
          index,
        }));
      }
    });
    return { ready, errors };
  }

  const readiness = (async () => {
    const errors = [];
    let documentState = document.readyState;
    try {
      documentState = await withTimeout(waitForDocument(), "document", "document readiness");
    } catch (error) {
      errors.push(failure("document", "document", error));
    }

    let fonts = [];
    if (document.fonts) {
      try {
        await withTimeout(document.fonts.ready, "fonts", "font readiness");
      } catch (error) {
        errors.push(failure("fonts", "font-set", error));
      }
      fonts = Array.from(document.fonts).map((font) => ({
        family: font.family,
        status: font.status,
        style: font.style,
        weight: font.weight,
      }));
      fonts.filter((font) => font.status === "error").forEach((font) => {
        errors.push(failure("fonts", "font-face", new Error("font face failed to load"), font));
      });
    } else {
      errors.push(failure("fonts", "font-set", new Error("FontFaceSet API is unavailable")));
    }

    const rendererResults = await Promise.allSettled(renderPromises.map((value, index) => withTimeout(
      Promise.resolve().then(() => typeof value === "function" ? value() : value),
      "renderers",
      "static renderer " + (index + 1),
    )));
    rendererResults.forEach((result, index) => {
      if (result.status === "rejected") {
        errors.push(failure("renderers", "static-renderer", result.reason, { index }));
      }
    });

    const images = Array.from(document.images);
    const backgroundValues = backgroundSources();
    const [imageResults, backgroundResults] = await Promise.all([
      settleResources(
        images,
        "images",
        (image, index) => image.currentSrc || image.src || "image " + (index + 1),
        waitForImage,
      ),
      settleResources(
        backgroundValues,
        "backgrounds",
        (source) => source,
        waitForBackground,
      ),
    ]);
    errors.push(...imageResults.errors);
    errors.push(...backgroundResults.errors);

    let autoscale = null;
    if (typeof window.zpresAutoscaleAll === "function") {
      try {
        autoscale = await withTimeout(window.zpresAutoscaleAll(), "autoscale", "autoscale and overflow pass");
      } catch (error) {
        errors.push(failure("autoscale", "autoscale", error));
      }
    } else {
      errors.push(failure("autoscale", "autoscale", new Error("window.zpresAutoscaleAll is unavailable")));
    }

    const result = {
      status: errors.length ? "failed" : "ready",
      errors,
      document_ready_state: documentState,
      fonts,
      images: imageResults.ready,
      backgrounds: backgroundResults.ready,
      renderers: { count: renderPromises.length },
      autoscale,
    };
    if (errors.length) {
      const error = new Error("static readiness failed with " + errors.length + " error(s)");
      error.name = "ZpresStaticReadinessError";
      error.details = result;
      throw error;
    }
    return result;
  })().then((result) => {
    window.zpresStaticReadyState = result;
    window.zpresStaticReadyError = null;
    body?.setAttribute("data-zpres-ready", "true");
    body?.removeAttribute("data-zpres-ready-error-count");
    return result;
  }, (error) => {
    const details = error?.details || {
      status: "failed",
      errors: [failure("readiness", "unexpected", error)],
    };
    let rejection = error;
    if (!error?.details) {
      rejection = new Error("static readiness failed unexpectedly: " + errorMessage(error));
      rejection.name = "ZpresStaticReadinessError";
      rejection.details = details;
    }
    window.zpresStaticReadyState = details;
    window.zpresStaticReadyError = details;
    body?.setAttribute("data-zpres-ready", "failed");
    body?.setAttribute("data-zpres-ready-error-count", String(details.errors.length));
    throw rejection;
  });

  window.zpresStaticReadyState = { status: "pending", errors: [] };
  window.zpresStaticReadyError = null;
  window.zpresStaticReady = readiness;
  readiness.catch(() => {});
})();
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::chromium::{
        ChromiumError, ChromiumSession, ChromiumSessionOptions, STATIC_READINESS_TIMEOUT,
    };
    use crate::deck::parse_source_text;
    use crate::pdf::PngViewport;
    use crate::theme;
    use serde_json::Value;
    use tempfile::tempdir;

    const MERMAID_BROWSER_GEOMETRY_JS: &str = r#"(() => {
      const slide = document.querySelector(__ZPRES_SLIDE_SELECTOR__);
      const svg = slide?.querySelector('.zpres-diagram-svg');
      if (!slide || !svg) return { error: 'missing Diagram SVG' };
      const svgBounds = svg.getBoundingClientRect();
      const viewBox = svg.viewBox.baseVal;
      const scale = Math.min(svgBounds.width / viewBox.width, svgBounds.height / viewBox.height);
      const rect = (bounds) => ({
        left: bounds.left, top: bounds.top, right: bounds.right, bottom: bounds.bottom,
        width: bounds.width, height: bounds.height,
      });
      const expand = (bounds, amount) => ({
        left: bounds.left - amount, top: bounds.top - amount,
        right: bounds.right + amount, bottom: bounds.bottom + amount,
      });
      const intersects = (first, second) => first.left < second.right
        && first.right > second.left && first.top < second.bottom && first.bottom > second.top;
      const nodes = Array.from(svg.querySelectorAll('.zpres-diagram-node rect'))
        .map((node) => rect(node.getBoundingClientRect()));
      const paths = Array.from(svg.querySelectorAll('.zpres-diagram-edge'));
      const labels = Array.from(svg.querySelectorAll('.zpres-diagram-edge-label')).map((label) => {
        const bounds = rect(label.getBoundingClientRect());
        const outline = (Number.parseFloat(getComputedStyle(label).strokeWidth) || 0) * scale / 2;
        const paint = expand(bounds, outline);
        const clearance = 8 * scale;
        const routeClear = paths.every((path) => {
          const length = path.getTotalLength();
          const matrix = path.getScreenCTM();
          if (!matrix) return false;
          const samples = Math.max(1, Math.ceil(length / 2));
          for (let index = 0; index <= samples; index += 1) {
            const point = path.getPointAtLength(length * index / samples);
            const screen = new DOMPoint(point.x, point.y).matrixTransform(matrix);
            if (screen.x >= paint.left - clearance && screen.x <= paint.right + clearance
              && screen.y >= paint.top - clearance && screen.y <= paint.bottom + clearance) {
              return false;
            }
          }
          return true;
        });
        return {
          authored: label.getAttribute('data-zpres-authored-label'),
          bounds,
          paint,
          inside_svg: paint.left >= svgBounds.left - 0.5
            && paint.top >= svgBounds.top - 0.5
            && paint.right <= svgBounds.right + 0.5
            && paint.bottom <= svgBounds.bottom + 0.5,
          node_clear: nodes.every((node) => !intersects(expand(paint, clearance), node)),
          route_clear: routeClear,
        };
      });
      return { svg: rect(svgBounds), scale, labels };
    })()"#;

    const MERMAID_BROWSER_ARROW_JS: &str = r#"(() => {
      const slide = document.querySelector(__ZPRES_SLIDE_SELECTOR__);
      const svg = slide?.querySelector('.zpres-diagram-svg');
      if (!slide || !svg) return { error: 'missing Diagram SVG' };
      const slideBounds = slide.getBoundingClientRect();
      const svgBounds = svg.getBoundingClientRect();
      const relative = (bounds) => ({
        x: bounds.left - slideBounds.left,
        y: bounds.top - slideBounds.top,
        width: bounds.width,
        height: bounds.height,
      });
      const arrows = Array.from(svg.querySelectorAll('.zpres-diagram-arrow')).map((arrow) => {
        const bounds = arrow.getBoundingClientRect();
        return {
          edge: arrow.getAttribute('data-zpres-mermaid-arrow'),
          bounds: relative(bounds),
          fill: getComputedStyle(arrow).fill,
          inside_svg: bounds.left >= svgBounds.left - 0.5
            && bounds.top >= svgBounds.top - 0.5
            && bounds.right <= svgBounds.right + 0.5
            && bounds.bottom <= svgBounds.bottom + 0.5,
        };
      });
      return {
        edge_count: svg.querySelectorAll('.zpres-diagram-edge').length,
        marker_ids: Array.from(svg.querySelectorAll('marker[id]')).map((marker) => marker.id),
        marker_references: Array.from(svg.querySelectorAll('[marker-end]'))
          .map((edge) => edge.getAttribute('marker-end')),
        arrows,
      };
    })()"#;

    fn rendered_theme(deck: &Deck) -> theme::RenderedTheme {
        let manifest = theme::load_named_theme(
            deck.metadata
                .theme
                .as_deref()
                .unwrap_or(theme::DEFAULT_THEME_NAME),
            deck.deck_root().unwrap_or_else(|| Path::new(".")),
            &[theme::builtin_theme_search_path()],
        )
        .unwrap();
        let params =
            theme::validate_theme_params_best_effort(&manifest, &deck.metadata.theme_params);
        theme::render_theme(&manifest, &params).unwrap()
    }

    fn browser_mermaid_geometry(browser: &mut ChromiumSession, slide_selector: &str) -> Value {
        let selector = serde_json::to_string(slide_selector).unwrap();
        let expression = MERMAID_BROWSER_GEOMETRY_JS.replace("__ZPRES_SLIDE_SELECTOR__", &selector);
        browser.evaluate_for_test(&expression).unwrap()
    }

    fn browser_mermaid_arrows(browser: &mut ChromiumSession, slide_selector: &str) -> Value {
        let selector = serde_json::to_string(slide_selector).unwrap();
        let expression = MERMAID_BROWSER_ARROW_JS.replace("__ZPRES_SLIDE_SELECTOR__", &selector);
        browser.evaluate_for_test(&expression).unwrap()
    }

    fn assert_clear_mermaid_browser_geometry(surface: &str, geometry: &Value) {
        assert!(
            geometry.get("error").is_none(),
            "{surface} Diagram geometry query failed: {geometry}"
        );
        let labels = geometry["labels"].as_array().expect("edge-label geometry");
        assert_eq!(
            labels.len(),
            2,
            "{surface} branching fixture must expose both authored edge labels: {geometry}"
        );
        for label in labels {
            assert_eq!(
                label["inside_svg"], true,
                "{surface} edge label paint escaped the SVG viewport: {label}"
            );
            assert_eq!(
                label["node_clear"], true,
                "{surface} edge label entered a node's immediate field: {label}"
            );
            assert_eq!(
                label["route_clear"], true,
                "{surface} edge label entered a route's immediate field: {label}"
            );
        }
    }

    fn assert_mermaid_arrow_raster(surface: &str, observation: &Value, png: &[u8]) {
        assert!(
            observation.get("error").is_none(),
            "{surface} Diagram arrow query failed: {observation}"
        );
        assert_eq!(
            observation["marker_ids"],
            serde_json::json!([]),
            "{surface} adaptive Diagram retained document-global marker IDs"
        );
        assert_eq!(
            observation["marker_references"],
            serde_json::json!([]),
            "{surface} adaptive Diagram retained fragment marker references"
        );
        let arrows = observation["arrows"]
            .as_array()
            .expect("arrow observations");
        assert_eq!(
            arrows.len(),
            observation["edge_count"].as_u64().unwrap() as usize,
            "{surface} must paint one explicit arrowhead for every edge: {observation}"
        );
        let image = image::load_from_memory(png).unwrap().to_rgba8();
        for arrow in arrows {
            assert_eq!(
                arrow["inside_svg"], true,
                "{surface} arrowhead escaped the Diagram viewport: {arrow}"
            );
            let fill = arrow["fill"].as_str().unwrap_or_default();
            assert!(
                fill != "none" && fill != "rgba(0, 0, 0, 0)",
                "{surface} arrowhead has no visible fill: {arrow}"
            );
            let bounds = &arrow["bounds"];
            let x = bounds["x"].as_f64().unwrap();
            let y = bounds["y"].as_f64().unwrap();
            let width = bounds["width"].as_f64().unwrap();
            let height = bounds["height"].as_f64().unwrap();
            assert!(
                width >= 4.0 && height >= 4.0,
                "{surface} arrowhead has no paintable area: {arrow}"
            );
            let left = x.floor().max(0.0) as u32;
            let top = y.floor().max(0.0) as u32;
            let right = (x + width).ceil().min(image.width() as f64) as u32;
            let bottom = (y + height).ceil().min(image.height() as f64) as u32;
            let mut accent_pixels = 0usize;
            for pixel_y in top..bottom {
                for pixel_x in left..right {
                    let [red, green, blue, alpha] = image.get_pixel(pixel_x, pixel_y).0;
                    if alpha > 0 && red > 70 && blue > 170 && blue > green.saturating_add(50) {
                        accent_pixels += 1;
                    }
                }
            }
            assert!(
                accent_pixels >= 4,
                "{surface} arrowhead did not leave violet raster evidence ({accent_pixels} pixels): {arrow}"
            );
        }
    }

    fn write_v1_reference_theme(path: &Path) -> theme::RenderedTheme {
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join("theme.toml"),
            r#"[theme]
name = "reference"
version = "0.1.0"
api_version = 1
stylesheet = "theme.css.tmpl"
print_stylesheet = "print.css.tmpl"
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(
            path.join("theme.css.tmpl"),
            ".zpres-api-v1 { --zpres-color-accent: #176b5b; }\n",
        )
        .unwrap();
        fs::write(
            path.join("print.css.tmpl"),
            ".zpres-api-v1 { --zpres-print-reference: true; }\n",
        )
        .unwrap();
        let manifest = theme::load_theme_manifest(&path.join("theme.toml")).unwrap();
        theme::render_theme(&manifest, &BTreeMap::new()).unwrap()
    }

    fn v1_reference_deck(source_path: &Path) -> Deck {
        parse_source_text(
            r#"---
title: "Theme API v1 reference"
footer: "Renderer-owned footer"
slide_numbers: true
aspect: "16:9"
---

# One semantic title

The primary explanation cites its source.[^source]

[^source]: A compact source note.

::: notes
This note remains outside the visual reading flow.
:::
"#,
            Some(source_path.to_path_buf()),
        )
        .unwrap()
    }

    #[test]
    fn v1_inline_markup_preserves_code_math_and_safe_text() {
        let source = r#"# A **clear** result

Use *emphasis*, **strong and *nested***, _italics_, and variable_name.

`$\invalid$ [^not-a-note] <b>` and \*literal\*.

Read [the **paper**](https://example.org/paper?a=1&b=2), [mail](mailto:author@example.org), [local](detail.html), and [unsafe](javascript:alert(1)).

The bound is _\(x_t\)_ and this stays text: <b>unsafe</b>.
"#;
        let deck = parse_source_text(source, None).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let html = render_debug_html(&deck, &rendered_theme(&deck));
        assert!(html.contains("A <strong>clear</strong> result"));
        assert!(html.contains("<em>emphasis</em>"));
        assert!(html.contains("<strong>strong and <em>nested</em></strong>"));
        assert!(html.contains("<em>italics</em>, and variable_name"));
        assert!(html.contains(r"<code>$\invalid$ [^not-a-note] &lt;b&gt;</code> and *literal*."));
        assert!(html.contains(
            r#"<a href="https://example.org/paper?a=1&amp;b=2">the <strong>paper</strong></a>"#
        ));
        assert!(html.contains(r#"href="mailto:author@example.org""#));
        assert!(html.contains(r#"href="detail.html""#));
        assert!(!html.contains(r#"href="javascript:"#));
        assert!(html.contains("[unsafe](javascript:alert(1))"));
        assert!(html.contains("&lt;b&gt;unsafe&lt;/b&gt;"));
        assert!(html.contains("<em><span class=\"debug-math-inline zpres-math-inline\""));
    }

    #[test]
    fn v1_citation_targets_are_unique_across_regions_slides_and_step_pages() {
        let source = r#"# First[^a-b]

:::: columns
Left column:
A claim[^a_b] and again[^a-b].

Right column:
The same source[^a_b].
::::

::: steps pdf="pages"
1. Establish the claim[^a-b].
2. Recheck it[^a_b].
:::

---

# Second

Repeated on another Slide[^a-b].

[^a-b]: First source.
[^a_b]: Second source.
"#;
        let deck = parse_source_text(source, None).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let theme = rendered_theme(&deck);
        for html in [
            render_debug_html(&deck, &theme),
            render_debug_print_html(&deck, &theme),
        ] {
            let ids = html
                .split(" id=\"")
                .skip(1)
                .filter_map(|part| part.split_once('"').map(|(id, _)| id))
                .filter(|id| id.starts_with("zpres-footnote-"))
                .collect::<Vec<_>>();
            let unique = ids.iter().copied().collect::<BTreeSet<_>>();
            assert!(!ids.is_empty());
            assert_eq!(ids.len(), unique.len(), "duplicate citation IDs: {ids:?}");
            let targets = html
                .split("data-zpres-footnote-target=\"")
                .skip(1)
                .map(|part| part.split_once('"').unwrap().0)
                .collect::<Vec<_>>();
            assert!(targets.len() >= 7);
            assert!(targets.iter().all(|target| unique.contains(target)));
            assert!(!html.contains("A claim[^"));
            assert!(html.contains("value=\"2\">Second source."));
        }
    }

    #[test]
    fn theme_api_v1_renders_explicit_regions_in_reading_order() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = v1_reference_deck(&temp.path().join("talk.zp.md"));

        let html = render_debug_html(&deck, &theme);
        let print_html = render_debug_print_html(&deck, &theme);

        for document in [&html, &print_html] {
            assert!(document.contains("data-zpres-theme-api=\"1\""));
            assert!(document.contains("class=\"zpres-slide-frame zpres-slide-canvas\""));
            assert!(document.contains("class=\"zpres-slide-content\""));
            assert!(document.contains("class=\"zpres-slide-header\""));
            assert!(document.contains("<h1 class=\"zpres-slide-title\" data-zpres-type-role=\"title\">One semantic title</h1>"));
            assert!(document.contains("class=\"zpres-slide-body\""));
            assert!(
                document.contains("class=\"zpres-slide-primary\" data-zpres-type-role=\"body\"")
            );
            assert!(document.contains("class=\"zpres-slide-supporting\" data-zpres-type-role=\"supporting\" aria-hidden=\"true\""));
            assert!(document.contains("class=\"zpres-slide-ornament\" aria-hidden=\"true\""));
            assert!(document.contains("class=\"zpres-slide-sources\" aria-label=\"Sources\""));
            assert!(document.contains("class=\"zpres-slide-footer\""));
            assert_eq!(
                document
                    .matches("<h1 class=\"zpres-slide-title\" data-zpres-type-role=\"title\">One semantic title</h1>")
                    .count(),
                1,
                "the authored title must not be repeated in primary flow"
            );
            assert!(!document.contains("zpres-block-heading\" data-block-type=\"heading\"><span"));
            assert!(!document.contains("<main class=\"zpres-slide-body\""));

            let frame = document
                .find("zpres-slide-frame zpres-slide-canvas")
                .unwrap();
            let content = document[frame..].find("zpres-slide-content").unwrap() + frame;
            let header = document[content..].find("zpres-slide-header").unwrap() + content;
            let body = document[header..].find("zpres-slide-body").unwrap() + header;
            let primary = document[body..].find("zpres-slide-primary").unwrap() + body;
            let sources = document[primary..].find("zpres-slide-sources").unwrap() + primary;
            let content_end = document[sources..].find("            </div>").unwrap() + sources;
            let footer = document[content_end..].find("zpres-slide-footer").unwrap() + content_end;
            assert!(frame < content);
            assert!(content < header);
            assert!(header < body);
            assert!(body < primary);
            assert!(primary < sources);
            assert!(sources < content_end);
            assert!(
                content_end < footer,
                "footer must sit outside autoscaled content"
            );
        }

        let frame_end = html
            .find("          </div>\n            <aside class=\"debug-block")
            .expect("hidden speaker notes should follow the frame");
        assert!(html[frame_end..].contains("zpres-speaker-notes-source"));
        assert!(html[frame_end..].contains(" hidden>"));
        assert_eq!(html.matches("<main").count(), 1);
        assert_eq!(print_html.matches("<main").count(), 0);
        assert!(html.contains("class=\"debug-diagnostics\""));
    }

    #[test]
    fn theme_api_v1_title_paint_is_explicit_and_omitted_splash_stays_absent() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = parse_source_text(
            r#"---
background_image:
  src: assets/field.svg
  title: paint
---

# Painted title
"#,
            Some(temp.path().join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &theme);
        let print_html = render_debug_print_html(&deck, &theme);

        for document in [&html, &print_html] {
            assert!(document.contains("data-background-phase=\"title\""));
            assert_eq!(
                document.matches("class=\"zpres-slide-background\"").count(),
                1
            );
            assert!(!document.contains("data-generated-slide=\"background-image\""));
        }
        assert_eq!(
            print_page_count_for_theme_with_options(&deck, &theme, StaticExportOptions::default(),),
            1
        );
        assert!(render_debug_print_page_html(&deck, &theme, 2).is_none());
    }

    #[test]
    fn theme_api_v1_notes_and_details_precede_the_first_section_splash() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = parse_source_text(
            r#"---
background_image:
  src: assets/field.svg
  splash: true
---

# Title

::: notes
Title notes.
:::

--

## Detail

::: notes
Detail notes.
:::

---

# Later Main
"#,
            Some(temp.path().join("talk.zp.md")),
        )
        .unwrap();

        let print_html = render_debug_print_html_with_options(
            &deck,
            &theme,
            StaticExportOptions {
                include_speaker_notes: true,
            },
        );

        assert!(print_html.contains("data-zpres-page-count=\"6\""));
        let ordered = [
            "data-page=\"1\" data-slide-id=\"section-1-main\"",
            "data-page=\"2\" data-slide-id=\"section-1-main\"",
            "data-page=\"3\" data-slide-id=\"section-1-detail-1\"",
            "data-page=\"4\" data-slide-id=\"section-1-detail-1\"",
            "data-page=\"5\" data-slide-id=\"background-image-splash\"",
            "data-page=\"6\" data-slide-id=\"section-2-main\"",
        ];
        let mut cursor = 0usize;
        for marker in ordered {
            let offset = print_html[cursor..]
                .find(marker)
                .unwrap_or_else(|| panic!("missing ordered print marker {marker}"));
            cursor += offset + marker.len();
        }
        assert_eq!(
            print_html
                .matches("data-generated-slide=\"speaker-notes\"")
                .count(),
            2
        );
    }

    #[test]
    fn theme_api_v1_bundle_uses_only_the_compiled_foundation_and_local_runtime() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = v1_reference_deck(&temp.path().join("talk.zp.md"));
        let bundle = temp.path().join("bundle");

        let publication = write_debug_html_bundle(&deck, &theme, &bundle).unwrap();
        let generation = publication.generation_path;

        let index = fs::read_to_string(generation.join("index.html")).unwrap();
        assert!(index.contains("assets/zpres-theme-api-v1.css"));
        assert!(index.contains("assets/theme.css"));
        assert!(!index.contains("zpres-module-scientific-data"));
        assert!(index.contains("assets/reveal.js"));
        assert!(!index.contains("assets/reveal.css"));
        assert!(!generation.join("assets/reveal.css").exists());
        assert_eq!(
            fs::read(generation.join(theme_api_v1::FOUNDATION_ASSET_PATH)).unwrap(),
            theme_api_v1::FOUNDATION_CSS.as_bytes()
        );
        assert!(generation.join("assets/reveal.js").is_file());
        assert!(generation.join("assets/theme.css").is_file());
        assert!(!index.contains("cdn.tailwindcss.com"));
        assert!(!theme_api_v1::FOUNDATION_CSS.contains("cdn.tailwindcss.com"));
        assert!(!theme_api_v1::FOUNDATION_CSS.contains("@theme"));
        assert!(theme_api_v1::FOUNDATION_CSS.contains("@layer zpres-module"));
        assert!(index.contains("window.zpresFitStage = fitStage"));
        assert!(theme_api_v1::FOUNDATION_CSS.contains(".zpres-speaker-notes-panel"));
        assert!(theme_api_v1::FOUNDATION_CSS.contains(".debug-diagnostics"));

        let (content_type, served) = core_runtime_asset(
            theme_api_v1::FOUNDATION_ASSET_PATH,
            theme::ThemeApiVersion::V1,
        )
        .unwrap();
        assert_eq!(content_type, "text/css; charset=utf-8");
        assert_eq!(served, theme_api_v1::FOUNDATION_CSS);
    }

    #[test]
    fn theme_dependencies_are_bundled_and_static_css_resolves_from_the_theme_root() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("theme # % ü");
        fs::create_dir_all(theme_dir.join("fonts")).unwrap();
        fs::create_dir_all(theme_dir.join("textures")).unwrap();
        let font_dependency = "fonts/reference # % ü.woff2";
        let texture_dependency = "textures/paper ü.svg";
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "asset-reference"
version = "0.1.0"
api_version = 1
fonts = ["fonts/reference # % ü.woff2"]
assets = ["textures/paper ü.svg"]
stylesheet = "theme.css.tmpl"
print_stylesheet = "print.css.tmpl"
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            "@font-face { font-family: 'Reference'; src: URL( 'fonts/reference%20%23%20%25%20%C3%BC.woff2?v=1#face' ); }\n.zpres-slide { background-image: url( textures/paper%20%C3%BC.svg#grain ); mask-image: url(#paper-filter); }\n.zpres-slide::before { content: \"url(ignored.svg)\"; background-image: url(\"data:image/svg+xml,%3Csvg/%3E\"); }\n",
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide { background-image: url('textures/paper ü.svg?print=1'); }\n",
        )
        .unwrap();
        fs::write(theme_dir.join(font_dependency), b"font bytes").unwrap();
        fs::write(
            theme_dir.join(texture_dependency),
            br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#,
        )
        .unwrap();
        let manifest = theme::load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
        let deck = v1_reference_deck(&temp.path().join("talk.zp.md"));
        let bundle = temp.path().join("bundle");

        let publication = write_debug_html_bundle(&deck, &rendered, &bundle).unwrap();
        let generation = publication.generation_path;
        let print_html = render_debug_print_html(&deck, &rendered);

        assert_eq!(
            fs::read(generation.join("assets").join(font_dependency)).unwrap(),
            b"font bytes"
        );
        assert_eq!(
            fs::read(generation.join("assets").join(texture_dependency)).unwrap(),
            br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#
        );
        let bundle_css = fs::read_to_string(generation.join("assets/theme.css")).unwrap();
        assert!(
            bundle_css.contains("url(\"fonts/reference%20%23%20%25%20%C3%BC.woff2?v=1#face\")")
        );
        assert!(bundle_css.contains("url(\"textures/paper%20%C3%BC.svg#grain\")"));
        assert!(bundle_css.contains("url(#paper-filter)"));
        assert!(bundle_css.contains("url(\"data:image/svg+xml,%3Csvg/%3E\")"));
        assert!(bundle_css.contains("content: \"url(ignored.svg)\""));
        assert!(!print_html.contains("<base"));
        assert!(print_html.contains(&format!(
            "url(\"{}?v=1#face\")",
            file_url(&theme_dir.join(font_dependency))
        )));
        assert!(print_html.contains(&format!(
            "url(\"{}?print=1\")",
            file_url(&theme_dir.join(texture_dependency))
        )));
    }

    #[test]
    fn bundle_claims_and_writes_cannot_overwrite_portable_path_aliases() {
        let mut claims = BTreeMap::new();
        claim_html_bundle_path(&mut claims, PathBuf::from("assets/theme.css"), "renderer").unwrap();

        let alias_error =
            claim_html_bundle_path(&mut claims, PathBuf::from("Assets/Theme.CSS"), "dependency")
                .unwrap_err();
        assert!(matches!(alias_error, HtmlError::BundlePathCollision { .. }));

        let prefix_error = claim_html_bundle_path(
            &mut claims,
            PathBuf::from("ASSETS/THEME.CSS/nested.svg"),
            "dependency",
        )
        .unwrap_err();
        assert!(matches!(
            prefix_error,
            HtmlError::BundlePathCollision { .. }
        ));

        let temp = tempdir().unwrap();
        let output = temp.path().join("renderer-owned.css");
        write_file(&output, "first").unwrap();
        let overwrite_error = write_file(&output, "second").unwrap_err();
        assert!(matches!(overwrite_error, HtmlError::Write { .. }));
        assert_eq!(fs::read_to_string(output).unwrap(), "first");
    }

    #[test]
    fn render_validation_rejects_theme_dependencies_at_renderer_paths() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("colliding-theme");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "colliding-theme"
version = "0.1.0"
api_version = 1
assets = ["theme.css"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(theme_dir.join("theme.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("print.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("theme.css"), "user asset").unwrap();
        let manifest = theme::load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let output = temp.path().join("unpublished");

        let error = theme::render_theme(&manifest, &BTreeMap::new()).unwrap_err();

        assert!(matches!(
            &error,
            theme::ThemeError::ReservedDependencyPath {
                dependency,
                asset_path,
                ..
            } if dependency == "theme.css" && asset_path == "assets/theme.css"
        ));
        assert!(error.to_string().contains("renderer-owned asset"));
        assert!(!output.exists());
    }

    #[test]
    fn theme_api_v1_bundle_rejects_unsupported_deck_before_creating_output() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = parse_source_text(
            "---\naspect: \"4:3\"\n---\n\n# Unsupported aspect\n\nBody\n",
            Some(temp.path().join("four-three.zp.md")),
        )
        .unwrap();
        let output = temp.path().join("unpublished").join("html");

        let error = write_debug_html_bundle(&deck, &theme, &output).unwrap_err();

        assert!(matches!(error, HtmlError::ThemeContract(_)));
        assert!(error.to_string().contains("supports only 16:9"));
        assert!(!output.exists());
    }

    #[test]
    fn theme_api_v1_stage_fits_and_letterboxes_smaller_screen_viewports() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = v1_reference_deck(&temp.path().join("talk.zp.md"));
        let bundle = temp.path().join("bundle");
        write_debug_html_bundle(&deck, &theme, &bundle).unwrap();
        let url = format!(
            "{}?zpres-visual-review=1",
            file_url(&bundle.join("index.html"))
        );
        let mut browser = match ChromiumSession::launch(ChromiumSessionOptions::default()) {
            Ok(browser) => browser,
            Err(ChromiumError::MissingChromium { .. }) => {
                eprintln!("skipping v1 stage-fit test: Chrome/Chromium was not found");
                return;
            }
            Err(error) => panic!("failed to launch Chromium for v1 stage-fit test: {error}"),
        };

        for (viewport, expected_y) in [
            (
                PngViewport {
                    width: 800,
                    height: 450,
                },
                0.0,
            ),
            (
                PngViewport {
                    width: 800,
                    height: 600,
                },
                75.0,
            ),
        ] {
            browser.load_screen_document(&url, viewport).unwrap();
            let readiness = browser
                .await_static_readiness(STATIC_READINESS_TIMEOUT)
                .unwrap();
            assert!(
                readiness.ready,
                "v1 stage-fit readiness failed: {readiness:?}"
            );
            let route = browser.screen_routes().unwrap().remove(0);
            let observation = browser.capture_screen_route(&route).unwrap().observation;
            let slide = observation.slide_bounds.unwrap();
            let canvas = observation.canvas_bounds.unwrap();
            assert!(
                (slide.x - 0.0).abs() <= 0.25,
                "unexpected slide x: {slide:?}"
            );
            assert!(
                (slide.y - expected_y).abs() <= 0.25,
                "unexpected slide y: {slide:?}"
            );
            assert!(
                (slide.width - 800.0).abs() <= 0.25,
                "unexpected slide width: {slide:?}"
            );
            assert!(
                (slide.height - 450.0).abs() <= 0.25,
                "unexpected slide height: {slide:?}"
            );
            assert_eq!(canvas, slide);
            assert_eq!(observation.document_scroll.unwrap().maximum(), 0.0);
        }
    }

    #[test]
    fn retired_generation_reload_reaches_the_current_html_presentation() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let first_deck = v1_reference_deck(&temp.path().join("first.zp.md"));
        let output = temp.path().join("dist");
        let first = write_debug_html_bundle(&first_deck, &theme, &output).unwrap();
        let root_url = format!(
            "{}?zpres-visual-review=1#/0/0",
            file_url(&output.join("index.html"))
        );
        let mut browser = match ChromiumSession::launch(ChromiumSessionOptions::default()) {
            Ok(browser) => browser,
            Err(ChromiumError::MissingChromium { .. }) => {
                eprintln!("skipping retired-generation reload test: Chrome/Chromium was not found");
                return;
            }
            Err(error) => panic!("failed to launch Chromium for publication test: {error}"),
        };
        browser
            .load_screen_document(&root_url, PngViewport::default())
            .unwrap();
        assert!(
            browser
                .await_static_readiness(STATIC_READINESS_TIMEOUT)
                .unwrap()
                .ready
        );
        assert_eq!(browser.screen_routes().unwrap().len(), 1);

        let second_deck = parse_source_text(
            "---\naspect: \"16:9\"\n---\n\n# First section\n\nFirst.\n\n---\n\n# Second section\n\nSecond.\n",
            Some(temp.path().join("second.zp.md")),
        )
        .unwrap();
        let second = write_debug_html_bundle(&second_deck, &theme, &output).unwrap();
        assert!(
            fs::read_to_string(first.generation_path.join("index.html"))
                .unwrap()
                .contains("zpres-retired-generation")
        );

        browser.reload_screen_document().unwrap();
        let readiness = browser
            .await_static_readiness(STATIC_READINESS_TIMEOUT)
            .unwrap();
        assert!(
            readiness.ready,
            "retired-generation reload did not reach the current presentation: {readiness:?}"
        );
        assert_eq!(browser.screen_routes().unwrap().len(), 2);
        assert_eq!(
            current_html_bundle_publication(&output)
                .unwrap()
                .unwrap()
                .generation,
            second.generation
        );
    }

    #[test]
    fn html_renderer_does_not_embed_theme_specific_css_constants() {
        let source = include_str!("html.rs");
        let forbidden = [
            ["SCIENCE", "_CSS"].concat(),
            ["PAPER_CHALK", "_CSS"].concat(),
            ["DARK_SPLASH", "_CSS"].concat(),
            ["WEDDING", "_CSS"].concat(),
            ["DEBUG", "_CSS"].concat(),
            ["theme", "_css_name"].concat(),
            ["theme", "_identity"].concat(),
        ];
        for pattern in forbidden {
            assert!(
                !source.contains(&pattern),
                "src/html.rs should not contain {pattern}"
            );
        }
    }

    #[test]
    fn renders_debug_html_without_remote_assets() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("class=\"reveal zpres-presentation\""));
        assert!(html.contains("zpres-theme-science"));
        assert!(html.contains("zpres-section-stack"));
        assert!(html.contains("zpres-slide"));
        assert!(html.contains("zpres-slide-frame zpres-slide-canvas"));
        assert!(html.contains("data-slide-id=\"section-8-detail-1\""));
        assert_eq!(html.matches("data-slide-role=\"detail\"").count(), 2);
        assert!(html.contains("data-slide-id=\"section-8-detail-2\""));
        assert!(html.contains("data-block-type=\"speaker-notes\""));
        assert!(html.contains("data-block-type=\"html-only\""));
        assert!(html.contains("data-block-type=\"math\""));
        assert!(html.contains("data-block-type=\"code\""));
        assert!(html.contains("data-block-type=\"table\""));
        assert!(html.contains("data-block-type=\"figure\""));
        assert!(html.contains("data-block-type=\"chart\""));
        assert!(html.contains("data-block-type=\"layout\""));
        assert!(html.contains("data-layout-kind=\"columns\""));
        assert!(html.contains("data-layout-widths=\"40/60\""));
        assert!(html.contains("data-slide-variant=\"comparison\""));
        assert!(html.contains("data-block-type=\"steps\""));
        assert!(html.contains("data-step-count=\"3\""));
        assert!(html.contains("data-step-pdf-policy=\"final-state\""));
        assert!(html.contains("zpres-block-steps"));
        assert!(html.contains("class=\"debug-step zpres-step fragment\""));
        assert!(REVEAL_JS.contains("is-step-gated"));
        assert!(html.contains("debug-chart-svg zpres-chart-svg"));
        assert!(html.contains("assets/phase-space.svg"));
        assert!(html.contains("A small synthetic phase-space sketch"));
        assert!(html.contains("<math"));
        assert!(html.contains("syntax-keyword"));
        assert!(!html.contains("unsupported directive: steps"));
        assert!(!html.contains("unsupported-until-issue-0012"));
        assert!(!html.contains("src=\"https://"));
        assert!(!html.contains("src=\"http://"));
        assert!(!html.contains("href=\"https://"));
        assert!(!html.contains("href=\"http://"));
    }

    #[test]
    fn live_runtime_exposes_dom_derived_strict_navigation() {
        assert!(REVEAL_JS.contains("window.zpresPresentation = Object.freeze"));
        assert!(REVEAL_JS.contains("routes,"));
        assert!(REVEAL_JS.contains("navigate,"));
        assert!(REVEAL_JS.contains("current,"));
        assert!(REVEAL_JS.contains("get ready()"));
        assert!(REVEAL_JS.contains("stacks.flatMap((stack, section) =>"));
        assert!(REVEAL_JS.contains("Array.from({ length: stepCount + 1 }, (_, step) =>"));
        assert!(
            REVEAL_JS.contains("const stepSuffix = route.step > 0 ? \"/\" + route.step : \"\"")
        );
        assert!(REVEAL_JS.contains("throw new RangeError"));
        let v1_runtime = theme_runtime_js(theme::ThemeApiVersion::V1);
        assert!(v1_runtime.contains("a[href], button, input, textarea, select"));
        assert!(v1_runtime.contains("if (handled) event.preventDefault();"));
        assert!(v1_runtime.contains("step.setAttribute(\"aria-current\", \"step\")"));
        assert!(!REVEAL_JS.contains("function clampState()"));
    }

    #[test]
    fn live_runtime_settles_the_production_state_before_resolving_navigation() {
        assert!(REVEAL_JS.contains("applyFragments();"));
        assert!(REVEAL_JS.contains("history.replaceState(null, \"\", routeHash(state))"));
        assert!(REVEAL_JS.contains("await Promise.resolve(window.zpresAutoscaleAll())"));
        assert!(REVEAL_JS.contains("root.getAnimations({ subtree: true })"));
        assert!(REVEAL_JS.contains("finiteAnimationsFor(activeSlide)"));
        assert!(REVEAL_JS.contains("--zpres-retain-leaving-slide"));
        assert!(REVEAL_JS.contains("previousActive.setAttribute(\"aria-hidden\", \"true\")"));
        assert!(REVEAL_JS.contains("previousActive.setAttribute(\"inert\", \"\")"));
        assert!(REVEAL_JS.contains("routeDirection(previousRoute, nextRoute)"));
        assert!(REVEAL_JS.contains("clearStaleLeavingSlides()"));
        assert!(REVEAL_JS.contains("...finiteAnimationsFor(previousActive)"));
        assert!(REVEAL_JS.contains("await twoFrames()"));
        assert!(!REVEAL_JS.contains(
            "window.setTimeout(() => activeSlide?.classList.remove(\"is-entering\"), 340)"
        ));
    }

    #[test]
    fn chart_data_errors_reach_source_diagnostics_and_static_readiness() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("chart.zp.md");
        let data = temp.path().join("points.csv");
        fs::write(&source, r#"# Chart

::: vega-lite
{"data":{"url":"points.csv"},"mark":"line","encoding":{"x":{"field":"x","title":"Input size"},"y":{"field":"y","title":"Runtime"}}}
:::
"#).unwrap();
        fs::write(&data, "x,y\n1,2\n2,4\n").unwrap();
        let valid = crate::deck::parse_source_file(&source).unwrap();
        assert!(
            !valid
                .diagnostics
                .iter()
                .any(crate::deck::Diagnostic::is_fatal)
        );
        let screen = render_debug_html(&valid, &rendered_theme(&valid));
        assert!(screen.contains("Input size</text>"));
        assert!(screen.contains("Runtime</text>"));
        crate::pdf::check_pdf_readiness(&valid).unwrap();

        // A dependency edited after parsing must also fail static readiness.
        fs::write(&data, "x,y\n1,2\n2,broken\n").unwrap();
        let output = temp.path().join("output");
        let build_error =
            write_debug_html_bundle(&valid, &rendered_theme(&valid), &output).unwrap_err();
        assert!(
            build_error
                .to_string()
                .contains("data row 2, field 'y' must be a finite number")
        );
        assert!(!output.exists());
        let error = crate::pdf::check_pdf_readiness(&valid).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("data row 2, field 'y' must be a finite number")
        );
        let invalid = crate::deck::parse_source_file(&source).unwrap();
        assert!(invalid.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("data row 2, field 'y' must be a finite number")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
        }));
        let screen = render_debug_html(&invalid, &rendered_theme(&invalid));
        assert!(screen.contains("data-zpres-unresolved=\"chart\""));
        assert!(!screen.contains("class=\"zpres-chart-line\""));
    }

    #[test]
    fn unsupported_chart_does_not_fall_back_to_json_placeholder() {
        let source = r#"# Chart

::: vega-lite
{ "data": { "url": "data/runtime.csv" }, "mark": "bar", "encoding": { "x": { "field": "n" }, "y": { "field": "ms" } } }
:::
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-zpres-unresolved=\"chart\""));
        assert!(!html.contains("<pre>{"));
        assert!(!html.contains("&quot;mark&quot;"));
    }

    #[test]
    fn renders_footnotes_with_superscript_refs_and_theme_hooks() {
        let source = r#"# Footnotes

A claim with a source[^paper] and math[^math].

[^paper]: Source details.
[^math]: A note with \(x_t\).
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("class=\"zpres-footnote-ref\""));
        assert!(
            html.contains("data-zpres-footnote-target=\"zpres-footnote-section-1-main-live-1\"")
        );
        assert!(print_html.contains("href=\"#zpres-footnote-section-1-main-print-final-1\""));
        assert!(html.contains("zpres-block-footnotes"));
        assert!(html.contains("data-block-type=\"footnotes\""));
        assert!(html.contains("data-footnote-label=\"math\""));
        assert!(html.contains("<math"));
        assert!(print_html.contains("zpres-block-footnotes"));
    }

    #[test]
    fn renders_mermaid_flowchart_as_static_svg() {
        let source = r#"# Diagram

```mermaid
flowchart LR
  A[Markdown source] -->|parse| B[Typed Deck model]
  B -->|apply Theme hook| C[Theme-aware surface]
  C -->|export| D[Static Output target]
```
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"diagram\""));
        assert!(html.contains("data-diagram-language=\"mermaid\""));
        assert!(html.contains("debug-diagram-svg zpres-diagram-svg"));
        assert!(html.contains("class=\"zpres-diagram-node\""));
        assert!(html.contains(">Static Output</tspan>"));
        assert!(html.contains(">target</tspan>"));
        assert!(html.contains(">apply</tspan>"));
        assert!(html.contains(">Theme hook</tspan>"));
        assert!(html.contains("<title>Mermaid flowchart, left to right</title>"));
        assert!(html.contains(
            "<desc>Nodes in source order: Node A: ‘Markdown source’. Node B: ‘Typed Deck model’. Node C: ‘Theme-aware surface’. Node D: ‘Static Output target’. Edges in source order: Edge A to B, label: ‘parse’. Edge B to C, label: ‘apply Theme hook’. Edge C to D, label: ‘export’.</desc>"
        ));
        assert!(
            html.contains(
                "aria-hidden=\"true\" data-zpres-authored-label=\"Static Output target\""
            )
        );
        assert!(
            html.contains("aria-hidden=\"true\" data-zpres-authored-label=\"apply Theme hook\"")
        );
        assert_eq!(html.matches("data-zpres-mermaid-arrow=").count(), 3);
        assert!(!html.contains("marker-end=\"url(#zpres-mermaid-arrow)\""));
        assert!(!html.contains("id=\"zpres-mermaid-arrow\""));
        assert!(html.contains("</tspan>\n<tspan"));
        assert_eq!(
            html.matches("text-anchor=\"middle\"").count(),
            7,
            "every Mermaid node and edge label must use its authored center coordinate"
        );
        assert!(print_html.contains("data-block-type=\"diagram\""));
    }

    #[test]
    fn adaptive_mermaid_layout_clears_wide_labels_and_branches() {
        let flowchart = parse_mermaid_flowchart(
            r#"flowchart LR
  A[WWWWWWWWWWWW] --> B[Branch one]
  A -->|branch label WWWWWWWW| C[Branch two]
  B --> D[Converged result]
  C --> D
  A -->|long shortcut WWWWWWWWWWWW| D"#,
        )
        .unwrap();
        let nodes = mermaid_flowchart_nodes(&flowchart);
        let node_layouts = mermaid_node_layouts(&nodes, true);
        let edge_layouts = mermaid_edge_layouts(&flowchart, true);
        let layout = mermaid_positioned_layout(&flowchart, &nodes, &node_layouts, &edge_layouts);
        let node_rects = mermaid_node_rects(&layout.positions, &node_layouts);

        assert!(
            node_layouts["A"].box_width - node_layouts["A"].text_width >= 23.9,
            "W-heavy Technical text must retain at least 12px clearance on both sides"
        );
        assert!(
            (layout.positions["B"].0 - layout.positions["C"].0).abs() < 0.1,
            "branch targets should share a topology layer rather than occupying the edge-label midpoint"
        );
        let mut label_rects = Vec::new();
        for (edge_index, edge) in flowchart.edges.iter().enumerate() {
            let Some(label) = edge_layouts[edge_index].as_ref() else {
                continue;
            };
            let center = layout.edges[edge_index].label_center.unwrap_or_else(|| {
                panic!(
                    "missing label center for {} -> {}",
                    edge.from.id, edge.to.id
                )
            });
            let label_rect =
                MermaidRect::from_center(center.0, center.1, label.text_width, label.box_height);
            assert!(label_rect.left >= 0.0 && label_rect.right <= layout.width);
            assert!(label_rect.top >= 0.0 && label_rect.bottom <= layout.height);
            for (node_id, node_rect) in &node_rects {
                assert!(
                    !label_rect.intersects(*node_rect, 7.9),
                    "edge {} -> {} label entered node {node_id}: {label_rect:?} vs {node_rect:?}",
                    edge.from.id,
                    edge.to.id
                );
            }
            for (path_index, geometry) in layout.edges.iter().enumerate() {
                for segment in geometry.points.windows(2) {
                    assert!(
                        !mermaid_segment_intersects_rect(
                            segment[0],
                            segment[1],
                            label_rect,
                            MERMAID_LABEL_CLEARANCE - 0.1,
                        ),
                        "edge {edge_index} label entered edge {path_index} path field: {label_rect:?} vs {segment:?}"
                    );
                }
            }
            for (prior_index, prior_rect) in &label_rects {
                assert!(
                    !label_rect.intersects(*prior_rect, 7.9),
                    "edge {edge_index} label entered edge {prior_index} label field: {label_rect:?} vs {prior_rect:?}"
                );
            }
            label_rects.push((edge_index, label_rect));
        }
        assert!(
            layout.edges[4].points.len() > 2,
            "the non-adjacent shortcut must route outside intermediate branch nodes"
        );
        for point in &layout.edges[4].points {
            assert!((0.0..=layout.width).contains(&point.0));
            assert!((0.0..=layout.height).contains(&point.1));
        }
    }

    #[test]
    fn dark_splash_adaptive_mermaid_labels_clear_screen_and_print_geometry() {
        let temp = tempdir().unwrap();
        let deck = parse_source_text(
            r#"---
theme: dark-splash
aspect: "16:9"
---

# Branch clearance

```mermaid
flowchart LR
  A[WWWWWWWWWWWW] --> B[Branch one]
  A -->|branch label WWWWWWWW| C[Branch two]
  B --> D[Converged result]
  C --> D
  A -->|long shortcut WWWWWWWWWWWW| D
```

---

# Later Main slide keeps its arrow

```mermaid
flowchart LR
  A[Family 👩‍👩‍👧‍👦 status] -->|keeps authored semantics| B[Ready]
```
"#,
            Some(temp.path().join("dark-splash-branches.zp.md")),
        )
        .unwrap();
        let theme = rendered_theme(&deck);
        let bundle = temp.path().join("bundle");
        write_debug_html_bundle(&deck, &theme, &bundle).unwrap();
        let print_path = temp.path().join("print.html");
        fs::write(&print_path, render_debug_print_html(&deck, &theme)).unwrap();
        let mut browser = match ChromiumSession::launch(ChromiumSessionOptions::default()) {
            Ok(browser) => browser,
            Err(ChromiumError::MissingChromium { .. }) => {
                eprintln!(
                    "skipping Dark Splash Diagram geometry test: Chrome/Chromium was not found"
                );
                return;
            }
            Err(error) => panic!("failed to launch Chromium for Diagram geometry test: {error}"),
        };

        let screen_url = format!(
            "{}?zpres-visual-review=1#/0/0",
            file_url(&bundle.join("index.html"))
        );
        browser
            .load_screen_document(&screen_url, PngViewport::default())
            .unwrap();
        let screen_readiness = browser
            .await_static_readiness(STATIC_READINESS_TIMEOUT)
            .unwrap();
        assert!(
            screen_readiness.ready,
            "Dark Splash screen Diagram was not ready: {screen_readiness:?}"
        );
        let routes = browser.screen_routes().unwrap();
        assert_eq!(routes.len(), 2);
        for route in &routes {
            let capture = browser.capture_screen_route(route).unwrap();
            let selector = format!(".zpres-slide[data-slide-id=\"{}\"]", route.slide_id);
            let surface = format!("screen {}", route.slide_id);
            assert_mermaid_arrow_raster(
                &surface,
                &browser_mermaid_arrows(&mut browser, &selector),
                &capture.png,
            );
            if route.slide_id == "section-1-main" {
                assert_clear_mermaid_browser_geometry(
                    &surface,
                    &browser_mermaid_geometry(&mut browser, &selector),
                );
            }
        }

        browser
            .load_print_document(&file_url(&print_path), PngViewport::default())
            .unwrap();
        let print_readiness = browser
            .await_static_readiness(STATIC_READINESS_TIMEOUT)
            .unwrap();
        assert!(
            print_readiness.ready,
            "Dark Splash print Diagram was not ready: {print_readiness:?}"
        );
        let pages = browser.print_pages().unwrap();
        assert_eq!(pages.len(), 2);
        for page in &pages {
            let capture = browser.capture_print_page(page.index).unwrap();
            let page_number = page.page.unwrap();
            let selector = format!(".zpres-print-slide[data-page=\"{page_number}\"]");
            let surface = format!("print page {page_number}");
            assert_mermaid_arrow_raster(
                &surface,
                &browser_mermaid_arrows(&mut browser, &selector),
                &capture.png,
            );
            if page_number == 1 {
                assert_clear_mermaid_browser_geometry(
                    &surface,
                    &browser_mermaid_geometry(&mut browser, &selector),
                );
            }
        }
    }

    #[test]
    fn adaptive_mermaid_layout_supports_every_authored_direction() {
        for direction in ["TD", "TB", "BT", "LR", "RL"] {
            let accessible_direction = match direction {
                "TD" | "TB" => "top to bottom",
                "BT" => "bottom to top",
                "LR" => "left to right",
                "RL" => "right to left",
                _ => unreachable!(),
            };
            let source = format!(
                "flowchart {direction}\n  A[Origin] -->|long adjacent authored label WWWWWWWW| B[Middle]\n  B --> C[Destination]\n  A -->|long non-direct authored label WWWWWWWW| C"
            );
            let flowchart = parse_mermaid_flowchart(&source).unwrap();
            let svg = render_mermaid_flowchart_svg(&flowchart);
            assert!(
                svg.contains(&format!(
                    "<title>Mermaid flowchart, {accessible_direction}</title>"
                )),
                "{direction} omitted its authored direction from the accessible name"
            );
            let nodes = mermaid_flowchart_nodes(&flowchart);
            let node_layouts = mermaid_node_layouts(&nodes, true);
            let edge_layouts = mermaid_edge_layouts(&flowchart, true);
            let layout =
                mermaid_positioned_layout(&flowchart, &nodes, &node_layouts, &edge_layouts);
            let node_rects = mermaid_node_rects(&layout.positions, &node_layouts);
            let adjacent = &layout.edges[0];
            let adjacent_label = edge_layouts[0].as_ref().unwrap();
            let adjacent_center = adjacent.label_center.unwrap();
            let adjacent_rect = MermaidRect::from_center(
                adjacent_center.0,
                adjacent_center.1,
                adjacent_label.text_width,
                adjacent_label.box_height,
            );
            let shortcut = &layout.edges[2];
            let label = edge_layouts[2].as_ref().unwrap();
            let center = shortcut.label_center.unwrap();
            let label_rect =
                MermaidRect::from_center(center.0, center.1, label.text_width, label.box_height);

            assert!(adjacent_rect.left >= 0.0 && adjacent_rect.right <= layout.width);
            assert!(adjacent_rect.top >= 0.0 && adjacent_rect.bottom <= layout.height);
            let adjacent_start = adjacent.points[0];
            let adjacent_end = adjacent.points[1];
            if matches!(direction, "LR" | "RL") {
                let edge_y = (adjacent_start.1 + adjacent_end.1) / 2.0;
                assert!(
                    adjacent_rect.bottom <= edge_y - MERMAID_LABEL_CLEARANCE + 0.1
                        || adjacent_rect.top >= edge_y + MERMAID_LABEL_CLEARANCE - 0.1,
                    "{direction} adjacent label remained in the edge's immediate visual field"
                );
            } else {
                let edge_x = (adjacent_start.0 + adjacent_end.0) / 2.0;
                assert!(
                    adjacent_rect.left >= edge_x + MERMAID_LABEL_CLEARANCE - 0.1
                        || adjacent_rect.right <= edge_x - MERMAID_LABEL_CLEARANCE + 0.1,
                    "{direction} adjacent label remained in the edge's immediate visual field"
                );
            }

            assert!(
                shortcut.points.len() > 2,
                "{direction} shortcut was not routed"
            );
            let final_segment = &shortcut.points[shortcut.points.len() - 2..];
            let final_dx = final_segment[1].0 - final_segment[0].0;
            let final_dy = final_segment[1].1 - final_segment[0].1;
            match direction {
                "TD" | "TB" => assert!(
                    final_dy > 0.0 && final_dx.abs() < 0.1,
                    "{direction} shortcut arrow did not enter the target through its top side"
                ),
                "BT" => assert!(
                    final_dy < 0.0 && final_dx.abs() < 0.1,
                    "BT shortcut arrow did not enter the target through its bottom side"
                ),
                "LR" => assert!(
                    final_dx > 0.0 && final_dy.abs() < 0.1,
                    "LR shortcut arrow did not enter the target through its left side"
                ),
                "RL" => assert!(
                    final_dx < 0.0 && final_dy.abs() < 0.1,
                    "RL shortcut arrow did not enter the target through its right side"
                ),
                _ => unreachable!(),
            }
            assert!(label_rect.left >= 0.0 && label_rect.right <= layout.width);
            assert!(label_rect.top >= 0.0 && label_rect.bottom <= layout.height);
            for (node_id, node_rect) in &node_rects {
                assert!(
                    !label_rect.intersects(*node_rect, 7.9),
                    "{direction} shortcut label entered node {node_id}: {label_rect:?} vs {node_rect:?}"
                );
            }
            for segment in shortcut.points.windows(2) {
                let segment_rect = MermaidRect {
                    left: segment[0].0.min(segment[1].0) - 1.0,
                    top: segment[0].1.min(segment[1].1) - 1.0,
                    right: segment[0].0.max(segment[1].0) + 1.0,
                    bottom: segment[0].1.max(segment[1].1) + 1.0,
                };
                for (node_id, node_rect) in &node_rects {
                    let is_endpoint = node_id == "A" || node_id == "C";
                    assert!(
                        is_endpoint || !segment_rect.intersects(*node_rect, 0.0),
                        "{direction} routed shortcut crossed unrelated node {node_id}"
                    );
                }
            }
            let (origin_x, origin_y) = layout.positions["A"];
            let (destination_x, destination_y) = layout.positions["C"];
            match direction {
                "TD" | "TB" => assert!(origin_y < destination_y),
                "BT" => assert!(origin_y > destination_y),
                "LR" => assert!(origin_x < destination_x),
                "RL" => assert!(origin_x > destination_x),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn adaptive_mermaid_labels_preserve_authored_accessible_semantics() {
        let flowchart = parse_mermaid_flowchart(
            "flowchart LR\n  A[Family & 👩‍👩‍👧‍👦  <status>] -->|keeps & <authored>  spacing| B[Ready]",
        )
        .unwrap();
        let nodes = mermaid_flowchart_nodes(&flowchart);
        let description = mermaid_accessible_description(&flowchart, &nodes);
        let svg = render_mermaid_flowchart_svg(&flowchart);

        assert_eq!(
            description,
            "Nodes in source order: Node A: ‘Family & 👩‍👩‍👧‍👦  <status>’. Node B: ‘Ready’. Edges in source order: Edge A to B, label: ‘keeps & <authored>  spacing’."
        );
        assert!(svg.contains(&format!("<desc>{}</desc>", escape_html(&description))));
        assert!(!svg.contains("aria-label="));
        assert_eq!(
            svg.matches("aria-hidden=\"true\" data-zpres-authored-label=")
                .count(),
            3
        );
        assert_eq!(svg.matches("data-zpres-authored-label=").count(), 3);
        assert_eq!(svg.matches("data-zpres-mermaid-arrow=").count(), 1);
        assert!(svg.contains("data-zpres-mermaid-arrow=\"A:B\" aria-hidden=\"true\""));
        assert!(svg.contains("data-zpres-authored-label=\"Family &amp; 👩‍👩‍👧‍👦  &lt;status&gt;\""));
        assert!(
            svg.contains("data-zpres-authored-label=\"keeps &amp; &lt;authored&gt;  spacing\"")
        );
        assert_eq!(description.matches("Family & 👩‍👩‍👧‍👦  <status>").count(), 1);
        assert_eq!(
            description.matches("keeps & <authored>  spacing").count(),
            1
        );
        assert_eq!(svg.matches("👩‍👩‍👧‍👦").count(), 3);
        assert!(!wrap_mermaid_label("👩‍👩‍👧‍👦").iter().any(|line| line.is_empty()));
        assert_eq!(wrap_mermaid_label("👩‍👩‍👧‍👦"), vec!["👩‍👩‍👧‍👦"]);
    }

    #[test]
    fn adaptive_mermaid_exposes_one_outer_accessible_image_without_authored_text_children() {
        let temp = tempdir().unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = parse_source_text(
            r#"---
aspect: "16:9"
---

# Accessible Diagram

```mermaid
flowchart LR
  A[WWWWWWWW family 👩‍👩‍👧‍👦 status] -->|keeps authored semantics| B[Ready]
```
"#,
            Some(temp.path().join("accessible-diagram.zp.md")),
        )
        .unwrap();
        let bundle = temp.path().join("bundle");
        write_debug_html_bundle(&deck, &theme, &bundle).unwrap();
        let url = format!(
            "{}?zpres-visual-review=1#/0/0",
            file_url(&bundle.join("index.html"))
        );
        let mut browser = match ChromiumSession::launch(ChromiumSessionOptions::default()) {
            Ok(browser) => browser,
            Err(ChromiumError::MissingChromium { .. }) => {
                eprintln!(
                    "skipping Mermaid accessibility-tree test: Chrome/Chromium was not found"
                );
                return;
            }
            Err(error) => panic!("failed to launch Chromium for Mermaid AX test: {error}"),
        };
        browser
            .load_screen_document(&url, PngViewport::default())
            .unwrap();
        let readiness = browser
            .await_static_readiness(STATIC_READINESS_TIMEOUT)
            .unwrap();
        assert!(
            readiness.ready,
            "Mermaid AX fixture was not ready: {readiness:?}"
        );

        let tree = browser.full_accessibility_tree().unwrap();
        let nodes = tree["nodes"].as_array().expect("AX tree nodes");
        let image_nodes = nodes
            .iter()
            .filter(|node| node["role"]["value"] == "image")
            .collect::<Vec<_>>();
        assert_eq!(
            image_nodes.len(),
            1,
            "the Diagram must expose exactly one accessible image node: {tree}"
        );
        let image = image_nodes[0];
        assert_eq!(image["name"]["value"], "Mermaid flowchart, left to right");
        assert_eq!(
            image["description"]["value"],
            "Nodes in source order: Node A: ‘WWWWWWWW family 👩‍👩‍👧‍👦 status’. Node B: ‘Ready’. Edges in source order: Edge A to B, label: ‘keeps authored semantics’."
        );

        let authored_text_children = nodes
            .iter()
            .filter(|node| {
                let role = node["role"]["value"].as_str().unwrap_or_default();
                let name = node["name"]["value"].as_str().unwrap_or_default();
                matches!(role, "generic" | "StaticText" | "InlineTextBox")
                    && [
                        "WWWWWWWW family 👩‍👩‍👧‍👦 status",
                        "Ready",
                        "keeps authored semantics",
                    ]
                    .iter()
                    .any(|authored| name.contains(authored))
            })
            .collect::<Vec<_>>();
        assert!(
            authored_text_children.is_empty(),
            "visual Diagram labels must not duplicate the outer image semantics: {authored_text_children:?}"
        );
    }

    #[test]
    fn renders_quote_blocks_with_semantic_theme_hooks() {
        let source = r#"# Quote

> Search is where the model becomes operational: \(x_{t+1}=f(x_t)\).
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"quote\""));
        assert!(html.contains("zpres-block-quote"));
        assert!(html.contains("<blockquote><p>Search is where the model becomes operational:"));
        assert!(html.contains("<math"));
    }

    #[test]
    fn renders_callout_blocks_with_kind_hooks() {
        let source = r#"# Callout

> [!IMPORTANT] Solver warning
> This changes the bound \(z\).
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"callout\""));
        assert!(html.contains("data-callout-kind=\"important\""));
        assert!(html.contains("zpres-block-callout"));
        assert!(html.contains("Solver warning"));
        assert!(html.contains("<math"));
    }

    #[test]
    fn renders_list_blocks_with_semantic_theme_hooks() {
        let source = r#"# Lists

- Model the state \(x_t\).
- Apply the branching rule.

1. Build the relaxation.
2. Check the bound.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"list\""));
        assert!(html.contains("data-list-kind=\"unordered\""));
        assert!(html.contains("data-list-kind=\"ordered\""));
        assert!(html.contains("zpres-block-list"));
        assert!(html.contains("<ul><li>Model the state"));
        assert!(html.contains("<ol><li>Build the relaxation."));
        assert!(html.contains("<math"));
    }

    #[test]
    fn renders_asterisk_lists_as_live_fragments_and_static_final_state() {
        let source = r#"# Fragmented

* First point.
* Second point.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-list-reveal=\"fragments\""));
        assert!(html.contains("<li class=\"fragment\" data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>"));
        assert!(html.contains("<li class=\"fragment\" data-step-index=\"2\" data-fragment-index=\"2\">Second point.</li>"));
        assert!(REVEAL_JS.contains(".zpres-block-list li.fragment"));
        assert!(print_html.contains("data-list-reveal=\"fragments\""));
        assert!(
            print_html
                .contains("<li data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>")
        );
        assert!(!print_html.contains("class=\"fragment\""));
    }

    #[test]
    fn renders_deckset_build_lists_command_as_live_fragments() {
        let source = r#"build-lists: true

# Built lists

- First point.
- Second point.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-list-reveal=\"fragments\""));
        assert!(html.contains(
            "<li class=\"fragment\" data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>"
        ));
        assert!(html.contains(
            "<li class=\"fragment\" data-step-index=\"2\" data-fragment-index=\"2\">Second point.</li>"
        ));
        assert!(print_html.contains("data-list-reveal=\"fragments\""));
        assert!(
            print_html
                .contains("<li data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>")
        );
        assert!(!print_html.contains("class=\"fragment\""));
        assert!(!html.contains("build-lists: true"));
    }

    #[test]
    fn renders_deckset_build_lists_slide_overrides() {
        let source = r#"build-lists: true

# Global build

- First point.
- Second point.

---

# Opt out

[.build-lists: false]

- Static point.
- Another static point.

---

# Opt back in

[.build-lists: all]

1. First ordered point.
2. Second ordered point.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        let global_slide = html
            .split("data-slide-id=\"section-1-main\"")
            .nth(1)
            .unwrap();
        let opt_out_slide = html
            .split("data-slide-id=\"section-2-main\"")
            .nth(1)
            .unwrap()
            .split("data-slide-id=\"section-3-main\"")
            .next()
            .unwrap();
        let opt_in_slide = html
            .split("data-slide-id=\"section-3-main\"")
            .nth(1)
            .unwrap();

        assert!(global_slide.contains("data-list-reveal=\"fragments\""));
        assert!(global_slide.contains("class=\"fragment\""));
        assert!(!opt_out_slide.contains("data-list-reveal=\"fragments\""));
        assert!(!opt_out_slide.contains("class=\"fragment\""));
        assert!(opt_in_slide.contains("data-list-kind=\"ordered\""));
        assert!(opt_in_slide.contains("data-list-reveal=\"fragments\""));
        assert!(opt_in_slide.contains("class=\"fragment\""));
    }

    #[test]
    fn renders_deckset_build_lists_not_first_mode() {
        let source = r#"build-lists: notFirst

# Built lists

- Already visible.
- First reveal.
- Second reveal.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-list-reveal=\"fragments\""));
        assert!(html.contains(
            "<ul><li>Already visible.</li><li class=\"fragment\" data-step-index=\"1\" data-fragment-index=\"1\">First reveal.</li><li class=\"fragment\" data-step-index=\"2\" data-fragment-index=\"2\">Second reveal.</li></ul>"
        ));
        assert!(print_html.contains(
            "<ul><li>Already visible.</li><li data-step-index=\"1\" data-fragment-index=\"1\">First reveal.</li><li data-step-index=\"2\" data-fragment-index=\"2\">Second reveal.</li></ul>"
        ));
        assert!(!print_html.contains("class=\"fragment\""));
    }

    #[test]
    fn renders_parenthesized_ordered_lists_as_live_fragments_and_static_final_state() {
        let source = r#"# Ordered fragmented

1) First point.
2) Second point.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-list-kind=\"ordered\""));
        assert!(html.contains("data-list-reveal=\"fragments\""));
        assert!(html.contains("<ol><li class=\"fragment\" data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>"));
        assert!(html.contains("<li class=\"fragment\" data-step-index=\"2\" data-fragment-index=\"2\">Second point.</li></ol>"));
        assert!(print_html.contains("data-list-kind=\"ordered\""));
        assert!(print_html.contains("data-list-reveal=\"fragments\""));
        assert!(
            print_html.contains(
                "<ol><li data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>"
            )
        );
        assert!(!print_html.contains("class=\"fragment\""));
    }

    #[test]
    fn renders_pandoc_incremental_list_divs_as_live_fragments_and_static_final_state() {
        let source = r#"# Incremental

::: {.incremental}
- First point.
- Second point.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-list-kind=\"unordered\""));
        assert!(html.contains("data-list-reveal=\"fragments\""));
        assert!(html.contains("<ul><li class=\"fragment\" data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>"));
        assert!(html.contains("<li class=\"fragment\" data-step-index=\"2\" data-fragment-index=\"2\">Second point.</li></ul>"));
        assert!(print_html.contains("data-list-reveal=\"fragments\""));
        assert!(
            print_html.contains(
                "<ul><li data-step-index=\"1\" data-fragment-index=\"1\">First point.</li>"
            )
        );
        assert!(!print_html.contains("class=\"fragment\""));
    }

    #[test]
    fn renders_fit_text_blocks_with_semantic_theme_hooks() {
        let source = r#"# Fit

[fit] Search becomes operational at \(x_t\).
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"fit-text\""));
        assert!(html.contains("zpres-block-fit-text"));
        assert!(html.contains("Search becomes operational"));
        assert!(html.contains("<math"));
    }

    #[test]
    fn renders_slide_classes_as_theme_hooks() {
        let source = r#"# Tagged

::: class lead result
:::

Content.
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("zpres-slide-class-lead"));
        assert!(html.contains("zpres-slide-class-result"));
        assert!(html.contains("data-slide-classes=\"lead result\""));
        assert!(!html.contains("classes lead, result"));
        assert!(print_html.contains("zpres-slide-class-lead"));
        assert!(print_html.contains("data-slide-classes=\"lead result\""));
    }

    #[test]
    fn renders_theme_resolved_slide_presets_as_hooks() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "../../themes"
---

# Preset slide

[.preset: spotlight]

Preset content.
"##;
        let mut deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();
        let rendered_theme = rendered_theme(&deck);
        theme::resolve_theme_slide_presets(&mut deck, &rendered_theme.manifest).unwrap();

        let html = render_debug_html(&deck, &rendered_theme);
        let print_html = render_debug_print_html(&deck, &rendered_theme);

        assert!(html.contains("data-slide-preset=\"spotlight\""));
        assert!(html.contains("zpres-slide-preset-spotlight"));
        assert!(html.contains("data-slide-preset=\"spotlight\""));
        assert!(html.contains("zpres-slide-class-lead"));
        assert!(html.contains("data-theme-param-accent=\"#2f6f66\""));
        assert!(html.contains("data-autoscale=\"true\""));
        assert!(html.contains("data-transition=\"fade\""));
        assert!(print_html.contains("data-slide-preset=\"spotlight\""));
        assert!(print_html.contains("zpres-slide-preset-spotlight"));
    }

    #[test]
    fn renders_slide_theme_params_as_local_theme_variables() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
---

# Dark local palette

::: theme mode=dark accent="#ff00ff"
:::

Content.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-theme-params=\"accent mode\""));
        assert!(html.contains("data-theme-param-mode=\"dark\""));
        assert!(html.contains("--zpres-param-mode: dark;"));
        assert!(html.contains("--zpres-color-background: #101a22;"));
        assert!(html.contains("--zpres-color-accent: #ff00ff;"));
        assert!(print_html.contains("data-theme-param-mode=\"dark\""));
        assert!(print_html.contains("--zpres-color-background: #101a22;"));
    }

    #[test]
    fn renders_slide_color_shorthands_as_local_theme_variables() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
---

# Local colors

[.background-color: #101820]
[.accent-color: "#ff00ff"]

Content.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-theme-params=\"accent background\""));
        assert!(html.contains("data-theme-param-background=\"#101820\""));
        assert!(html.contains("--zpres-color-background: #101820;"));
        assert!(html.contains("--zpres-color-accent: #ff00ff;"));
        assert!(print_html.contains("--zpres-color-background: #101820;"));
    }

    #[test]
    fn renders_slide_metadata_block_as_theme_hooks() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
---

# Metadata driven slide

::: slide
variant: claim
classes: [hero, branded]
theme:
  mode: dark
  accent: "#ff00ff"
colors:
  background: "#101820"
background:
  src: fixtures/canonical/assets/phase-space.svg
  split: right:35%
  dim: 86
:::

Content.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-slide-variant=\"claim\""));
        assert!(html.contains("zpres-slide-class-hero"));
        assert!(html.contains("data-slide-classes=\"hero branded\""));
        assert!(html.contains("data-theme-params=\"accent background mode\""));
        assert!(html.contains("data-theme-param-background=\"#101820\""));
        assert!(html.contains("--zpres-color-background: #101820;"));
        assert!(html.contains("--zpres-color-accent: #ff00ff;"));
        assert!(html.contains("data-background-split=\"right\""));
        assert!(html.contains("--zpres-background-split-size: 35%"));
        assert!(print_html.contains("data-slide-variant=\"claim\""));
        assert!(print_html.contains("data-background-split=\"right\""));
        assert!(print_html.contains("--zpres-color-background: #101820;"));
    }

    #[test]
    fn renders_slide_footers_and_slide_number_controls() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
footer: "Global footer"
slide_numbers: true
---

# First

[.footer: Local footer]
[.slidenumbers: false]

Content.

---

# Second

[.hide-footer]

Content.

---

# Third

::: slide
footer: "Metadata footer"
slide_numbers: false
:::

Content.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-footer-content=\"true\""));
        assert!(html.contains("data-slide-numbers=\"false\""));
        assert!(html.contains("zpres-slide-footer-content\">Local footer</span>"));
        assert!(!html.contains("zpres-slide-footer-content\">Local footer</span>\n            <span class=\"zpres-slide-footer-number\">1 / 3</span>"));
        assert!(html.contains("data-footer-hidden=\"true\""));
        assert!(html.contains("zpres-slide-footer-content\">Metadata footer</span>"));
        assert!(html.contains("zpres-slide-footer\" data-footer-mode=\"slide-number\" data-slide-number=\"1\" data-slide-count=\"3\""));
        assert!(print_html.contains("zpres-slide-footer-content\">Local footer</span>"));
        assert!(print_html.contains("data-slide-count=\"3\""));
    }

    #[test]
    fn renders_slide_number_as_semantic_text() {
        let source = r#"---
slide_numbers: true
---

# Numbered
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("<span class=\"zpres-slide-footer-number\">1 / 1</span>"));
        assert!(!html.contains("<svg class=\"zpres-slide-footer-number\""));
    }

    #[test]
    fn renders_marpit_comment_directives_as_slide_hooks() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
---

<!--
paginate: true
footer: Global footer
class: lead
backgroundColor: "#f8fafc"
color: "#111827"
backgroundImage: url('assets/phase-space.svg')
backgroundPosition: left top
backgroundSize: contain
-->

# First

---

<!--
_paginate: false
_footer: Local footer
_class: result
_backgroundColor: "#222222"
_backgroundPosition: right bottom
_backgroundSize: cover
-->

# Second

---

# Third
"##;
        let deck = parse_source_text(
            source,
            Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures/canonical/canonical.zp.md"),
            ),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-slide-id=\"section-1-main\""));
        assert!(html.contains("data-slide-classes=\"lead\""));
        assert!(html.contains("zpres-slide-footer-content\">Global footer</span>"));
        assert!(html.contains("data-slide-numbers=\"true\""));
        assert!(html.contains("--zpres-color-background: #f8fafc;"));
        assert!(html.contains("--zpres-color-text: #111827;"));
        assert!(html.contains("data-background-phase=\"content\""));
        assert!(html.contains("--zpres-background-position: left top"));
        assert!(html.contains("--zpres-background-fit: contain"));
        assert!(html.contains("data-slide-classes=\"lead result\""));
        assert!(html.contains("zpres-slide-footer-content\">Local footer</span>"));
        assert!(html.contains("data-slide-numbers=\"false\""));
        assert!(html.contains("--zpres-color-background: #222222;"));
        assert!(html.contains("--zpres-background-position: right bottom"));
        assert!(html.contains("--zpres-background-fit: cover"));
        assert!(print_html.contains("zpres-slide-class-result"));
        assert!(print_html.contains("zpres-slide-footer-content\">Global footer</span>"));
        assert!(print_html.contains("/fixtures/canonical/assets/phase-space.svg"));
    }

    #[test]
    fn renders_autoscale_hooks_and_content_wrapper() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
autoscale: true
---

# Autoscaled

Dense content should shrink instead of being clipped.

---

# Not autoscaled

[.autoscale: false]

This slide opts out.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains(
            "data-slide-id=\"section-1-main\" data-slide-role=\"main\" data-autoscale=\"true\""
        ));
        assert!(html.contains(
            "data-slide-id=\"section-2-main\" data-slide-role=\"main\" data-autoscale=\"false\""
        ));
        assert!(html.contains("class=\"zpres-slide-content\""));
        assert!(html.contains("window.zpresAutoscaleAll"));
        assert!(REVEAL_JS.contains("zpresAutoscaleAll"));
        assert!(print_html.contains("data-autoscale=\"true\""));
        assert!(print_html.contains("data-zpres-autoscale-ready"));
        assert!(print_html.contains("data-zpres-ready=\"pending\""));
    }

    #[test]
    fn print_html_exposes_browser_owned_static_readiness() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-zpres-ready=\"pending\" data-zpres-ready-target=\"pdf\""));
        assert!(!html.contains("data-zpres-ready=\"true\" data-zpres-ready-target=\"pdf\""));
        assert!(html.contains("window.zpresStaticReady = readiness"));
        assert!(
            html.contains("window.zpresStaticReadyState = { status: \"pending\", errors: [] }")
        );
        assert!(html.contains("await withTimeout(document.fonts.ready"));
        assert!(html.contains("font.status === \"error\""));
        assert!(html.contains("const images = Array.from(document.images)"));
        assert!(html.contains("image.loading = \"eager\""));
        assert!(html.contains("const backgroundValues = backgroundSources()"));
        assert!(html.contains("window.zpresStaticRenderPromises"));
        assert!(html.contains("autoscale = await withTimeout(window.zpresAutoscaleAll()"));
        assert!(html.contains("body?.setAttribute(\"data-zpres-ready\", \"true\")"));
        assert!(html.contains("body?.setAttribute(\"data-zpres-ready\", \"failed\")"));
        assert!(html.contains("window.zpresStaticReadyError = details"));

        let final_frame = html.find("await nextFrame();").unwrap();
        let overflow_check = html.find("const overflow = !fits").unwrap();
        let autoscale_ready = html
            .find("body?.setAttribute(\"data-zpres-autoscale-ready\", \"true\")")
            .unwrap();
        assert!(final_frame < overflow_check);
        assert!(overflow_check < autoscale_ready);
    }

    #[test]
    fn live_html_exposes_the_same_browser_owned_readiness_contract() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("zpresVisualReviewParams.get(\"zpres-visual-review\") === \"1\""));
        assert!(
            html.contains("document.body?.setAttribute(\"data-zpres-ready-target\", \"screen\")")
        );
        assert!(html.contains("window.zpresStaticReady = readiness"));
        assert!(
            html.contains("window.zpresStaticReadyState = { status: \"pending\", errors: [] }")
        );
        assert!(html.contains("autoscale = await withTimeout(window.zpresAutoscaleAll()"));
        assert!(html.contains("body?.setAttribute(\"data-zpres-ready\", \"true\")"));
    }

    #[test]
    fn renders_transition_hooks_for_live_and_static_output() {
        let source = r##"---
theme: "paper-chalk"
theme_dirs:
  - "themes"
transition: fade
---

# Global transition

Content.

---

# Slide transition

[.transition: zoom]

Content.

---

# Metadata transition

::: slide
transition: none
:::

Content.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("talk.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains(
            "data-slide-id=\"section-1-main\" data-slide-role=\"main\" data-transition=\"fade\""
        ));
        assert!(html.contains(
            "data-slide-id=\"section-2-main\" data-slide-role=\"main\" data-transition=\"zoom\""
        ));
        assert!(html.contains(
            "data-slide-id=\"section-3-main\" data-slide-role=\"main\" data-transition=\"none\""
        ));
        assert!(REVEAL_JS.contains("is-entering"));
        assert!(print_html.contains("data-transition=\"zoom\""));
    }

    #[test]
    fn renders_media_blocks_for_html_and_static_print() {
        let source = r#"# Media

::: video src="assets/clip.mp4?t=1m30s" poster="assets/poster.png" title="Demo clip" width="62%" fit="cover" align="center" controls=true loop=true muted=true autoadvance=true
A short demo clip.
:::

::: audio src="assets/voice.mp3" title="Narration" start="45" loop=true mute=true hide=true
Listen to the narration.
:::

::: iframe src="https://example.com/demo" title="Remote demo" poster="assets/poster.png"
Remote demo fallback.
:::

![video right 50% fill loop mute hide autoadvance poster="assets/poster.png" title="Markdown clip" alt="Markdown poster"](assets/clip.mp4?t=2m3s "Markdown video caption")

![youtube poster="assets/poster.png" title="Conference talk"](https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=30s "Conference talk clip")
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"media\""));
        assert!(html.contains("data-media-kind=\"video\""));
        assert!(html.contains("data-media-start=\"90\""));
        assert!(html.contains("data-media-fit=\"cover\""));
        assert!(html.contains("data-media-align=\"center\""));
        assert!(html.contains("--zpres-media-width: 62%;"));
        assert!(html.contains("--zpres-media-fit: cover;"));
        assert!(html.contains("--zpres-media-align: center;"));
        assert!(html.contains("<video src=\"assets/clip.mp4#t=90\""));
        assert!(html.contains("poster=\"assets/poster.png\""));
        assert!(html.contains("title=\"Demo clip\""));
        assert!(!html.contains("controls autoplay"));
        assert!(html.contains("controls loop muted playsinline"));
        assert!(html.contains("<audio src=\"assets/voice.mp3#t=45\""));
        assert!(html.contains("title=\"Narration\""));
        assert!(html.contains("controls loop muted preload=\"metadata\""));
        assert!(html.contains("data-media-hidden=\"true\""));
        assert!(html.contains("data-media-autoadvance=\"true\""));
        assert!(html.contains("<iframe src=\"https://example.com/demo\" title=\"Remote demo\""));
        assert!(html.contains("<video src=\"assets/clip.mp4#t=123\""));
        assert!(html.contains("<video src=\"assets/clip.mp4#t=123\" poster=\"assets/poster.png\" title=\"Markdown clip\" controls loop muted playsinline"));
        assert!(html.contains("data-media-fit=\"fill\""));
        assert!(html.contains("data-media-align=\"end\""));
        assert!(html.contains("--zpres-media-width: 50%;"));
        assert!(html.contains("title=\"Markdown clip\""));
        assert!(html.contains("Markdown video caption"));
        assert!(html.contains("<iframe src=\"https://www.youtube.com/embed/dQw4w9WgXcQ?start=30\" title=\"Conference talk\""));
        assert!(html.contains("Conference talk clip"));
        assert!(html.contains("zpres-block-media"));
        assert!(REVEAL_JS.contains("data-media-autoadvance"));
        assert!(REVEAL_JS.contains("nextStepOrSection();"));

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("data-media-hidden=\"true\""));
        assert!(print_html.contains("data-media-autoadvance=\"true\""));
        assert!(print_html.contains("data-media-fit=\"cover\""));
        assert!(print_html.contains("--zpres-media-width: 62%;"));
        assert!(print_html.contains("file://"));
        assert!(print_html.contains("/fixtures/canonical/assets/poster.png"));
        assert!(print_html.contains("zpres-media-fallback"));
        assert!(!print_html.contains("<video"));
        assert!(!print_html.contains("<iframe"));
    }

    #[test]
    fn renders_figure_options_as_theme_contract_attributes() {
        let source = r#"# Figure

::: figure src="assets/phase-space.svg" alt="Phase-space sketch" width="68%" height="45vh" fit="cover" align="center" dim="18" grayscale="35" saturate="72" blur="2" radius="14"
Synthetic phase-space sketch.
:::
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"figure\""));
        assert!(html.contains(
            "<img src=\"assets/phase-space.svg\" alt=\"Phase-space sketch\" loading=\"lazy\" decoding=\"async\">"
        ));
        assert!(html.contains("data-figure-fit=\"cover\""));
        assert!(html.contains("data-figure-align=\"center\""));
        assert!(html.contains("data-figure-radius=\"14\""));
        assert!(html.contains("data-figure-treatment=\"true\""));
        assert!(html.contains("--zpres-figure-width: 68%;"));
        assert!(html.contains("--zpres-figure-height: 45vh;"));
        assert!(html.contains("--zpres-figure-fit: cover;"));
        assert!(html.contains("--zpres-figure-align: center;"));
        assert!(html.contains("--zpres-figure-dim: 0.18;"));
        assert!(html.contains("--zpres-figure-brightness: 0.82;"));
        assert!(html.contains("--zpres-figure-gray: 0.35;"));
        assert!(html.contains("--zpres-figure-saturate: 0.72;"));
        assert!(html.contains("--zpres-figure-blur: 2px;"));
        assert!(html.contains("--zpres-figure-radius: 14px;"));
        assert!(print_html.contains("<img src=\"file://"));
        assert!(print_html.contains("alt=\"Phase-space sketch\">"));
        assert!(!print_html.contains("loading=\"lazy\""));
        assert!(!print_html.contains("decoding=\"async\""));
    }

    #[test]
    fn renders_remote_figure_with_static_fallback_for_print_html() {
        let source = r#"# Remote figure

![pdf-src="assets/phase-space.svg" alt="Remote sketch"](https://example.com/remote-plot.png "Remote sketch")
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains(
            "<img src=\"https://example.com/remote-plot.png\" alt=\"Remote sketch\" loading=\"lazy\" decoding=\"async\">"
        ));
        assert!(print_html.contains("<img src=\"file://"));
        assert!(print_html.contains("/fixtures/canonical/assets/phase-space.svg"));
        assert!(print_html.contains("alt=\"Remote sketch\">"));
        assert!(!print_html.contains("https://example.com/remote-plot.png"));
        assert!(!print_html.contains("loading=\"lazy\""));
        assert!(!print_html.contains("decoding=\"async\""));
    }

    #[test]
    fn renders_gif_figure_with_static_fallback_for_print_html() {
        let source = r#"# GIF figure

![static-src="assets/phase-space.svg" alt="Animated sketch"](assets/solver.gif "Animated sketch")
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains(
            "<img src=\"assets/solver.gif\" alt=\"Animated sketch\" loading=\"lazy\" decoding=\"async\">"
        ));
        assert!(print_html.contains("<img src=\"file://"));
        assert!(print_html.contains("phase-space.svg"));
        assert!(!print_html.contains("assets/solver.gif"));
    }

    #[test]
    fn renders_inline_image_gallery_with_theme_hooks() {
        let source = r#"# Gallery

        ![inline fill columns=2 corner-radius(10) alt="First phase"](assets/phase-space.svg "First")
        ![inline fit dim=12 radius=0.5rem alt="Second phase"](assets/phase-space.svg "Second")
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"gallery\""));
        assert!(html.contains("data-gallery-count=\"2\""));
        assert!(html.contains("data-gallery-columns=\"2\""));
        assert!(html.contains("--zpres-gallery-columns: 2;"));
        assert!(html.contains("class=\"zpres-gallery-item\""));
        assert!(html.contains("data-figure-fit=\"fill\""));
        assert!(html.contains("data-figure-radius=\"10\""));
        assert!(html.contains("--zpres-figure-radius: 0.5rem;"));
        assert!(html.contains("data-figure-treatment=\"true\""));
        assert!(html.contains("loading=\"lazy\" decoding=\"async\""));
        assert!(html.contains("alt=\"First phase\""));
        assert!(html.contains("<figcaption data-zpres-type-role=\"micro\">Second</figcaption>"));
        assert!(print_html.contains("data-block-type=\"gallery\""));
        assert!(!print_html.contains("loading=\"lazy\""));
        assert!(!print_html.contains("decoding=\"async\""));
    }

    #[test]
    fn chart_offsets_keep_narrow_ranges_interpretable() {
        for start in [100_000_000.0, 100_000_003.0, -100_000_003.0] {
            let points = [
                ChartPoint {
                    x: start,
                    y: start,
                    series: "A".to_string(),
                    lower: None,
                    upper: None,
                },
                ChartPoint {
                    x: start + 10.0,
                    y: start + 10.0,
                    series: "A".to_string(),
                    lower: None,
                    upper: None,
                },
            ];
            let offset = chart_axis_offset(start, start + 10.0);
            assert_eq!(offset, start);
            for tick in 0..=4 {
                let residual = 10.0 * tick as f64 / 4.0;
                let label = chart_tick_label(residual, 2.5);
                assert_eq!(label.parse::<f64>().unwrap() + offset, start + residual);
            }
            let svg = line_chart_svg(&points, "Input", "Runtime", Some("Method"));
            let label = chart_offset_label(offset);
            assert!(svg.contains(&format!(">{label}</text>")));
            assert!(svg.contains(&format!(">Input ({label})</text>")));
            assert!(svg.contains(">10</text>"));
            assert!(svg.contains(">0</text>"));
        }
        assert_eq!(chart_axis_offset(10.0, 50.0), 0.0);
        assert_eq!(chart_axis_offset(0.000001, 0.000005), 0.0);
    }

    #[test]
    fn chart_ticks_keep_readable_precision_across_numeric_scales() {
        for (value, step, expected) in [
            (0.0, 1.0, "0"),
            (29.5, 17.5, "29.5"),
            (-0.125, 0.25, "-0.125"),
            (0.30000000000000004, 0.1, "0.3"),
            (0.00000125, 0.00000025, "1.25e-6"),
            (1_000_000.0, 250_000.0, "1e6"),
            (100_000_002.5, 2.5, "1.000000025e8"),
        ] {
            assert_eq!(chart_tick_label(value, step), expected);
        }
        let points = [
            ChartPoint {
                x: 10.0,
                y: 12.0,
                series: String::new(),
                lower: None,
                upper: None,
            },
            ChartPoint {
                x: 50.0,
                y: 82.0,
                series: String::new(),
                lower: None,
                upper: None,
            },
        ];
        let svg = line_chart_svg(&points, "Size", "Runtime", None);
        assert_eq!(svg.matches("data-chart-axis=\"x\"").count(), 5);
        assert_eq!(svg.matches("data-chart-axis=\"y\"").count(), 5);
        for label in ["10", "50", "12", "82", "29.5"] {
            assert!(svg.contains(&format!(">{label}</text>")));
        }
    }

    #[test]
    fn chart_svg_exposes_series_aware_marks_and_technical_roles() {
        let points = vec![
            ChartPoint {
                x: 1.0,
                y: 3.0,
                series: "candidate".to_string(),
                lower: Some(2.5),
                upper: Some(3.5),
            },
            ChartPoint {
                x: 2.0,
                y: 4.0,
                series: "candidate".to_string(),
                lower: Some(3.4),
                upper: Some(4.6),
            },
            ChartPoint {
                x: 1.0,
                y: 4.5,
                series: "reference".to_string(),
                lower: Some(4.0),
                upper: Some(5.0),
            },
            ChartPoint {
                x: 2.0,
                y: 5.5,
                series: "reference".to_string(),
                lower: Some(4.9),
                upper: Some(6.1),
            },
        ];

        let svg = line_chart_svg(&points, "size", "runtime", Some("model"));

        assert!(svg.contains("data-chart-role=\"legend\""));
        assert!(!svg.contains("data-chart-role=\"direct-annotation\""));
        assert!(svg.contains("class=\"zpres-chart-line\" data-chart-role=\"line\""));
        assert!(svg.contains("class=\"zpres-chart-point\" data-chart-role=\"point\""));
        assert!(!svg.contains("zpres-chart-point zpres-chart-direct-annotation-point"));
        assert!(svg.contains("class=\"zpres-chart-mark zpres-chart-uncertainty\""));
        assert!(svg.contains("data-chart-role=\"uncertainty-cap\""));
        assert!(svg.contains("data-chart-role=\"legend-mark\""));
        assert!(svg.contains("data-chart-series-index=\"0\""));
        assert!(svg.contains("data-chart-series-index=\"1\""));
        assert_eq!(svg.matches("class=\"zpres-chart-line\"").count(), 2);
        assert_eq!(
            svg.matches("data-chart-role=\"direct-annotation-point\"")
                .count(),
            0
        );
        assert!(svg.contains("data-chart-role=\"uncertainty\""));
        assert!(svg.contains("data-zpres-type-role=\"technical\""));
        assert!(
            svg.contains(
                "x=\"-132\" y=\"32\" class=\"debug-chart-axis-label zpres-chart-axis-label\" data-zpres-type-role=\"technical\" data-chart-role=\"axis-label\" transform=\"rotate(-90)\" text-anchor=\"middle\">runtime</text>"
            ),
            "the y-axis label anchor and rotation must retain a half-margin clearance inside the 820x330 viewBox: {svg}"
        );
        let source_anchor = (-132.0_f64, 32.0_f64);
        let rotated_anchor = (source_anchor.1, -source_anchor.0);
        assert!((0.0..=820.0).contains(&rotated_anchor.0));
        assert!((0.0..=330.0).contains(&rotated_anchor.1));
        assert_eq!(svg.matches("text-anchor=\"middle\"").count(), 5);
    }

    #[test]
    fn renders_inline_gallery_static_fallbacks_for_print_html() {
        let source = r#"# Gallery

![inline fill static-src="assets/phase-space.svg" alt="Animated phase"](assets/phase.gif "Animated")
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("src=\"assets/phase.gif\""));
        assert!(html.contains("alt=\"Animated phase\""));
        assert!(print_html.contains("phase-space.svg"));
        assert!(!print_html.contains("assets/phase.gif"));
    }

    #[test]
    fn renders_rich_markdown_inside_layout_regions() {
        let source = r#"# Layout

::: columns widths="1/1"
Left column:
![width=45% alt="Inset"](assets/phase-space.svg "Inset figure")

$$
x^2
$$

```rust
fn main() {}
```

Right column:
| A | B |
|---|---|
| 1 | 2 |

- item with \(x\)
:::
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-block-type=\"layout\""));
        assert!(html.contains(
            "<img src=\"assets/phase-space.svg\" alt=\"Inset\" loading=\"lazy\" decoding=\"async\">"
        ));
        assert!(
            html.contains("<figcaption data-zpres-type-role=\"micro\">Inset figure</figcaption>")
        );
        assert!(html.contains("--zpres-figure-width: 45%;"));
        assert!(html.contains("class=\"debug-math-display zpres-math-display\""));
        assert!(html.contains("debug-block-label zpres-block-label\">code: rust</span>"));
        assert!(html.contains("data-block-type=\"table\""));
        assert!(
            html.contains("<li>item with <span class=\"debug-math-inline zpres-math-inline\">")
        );

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("file://"));
        assert!(print_html.contains("/fixtures/canonical/assets/phase-space.svg"));
    }

    #[test]
    fn renders_single_dollar_math_in_paragraphs_tables_and_figure_captions() {
        let source = r#"# Inline math surfaces

The bound is $z \leq 5$.

## Bound $z$

| State | Bound |
| --- | --- |
| active | $z=4$ |

::: figure src="assets/phase-space.svg" alt="Phase space"
Equality keeps $z=5$ feasible.
:::
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert_eq!(html.matches("zpres-math-inline").count(), 4);
        assert!(html.contains("zpres-block-heading"));
        assert!(html.contains("<figcaption data-zpres-type-role=\"micro\">Equality keeps <span"));
    }

    #[test]
    fn percentage_column_tracks_reserve_the_declared_gap() {
        let values = LayoutValues {
            widths: Some(vec![
                LayoutSize::Arbitrary {
                    value: "35%".to_string(),
                },
                LayoutSize::Arbitrary {
                    value: "65%".to_string(),
                },
            ]),
            gap: Some(LayoutSize::Scale { step: 4 }),
            align: Some(LayoutAlign::Start),
            ..LayoutValues::default()
        };

        assert_eq!(
            layout_style(LayoutKind::Columns, &values),
            "display:grid;grid-template-columns:minmax(0,35fr) minmax(0,65fr);gap:1rem;align-items:start;"
        );

        let partial_widths = LayoutValues {
            widths: Some(vec![
                LayoutSize::Arbitrary {
                    value: "30%".to_string(),
                },
                LayoutSize::Arbitrary {
                    value: "60%".to_string(),
                },
            ]),
            gap: Some(LayoutSize::Scale { step: 4 }),
            ..LayoutValues::default()
        };
        assert!(
            layout_style(LayoutKind::Columns, &partial_widths)
                .contains("grid-template-columns:30% 60%"),
            "non-total percentage tracks retain their explicitly authored unused space"
        );
    }

    #[test]
    fn renders_speaker_notes_as_private_presenter_panel_content() {
        let source = r#"# Talk

Visible slide text.

::: notes
Remember to pause before the result.
:::

^ Mention the backup slide.

---

# Imported notes

Visible slide text.

Note: Reveal-style reminder.

---

# Quarto notes

Visible slide text.

::: {.notes}
Quarto-style reminder.
:::

---

# Slidev notes

Visible slide text.

<!-- Slidev comment reminder. -->

---

# Click notes

::: notes
Opening setup.
[click] Mention the first reveal.
[click:3] Mention the skipped reveal.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("id=\"zpres-speaker-notes-panel\""));
        assert!(html.contains("data-speaker-notes-content"));
        assert!(
            html.contains("zpres-speaker-notes-source\" data-block-type=\"speaker-notes\" hidden")
        );
        assert!(
            html.contains("zpres-speaker-notes-body\"><p>Remember to pause before the result.</p>")
        );
        assert!(html.contains("zpres-speaker-notes-body\"><p>Mention the backup slide.</p>"));
        assert!(html.contains("zpres-speaker-notes-body\"><p>Reveal-style reminder.</p>"));
        assert!(html.contains("zpres-speaker-notes-body\"><p>Quarto-style reminder.</p>"));
        assert!(html.contains("zpres-speaker-notes-body\"><p>Slidev comment reminder.</p>"));
        assert!(html.contains("data-note-click-index=\"1\">Mention the first reveal.</span>"));
        assert!(html.contains("data-note-click-index=\"3\">Mention the skipped reveal.</span>"));
        assert!(!html.contains("[click] Mention the first reveal."));
        assert!(REVEAL_JS.contains("toggleSpeakerNotes"));
        assert!(REVEAL_JS.contains("applySpeakerNoteClickMarkers"));
        assert!(REVEAL_JS.contains("currentSlide()?.querySelectorAll(\".zpres-block-speaker-notes .zpres-speaker-notes-body\")"));

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("data-block-type=\"speaker-notes\" hidden"));
        assert!(
            print_html.contains("data-note-click-index=\"1\">Mention the first reveal.</span>")
        );
        assert!(!print_html.contains("id=\"zpres-speaker-notes-panel\""));
    }

    #[test]
    fn audience_html_excludes_speaker_notes_and_presenter_panel() {
        let source = r#"# Public talk

Visible slide text.

::: notes
Private rehearsal reminder.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html_with_options(
            &deck,
            &rendered_theme(&deck),
            LiveHtmlOptions {
                include_speaker_notes: false,
            },
        );

        assert!(html.contains("Visible slide text."));
        assert!(!html.contains("Private rehearsal reminder."));
        assert!(!html.contains("zpres-speaker-notes-source"));
        assert!(!html.contains("id=\"zpres-speaker-notes-panel\""));
    }

    #[test]
    fn renders_code_reveal_lines_as_fragments() {
        let source = r#"# Algorithm

```pseudo reveal="1-2|4"
Input: active sites A
build conflict graph
compute matching
return bound
```
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-code-reveal-steps=\"2\""));
        assert!(
            html.contains(
                "class=\"debug-code-line zpres-code-line fragment\" data-line=\"1\" data-step-index=\"1\""
            )
        );
        assert!(
            html.contains(
                "class=\"debug-code-line zpres-code-line fragment\" data-line=\"4\" data-step-index=\"2\""
            )
        );
        assert!(
            html.contains("class=\"debug-code-line zpres-code-line is-visible\" data-line=\"3\"")
        );
        assert!(REVEAL_JS.contains(".zpres-step.fragment, .zpres-code-line.fragment"));
    }

    #[test]
    fn renders_print_code_reveal_against_pdf_step_state() {
        let source = r#"# Algorithm

::: steps pdf="pages"
1. Build graph.
2. Return bound.
:::

```pseudo reveal="1-2|4"
Input: active sites A
build conflict graph
compute matching
return bound
```
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert_eq!(
            html.matches("class=\"zpres-print-slide zpres-slide")
                .count(),
            2
        );
        assert!(html.contains("data-code-line-hidden=\"true\""));
        assert!(
            html.contains("class=\"debug-code-line zpres-code-line is-visible\" data-line=\"4\"")
                || html.contains("class=\"zpres-code-line is-visible\" data-line=\"4\"")
        );
    }

    #[test]
    fn writes_local_html_bundle() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();
        let temp = tempdir().unwrap();
        let bundle = temp.path().join("bundle");

        let publication = write_debug_html_bundle(&deck, &rendered_theme(&deck), &bundle).unwrap();
        let generation = publication.generation_path;

        assert!(bundle.join("index.html").exists());
        assert!(
            generation
                .join(theme_api_v1::FOUNDATION_ASSET_PATH)
                .exists()
        );
        assert!(generation.join("assets/theme.css").exists());
        assert!(generation.join("assets/reveal.js").exists());
        assert!(generation.join("assets/phase-space.svg").exists());
        assert!(generation.join("data/runtime.csv").exists());
        assert!(
            std::fs::read_to_string(generation.join("assets/theme.css"))
                .unwrap()
                .contains("science-theme")
        );
    }

    #[test]
    fn republishing_html_preserves_root_peers_and_retains_complete_generations() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("dist");
        let v1_source = include_str!("../fixtures/canonical/canonical.zp.md");
        let v1_deck = parse_source_text(
            v1_source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();
        let first = write_debug_html_bundle(&v1_deck, &rendered_theme(&v1_deck), &output).unwrap();
        fs::write(output.join("deck.pdf"), b"pdf peer").unwrap();
        fs::write(output.join("speaker-notes.txt"), b"notes peer").unwrap();
        fs::create_dir(output.join("user-files")).unwrap();
        fs::write(output.join("user-files/keep.bin"), [0, 1, 2, 255]).unwrap();

        let v1_theme = write_v1_reference_theme(&temp.path().join("v1-theme"));
        let v1_deck = v1_reference_deck(&temp.path().join("v1-talk.zp.md"));
        let second = write_debug_html_bundle(&v1_deck, &v1_theme, &output).unwrap();

        assert_ne!(first.generation, second.generation);
        assert!(
            first
                .generation_path
                .join(theme_api_v1::FOUNDATION_ASSET_PATH)
                .is_file()
        );
        assert!(!second.generation_path.join("assets/reveal.css").exists());
        assert!(
            second
                .generation_path
                .join(theme_api_v1::FOUNDATION_ASSET_PATH)
                .is_file()
        );
        assert_eq!(fs::read(output.join("deck.pdf")).unwrap(), b"pdf peer");
        assert_eq!(
            fs::read(output.join("speaker-notes.txt")).unwrap(),
            b"notes peer"
        );
        assert_eq!(
            fs::read(output.join("user-files/keep.bin")).unwrap(),
            [0, 1, 2, 255]
        );
        let current = current_html_bundle_publication(&output).unwrap().unwrap();
        assert_eq!(current.generation, second.generation);
        assert_eq!(current.generation_path, second.generation_path);
    }

    #[test]
    fn late_dependency_copy_failure_keeps_the_last_good_html_generation() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("dist");
        let baseline_theme = write_v1_reference_theme(&temp.path().join("baseline-theme"));
        let deck = v1_reference_deck(&temp.path().join("talk.zp.md"));
        let baseline = write_debug_html_bundle(&deck, &baseline_theme, &output).unwrap();
        let baseline_pointer = fs::read(output.join("index.html")).unwrap();

        let broken_theme_dir = temp.path().join("broken-theme");
        fs::create_dir(&broken_theme_dir).unwrap();
        fs::write(
            broken_theme_dir.join("theme.toml"),
            r#"[theme]
name = "broken-copy"
version = "0.1.0"
api_version = 1
assets = ["texture.svg"]
stylesheet = "theme.css.tmpl"
print_stylesheet = "print.css.tmpl"
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(broken_theme_dir.join("theme.css.tmpl"), "").unwrap();
        fs::write(broken_theme_dir.join("print.css.tmpl"), "").unwrap();
        fs::write(broken_theme_dir.join("texture.svg"), "<svg/>").unwrap();
        let manifest = theme::load_theme_manifest(&broken_theme_dir.join("theme.toml")).unwrap();
        let broken_theme = theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
        fs::remove_file(broken_theme_dir.join("texture.svg")).unwrap();

        let error = write_debug_html_bundle(&deck, &broken_theme, &output).unwrap_err();

        assert!(matches!(error, HtmlError::Publication { .. }));
        assert_eq!(
            fs::read(output.join("index.html")).unwrap(),
            baseline_pointer
        );
        let current = current_html_bundle_publication(&output).unwrap().unwrap();
        assert_eq!(current.generation, baseline.generation);
    }

    #[test]
    fn publishing_refuses_an_unowned_root_index_without_modifying_it() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("user-site");
        fs::create_dir(&output).unwrap();
        let user_index = b"<!doctype html><h1>My site</h1>";
        fs::write(output.join("index.html"), user_index).unwrap();
        let theme = write_v1_reference_theme(&temp.path().join("theme"));
        let deck = v1_reference_deck(&temp.path().join("talk.zp.md"));

        let error = write_debug_html_bundle(&deck, &theme, &output).unwrap_err();

        assert!(matches!(error, HtmlError::Publication { .. }));
        assert_eq!(fs::read(output.join("index.html")).unwrap(), user_index);
        assert!(
            !output
                .join(publication::HTML_GENERATIONS_DIRECTORY)
                .exists()
        );
    }

    #[test]
    fn publication_metadata_paths_are_reserved_before_staging() {
        let temp = tempdir().unwrap();
        let source_path = temp.path().join("talk.zp.md");
        fs::write(
            temp.path().join(publication::HTML_PRESENTATION_FILE),
            "peer",
        )
        .unwrap();
        let deck = parse_source_text(
            &format!(
                "# Reserved dependency\n\n![Local peer]({} \"Reserved publication peer\")\n",
                publication::HTML_PRESENTATION_FILE
            ),
            Some(source_path),
        )
        .unwrap();
        let output = temp.path().join("dist");

        let error = write_debug_html_bundle(&deck, &rendered_theme(&deck), &output).unwrap_err();

        assert!(matches!(error, HtmlError::BundlePathCollision { .. }));
        assert!(
            error
                .to_string()
                .contains(publication::HTML_PRESENTATION_FILE)
        );
        assert!(!output.exists());
    }

    #[test]
    fn debug_theme_contract_exposes_structure() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("themes")
            .join("debug")
            .join("theme.toml");
        let manifest = theme::load_theme_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.name, "debug");
        assert!(
            manifest
                .output_targets
                .iter()
                .any(|target| target == "html")
        );
        assert!(manifest.output_targets.iter().any(|target| target == "pdf"));
        assert!(
            manifest
                .slide_variants
                .iter()
                .any(|variant| variant == "comparison")
        );
        assert!(manifest.parameters.contains_key("accent"));

        let source = include_str!("../themes/debug/specimen.zp.md");
        let deck =
            parse_source_text(source, Some(PathBuf::from("themes/debug/specimen.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-zpres-theme-api=\"1\""));
        assert!(html.contains("class=\"reveal zpres-presentation\""));
        assert!(html.contains("zpres-theme-debug"));
        assert!(html.contains("data-slide-role=\"main\""));
        assert!(html.contains("data-slide-role=\"detail\""));
        assert!(html.contains("data-slide-preset=\"spotlight\""));
        assert!(html.contains("data-autoscale=\"true\""));
        assert!(html.contains("data-zpres-type-role=\"display\""));
        assert!(html.contains("data-zpres-type-role=\"body\""));
        assert!(html.contains("data-zpres-type-role=\"supporting\""));
        assert!(html.contains("params.get(\"zpres-debug\") !== \"1\""));
        assert!(html.contains("slide.dataset.zpresDebugIdentity"));
        assert!(html.contains("slide.dataset.zpresDebugRole"));
        assert!(html.contains("slide.dataset.zpresDebugRoute"));
        assert!(html.contains("slide.dataset.zpresDebugTarget"));
        assert!(!html.contains("data-zpres-debug=\"enabled\""));

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("zpres-print-slide zpres-slide"));
        assert!(print_html.contains("data-pdf-step-state=\"final\""));
        assert!(print_html.contains("zpres-speaker-notes-source"));
    }

    #[test]
    fn science_theme_manifest_and_rendering_are_presentational() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("themes")
            .join("science")
            .join("theme.toml");
        let manifest = theme::load_theme_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.name, "science");
        assert_eq!(manifest.api_version, 1);
        assert_eq!(manifest.modules, vec!["scientific-data"]);
        assert!(manifest.parameters.contains_key("accent"));
        assert!(manifest.parameters.contains_key("footer"));
        assert!(manifest.parameters.contains_key("mode"));
        assert!(!manifest.parameters.contains_key("type_scale"));
        assert!(manifest.parameters.contains_key("font_heading"));

        let source = include_str!("../fixtures/theme-api-v1/science-reference.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from(
                "fixtures/theme-api-v1/science-reference.zp.md",
            )),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        assert!(html.contains("data-zpres-theme-api=\"1\""));
        assert!(html.contains("zpres-theme-science"));
        assert!(html.contains("zpres-module-scientific-data"));
        assert!(html.contains("data-slide-variant=\"figure\""));
        assert!(html.contains("data-slide-role=\"detail\""));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-science .zpres-slide-frame")
        );

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(
            print_html
                .contains("zpres-theme-science zpres-module-scientific-data zpres-print-body")
        );
        assert!(print_html.contains("data-pdf-step-state=\"final\""));
    }

    #[test]
    fn renders_deck_background_image_splash_and_content_treatment() {
        let source = r##"---
title: "Background deck"
theme: "science"
background_image:
  src: "assets/phase-space.svg"
  alt: "Phase-space sketch"
  intent: contextual
  splash: true
  position: "center 45%"
  dim: 82
  grayscale: 40
  saturate: 65
---

# Title

---

# Content

Readable body text.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-generated-slide=\"background-image\""));
        assert!(html.contains("class=\"zpres-section-stack zpres-background-splash-stack\""));
        assert!(html.contains("data-background-phase=\"title\""));
        assert!(html.contains("data-background-phase=\"splash\""));
        assert!(html.contains("data-background-phase=\"content\""));
        assert!(html.contains("alt=\"Phase-space sketch\""));
        assert!(html.contains("--zpres-background-image: url(&quot;assets/phase-space.svg&quot;)"));
        assert!(html.contains("--zpres-background-position: center 45%"));
        assert!(html.contains("--zpres-background-dim: 0.82"));

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("data-zpres-page-count=\"3\""));
        assert!(print_html.contains("data-generated-slide=\"background-image\""));
        assert!(print_html.contains("file://"));
        assert!(print_html.contains("/fixtures/canonical/assets/phase-space.svg"));
    }

    #[test]
    fn renders_slide_background_directive_as_slide_specific_background() {
        let source = r##"---
title: "Slide background deck"
theme: "science"
background_image:
  src: "assets/phase-space.svg"
  position: "center 45%"
---

# Title

---

# Visual content

::: background src="assets/phase-space.svg" alt="Phase-space detail" position="center 12%" dim="88"
:::

Readable foreground text.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-slide-id=\"section-2-main\""));
        assert!(html.contains("--zpres-background-position: center 12%"));
        assert!(html.contains("--zpres-background-dim: 0.88"));
        assert!(html.contains("data-background-phase=\"content\""));

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("file://"));
        assert!(print_html.contains("/fixtures/canonical/assets/phase-space.svg"));
        assert!(print_html.contains("--zpres-background-position: center 12%"));
    }

    #[test]
    fn renders_split_background_shorthand_as_layout_hooks() {
        let source = r##"# About

![bg right:35% alt="Portrait"](assets/phase-space.svg)

Text beside the image.
"##;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("data-background-split=\"right\""));
        assert!(html.contains("--zpres-background-split-size: 35%"));
        assert!(print_html.contains("data-background-split=\"right\""));
        assert!(print_html.contains("--zpres-background-split-size: 35%"));
    }

    #[test]
    fn dark_splash_theme_manifest_and_palette_overrides_are_presentational() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("themes")
            .join("dark-splash")
            .join("theme.toml");
        let manifest = theme::load_theme_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.name, "dark-splash");
        assert_eq!(manifest.api_version, 1);
        assert_eq!(manifest.modules, vec!["scientific-data"]);
        assert_eq!(
            manifest.style.family.as_deref(),
            Some("midnight-constraint-atlas")
        );
        assert!(manifest.style.inspiration.is_empty());
        assert!(manifest.color_variants.contains_key("violet"));
        assert!(manifest.color_variants.contains_key("cyan"));
        assert!(manifest.parameters.contains_key("variant"));
        assert!(manifest.parameters.contains_key("accent"));
        assert!(manifest.parameters.contains_key("background"));
        for color_slot in theme::STANDARD_COLOR_SLOTS {
            assert!(manifest.parameters.contains_key(*color_slot));
        }
        let resolved = theme::validate_theme_params(
            &manifest,
            &BTreeMap::from([
                ("variant".to_string(), "cyan".to_string()),
                ("accent".to_string(), "#ff00ff".to_string()),
            ]),
        )
        .unwrap();
        assert_eq!(resolved.get("background").unwrap(), "#060b0f");
        assert_eq!(resolved.get("accent").unwrap(), "#ff00ff");

        let source = r##"---
title: "Dark splash"
theme: "dark-splash"
theme_params:
  variant: "cyan"
  accent: "#ff00ff"
---

# The collaboration spectrum.

Quality of judgment degrades when tired.
"##;
        let deck = parse_source_text(source, Some(PathBuf::from("dark.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        assert!(html.contains("zpres-theme-dark-splash"));
        assert!(html.contains("data-slide-role=\"main\""));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-background: #060b0f")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-accent: #ff00ff")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-dark-splash .zpres-slide-frame")
        );

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("zpres-theme-dark-splash"));
        assert!(print_html.contains("--zpres-color-accent: #ff00ff"));
    }

    #[test]
    fn paper_chalk_theme_manifest_and_modes_are_presentational() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("themes")
            .join("paper-chalk")
            .join("theme.toml");
        let manifest = theme::load_theme_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.name, "paper-chalk");
        assert_eq!(manifest.style.family.as_deref(), Some("field-notebook"));
        assert!(manifest.parameters.contains_key("mode"));
        for color_slot in theme::STANDARD_COLOR_SLOTS {
            assert!(manifest.parameters.contains_key(*color_slot));
        }
        let resolved = theme::validate_theme_params(
            &manifest,
            &BTreeMap::from([
                ("mode".to_string(), "dark".to_string()),
                ("accent".to_string(), "#f7e26b".to_string()),
            ]),
        )
        .unwrap();
        assert_eq!(resolved.get("mode").unwrap(), "dark");
        assert_eq!(resolved.get("accent").unwrap(), "#f7e26b");

        let source = r##"---
title: "Sudoku lab"
theme: "paper-chalk"
theme_params:
  mode: "dark"
---

# Example Deck

Constraint propagation changes regime across puzzle sizes.

--

## Scheme detail

One optional propagation trace.
"##;
        let deck = parse_source_text(source, Some(PathBuf::from("sudoku.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        assert!(html.contains("class=\"reveal zpres-presentation\""));
        assert!(html.contains("zpres-theme-paper-chalk"));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-background: #101a22")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-accent: #82afe8")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-accent-alt: #d88799")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-paper-chalk .zpres-slide-frame::before")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-paper-chalk .zpres-block-table")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("repeating-linear-gradient(0deg")
        );
        assert!(html.contains("data-slide-role=\"detail\""));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-paper-chalk .zpres-slide-footer::before")
        );
        assert!(rendered_theme(&deck).screen_css.contains(
            ".zpres-section-stack:has(> .zpres-slide[data-slide-role=\"detail\"]) > .zpres-slide[data-slide-role=\"main\"] .zpres-slide-footer::before"
        ));
        assert!(
            rendered_theme(&deck).screen_css.contains(
                ".zpres-theme-paper-chalk .zpres-print-slide .zpres-slide-footer::before"
            )
        );

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("zpres-theme-paper-chalk"));
        assert!(print_html.contains("zpres-print-body"));
        assert!(print_html.contains("--zpres-param-mode: dark"));

        let light_source = r##"---
title: "Sudoku lab"
theme: "paper-chalk"
---

# Example Deck
"##;
        let light_deck =
            parse_source_text(light_source, Some(PathBuf::from("sudoku.zp.md"))).unwrap();
        let light_html = render_debug_html(&light_deck, &rendered_theme(&light_deck));
        assert!(light_html.contains("class=\"reveal zpres-presentation\""));
        assert!(
            rendered_theme(&light_deck)
                .screen_css
                .contains("--zpres-color-background: #f4f0e6")
        );
        assert!(
            rendered_theme(&light_deck)
                .screen_css
                .contains("--zpres-color-surface: #fffdf4")
        );
        assert!(
            rendered_theme(&light_deck)
                .screen_css
                .contains("--zpres-color-text: #162b4d")
        );
        assert!(
            rendered_theme(&light_deck)
                .screen_css
                .contains("--paper-background: #f4f0e6")
        );
        for marker in [
            "zpres-slide-class-paper-evidence-canvas",
            "zpres-slide-class-paper-comparison-baseline",
            "zpres-slide-class-annotation-delta-bracket",
            "zpres-slide-class-paper-derivation-path",
            "zpres-slide-class-paper-detail-ledger",
            "zpres-slide-class-paper-detail-trace",
        ] {
            assert!(
                rendered_theme(&light_deck).screen_css.contains(marker),
                "missing {marker}"
            );
        }
    }

    #[test]
    fn wedding_theme_manifest_and_corner_svg_decorations_are_presentational() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("themes")
            .join("wedding")
            .join("theme.toml");
        let manifest = theme::load_theme_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.name, "wedding");
        assert_eq!(manifest.api_version, 1);
        assert_eq!(manifest.modules, vec!["scientific-data"]);
        assert_eq!(manifest.style.family.as_deref(), Some("wedding-editorial"));
        assert!(manifest.style.inspiration.is_empty());
        assert!(manifest.color_variants.contains_key("garden"));
        assert!(manifest.color_variants.contains_key("ivory"));
        assert!(manifest.parameters.contains_key("variant"));
        assert!(manifest.parameters.contains_key("ornament_style"));
        for color_slot in theme::STANDARD_COLOR_SLOTS {
            assert!(manifest.parameters.contains_key(*color_slot));
        }
        let resolved = theme::validate_theme_params(
            &manifest,
            &BTreeMap::from([
                ("variant".to_string(), "slate-rose".to_string()),
                ("accent".to_string(), "#a64257".to_string()),
                ("ornament_style".to_string(), "bow".to_string()),
            ]),
        )
        .unwrap();
        assert_eq!(resolved.get("background").unwrap(), "#e8e5df");
        assert_eq!(resolved.get("accent").unwrap(), "#a64257");
        assert_eq!(resolved.get("ornament_style").unwrap(), "bow");

        let source = r##"---
title: "Wedding conference"
theme: "wedding"
theme_params:
  variant: "slate-rose"
  accent: "#a64257"
  ornament_style: "vine"
---

# A readable celebration.

::: theme ornament_style="wildflower"
:::

::: class bow
:::

Formal decorative framing should not compete with scientific content.
"##;
        let deck = parse_source_text(source, Some(PathBuf::from("wedding.zp.md"))).unwrap();

        let html = render_debug_html(&deck, &rendered_theme(&deck));
        assert!(html.contains("class=\"reveal zpres-presentation\""));
        assert!(html.contains("zpres-theme-wedding"));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-background: #e8e5df")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-color-accent: #a64257")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-wedding .zpres-slide-ornament::before")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-wedding .zpres-slide-ornament::after")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--wedding-ornament: url(\"assets/botanical.svg\")")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("[data-theme-param-ornament-style=\"vine\"]")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("url(\"assets/vine.svg\")")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-theme-wedding .zpres-slide-ornament::after { display: none; }")
        );
        assert!(rendered_theme(&deck).screen_css.contains(
            ".zpres-theme-wedding .zpres-slide[data-slide-variant=\"section-title\"] .zpres-slide-ornament::after"
        ));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("inset: auto auto -18px -9px")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("clip-path: inset(0 0 0 9px)")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("mask: var(--wedding-ornament) right center / 180px 135px no-repeat")
        );
        assert!(rendered_theme(&deck).screen_css.contains("width: 38px"));
        assert!(rendered_theme(&deck).screen_css.contains("max-width: 19ch"));
        assert!(rendered_theme(&deck).screen_css.contains("rotate: 180deg"));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-retain-leaving-slide: 1")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("@keyframes wedding-sheet-leave-forward")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("animation: wedding-sheet-leave-forward 500ms")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("translate(-1320px, -96px) rotate(-5deg)")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("@keyframes wedding-sheet-enter-backward")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("animation: wedding-pile-enter 500ms")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-slide.is-entering[data-zpres-transition-direction=\"backward\"]:not(.zpres-background-splash-slide) > .zpres-slide-background { opacity: 0; }")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-slide.is-leaving::after { display: none; }")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("background-color: var(--wedding-stock-active) !important")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-section-stack:nth-last-child(2) .zpres-slide::after")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-section-stack:last-child .zpres-slide::before")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("@media (prefers-reduced-motion: reduce)")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-data-primary-measure: none")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("[data-background-split] .zpres-slide-frame")
        );
        assert!(rendered_theme(&deck).screen_css.contains("width: 156px"));
        assert!(rendered_theme(&deck).screen_css.contains("height: 116px"));
        for role in ["frame", "join", "point", "boundary", "celebrate"] {
            assert!(
                rendered_theme(&deck)
                    .screen_css
                    .contains(&format!("zpres-slide-class-ornament-{role}"))
            );
        }
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("[data-theme-param-ornament-style=\"wildflower\"]")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-slide.zpres-slide-class-claim-long")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("font-family: \"Snell Roundhand\"")
        );
        assert!(rendered_theme(&deck).screen_css.contains("Bodoni 72"));
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(".zpres-block-code pre")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains(":has(> .zpres-slide-background)")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("backdrop-filter: blur(1px)")
        );
        assert!(
            rendered_theme(&deck)
                .screen_css
                .contains("--zpres-data-figure-max-block: 466px")
        );
        assert!(html.contains("data-theme-param-ornament-style=\"wildflower\""));
        assert!(html.contains("zpres-slide-class-bow"));

        let print_html = render_debug_print_html(&deck, &rendered_theme(&deck));
        assert!(print_html.contains("zpres-theme-wedding"));
        assert!(print_html.contains("zpres-print-body"));
        assert!(print_html.contains("--zpres-color-accent: #a64257"));
        assert!(print_html.contains("data-theme-param-ornament-style=\"wildflower\""));
        assert!(print_html.contains(".zpres-theme-wedding .zpres-slide-ornament::before"));
    }

    #[test]
    fn renders_print_html_with_one_page_per_pdf_slide() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert_eq!(
            html.matches("class=\"zpres-print-slide zpres-slide")
                .count(),
            10
        );
        assert!(html.contains("data-slide-id=\"section-8-detail-2\""));
        assert!(html.contains("data-pdf-step-state=\"final\""));
        assert!(html.contains("Start from the model."));
        assert!(html.contains("Show the branching consequence."));
        assert!(!html.contains("class=\"debug-step zpres-step fragment\""));
        assert!(html.contains("file://"));
        assert!(html.contains("/fixtures/canonical/assets/phase-space.svg"));
        assert!(html.contains("debug-chart-svg zpres-chart-svg"));
        assert!(!html.contains("assets/reveal.js"));
    }

    #[test]
    fn percent_encodes_reserved_and_non_ascii_characters_in_print_asset_urls() {
        let temp = tempdir().unwrap();
        let deck_root = temp.path().join("source # % ü");
        let source_path = deck_root.join("talk.zp.md");
        let source = "# Talk\n\n![Figure](assets/plot.svg)\n";
        let deck = parse_source_text(source, Some(source_path)).unwrap();

        let html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert!(html.contains("source%20%23%20%25%20%C3%BC/assets/plot.svg"));
        assert!(!html.contains("source # % ü/assets/plot.svg"));
    }

    #[test]
    fn renders_single_print_page_html_for_png_export() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let html = render_debug_print_page_html(&deck, &rendered_theme(&deck), 2).unwrap();

        assert_eq!(
            print_page_count_for_theme_with_options(
                &deck,
                &rendered_theme(&deck),
                StaticExportOptions::default()
            ),
            10
        );
        assert_eq!(
            html.matches("class=\"zpres-print-slide zpres-slide")
                .count(),
            1
        );
        assert!(html.contains("data-zpres-ready=\"pending\""));
        assert!(html.contains("window.zpresStaticReady = readiness"));
        assert!(html.contains("data-zpres-page-count=\"1\""));
        assert!(html.contains("data-page=\"2\""));
        assert!(render_debug_print_page_html(&deck, &rendered_theme(&deck), 11).is_none());
    }

    #[test]
    fn renders_opt_in_speaker_notes_pages_for_static_export() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();
        let options = StaticExportOptions {
            include_speaker_notes: true,
        };

        let html = render_debug_print_html_with_options(&deck, &rendered_theme(&deck), options);

        assert_eq!(
            print_page_count_for_theme_with_options(
                &deck,
                &rendered_theme(&deck),
                StaticExportOptions::default()
            ),
            10
        );
        assert_eq!(
            print_page_count_for_theme_with_options(&deck, &rendered_theme(&deck), options),
            11
        );
        assert_eq!(
            html.matches("class=\"zpres-print-slide zpres-slide")
                .count(),
            11
        );
        assert!(html.contains("data-generated-slide=\"speaker-notes\""));
        assert!(html.contains("zpres-slide-content zpres-speaker-notes-print-content"));
        assert!(html.contains("zpres-speaker-notes-print-body"));
        assert!(
            html.contains(
                "The title slide should stay boring: if this fails, the pipeline is broken"
            )
        );
        let notes_page =
            render_debug_print_page_html_with_options(&deck, &rendered_theme(&deck), 2, options)
                .unwrap();
        assert!(notes_page.contains("data-zpres-page-count=\"1\""));
        assert!(notes_page.contains("data-generated-slide=\"speaker-notes\""));
    }

    #[test]
    fn renders_print_html_with_one_page_per_step_when_requested() {
        let source = r#"# Claim

::: steps pdf="pages"
1. First reveal.
2. Second reveal.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let html = render_debug_print_html(&deck, &rendered_theme(&deck));

        assert_eq!(
            html.matches("class=\"zpres-print-slide zpres-slide")
                .count(),
            2
        );
        assert!(html.contains("data-pdf-step=\"1\""));
        assert!(html.contains("data-pdf-step=\"2\""));
        assert!(html.contains("data-pdf-step-state=\"up-to\""));
    }
}
