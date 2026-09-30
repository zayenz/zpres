use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Deck {
    pub source_path: Option<PathBuf>,
    pub metadata: DeckMetadata,
    pub sections: Vec<Section>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Deck {
    pub fn pdf_slide_order(&self) -> Vec<&Slide> {
        self.sections
            .iter()
            .flat_map(|section| {
                std::iter::once(&section.main_slide).chain(section.detail_slides.iter())
            })
            .collect()
    }

    pub fn pdf_pages(&self) -> Vec<PdfPage<'_>> {
        self.pdf_slide_order()
            .into_iter()
            .flat_map(|slide| match slide_step_pdf_policy(slide) {
                Some(StepPdfPolicy::OnePagePerStep) => {
                    let count = slide_step_count(slide);
                    (1..=count)
                        .map(move |step| PdfPage {
                            slide,
                            step_state: PdfStepState::UpTo { step },
                        })
                        .collect::<Vec<_>>()
                }
                Some(StepPdfPolicy::FinalState) | None => vec![PdfPage {
                    slide,
                    step_state: PdfStepState::Final,
                }],
            })
            .collect()
    }

    pub fn deck_root(&self) -> Option<&Path> {
        self.source_path.as_ref().and_then(|path| path.parent())
    }

    pub fn local_asset_references(&self) -> Vec<&str> {
        self.local_dependency_references()
    }

    pub fn local_dependency_references(&self) -> Vec<&str> {
        let mut references = Vec::new();
        if let Some(background_image) = &self.metadata.background_image
            && !looks_remote_or_fragment(&background_image.src)
            && is_safe_local_asset_reference(&background_image.src)
        {
            references.push(background_image.src.as_str());
        }
        for section in &self.sections {
            collect_slide_local_dependency_references(&section.main_slide, &mut references);
            for slide in &section.detail_slides {
                collect_slide_local_dependency_references(slide, &mut references);
            }
        }
        references.sort_unstable();
        references.dedup();
        references
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfPage<'a> {
    pub slide: &'a Slide,
    pub step_state: PdfStepState,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfStepState {
    Final,
    UpTo { step: usize },
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct DeckMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub theme: Option<String>,
    pub aspect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<DeckBackgroundImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<SlideTransition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_corner_radius: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_lists: Option<BuildListsMode>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub theme_params: BTreeMap<String, String>,
    pub extra: BTreeMap<String, String>,
    #[serde(skip)]
    pub theme_api_v1_diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeckBackgroundImage {
    pub src: String,
    pub intent: BackgroundImageIntent,
    #[serde(skip)]
    pub intent_explicit: bool,
    pub alt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub position: String,
    pub fit: BackgroundImageFit,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<BackgroundImageSplit>,
    pub dim: u8,
    pub grayscale: u8,
    pub saturate: u8,
    pub blur: u8,
    pub splash: bool,
    #[serde(skip)]
    pub splash_explicit: bool,
    #[serde(default, skip_serializing_if = "is_clean_background_title_application")]
    pub title_application: BackgroundTitleApplication,
    #[serde(skip)]
    pub source_span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundImageIntent {
    Decorative,
    #[default]
    Contextual,
    Evidence,
}

impl DeckBackgroundImage {
    pub const DEFAULT_DIM: u8 = 78;
    pub const DEFAULT_GRAYSCALE: u8 = 30;
    pub const DEFAULT_SATURATE: u8 = 70;
    pub const DEFAULT_BLUR: u8 = 0;
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundTitleApplication {
    #[default]
    Clean,
    Paint,
}

fn is_clean_background_title_application(value: &BackgroundTitleApplication) -> bool {
    *value == BackgroundTitleApplication::Clean
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundImageFit {
    Cover,
    Contain,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BackgroundImageSplit {
    pub side: BackgroundImageSplitSide,
    pub size: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundImageSplitSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BuildListsMode {
    All,
    NotFirst,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Section {
    pub index: usize,
    pub main_slide: Slide,
    pub detail_slides: Vec<Slide>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Slide {
    pub id: String,
    pub role: SlideRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    pub variant: Option<SlideVariant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub theme_params: BTreeMap<String, String>,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<DeckBackgroundImage>,
    #[serde(default, skip_serializing_if = "SlideFooter::is_default")]
    pub footer: SlideFooter,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<SlideTransition>,
    pub blocks: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SlideTransition {
    None,
    Fade,
    Slide,
    Zoom,
}

impl SlideTransition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fade => "fade",
            Self::Slide => "slide",
            Self::Zoom => "zoom",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct SlideFooter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_numbers: Option<bool>,
}

impl SlideFooter {
    pub(crate) fn is_default(&self) -> bool {
        self.content.is_none() && !self.hidden && self.slide_numbers.is_none()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlideRole {
    Main,
    Detail,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlideVariant {
    Claim,
    Figure,
    Comparison,
    Derivation,
    SectionTitle,
    Dense,
}

impl SlideVariant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Figure => "figure",
            Self::Comparison => "comparison",
            Self::Derivation => "derivation",
            Self::SectionTitle => "section-title",
            Self::Dense => "dense",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        markdown: String,
        inline_math: Vec<String>,
    },
    FitText {
        markdown: String,
        inline_math: Vec<String>,
    },
    Quote {
        markdown: String,
        inline_math: Vec<String>,
    },
    Callout {
        kind: CalloutKind,
        title: Option<String>,
        markdown: String,
        inline_math: Vec<String>,
    },
    List {
        ordered: bool,
        reveal: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        reveal_skip_first: bool,
        items: Vec<ListItem>,
    },
    Math {
        display: bool,
        latex: String,
    },
    Code {
        language: Option<String>,
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reveal: Option<CodeReveal>,
    },
    Table {
        headers: Vec<String>,
        alignments: Vec<TableAlignment>,
        rows: Vec<Vec<String>>,
    },
    Figure {
        src: String,
        alt: String,
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        static_src: Option<String>,
        #[serde(skip_serializing_if = "FigureOptions::is_default")]
        options: FigureOptions,
    },
    Footnotes {
        notes: Vec<Footnote>,
    },
    Gallery {
        items: Vec<GalleryItem>,
        columns: Option<u8>,
    },
    Media {
        kind: MediaKind,
        src: String,
        title: Option<String>,
        caption: Option<String>,
        poster: Option<String>,
        alt: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_time: Option<u32>,
        #[serde(skip_serializing_if = "FigureOptions::is_default")]
        options: FigureOptions,
        autoplay: bool,
        controls: bool,
        loop_playback: bool,
        muted: bool,
        autoadvance: bool,
        visual_hidden: bool,
    },
    Diagram {
        language: DiagramLanguage,
        source: String,
    },
    Chart {
        format: ChartFormat,
        spec: serde_json::Value,
        data: Option<ChartData>,
    },
    Steps {
        pdf_policy: StepPdfPolicy,
        steps: Vec<Step>,
    },
    Layout {
        kind: LayoutKind,
        values: LayoutValues,
        regions: Vec<LayoutRegion>,
    },
    SpeakerNotes {
        markdown: String,
    },
    HtmlOnly {
        html: String,
    },
    UnsupportedDirective {
        name: String,
        body: String,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListItem {
    pub markdown: String,
    pub inline_math: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GalleryItem {
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub static_src: Option<String>,
    pub alt: String,
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "FigureOptions::is_default")]
    pub options: FigureOptions,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Footnote {
    pub label: String,
    pub number: usize,
    pub markdown: String,
    pub inline_math: Vec<String>,
}

#[derive(Debug, Clone)]
struct FootnoteDefinition {
    markdown: String,
    line: usize,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl CalloutKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Tip => "tip",
            Self::Important => "important",
            Self::Warning => "warning",
            Self::Caution => "caution",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaKind {
    Video,
    Audio,
    Iframe,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DiagramLanguage {
    Mermaid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MermaidFlowchart {
    pub(crate) direction: MermaidDirection,
    pub(crate) edges: Vec<MermaidEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MermaidDirection {
    TopDown,
    BottomTop,
    LeftRight,
    RightLeft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MermaidEdge {
    pub(crate) from: MermaidNode,
    pub(crate) to: MermaidNode,
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MermaidNode {
    pub(crate) id: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FigureOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<FigureFit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<FigureAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grayscale: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saturate: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<String>,
}

impl FigureOptions {
    pub(crate) fn is_default(&self) -> bool {
        self.width.is_none()
            && self.height.is_none()
            && self.fit.is_none()
            && self.align.is_none()
            && self.dim.is_none()
            && self.grayscale.is_none()
            && self.saturate.is_none()
            && self.blur.is_none()
            && self.radius.is_none()
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FigureFit {
    Contain,
    Cover,
    Fill,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FigureAlign {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepPdfPolicy {
    FinalState,
    OnePagePerStep,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Step {
    pub index: usize,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodeReveal {
    pub groups: Vec<CodeRevealGroup>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodeRevealGroup {
    pub index: usize,
    pub ranges: Vec<CodeLineRange>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodeLineRange {
    pub start: usize,
    pub end: usize,
}

pub(crate) fn slide_step_pdf_policy(slide: &Slide) -> Option<StepPdfPolicy> {
    let mut has_final_state_steps = false;
    for block in &slide.blocks {
        if let Some(pdf_policy) = content_block_step_pdf_policy(block) {
            if pdf_policy == StepPdfPolicy::OnePagePerStep {
                return Some(StepPdfPolicy::OnePagePerStep);
            }
            has_final_state_steps = true;
        }
    }
    has_final_state_steps.then_some(StepPdfPolicy::FinalState)
}

fn content_block_step_pdf_policy(block: &ContentBlock) -> Option<StepPdfPolicy> {
    match block {
        ContentBlock::Steps { pdf_policy, .. } => Some(*pdf_policy),
        ContentBlock::Layout {
            values, regions, ..
        } => values.step_pdf_policy.or_else(|| {
            regions
                .iter()
                .flat_map(|region| &region.blocks)
                .filter_map(content_block_step_pdf_policy)
                .find(|policy| *policy == StepPdfPolicy::OnePagePerStep)
                .or_else(|| {
                    regions
                        .iter()
                        .flat_map(|region| &region.blocks)
                        .any(|block| content_block_step_pdf_policy(block).is_some())
                        .then_some(StepPdfPolicy::FinalState)
                })
        }),
        _ => None,
    }
}

pub(crate) fn slide_step_count(slide: &Slide) -> usize {
    slide
        .blocks
        .iter()
        .filter_map(content_block_static_step_count)
        .max()
        .unwrap_or(1)
}

fn content_block_static_step_count(block: &ContentBlock) -> Option<usize> {
    match block {
        ContentBlock::Steps { steps, .. } => Some(steps.len()),
        ContentBlock::Code {
            reveal: Some(reveal),
            ..
        } => reveal.groups.last().map(|group| group.index),
        ContentBlock::Layout {
            values, regions, ..
        } => derivation_stage_count(values, regions).or_else(|| {
            regions
                .iter()
                .flat_map(|region| &region.blocks)
                .filter_map(content_block_static_step_count)
                .max()
        }),
        _ => None,
    }
}

pub(crate) fn slide_screen_step_count(slide: &Slide) -> usize {
    slide
        .blocks
        .iter()
        .filter_map(content_block_screen_step_count)
        .max()
        .unwrap_or(0)
}

fn content_block_screen_step_count(block: &ContentBlock) -> Option<usize> {
    match block {
        ContentBlock::Steps { steps, .. } => Some(steps.len()),
        ContentBlock::List {
            reveal: true,
            reveal_skip_first,
            items,
            ..
        } => Some(items.len().saturating_sub(usize::from(*reveal_skip_first))),
        ContentBlock::Code {
            reveal: Some(reveal),
            ..
        } => reveal.groups.last().map(|group| group.index),
        ContentBlock::Layout {
            values, regions, ..
        } => derivation_stage_count(values, regions).or_else(|| {
            regions
                .iter()
                .flat_map(|region| &region.blocks)
                .filter_map(content_block_screen_step_count)
                .max()
        }),
        _ => None,
    }
}

fn derivation_stage_count(values: &LayoutValues, regions: &[LayoutRegion]) -> Option<usize> {
    values.step_pdf_policy.map(|_| {
        regions
            .iter()
            .filter(|region| region.derivation_step.is_some())
            .count()
    })
}

fn content_block_has_progression(block: &ContentBlock) -> bool {
    match block {
        ContentBlock::Steps { .. }
        | ContentBlock::Code {
            reveal: Some(_), ..
        }
        | ContentBlock::List { reveal: true, .. } => true,
        ContentBlock::Layout { regions, .. } => regions
            .iter()
            .flat_map(|region| &region.blocks)
            .any(content_block_has_progression),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    Columns,
    Grid,
    Stack,
    Overlay,
    Aside,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LayoutValues {
    pub widths: Option<Vec<LayoutSize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<LayoutSize>>,
    pub gap: Option<LayoutSize>,
    pub align: Option<LayoutAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_policy: Option<OverlayPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aside_width: Option<AsideWidth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_pdf_policy: Option<StepPdfPolicy>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayoutSize {
    Scale { step: u8 },
    Fraction { units: u8 },
    Arbitrary { value: String },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutAlign {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LayoutRegion {
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_placement: Option<GridPlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<LayoutRegionRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_placement: Option<OverlayPlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_step: Option<usize>,
    pub blocks: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GridPlacement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u8>,
    pub column_span: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<u8>,
    pub row_span: u8,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutRegionRole {
    Base,
    Annotation,
    Primary,
    Supporting,
    Stable,
    Change,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPolicy {
    EdgeOnly,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayAnchor {
    TopStart,
    TopEnd,
    BottomStart,
    BottomEnd,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayWidth {
    Compact,
    Standard,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct OverlayPlacement {
    pub anchor: OverlayAnchor,
    pub width: OverlayWidth,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsideWidth {
    Compact,
    Standard,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChartFormat {
    VegaLite,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChartData {
    pub url: String,
    pub format: ChartDataFormat,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChartDataFormat {
    Csv,
    Json,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TableAlignment {
    Default,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub span: Option<SourceSpan>,
    pub message: String,
}

impl Diagnostic {
    pub fn error(span: Option<SourceSpan>, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            span,
            message: message.into(),
        }
    }

    pub fn warning(span: Option<SourceSpan>, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            span,
            message: message.into(),
        }
    }

    pub fn is_fatal(&self) -> bool {
        self.severity == DiagnosticSeverity::Error
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceSpan {
    pub source_path: Option<PathBuf>,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("source file must use the .zp.md extension: {path}")]
    UnsupportedExtension { path: PathBuf },
    #[error("failed to read source file {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid source front matter in {path}{location}: {source}")]
    FrontMatter {
        path: PathBuf,
        location: String,
        source: serde_yaml::Error,
    },
}

pub fn parse_source_file(path: impl AsRef<Path>) -> Result<Deck, ParseError> {
    let path = path.as_ref();
    if !path.to_string_lossy().ends_with(".zp.md") {
        return Err(ParseError::UnsupportedExtension {
            path: path.to_path_buf(),
        });
    }

    let text = fs::read_to_string(path).map_err(|source| ParseError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse_source_text(&text, Some(path.to_path_buf()))
}

pub fn parse_source_text(text: &str, source_path: Option<PathBuf>) -> Result<Deck, ParseError> {
    let split = split_front_matter(text);
    let (mut metadata, body, body_start_line, unterminated_front_matter) = match split {
        FrontMatterSplit::Valid {
            front_matter,
            body,
            body_start_line,
        } => {
            let path = source_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("<source>"));
            (
                parse_metadata(front_matter, source_path.clone()).map_err(|source| {
                    ParseError::FrontMatter {
                        path,
                        location: yaml_location(&source),
                        source,
                    }
                })?,
                body,
                body_start_line,
                false,
            )
        }
        FrontMatterSplit::Absent { body } => (DeckMetadata::default(), body, 1, false),
        FrontMatterSplit::Unterminated => (DeckMetadata::default(), "", 1, true),
    };

    let deck_root = source_path
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let mut parser = SourceParser::new(
        body,
        body_start_line,
        source_path.clone(),
        deck_root,
        metadata
            .image_corner_radius
            .clone()
            .filter(|radius| is_safe_css_size(radius)),
        metadata.build_lists,
    );
    let (sections, diagnostics, image_corner_radius, theme_api_v1_diagnostics) =
        parser.parse_sections();
    metadata
        .theme_api_v1_diagnostics
        .extend(theme_api_v1_diagnostics);
    if image_corner_radius.is_some() {
        metadata.image_corner_radius = image_corner_radius;
    }
    let mut diagnostics = diagnostics;
    if unterminated_front_matter {
        diagnostics.insert(
            0,
            Diagnostic::error(
                Some(SourceSpan {
                    source_path: source_path.clone(),
                    line: 1,
                    column: 1,
                }),
                "unterminated front matter; expected a closing --- delimiter",
            ),
        );
    }
    validate_metadata_dependencies(&metadata, source_path.clone(), &mut diagnostics);
    validate_metadata_image_corner_radius(&metadata, source_path.clone(), &mut diagnostics);

    Ok(Deck {
        source_path,
        metadata,
        sections,
        diagnostics,
    })
}

fn parse_metadata(
    front_matter: &str,
    source_path: Option<PathBuf>,
) -> Result<DeckMetadata, serde_yaml::Error> {
    let value: serde_yaml::Value = serde_yaml::from_str(front_matter)?;
    let mut metadata = DeckMetadata::default();
    let background_spans = front_matter_background_spans(front_matter, source_path);
    let serde_yaml::Value::Mapping(mapping) = value else {
        return Ok(metadata);
    };

    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            continue;
        };
        match key {
            "theme_params" => {
                metadata.theme_params = mapping_to_string_map(&value);
            }
            "title" => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.title = Some(value);
                }
            }
            "author" => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.author = Some(value);
                }
            }
            "theme" => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.theme = Some(value);
                }
            }
            "aspect" => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.aspect = Some(value);
                }
            }
            "background_image" => {
                metadata
                    .theme_api_v1_diagnostics
                    .extend(validate_v1_deck_background_value(&value, &background_spans));
                metadata.background_image =
                    background_image_from_yaml(&value, background_spans.for_field("src"));
            }
            "footer" => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.footer = Some(value);
                }
            }
            "slide_numbers" | "slide-numbers" | "slidenumbers" => {
                metadata.slide_numbers = yaml_bool(&value);
            }
            "autoscale" => {
                metadata.autoscale = yaml_bool(&value);
            }
            "transition" => {
                metadata.transition = slide_transition_from_yaml(&value);
            }
            "slide_transition" | "slide-transition" => {
                metadata.transition = slide_transition_from_yaml(&value);
            }
            "image_corner_radius" | "image-corner-radius" => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.image_corner_radius = Some(value);
                }
            }
            "build_lists" | "build-lists" => {
                metadata.build_lists = build_lists_from_yaml(&value);
            }
            _ => {
                if let Some(value) = scalar_to_string(&value) {
                    metadata.extra.insert(key.to_string(), value);
                }
            }
        }
    }

    Ok(metadata)
}

#[derive(Debug, Default)]
struct BackgroundAuthoringSpans {
    form: Option<SourceSpan>,
    fields: BTreeMap<String, SourceSpan>,
}

impl BackgroundAuthoringSpans {
    fn for_field(&self, field: &str) -> Option<SourceSpan> {
        self.fields
            .get(field)
            .cloned()
            .or_else(|| self.form.clone())
    }
}

fn front_matter_background_spans(
    front_matter: &str,
    source_path: Option<PathBuf>,
) -> BackgroundAuthoringSpans {
    let mut spans = BackgroundAuthoringSpans::default();
    let mut background_indent = None;
    for (offset, line) in front_matter.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len().saturating_sub(line.trim_start().len());
        if let Some(base_indent) = background_indent {
            if indent <= base_indent {
                break;
            }
            if let Some((raw_key, _)) = trimmed.split_once(':') {
                let key = raw_key.trim().trim_matches(['\'', '"']);
                if !key.is_empty() {
                    spans.fields.entry(key.to_string()).or_insert(SourceSpan {
                        source_path: source_path.clone(),
                        line: offset + 2,
                        column: indent + 1,
                    });
                }
            }
            continue;
        }
        let Some((raw_key, _)) = trimmed.split_once(':') else {
            continue;
        };
        if raw_key.trim().trim_matches(['\'', '"']) != "background_image" {
            continue;
        }
        spans.form = Some(SourceSpan {
            source_path: source_path.clone(),
            line: offset + 2,
            column: indent + 1,
        });
        background_indent = Some(indent);
    }
    spans
}

fn validate_v1_deck_background_value(
    value: &serde_yaml::Value,
    spans: &BackgroundAuthoringSpans,
) -> Vec<Diagnostic> {
    let serde_yaml::Value::Mapping(mapping) = value else {
        return if matches!(value, serde_yaml::Value::String(_)) {
            Vec::new()
        } else {
            vec![Diagnostic::error(
                spans.form.clone(),
                "Theme API v1 background_image must be a path string or a mapping",
            )]
        };
    };

    const FIELDS: &[&str] = &[
        "src",
        "intent",
        "alt",
        "description",
        "position",
        "fit",
        "split",
        "dim",
        "grayscale",
        "saturate",
        "blur",
        "splash",
        "title",
    ];
    let mut diagnostics = Vec::new();
    for key in mapping.keys() {
        let Some(key) = key.as_str() else {
            diagnostics.push(Diagnostic::error(
                spans.form.clone(),
                "Theme API v1 background_image field names must be strings",
            ));
            continue;
        };
        if !FIELDS.contains(&key) {
            diagnostics.push(Diagnostic::error(
                spans.for_field(key),
                format!("Theme API v1 background_image has unknown field '{key}'"),
            ));
        }
    }

    match mapping.get(serde_yaml::Value::String("src".to_string())) {
        Some(serde_yaml::Value::String(src)) if !src.trim().is_empty() => {}
        Some(_) => diagnostics.push(Diagnostic::error(
            spans.for_field("src"),
            "Theme API v1 background_image src must be a non-empty path string",
        )),
        None => diagnostics.push(Diagnostic::error(
            spans.form.clone(),
            "Theme API v1 background_image is missing required field 'src'",
        )),
    }

    for field in ["alt", "description", "position"] {
        if let Some(value) = mapping.get(serde_yaml::Value::String(field.to_string()))
            && !matches!(value, serde_yaml::Value::String(_))
        {
            diagnostics.push(Diagnostic::error(
                spans.for_field(field),
                format!("Theme API v1 background_image {field} must be a string"),
            ));
        }
    }
    match mapping.get(serde_yaml::Value::String("intent".to_string())) {
        Some(value) if value.as_str().and_then(parse_background_image_intent).is_some() => {}
        Some(_) => diagnostics.push(Diagnostic::error(
            spans.for_field("intent"),
            "Theme API v1 background_image intent must be 'decorative', 'contextual', or 'evidence'",
        )),
        None => {}
    }
    if let Some(serde_yaml::Value::String(position)) =
        mapping.get(serde_yaml::Value::String("position".to_string()))
        && !is_safe_css_token_list(position)
    {
        diagnostics.push(Diagnostic::error(
            spans.for_field("position"),
            format!(
                "Theme API v1 background_image position contains unsupported CSS tokens '{position}'"
            ),
        ));
    }
    if let Some(value) = mapping.get(serde_yaml::Value::String("fit".to_string()))
        && value
            .as_str()
            .and_then(parse_background_image_fit)
            .is_none()
    {
        diagnostics.push(Diagnostic::error(
            spans.for_field("fit"),
            "Theme API v1 background_image fit must be 'cover' or 'contain'",
        ));
    }
    if let Some(value) = mapping.get(serde_yaml::Value::String("split".to_string()))
        && value
            .as_str()
            .and_then(parse_background_image_split)
            .is_none()
    {
        diagnostics.push(Diagnostic::error(
            spans.for_field("split"),
            "Theme API v1 background_image split must be 'left' or 'right' with an optional 10%-90% size",
        ));
    }
    if let Some(value) = mapping.get(serde_yaml::Value::String("title".to_string()))
        && value
            .as_str()
            .and_then(parse_background_title_application)
            .is_none()
    {
        diagnostics.push(Diagnostic::error(
            spans.for_field("title"),
            "Theme API v1 background_image title must be 'clean' or 'paint'",
        ));
    }
    if let Some(value) = mapping.get(serde_yaml::Value::String("splash".to_string()))
        && yaml_bool(value).is_none()
    {
        diagnostics.push(Diagnostic::error(
            spans.for_field("splash"),
            "Theme API v1 background_image splash must be true or false",
        ));
    }
    for (field, maximum) in [
        ("dim", 100),
        ("grayscale", 100),
        ("saturate", 100),
        ("blur", 24),
    ] {
        let Some(value) = mapping.get(serde_yaml::Value::String(field.to_string())) else {
            continue;
        };
        match mapping_u8(mapping, field) {
            Some(value) if value <= maximum => {}
            Some(value) => diagnostics.push(Diagnostic::error(
                spans.for_field(field),
                format!(
                    "Theme API v1 background_image {field} must be between 0 and {maximum}, found {value}"
                ),
            )),
            None => diagnostics.push(Diagnostic::error(
                spans.for_field(field),
                format!(
                    "Theme API v1 background_image {field} must be a whole number between 0 and {maximum}, found {value:?}"
                ),
            )),
        }
    }
    diagnostics
}

fn background_image_from_yaml(
    value: &serde_yaml::Value,
    source_span: Option<SourceSpan>,
) -> Option<DeckBackgroundImage> {
    match value {
        serde_yaml::Value::String(src) => Some(DeckBackgroundImage {
            src: src.clone(),
            intent: BackgroundImageIntent::Contextual,
            intent_explicit: false,
            alt: String::new(),
            description: None,
            position: "center center".to_string(),
            fit: BackgroundImageFit::Cover,
            split: None,
            dim: DeckBackgroundImage::DEFAULT_DIM,
            grayscale: DeckBackgroundImage::DEFAULT_GRAYSCALE,
            saturate: DeckBackgroundImage::DEFAULT_SATURATE,
            blur: DeckBackgroundImage::DEFAULT_BLUR,
            splash: true,
            splash_explicit: false,
            title_application: BackgroundTitleApplication::Clean,
            source_span,
        }),
        serde_yaml::Value::Mapping(mapping) => {
            let src = mapping_string(mapping, "src")?;
            Some(DeckBackgroundImage {
                src,
                intent: mapping_string(mapping, "intent")
                    .as_deref()
                    .and_then(parse_background_image_intent)
                    .unwrap_or_default(),
                intent_explicit: mapping
                    .contains_key(serde_yaml::Value::String("intent".to_string())),
                alt: mapping_string(mapping, "alt").unwrap_or_default(),
                description: mapping_string(mapping, "description")
                    .filter(|description| !description.trim().is_empty()),
                position: mapping_string(mapping, "position")
                    .unwrap_or_else(|| "center center".to_string()),
                fit: mapping_string(mapping, "fit")
                    .as_deref()
                    .and_then(parse_background_image_fit)
                    .unwrap_or(BackgroundImageFit::Cover),
                split: mapping_string(mapping, "split")
                    .as_deref()
                    .and_then(parse_background_image_split),
                dim: mapping_u8(mapping, "dim").unwrap_or(DeckBackgroundImage::DEFAULT_DIM),
                grayscale: mapping_u8(mapping, "grayscale")
                    .unwrap_or(DeckBackgroundImage::DEFAULT_GRAYSCALE),
                saturate: mapping_u8(mapping, "saturate")
                    .unwrap_or(DeckBackgroundImage::DEFAULT_SATURATE),
                blur: mapping_u8(mapping, "blur").unwrap_or(DeckBackgroundImage::DEFAULT_BLUR),
                splash: mapping_bool(mapping, "splash").unwrap_or(true),
                splash_explicit: mapping
                    .contains_key(serde_yaml::Value::String("splash".to_string())),
                title_application: mapping_string(mapping, "title")
                    .as_deref()
                    .and_then(parse_background_title_application)
                    .unwrap_or_default(),
                source_span,
            })
        }
        _ => None,
    }
}

fn parse_background_title_application(value: &str) -> Option<BackgroundTitleApplication> {
    match value {
        "clean" => Some(BackgroundTitleApplication::Clean),
        "paint" => Some(BackgroundTitleApplication::Paint),
        _ => None,
    }
}

fn parse_background_image_intent(value: &str) -> Option<BackgroundImageIntent> {
    match value {
        "decorative" => Some(BackgroundImageIntent::Decorative),
        "contextual" => Some(BackgroundImageIntent::Contextual),
        "evidence" => Some(BackgroundImageIntent::Evidence),
        _ => None,
    }
}

fn mapping_string(mapping: &serde_yaml::Mapping, key: &str) -> Option<String> {
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .and_then(scalar_to_string)
}

fn mapping_u8(mapping: &serde_yaml::Mapping, key: &str) -> Option<u8> {
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .and_then(|value| match value {
            serde_yaml::Value::Number(value) => {
                value.as_u64().and_then(|value| u8::try_from(value).ok())
            }
            serde_yaml::Value::String(value) => value.parse().ok(),
            _ => None,
        })
}

fn mapping_bool(mapping: &serde_yaml::Mapping, key: &str) -> Option<bool> {
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .and_then(yaml_bool)
}

fn yaml_bool(value: &serde_yaml::Value) -> Option<bool> {
    match value {
        serde_yaml::Value::Bool(value) => Some(*value),
        serde_yaml::Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn build_lists_from_yaml(value: &serde_yaml::Value) -> Option<BuildListsMode> {
    if let Some(value) = yaml_bool(value) {
        return value.then_some(BuildListsMode::All);
    }
    scalar_to_string(value).and_then(|value| parse_build_lists_value(&value).flatten())
}

fn slide_transition_from_yaml(value: &serde_yaml::Value) -> Option<SlideTransition> {
    if let Some(enabled) = yaml_bool(value) {
        return Some(if enabled {
            SlideTransition::Fade
        } else {
            SlideTransition::None
        });
    }
    scalar_to_string(value).and_then(|value| parse_slide_transition(&value))
}

fn parse_background_image_fit(value: &str) -> Option<BackgroundImageFit> {
    match value {
        "cover" => Some(BackgroundImageFit::Cover),
        "contain" => Some(BackgroundImageFit::Contain),
        _ => None,
    }
}

fn parse_background_image_split(value: &str) -> Option<BackgroundImageSplit> {
    let (side, size) = value
        .split_once(':')
        .map_or((value, "50%"), |(side, size)| {
            (side, if size.is_empty() { "50%" } else { size })
        });
    let side = match side {
        "left" => BackgroundImageSplitSide::Left,
        "right" => BackgroundImageSplitSide::Right,
        _ => return None,
    };
    if !is_percentage_size(size) {
        return None;
    }
    Some(BackgroundImageSplit {
        side,
        size: size.to_string(),
    })
}

fn is_percentage_size(value: &str) -> bool {
    let Some(number) = value.strip_suffix('%') else {
        return false;
    };
    let Ok(percent) = number.parse::<u8>() else {
        return false;
    };
    (10..=90).contains(&percent)
}

fn mapping_to_string_map(value: &serde_yaml::Value) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let serde_yaml::Value::Mapping(mapping) = value else {
        return values;
    };
    for (key, value) in mapping {
        let (Some(key), Some(value)) = (key.as_str(), scalar_to_string(value)) else {
            continue;
        };
        values.insert(key.to_string(), value);
    }
    values
}

fn validate_metadata_dependencies(
    metadata: &DeckMetadata,
    source_path: Option<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(background_image) = &metadata.background_image else {
        return;
    };
    let background_span = background_image
        .source_span
        .clone()
        .or_else(|| metadata_span(source_path.clone()));
    if background_image.src.trim().is_empty() {
        diagnostics.push(Diagnostic::error(
            background_span.clone(),
            "background_image is missing required src value",
        ));
        return;
    }
    if looks_remote_or_fragment(&background_image.src) {
        diagnostics.push(Diagnostic::error(
            background_span.clone(),
            format!(
                "background_image references unsupported image URL '{}'",
                background_image.src
            ),
        ));
        return;
    }
    if !is_safe_local_asset_reference(&background_image.src) {
        diagnostics.push(Diagnostic::error(
            background_span.clone(),
            format!(
                "background_image references unsupported local asset path '{}'",
                background_image.src
            ),
        ));
        return;
    }
    if let Some(deck_root) = source_path.as_ref().and_then(|path| path.parent())
        && !deck_root.join(&background_image.src).exists()
    {
        diagnostics.push(Diagnostic::error(
            background_span.clone(),
            format!(
                "background_image references missing local asset '{}'",
                background_image.src
            ),
        ));
    }
    if !is_safe_css_token_list(&background_image.position) {
        diagnostics.push(Diagnostic::error(
            background_span.clone(),
            format!(
                "background_image position contains unsupported CSS tokens '{}'",
                background_image.position
            ),
        ));
    }
    for (name, value) in [
        ("dim", background_image.dim),
        ("grayscale", background_image.grayscale),
        ("saturate", background_image.saturate),
    ] {
        if value > 100 {
            diagnostics.push(Diagnostic::error(
                background_span.clone(),
                format!("background_image {name} must be between 0 and 100, found {value}"),
            ));
        }
    }
    if background_image.blur > 24 {
        diagnostics.push(Diagnostic::error(
            background_span,
            format!(
                "background_image blur must be between 0 and 24 pixels, found {}",
                background_image.blur
            ),
        ));
    }
}

fn validate_metadata_image_corner_radius(
    metadata: &DeckMetadata,
    source_path: Option<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(radius) = &metadata.image_corner_radius else {
        return;
    };
    if !is_safe_css_size(radius) {
        diagnostics.push(Diagnostic::error(
            metadata_span(source_path),
            format!("image_corner_radius uses unsupported CSS size '{radius}'"),
        ));
    }
}

fn metadata_span(source_path: Option<PathBuf>) -> Option<SourceSpan> {
    Some(SourceSpan {
        source_path,
        line: 1,
        column: 1,
    })
}

fn scalar_to_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(value) => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        serde_yaml::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontMatterSplit<'a> {
    Absent {
        body: &'a str,
    },
    Valid {
        front_matter: &'a str,
        body: &'a str,
        body_start_line: usize,
    },
    Unterminated,
}

pub(crate) fn split_front_matter(text: &str) -> FrontMatterSplit<'_> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let opening_end = text.find('\n').unwrap_or(text.len());
    let opening = text[..opening_end]
        .strip_suffix('\r')
        .unwrap_or(&text[..opening_end]);
    if opening != "---" {
        return FrontMatterSplit::Absent { body: text };
    }
    if opening_end == text.len() {
        return FrontMatterSplit::Unterminated;
    }

    let front_matter_start = opening_end + 1;
    let mut line_start = front_matter_start;
    let mut line_number = 2usize;
    while line_start < text.len() {
        let relative_line_end = text[line_start..].find('\n');
        let line_end = relative_line_end.map_or(text.len(), |offset| line_start + offset);
        let line = text[line_start..line_end]
            .strip_suffix('\r')
            .unwrap_or(&text[line_start..line_end]);
        if line.trim() == "---" {
            let body_start = relative_line_end.map_or(text.len(), |_| line_end + 1);
            return FrontMatterSplit::Valid {
                front_matter: &text[front_matter_start..line_start],
                body: &text[body_start..],
                body_start_line: line_number + 1,
            };
        }
        let Some(_) = relative_line_end else {
            break;
        };
        line_start = line_end + 1;
        line_number += 1;
    }
    FrontMatterSplit::Unterminated
}

#[derive(Debug)]
struct SourceParser<'a> {
    lines: Vec<SourceLine<'a>>,
    top_level_lines: Vec<bool>,
    body_start_line: usize,
    source_path: Option<PathBuf>,
    deck_root: Option<PathBuf>,
    image_corner_radius: Option<String>,
    build_lists: Option<BuildListsMode>,
    footnote_definitions: BTreeMap<String, FootnoteDefinition>,
    marpit_defaults: MarpitSlideDefaults,
    diagnostics: Vec<Diagnostic>,
    theme_api_v1_diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy)]
struct SourceLine<'a> {
    number: usize,
    text: &'a str,
}

#[derive(Debug, Clone, Default)]
struct MarpitSlideDefaults {
    classes: Vec<String>,
    theme_params: BTreeMap<String, String>,
    background_image: Option<DeckBackgroundImage>,
    footer: SlideFooter,
}

#[derive(Debug)]
struct SectionBuilder {
    main_slide: Option<Slide>,
    detail_slides: Vec<Slide>,
    pending_detail_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingSlideRole {
    Main,
    Detail,
}

impl<'a> SourceParser<'a> {
    fn new(
        body: &'a str,
        body_start_line: usize,
        source_path: Option<PathBuf>,
        deck_root: Option<PathBuf>,
        image_corner_radius: Option<String>,
        build_lists: Option<BuildListsMode>,
    ) -> Self {
        let lines = body
            .lines()
            .enumerate()
            .map(|(offset, text)| SourceLine {
                number: body_start_line + offset,
                text,
            })
            .collect::<Vec<_>>();
        let top_level_lines = top_level_line_mask(&lines);
        let mut parser = Self {
            lines,
            top_level_lines,
            body_start_line,
            source_path,
            deck_root,
            image_corner_radius,
            build_lists,
            footnote_definitions: BTreeMap::new(),
            marpit_defaults: MarpitSlideDefaults::default(),
            diagnostics: Vec::new(),
            theme_api_v1_diagnostics: Vec::new(),
        };
        parser.collect_deck_commands();
        parser.collect_footnote_definitions();
        parser
    }

    fn collect_deck_commands(&mut self) {
        for (index, line) in self.lines.clone().into_iter().enumerate() {
            if !self.top_level_lines[index] {
                continue;
            }
            let trimmed = line.text.trim();
            if let Some(value) = parse_image_corner_radius_command(trimmed) {
                if is_safe_css_size(&value) {
                    self.image_corner_radius = Some(value);
                } else {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("image-corner-radius uses unsupported CSS size '{value}'"),
                    ));
                }
            }
            if let Some(value) = parse_build_lists_command(trimmed) {
                self.build_lists = value;
            }
        }
    }

    fn collect_footnote_definitions(&mut self) {
        for (index, line) in self.lines.clone().into_iter().enumerate() {
            if !self.top_level_lines[index] {
                continue;
            }
            let Some((label, markdown)) = parse_footnote_definition_line(line.text.trim()) else {
                continue;
            };
            if self.footnote_definitions.contains_key(&label) {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("duplicate footnote definition '[^{label}]'"),
                ));
                continue;
            }
            self.footnote_definitions.insert(
                label,
                FootnoteDefinition {
                    markdown,
                    line: line.number,
                },
            );
        }
    }

    fn parse_sections(
        &mut self,
    ) -> (
        Vec<Section>,
        Vec<Diagnostic>,
        Option<String>,
        Vec<Diagnostic>,
    ) {
        let mut sections = Vec::new();
        let mut section = SectionBuilder {
            main_slide: None,
            detail_slides: Vec::new(),
            pending_detail_count: 0,
        };
        let mut pending_role = PendingSlideRole::Main;
        let mut pending_lines = Vec::new();
        let mut section_index = 1usize;

        for (line_index, source_line) in self.lines.clone().into_iter().enumerate() {
            match self.top_level_lines[line_index]
                .then(|| separator_kind(source_line.text))
                .flatten()
            {
                Some(SeparatorKind::Section) => {
                    self.finish_slide(&mut section, section_index, pending_role, &pending_lines);
                    self.finish_section(&mut sections, section_index, section);
                    section_index += 1;
                    section = SectionBuilder {
                        main_slide: None,
                        detail_slides: Vec::new(),
                        pending_detail_count: 0,
                    };
                    pending_role = PendingSlideRole::Main;
                    pending_lines.clear();
                }
                Some(SeparatorKind::Detail) => {
                    if pending_role == PendingSlideRole::Main
                        && slide_lines_are_empty(&pending_lines)
                    {
                        self.diagnostics.push(Diagnostic::error(
                            self.span_for_line(source_line),
                            "detail separator must follow main slide content within a section",
                        ));
                    }
                    self.finish_slide(&mut section, section_index, pending_role, &pending_lines);
                    pending_role = PendingSlideRole::Detail;
                    pending_lines.clear();
                }
                None => {
                    if self.top_level_lines[line_index] && is_malformed_separator(source_line.text)
                    {
                        self.diagnostics.push(Diagnostic::error(
                            self.span_for_line(source_line),
                            "separator must be exactly --- for a section or -- for a detail slide",
                        ));
                    }
                    pending_lines.push(source_line);
                }
            }
        }

        self.finish_slide(&mut section, section_index, pending_role, &pending_lines);
        self.finish_section(&mut sections, section_index, section);

        if sections.is_empty() {
            let slide = self.parse_slide(1, SlideRole::Main, 0, &[]);
            sections.push(Section {
                index: 1,
                main_slide: slide,
                detail_slides: Vec::new(),
            });
        }

        (
            sections,
            std::mem::take(&mut self.diagnostics),
            self.image_corner_radius.clone(),
            std::mem::take(&mut self.theme_api_v1_diagnostics),
        )
    }

    fn finish_slide(
        &mut self,
        section: &mut SectionBuilder,
        section_index: usize,
        role: PendingSlideRole,
        lines: &[SourceLine<'a>],
    ) {
        if role == PendingSlideRole::Detail {
            section.pending_detail_count += 1;
        }

        if slide_lines_are_empty(lines) {
            return;
        }

        let slide_role = match role {
            PendingSlideRole::Main => SlideRole::Main,
            PendingSlideRole::Detail => SlideRole::Detail,
        };
        let detail_index =
            (role == PendingSlideRole::Detail).then_some(section.pending_detail_count);
        let slide = self.parse_slide(section_index, slide_role, detail_index.unwrap_or(0), lines);

        match role {
            PendingSlideRole::Main => section.main_slide = Some(slide),
            PendingSlideRole::Detail => section.detail_slides.push(slide),
        }
    }

    fn finish_section(
        &mut self,
        sections: &mut Vec<Section>,
        section_index: usize,
        section: SectionBuilder,
    ) {
        if section.main_slide.is_none() && section.detail_slides.is_empty() {
            return;
        }

        let main_slide = section.main_slide.unwrap_or_else(|| {
            self.diagnostics.push(Diagnostic::error(
                Some(SourceSpan {
                    source_path: self.source_path.clone(),
                    line: self.body_start_line,
                    column: 1,
                }),
                "section is missing a main slide",
            ));
            Slide {
                id: format!("section-{section_index}-main"),
                role: SlideRole::Main,
                preset: None,
                variant: None,
                classes: Vec::new(),
                theme_params: BTreeMap::new(),
                title: None,
                background_image: None,
                footer: SlideFooter::default(),
                autoscale: None,
                transition: None,
                blocks: Vec::new(),
            }
        });

        sections.push(Section {
            index: section_index,
            main_slide,
            detail_slides: section.detail_slides,
        });
    }

    fn parse_slide<'b>(
        &mut self,
        section_index: usize,
        role: SlideRole,
        detail_index: usize,
        lines: &[SourceLine<'b>],
    ) -> Slide {
        let mut slide = self.parse_slide_content(section_index, role, detail_index, lines, None);
        self.normalize_comparison_pattern(slide.variant, &mut slide.blocks);
        self.append_slide_footnotes(&mut slide.blocks);
        slide
    }

    fn parse_slide_content<'b>(
        &mut self,
        section_index: usize,
        role: SlideRole,
        detail_index: usize,
        lines: &[SourceLine<'b>],
        region: Option<LayoutKind>,
    ) -> Slide {
        let mut blocks = Vec::new();
        let mut title = None;
        let mut preset = None;
        let mut variant = None;
        let mut classes = self.marpit_defaults.classes.clone();
        let mut theme_params = self.marpit_defaults.theme_params.clone();
        let mut background_image = self.marpit_defaults.background_image.clone();
        let mut footer = self.marpit_defaults.footer.clone();
        let mut autoscale = None;
        let mut transition = None;
        let mut build_lists = self.build_lists;
        let mut paragraph = Vec::new();
        let mut index = 0usize;

        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.text.trim();

            if trimmed.is_empty() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                index += 1;
                continue;
            }

            if let Some(kind) = region
                && (parse_image_corner_radius_command(trimmed).is_some()
                    || parse_build_lists_command(trimmed).is_some()
                    || parse_slide_theme_shorthand(trimmed).is_some()
                    || parse_slide_preset_shorthand(trimmed).is_some()
                    || parse_slide_footer_shorthand(trimmed).is_some()
                    || parse_autoscale_shorthand(trimmed).is_some()
                    || parse_build_lists_shorthand(trimmed).is_some()
                    || parse_slide_transition_shorthand(trimmed).is_some())
            {
                self.reject_region_content(kind, "slide metadata", line);
                index += 1;
                continue;
            }

            if parse_image_corner_radius_command(trimmed).is_some() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                index += 1;
                continue;
            }

            if parse_build_lists_command(trimmed).is_some() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                index += 1;
                continue;
            }

            if parse_footnote_definition_line(trimmed).is_some() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                index += 1;
                continue;
            }

            if let Some((name, value)) = parse_slide_theme_shorthand(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                theme_params.insert(name, value);
                index += 1;
                continue;
            }

            if let Some(value) = parse_slide_preset_shorthand(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                self.apply_slide_preset(&value, line, &mut preset);
                index += 1;
                continue;
            }

            if let Some(command) = parse_slide_footer_shorthand(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                self.apply_slide_footer_command(command, line, &mut footer);
                index += 1;
                continue;
            }

            if let Some(value) = parse_autoscale_shorthand(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                autoscale = Some(value);
                index += 1;
                continue;
            }

            if let Some(value) = parse_build_lists_shorthand(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                build_lists = value;
                index += 1;
                continue;
            }

            if let Some(value) = parse_slide_transition_shorthand(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                transition = Some(value);
                index += 1;
                continue;
            }

            if let Some(info) = code_fence_start(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (code, next_index, closed) = collect_code_fence(lines, index + 1);
                if !closed {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        "code block is missing a closing ``` fence",
                    ));
                }
                let parsed_info = self.parse_code_fence_info(info.as_deref(), line);
                if parsed_info.language.as_deref() == Some("vega-lite") {
                    if let Some(chart) = self.parse_chart_block(&code, None, line) {
                        blocks.push(chart);
                    }
                } else if parsed_info.language.as_deref() == Some("mermaid") {
                    if let Some(diagram) = self.parse_mermaid_block(&code, line) {
                        blocks.push(diagram);
                    }
                } else {
                    if let Some(reveal) = &parsed_info.reveal {
                        self.validate_code_reveal_ranges(reveal, &code, line);
                    }
                    blocks.push(ContentBlock::Code {
                        language: parsed_info.language,
                        code,
                        reveal: parsed_info.reveal,
                    });
                }
                index = next_index;
                continue;
            }

            if trimmed == "$$" {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (latex, next_index, closed) = collect_display_math(lines, index + 1);
                if !closed {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        "display math block is missing a closing $$ marker",
                    ));
                }
                self.validate_math(&latex, true, line);
                blocks.push(ContentBlock::Math {
                    display: true,
                    latex,
                });
                index = next_index;
                continue;
            }

            if let Some((table, next_index)) = collect_table(lines, index) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                blocks.push(table);
                index = next_index;
                continue;
            }

            if let Some(markdown) = parse_fit_text_line(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                blocks.push(self.fit_text_block(markdown.to_string(), line));
                index += 1;
                continue;
            }

            if list_line_kind(trimmed).is_some() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (list, next_index) = self.collect_list(lines, index, build_lists);
                blocks.push(list);
                index = next_index;
                continue;
            }

            if trimmed.starts_with('>') {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (quote, next_index) = self.collect_quote(lines, index);
                blocks.push(quote);
                index = next_index;
                continue;
            }

            if let Some(background) = self.parse_markdown_background_image(trimmed, line) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                if let Some(kind) = region {
                    self.reject_region_content(kind, "slide background", line);
                } else {
                    background_image = Some(background);
                }
                index += 1;
                continue;
            }

            if let Some(media) = self.parse_markdown_media(trimmed, line) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                blocks.push(media);
                index += 1;
                continue;
            }

            if markdown_image_is_inline(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (gallery, next_index) = self.collect_inline_gallery(lines, index);
                if let Some(gallery) = gallery {
                    blocks.push(gallery);
                }
                index = next_index;
                continue;
            }

            if let Some(figure) = parse_markdown_image(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                self.validate_figure_asset(&figure.src, line);
                if let Some(static_src) = &figure.static_src {
                    self.validate_figure_asset(static_src, line);
                }
                if self.validate_figure_options_parse(&figure.options, line) {
                    let (caption, next_index) = if figure.caption.is_none() {
                        lines
                            .get(index + 1)
                            .and_then(|line| parse_implicit_figure_caption(line.text.trim()))
                            .map_or((figure.caption, index + 1), |caption| {
                                (Some(caption), index + 2)
                            })
                    } else {
                        (figure.caption, index + 1)
                    };
                    blocks.push(ContentBlock::Figure {
                        src: figure.src,
                        alt: figure.alt,
                        caption,
                        static_src: figure.static_src,
                        options: self.figure_options_with_defaults(figure.options.options),
                    });
                    index = next_index;
                    continue;
                }
                index += 1;
                continue;
            }

            if parse_caret_note_line(trimmed).is_some() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (markdown, next_index) = collect_caret_notes(lines, index);
                if let Some(kind) = region {
                    self.reject_region_content(kind, "speaker notes", line);
                } else {
                    blocks.push(ContentBlock::SpeakerNotes { markdown });
                }
                index = next_index;
                continue;
            }

            if parse_reveal_note_marker(trimmed).is_some() {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (markdown, next_index) = collect_reveal_notes(lines, index);
                if let Some(kind) = region {
                    self.reject_region_content(kind, "speaker notes", line);
                } else {
                    blocks.push(ContentBlock::SpeakerNotes { markdown });
                }
                index = next_index;
                continue;
            }

            if html_comment_start(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let (markdown, next_index, closed) = collect_html_comment(lines, index);
                if closed {
                    let directives = parse_marpit_comment_directives(&markdown);
                    if !directives.is_empty() {
                        if let Some(kind) = region {
                            self.reject_region_content(kind, "slide metadata", line);
                            index = next_index;
                            continue;
                        }
                        self.apply_marpit_comment_directives(
                            &directives,
                            line,
                            &mut classes,
                            &mut theme_params,
                            &mut background_image,
                            &mut footer,
                        );
                    } else if slidev_comment_is_speaker_note(&markdown)
                        && remaining_slide_lines_are_blank(lines, next_index)
                    {
                        if let Some(kind) = region {
                            self.reject_region_content(kind, "speaker notes", line);
                        } else {
                            blocks.push(ContentBlock::SpeakerNotes { markdown });
                        }
                    }
                }
                index = next_index;
                continue;
            }

            if let Some(directive) = directive_start(trimmed) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                let name = directive.name.as_str();
                let rest = directive.rest;
                let (body, next_index, closed) =
                    collect_directive_body(lines, index + 1, directive.fence_len);
                if !closed {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("directive '{name}' is missing a closing ::: marker"),
                    ));
                }
                if let Some(kind) = region
                    && matches!(
                        name,
                        "columns"
                            | "grid"
                            | "stack"
                            | "overlay"
                            | "aside"
                            | "comparison"
                            | "derivation"
                            | "notes"
                            | "background"
                            | "background-image"
                            | "background-color"
                            | "surface-color"
                            | "text-color"
                            | "accent-color"
                            | "muted-color"
                            | "rule-color"
                            | "footer"
                            | "hide-footer"
                            | "slide-number"
                            | "slide-numbers"
                            | "slidenumbers"
                            | "paginate"
                            | "autoscale"
                            | "transition"
                            | "slide-transition"
                            | "preset"
                            | "slide"
                            | "slide-meta"
                            | "slide-metadata"
                            | "variant"
                            | "class"
                            | "classes"
                            | "theme"
                            | "theme-params"
                            | "theme-param"
                    )
                {
                    self.reject_region_content(kind, &format!("directive '{name}'"), line);
                    index = next_index;
                    continue;
                }
                let raw_body = body;
                let placeholder_body = if rest.is_empty() {
                    raw_body.clone()
                } else if raw_body.is_empty() {
                    rest.to_string()
                } else {
                    format!("{rest}\n{raw_body}")
                };
                match name {
                    "notes" => blocks.push(ContentBlock::SpeakerNotes { markdown: raw_body }),
                    "html" => blocks.push(ContentBlock::HtmlOnly { html: raw_body }),
                    "figure" => {
                        let src = directive_attribute(rest, "src");
                        let static_src = directive_static_image_src(rest);
                        let caption = directive_attribute(rest, "caption").or_else(|| {
                            (!raw_body.trim().is_empty()).then(|| raw_body.trim().to_string())
                        });
                        let alt = directive_attribute(rest, "alt")
                            .or_else(|| caption.clone())
                            .unwrap_or_default();
                        if let Some(src) = src {
                            if let Some(options) = self.parse_figure_options(rest, line) {
                                self.validate_figure_asset(&src, line);
                                if let Some(static_src) = &static_src {
                                    self.validate_figure_asset(static_src, line);
                                }
                                blocks.push(ContentBlock::Figure {
                                    src,
                                    alt,
                                    caption,
                                    static_src,
                                    options: self.figure_options_with_defaults(options),
                                });
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "figure directive is missing required src attribute",
                            ));
                        }
                    }
                    "media" | "video" | "audio" | "iframe" | "embed" | "youtube" | "vimeo" => {
                        if let Some(media) = self.parse_media_block(name, rest, &raw_body, line) {
                            blocks.push(media);
                        }
                    }
                    "fit" | "fit-text" => {
                        let markdown = directive_markdown(rest, &raw_body);
                        if markdown.trim().is_empty() {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "fit directive needs text content",
                            ));
                        } else {
                            blocks.push(self.fit_text_block(markdown, line));
                        }
                    }
                    "background" | "background-image" => {
                        if let Some(background) =
                            self.parse_slide_background_block(rest, &raw_body, line)
                        {
                            background_image = Some(background);
                        }
                    }
                    "background-color" | "surface-color" | "text-color" | "accent-color"
                    | "muted-color" | "rule-color" => {
                        if let Some(parameter_name) = slide_theme_shorthand_param_name(name) {
                            if let Some(value) = directive_shorthand_value(rest, &raw_body) {
                                theme_params.insert(parameter_name.to_string(), value);
                            } else {
                                self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!("{name} directive needs a color value"),
                                ));
                            }
                        }
                    }
                    "footer" => {
                        if let Some(value) = directive_shorthand_value(rest, &raw_body) {
                            self.apply_slide_footer_command(
                                SlideFooterCommand::Content(value),
                                line,
                                &mut footer,
                            );
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "footer directive needs text content",
                            ));
                        }
                    }
                    "hide-footer" => {
                        self.apply_slide_footer_command(
                            SlideFooterCommand::Hide,
                            line,
                            &mut footer,
                        );
                    }
                    "slide-number" | "slide-numbers" | "slidenumbers" | "paginate" => {
                        if let Some(value) = directive_shorthand_value(rest, &raw_body) {
                            if let Some(show) = parse_bool_token(&value) {
                                self.apply_slide_footer_command(
                                    SlideFooterCommand::SlideNumbers(show),
                                    line,
                                    &mut footer,
                                );
                            } else {
                                self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!(
                                        "{name} directive expected true or false, found '{value}'"
                                    ),
                                ));
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                format!("{name} directive needs true or false"),
                            ));
                        }
                    }
                    "autoscale" => {
                        if let Some(value) = directive_shorthand_value(rest, &raw_body) {
                            if let Some(value) = parse_bool_token(&value) {
                                autoscale = Some(value);
                            } else {
                                self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!(
                                        "autoscale directive expected true or false, found '{value}'"
                                    ),
                                ));
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "autoscale directive needs true or false",
                            ));
                        }
                    }
                    "transition" | "slide-transition" => {
                        if let Some(value) = directive_shorthand_value(rest, &raw_body) {
                            if let Some(value) = parse_slide_transition(&value) {
                                transition = Some(value);
                            } else {
                                self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!("unknown slide transition '{value}'"),
                                ));
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                format!("{name} directive needs a transition name"),
                            ));
                        }
                    }
                    "preset" => {
                        let raw_preset = directive_attribute(rest, "name")
                            .or_else(|| first_directive_word(rest))
                            .or_else(|| directive_shorthand_value(rest, &raw_body));
                        if let Some(raw_preset) = raw_preset {
                            self.apply_slide_preset(&raw_preset, line, &mut preset);
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "preset directive is missing a preset name",
                            ));
                        }
                    }
                    "slide" | "slide-meta" | "slide-metadata" => {
                        self.apply_slide_metadata_block(
                            rest,
                            &raw_body,
                            line,
                            &mut preset,
                            &mut variant,
                            &mut classes,
                            &mut theme_params,
                            &mut background_image,
                            &mut footer,
                            &mut autoscale,
                            &mut transition,
                        );
                    }
                    "vega-lite" => {
                        let data_url = directive_attribute(rest, "data");
                        if let Some(chart) = self.parse_chart_block(&raw_body, data_url, line) {
                            blocks.push(chart);
                        }
                    }
                    "mermaid" => {
                        if let Some(diagram) = self.parse_mermaid_block(&raw_body, line) {
                            blocks.push(diagram);
                        }
                    }
                    "steps" => {
                        if let Some(steps) = self.parse_steps_block(rest, &raw_body, line) {
                            blocks.push(steps);
                        }
                    }
                    "derivation" => {
                        if variant.is_some_and(|variant| variant != SlideVariant::Derivation) {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "derivation block conflicts with the Slide's existing variant",
                            ));
                        } else {
                            variant = Some(SlideVariant::Derivation);
                        }
                        if let Some(derivation) = self.parse_derivation_block(rest, &raw_body, line)
                        {
                            blocks.push(derivation);
                        }
                    }
                    "comparison" => {
                        if variant.is_some_and(|variant| variant != SlideVariant::Comparison) {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "comparison block conflicts with the Slide's existing variant",
                            ));
                        } else {
                            variant = Some(SlideVariant::Comparison);
                        }
                        if let Some(comparison) = self.parse_comparison_block(rest, &raw_body, line)
                        {
                            blocks.push(comparison);
                        }
                    }
                    "variant" => {
                        let raw_variant = directive_attribute(rest, "name")
                            .or_else(|| first_directive_word(rest));
                        if let Some(raw_variant) = raw_variant {
                            match parse_slide_variant(&raw_variant) {
                                Some(parsed) => variant = Some(parsed),
                                None => self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!("unknown slide variant '{raw_variant}'"),
                                )),
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "variant directive is missing a variant name",
                            ));
                        }
                    }
                    "class" | "classes" => {
                        let raw_classes = parse_slide_class_tokens(rest);
                        if raw_classes.is_empty() {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "class directive is missing one or more class names",
                            ));
                        }
                        for raw_class in raw_classes {
                            if is_valid_slide_class(&raw_class) {
                                if !classes.contains(&raw_class) {
                                    classes.push(raw_class);
                                }
                            } else {
                                self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!(
                                        "invalid slide class '{raw_class}'; use lowercase letters, numbers, and hyphens, starting with a letter"
                                    ),
                                ));
                            }
                        }
                    }
                    "theme" | "theme-params" | "theme-param" => {
                        let params = parse_theme_param_tokens(rest);
                        if params.is_empty() {
                            self.diagnostics.push(Diagnostic::error(
                                self.span_for_line(line),
                                "theme directive needs one or more key=value parameters",
                            ));
                        }
                        for (name, value) in params {
                            if is_valid_theme_param_name(&name) {
                                theme_params.insert(name, value);
                            } else {
                                self.diagnostics.push(Diagnostic::error(
                                    self.span_for_line(line),
                                    format!(
                                        "invalid theme parameter name '{name}'; use letters, numbers, underscores, or hyphens, starting with a letter"
                                    ),
                                ));
                            }
                        }
                    }
                    "columns" | "grid" | "stack" | "overlay" | "aside" => {
                        if let Some(layout) = self.parse_layout_block(name, rest, &raw_body, line) {
                            blocks.push(layout);
                        }
                    }
                    "incremental" | "nonincremental" => {
                        if let Some(list) = self.parse_incremental_list_block(name, &raw_body, line)
                        {
                            blocks.push(list);
                        }
                    }
                    _ => {
                        self.diagnostics.push(Diagnostic::warning(
                            self.span_for_line(line),
                            format!("unsupported directive '{name}'"),
                        ));
                        self.validate_local_directive_dependencies(name, rest, line);
                        blocks.push(ContentBlock::UnsupportedDirective {
                            name: name.to_string(),
                            body: placeholder_body,
                        });
                    }
                }
                index = next_index;
                continue;
            }

            if let Some((level, text)) = heading(line.text) {
                self.flush_paragraph(&mut blocks, &mut paragraph);
                if title.is_none() {
                    title = Some(text.clone());
                }
                blocks.push(ContentBlock::Heading { level, text });
                index += 1;
                continue;
            }

            if looks_like_raw_html(trimmed) {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    "raw HTML must be wrapped in an explicit ::: html content block",
                ));
            }

            paragraph.push(line);
            index += 1;
        }

        self.flush_paragraph(&mut blocks, &mut paragraph);

        Slide {
            id: slide_id(section_index, role, detail_index),
            role,
            preset,
            variant,
            classes,
            theme_params,
            title,
            background_image,
            footer,
            autoscale,
            transition,
            blocks,
        }
    }

    fn append_slide_footnotes(&mut self, blocks: &mut Vec<ContentBlock>) {
        let mut labels = Vec::new();
        for block in blocks.iter() {
            collect_block_footnote_references(block, &mut labels);
        }
        labels.sort_by_key(|(_, order)| *order);
        let mut seen = BTreeMap::<String, usize>::new();
        let mut notes = Vec::new();
        for (label, _) in labels {
            if seen.contains_key(&label) {
                continue;
            }
            let number = seen.len() + 1;
            seen.insert(label.clone(), number);
            let Some(definition) = self.footnote_definitions.get(&label).cloned() else {
                self.diagnostics.push(Diagnostic::error(
                    Some(SourceSpan {
                        source_path: self.source_path.clone(),
                        line: self.body_start_line,
                        column: 1,
                    }),
                    format!("footnote reference '[^{label}]' has no matching definition"),
                ));
                continue;
            };
            let inline_math = inline_math_segments(&definition.markdown);
            for latex in &inline_math {
                self.validate_math(
                    latex,
                    false,
                    SourceLine {
                        number: definition.line,
                        text: "",
                    },
                );
            }
            notes.push(Footnote {
                label,
                number,
                markdown: definition.markdown,
                inline_math,
            });
        }
        if !notes.is_empty() {
            blocks.push(ContentBlock::Footnotes { notes });
        }
    }

    fn flush_paragraph<'b>(
        &mut self,
        blocks: &mut Vec<ContentBlock>,
        paragraph: &mut Vec<SourceLine<'b>>,
    ) {
        if paragraph.is_empty() {
            return;
        }
        let markdown = paragraph
            .iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n");
        let inline_math = inline_math_segments(&markdown);
        if let Some(line) = paragraph.first().copied() {
            for latex in &inline_math {
                self.validate_math(latex, false, line);
            }
        }
        blocks.push(ContentBlock::Paragraph {
            markdown,
            inline_math,
        });
        paragraph.clear();
    }

    fn fit_text_block(&mut self, markdown: String, line: SourceLine<'_>) -> ContentBlock {
        let inline_math = inline_math_segments(&markdown);
        for latex in &inline_math {
            self.validate_math(latex, false, line);
        }
        ContentBlock::FitText {
            markdown,
            inline_math,
        }
    }

    fn collect_quote<'b>(
        &mut self,
        lines: &[SourceLine<'b>],
        start_index: usize,
    ) -> (ContentBlock, usize) {
        let mut markdown_lines = Vec::new();
        let mut index = start_index;
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.text.trim();
            if trimmed.is_empty() {
                break;
            }
            let Some(markdown) = quote_line_markdown(trimmed) else {
                break;
            };
            markdown_lines.push(markdown);
            index += 1;
        }
        while markdown_lines.last().is_some_and(|line| line.is_empty()) {
            markdown_lines.pop();
        }
        let markdown = markdown_lines.join("\n");
        let block = if let Some((kind, title, body)) = parse_callout_quote(&markdown) {
            let inline_math = inline_math_segments(&body);
            if let Some(line) = lines.get(start_index).copied() {
                for latex in &inline_math {
                    self.validate_math(latex, false, line);
                }
            }
            ContentBlock::Callout {
                kind,
                title,
                markdown: body,
                inline_math,
            }
        } else {
            let inline_math = inline_math_segments(&markdown);
            if let Some(line) = lines.get(start_index).copied() {
                for latex in &inline_math {
                    self.validate_math(latex, false, line);
                }
            }
            ContentBlock::Quote {
                markdown,
                inline_math,
            }
        };
        (block, index)
    }

    fn collect_list<'b>(
        &mut self,
        lines: &[SourceLine<'b>],
        start_index: usize,
        build_lists: Option<BuildListsMode>,
    ) -> (ContentBlock, usize) {
        let first_marker = list_line_kind(lines[start_index].text.trim());
        let ordered = first_marker.is_some_and(list_line_kind_is_ordered);
        let reveal_mode = first_marker
            .map(|kind| list_line_reveal_mode(kind, build_lists))
            .unwrap_or(ListRevealMode::None);
        let reveal = reveal_mode.reveals();
        let reveal_skip_first = reveal_mode == ListRevealMode::SkipFirst;
        let mut items = Vec::new();
        let mut index = start_index;
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.text.trim();
            let Some((kind, markdown)) = parse_list_line(trimmed) else {
                break;
            };
            if list_line_kind_is_ordered(kind) != ordered
                || list_line_reveal_mode(kind, build_lists) != reveal_mode
            {
                break;
            }
            let inline_math = inline_math_segments(&markdown);
            for latex in &inline_math {
                self.validate_math(latex, false, line);
            }
            items.push(ListItem {
                markdown,
                inline_math,
            });
            index += 1;
        }
        (
            ContentBlock::List {
                ordered,
                reveal,
                reveal_skip_first,
                items,
            },
            index,
        )
    }

    fn parse_incremental_list_block(
        &mut self,
        name: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let reveal = name == "incremental";
        let mut ordered = None;
        let mut items = Vec::new();
        for source_line in body.lines() {
            let trimmed = source_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((kind, markdown)) = parse_list_line(trimmed) else {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{name} directive must contain only a single Markdown list"),
                ));
                return None;
            };
            let item_ordered = list_line_kind_is_ordered(kind);
            if ordered.is_some_and(|ordered| ordered != item_ordered) {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{name} directive cannot mix ordered and unordered list items"),
                ));
                return None;
            }
            ordered = Some(item_ordered);
            let inline_math = inline_math_segments(&markdown);
            for latex in &inline_math {
                self.validate_math(latex, false, line);
            }
            items.push(ListItem {
                markdown,
                inline_math,
            });
        }
        if items.is_empty() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{name} directive must contain at least one list item"),
            ));
            return None;
        }
        Some(ContentBlock::List {
            ordered: ordered.unwrap_or(false),
            reveal,
            reveal_skip_first: false,
            items,
        })
    }

    fn validate_math(&mut self, latex: &str, display: bool, line: SourceLine<'_>) {
        let Ok(opts) = katex::Opts::builder()
            .display_mode(display)
            .output_type(katex::OutputType::Mathml)
            .throw_on_error(true)
            .build()
        else {
            return;
        };
        if let Err(error) = katex::render_with_opts(latex, &opts) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("unsupported KaTeX math: {error}"),
            ));
        }
    }

    fn parse_code_fence_info(
        &mut self,
        info: Option<&str>,
        line: SourceLine<'_>,
    ) -> ParsedCodeFenceInfo {
        let Some(info) = info.map(str::trim).filter(|info| !info.is_empty()) else {
            return ParsedCodeFenceInfo::default();
        };
        let (language, attributes) = match info.split_once(char::is_whitespace) {
            Some((first, rest)) if !first.contains('=') => {
                (Some(first.to_string()), rest.trim().to_string())
            }
            None if !info.contains('=') => (Some(info.to_string()), String::new()),
            _ => (None, info.to_string()),
        };
        let reveal = directive_attribute(&attributes, "reveal").and_then(|value| {
            match parse_code_reveal(&value) {
                Some(reveal) => Some(reveal),
                None => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("invalid code reveal line groups '{value}'"),
                    ));
                    None
                }
            }
        });
        ParsedCodeFenceInfo { language, reveal }
    }

    fn validate_code_reveal_ranges(
        &mut self,
        reveal: &CodeReveal,
        code: &str,
        line: SourceLine<'_>,
    ) {
        let line_count = code.lines().count();
        for group in &reveal.groups {
            for range in &group.ranges {
                if range.end > line_count {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!(
                            "code reveal group {} references line {} but the block has {line_count} line(s)",
                            group.index, range.end
                        ),
                    ));
                }
            }
        }
    }

    fn span_for_line(&self, line: SourceLine<'_>) -> Option<SourceSpan> {
        Some(SourceSpan {
            source_path: self.source_path.clone(),
            line: line.number,
            column: first_nonspace_column(line.text),
        })
    }

    fn validate_local_directive_dependencies(
        &mut self,
        name: &str,
        attributes: &str,
        line: SourceLine<'_>,
    ) {
        let Some(deck_root) = &self.deck_root else {
            return;
        };
        for key in ["src", "data"] {
            let Some(value) = directive_attribute(attributes, key) else {
                continue;
            };
            if looks_remote_or_fragment(&value) {
                continue;
            }
            let path = deck_root.join(&value);
            if !path.exists() {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "directive '{name}' references missing local {key} dependency '{value}'"
                    ),
                ));
            }
        }
    }

    fn parse_steps_block(
        &mut self,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let pdf_policy = match directive_attribute(attributes, "pdf") {
            Some(value) => match parse_step_pdf_policy(&value) {
                Some(policy) => policy,
                None => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("invalid steps pdf policy '{value}'"),
                    ));
                    return None;
                }
            },
            None => StepPdfPolicy::FinalState,
        };

        let mut steps = Vec::new();
        let mut expected_index = 1usize;
        for source_line in body.lines() {
            if source_line.trim().is_empty() {
                continue;
            }
            let Some((index, markdown)) = parse_ordered_step(source_line) else {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    "steps directive entries must be ordered list items",
                ));
                return None;
            };
            if index != expected_index {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("steps directive expected item {expected_index}, found item {index}"),
                ));
                return None;
            }
            steps.push(Step { index, markdown });
            expected_index += 1;
        }

        if steps.is_empty() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "steps directive must contain at least one step",
            ));
            return None;
        }

        Some(ContentBlock::Steps { pdf_policy, steps })
    }

    fn apply_slide_metadata_block(
        &mut self,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
        preset: &mut Option<String>,
        variant: &mut Option<SlideVariant>,
        classes: &mut Vec<String>,
        theme_params: &mut BTreeMap<String, String>,
        background_image: &mut Option<DeckBackgroundImage>,
        footer: &mut SlideFooter,
        autoscale: &mut Option<bool>,
        transition: &mut Option<SlideTransition>,
    ) {
        if let Some(raw_variant) = directive_attribute(attributes, "variant")
            .or_else(|| directive_attribute(attributes, "name"))
        {
            self.apply_slide_variant(&raw_variant, line, variant);
        }
        if let Some(raw_preset) = directive_attribute(attributes, "preset") {
            self.apply_slide_preset(&raw_preset, line, preset);
        }
        for raw_class in parse_slide_metadata_class_attributes(attributes) {
            self.apply_slide_class(&raw_class, line, classes);
        }
        for (name, value) in parse_theme_param_tokens(attributes) {
            self.apply_slide_theme_param(&name, value, line, theme_params);
        }

        let body = body.trim();
        if body.is_empty() {
            if attributes.trim().is_empty() {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    "slide metadata directive needs YAML body content or attributes",
                ));
            }
            return;
        }

        let value = match serde_yaml::from_str::<serde_yaml::Value>(body) {
            Ok(value) => value,
            Err(error) => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "slide metadata YAML is invalid{}: {error}",
                        yaml_location(&error)
                    ),
                ));
                return;
            }
        };
        let serde_yaml::Value::Mapping(mapping) = value else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "slide metadata directive body must be a YAML mapping",
            ));
            return;
        };

        for (key, value) in mapping {
            let Some(key) = key.as_str() else {
                continue;
            };
            match key.replace('_', "-").as_str() {
                "variant" => {
                    if let Some(raw_variant) = scalar_to_string(&value) {
                        self.apply_slide_variant(&raw_variant, line, variant);
                    }
                }
                "preset" => {
                    if let Some(raw_preset) = scalar_to_string(&value) {
                        self.apply_slide_preset(&raw_preset, line, preset);
                    }
                }
                "class" | "classes" => {
                    for raw_class in yaml_string_list(&value) {
                        self.apply_slide_class(&raw_class, line, classes);
                    }
                }
                "theme" | "theme-params" | "theme-param" => {
                    for (name, value) in yaml_mapping_to_string_map(&value) {
                        self.apply_slide_theme_param(&name, value, line, theme_params);
                    }
                }
                "colors" | "color" => {
                    for (name, value) in yaml_mapping_to_string_map(&value) {
                        let parameter_name = slide_theme_shorthand_param_name(&name)
                            .unwrap_or(name.as_str())
                            .to_string();
                        self.apply_slide_theme_param(&parameter_name, value, line, theme_params);
                    }
                }
                "background" | "background-image" => {
                    let span = self.span_for_line(line);
                    let spans = BackgroundAuthoringSpans {
                        form: span.clone(),
                        fields: BTreeMap::new(),
                    };
                    self.theme_api_v1_diagnostics
                        .extend(validate_v1_deck_background_value(&value, &spans));
                    if let serde_yaml::Value::Mapping(mapping) = &value {
                        for field in ["title", "splash"] {
                            if mapping.contains_key(serde_yaml::Value::String(field.to_string())) {
                                self.theme_api_v1_diagnostics.push(Diagnostic::error(
                                    span.clone(),
                                    format!(
                                        "Theme API v1 background field '{field}' is Deck-only and cannot be used on a Slide"
                                    ),
                                ));
                            }
                        }
                    }
                    if let Some(background) = background_image_from_yaml(&value, span.clone()) {
                        self.validate_slide_metadata_background(&background, line);
                        *background_image = Some(DeckBackgroundImage {
                            splash: false,
                            splash_explicit: false,
                            title_application: BackgroundTitleApplication::Clean,
                            ..background
                        });
                    } else {
                        self.diagnostics.push(Diagnostic::error(
                            self.span_for_line(line),
                            "slide metadata background needs a src string or mapping with src",
                        ));
                    }
                }
                "footer" => {
                    if let Some(content) = scalar_to_string(&value) {
                        self.apply_slide_footer_command(
                            SlideFooterCommand::Content(content),
                            line,
                            footer,
                        );
                    }
                }
                "hide-footer" | "hide_footer" => {
                    if yaml_bool(&value).unwrap_or(true) {
                        self.apply_slide_footer_command(SlideFooterCommand::Hide, line, footer);
                    }
                }
                "slide-numbers" | "slide_numbers" | "slidenumbers" | "paginate" => {
                    if let Some(show) = yaml_bool(&value) {
                        self.apply_slide_footer_command(
                            SlideFooterCommand::SlideNumbers(show),
                            line,
                            footer,
                        );
                    }
                }
                "autoscale" => {
                    if let Some(value) = yaml_bool(&value) {
                        *autoscale = Some(value);
                    }
                }
                "transition" | "slide-transition" | "slide_transition" => {
                    if let Some(value) = slide_transition_from_yaml(&value) {
                        *transition = Some(value);
                    }
                }
                other => self.diagnostics.push(Diagnostic::warning(
                    self.span_for_line(line),
                    format!("unsupported slide metadata key '{other}'"),
                )),
            }
        }
    }

    fn apply_marpit_comment_directives(
        &mut self,
        directives: &[MarpitCommentDirective],
        line: SourceLine<'_>,
        classes: &mut Vec<String>,
        theme_params: &mut BTreeMap<String, String>,
        background_image: &mut Option<DeckBackgroundImage>,
        footer: &mut SlideFooter,
    ) {
        for directive in directives {
            match directive.name.as_str() {
                "paginate" => {
                    if let Some(show) = parse_bool_token(&directive.value) {
                        self.apply_slide_footer_command(
                            SlideFooterCommand::SlideNumbers(show),
                            line,
                            footer,
                        );
                        if !directive.spot {
                            self.marpit_defaults.footer.slide_numbers = Some(show);
                        }
                    } else {
                        self.diagnostics.push(Diagnostic::error(
                            self.span_for_line(line),
                            format!(
                                "Marpit paginate directive expected true or false, found '{}'",
                                directive.value
                            ),
                        ));
                    }
                }
                "footer" => {
                    self.apply_slide_footer_command(
                        SlideFooterCommand::Content(directive.value.clone()),
                        line,
                        footer,
                    );
                    if !directive.spot && !directive.value.trim().is_empty() {
                        self.marpit_defaults.footer.content =
                            Some(directive.value.trim().to_string());
                        self.marpit_defaults.footer.hidden = false;
                    }
                }
                "class" => {
                    let raw_classes = marpit_class_tokens(&directive.value);
                    for raw_class in &raw_classes {
                        self.apply_slide_class(raw_class, line, classes);
                    }
                    if !directive.spot {
                        let mut defaults = Vec::new();
                        for raw_class in raw_classes {
                            if is_valid_slide_class(&raw_class)
                                && !defaults.iter().any(|class| class == &raw_class)
                            {
                                defaults.push(raw_class);
                            }
                        }
                        self.marpit_defaults.classes = defaults;
                    }
                }
                "background-color" | "color" => {
                    let parameter_name = if directive.name == "background-color" {
                        "background"
                    } else {
                        "text"
                    };
                    self.apply_slide_theme_param(
                        parameter_name,
                        directive.value.clone(),
                        line,
                        theme_params,
                    );
                    if !directive.spot {
                        self.marpit_defaults
                            .theme_params
                            .insert(parameter_name.to_string(), directive.value.clone());
                    }
                }
                "background-image" => {
                    if let Some(background) =
                        self.marpit_background_image_from_directives(directives, line)
                    {
                        *background_image = Some(background.clone());
                        if !directive.spot {
                            self.marpit_defaults.background_image = Some(background);
                        }
                    }
                }
                "background-position" | "background-size" => {
                    if let Some(existing) = background_image.as_mut() {
                        self.apply_marpit_background_property(
                            existing,
                            &directive.name,
                            &directive.value,
                            line,
                        );
                    }
                    if !directive.spot
                        && let Some(mut existing) = self.marpit_defaults.background_image.clone()
                    {
                        self.apply_marpit_background_property(
                            &mut existing,
                            &directive.name,
                            &directive.value,
                            line,
                        );
                        self.marpit_defaults.background_image = Some(existing);
                    }
                }
                _ => {}
            }
        }
    }

    fn marpit_background_image_from_directives(
        &mut self,
        directives: &[MarpitCommentDirective],
        line: SourceLine<'_>,
    ) -> Option<DeckBackgroundImage> {
        let image = directives
            .iter()
            .rev()
            .find(|directive| directive.name == "background-image")?;
        let Some(src) = marpit_background_image_src(&image.value) else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!(
                    "Marpit backgroundImage directive needs a URL or path, found '{}'",
                    image.value
                ),
            ));
            return None;
        };
        self.validate_background_asset("Marpit backgroundImage", &src, line);
        let mut background = DeckBackgroundImage {
            src,
            intent: BackgroundImageIntent::Contextual,
            intent_explicit: false,
            alt: String::new(),
            description: None,
            position: "center center".to_string(),
            fit: BackgroundImageFit::Cover,
            split: None,
            dim: DeckBackgroundImage::DEFAULT_DIM,
            grayscale: DeckBackgroundImage::DEFAULT_GRAYSCALE,
            saturate: DeckBackgroundImage::DEFAULT_SATURATE,
            blur: DeckBackgroundImage::DEFAULT_BLUR,
            splash: false,
            splash_explicit: false,
            title_application: BackgroundTitleApplication::Clean,
            source_span: self.span_for_line(line),
        };
        for directive in directives
            .iter()
            .filter(|directive| directive.spot == image.spot)
        {
            if matches!(
                directive.name.as_str(),
                "background-position" | "background-size"
            ) {
                self.apply_marpit_background_property(
                    &mut background,
                    &directive.name,
                    &directive.value,
                    line,
                );
            }
        }
        Some(background)
    }

    fn apply_marpit_background_property(
        &mut self,
        background: &mut DeckBackgroundImage,
        name: &str,
        value: &str,
        line: SourceLine<'_>,
    ) {
        match name {
            "background-position" => {
                if is_safe_css_token_list(value) {
                    background.position = value.to_string();
                } else {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!(
                            "Marpit backgroundPosition contains unsupported CSS tokens '{value}'"
                        ),
                    ));
                }
            }
            "background-size" => match parse_background_image_fit(value) {
                Some(fit) => background.fit = fit,
                None => self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "Marpit backgroundSize supports cover or contain for static export, found '{value}'"
                    ),
                )),
            },
            _ => {}
        }
    }

    fn apply_slide_variant(
        &mut self,
        raw_variant: &str,
        line: SourceLine<'_>,
        variant: &mut Option<SlideVariant>,
    ) {
        match parse_slide_variant(raw_variant) {
            Some(parsed) => *variant = Some(parsed),
            None => self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("unknown slide variant '{raw_variant}'"),
            )),
        }
    }

    fn apply_slide_preset(
        &mut self,
        raw_preset: &str,
        line: SourceLine<'_>,
        preset: &mut Option<String>,
    ) {
        if is_valid_slide_preset(raw_preset) {
            *preset = Some(raw_preset.to_string());
        } else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!(
                    "invalid slide preset '{raw_preset}'; use lowercase letters, numbers, and hyphens, starting with a letter"
                ),
            ));
        }
    }

    fn apply_slide_class(
        &mut self,
        raw_class: &str,
        line: SourceLine<'_>,
        classes: &mut Vec<String>,
    ) {
        if is_valid_slide_class(raw_class) {
            if !classes.iter().any(|class| class == raw_class) {
                classes.push(raw_class.to_string());
            }
        } else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!(
                    "invalid slide class '{raw_class}'; use lowercase letters, numbers, and hyphens, starting with a letter"
                ),
            ));
        }
    }

    fn apply_slide_theme_param(
        &mut self,
        name: &str,
        value: String,
        line: SourceLine<'_>,
        theme_params: &mut BTreeMap<String, String>,
    ) {
        if is_valid_theme_param_name(name) {
            theme_params.insert(name.to_string(), value);
        } else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!(
                    "invalid theme parameter name '{name}'; use letters, numbers, underscores, or hyphens, starting with a letter"
                ),
            ));
        }
    }

    fn validate_slide_metadata_background(
        &mut self,
        background: &DeckBackgroundImage,
        line: SourceLine<'_>,
    ) {
        self.validate_background_asset("slide metadata background", &background.src, line);
        if !is_safe_css_token_list(&background.position) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!(
                    "slide metadata background position contains unsupported CSS tokens '{}'",
                    background.position
                ),
            ));
        }
        self.validate_background_treatment(
            "slide metadata background",
            background.dim,
            background.grayscale,
            background.saturate,
            background.blur,
            line,
        );
    }

    fn apply_slide_footer_command(
        &mut self,
        command: SlideFooterCommand,
        line: SourceLine<'_>,
        footer: &mut SlideFooter,
    ) {
        match command {
            SlideFooterCommand::Content(content) => {
                let content = content.trim();
                if content.is_empty() {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        "slide footer text cannot be empty",
                    ));
                } else {
                    footer.content = Some(content.to_string());
                    footer.hidden = false;
                }
            }
            SlideFooterCommand::Hide => {
                footer.hidden = true;
            }
            SlideFooterCommand::SlideNumbers(show) => {
                footer.slide_numbers = Some(show);
            }
        }
    }

    fn parse_layout_block(
        &mut self,
        name: &str,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let kind = match name {
            "columns" => LayoutKind::Columns,
            "grid" => LayoutKind::Grid,
            "stack" => LayoutKind::Stack,
            "overlay" => LayoutKind::Overlay,
            "aside" => LayoutKind::Aside,
            _ => return None,
        };
        let mut values = self.parse_layout_values(kind, attributes, line)?;
        let parsed_regions = parse_layout_region_sources(kind, body);
        if matches!(
            kind,
            LayoutKind::Grid | LayoutKind::Stack | LayoutKind::Overlay | LayoutKind::Aside
        ) && parsed_regions
            .regions
            .iter()
            .any(|region| region.directive_offset.is_none())
        {
            let region_name = match kind {
                LayoutKind::Grid => "cell",
                LayoutKind::Stack => "item",
                LayoutKind::Overlay => "base/annotation",
                LayoutKind::Aside => "primary/supporting",
                LayoutKind::Columns => unreachable!(),
            };
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{kind:?} requires explicit four-colon ':::: {region_name}' regions"),
            ));
            return None;
        }
        if kind == LayoutKind::Grid && values.tracks.is_none() {
            values.tracks = Some(vec![
                LayoutSize::Fraction { units: 1 },
                LayoutSize::Fraction { units: 1 },
            ]);
        }
        if values.widths.is_none() {
            values.widths = parsed_regions.widths;
        }
        let regions = parsed_regions
            .regions
            .into_iter()
            .map(|region| self.parse_layout_region(kind, region, line))
            .collect::<Vec<_>>();
        if matches!(kind, LayoutKind::Grid | LayoutKind::Stack) && regions.len() < 2 {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{kind:?} requires at least two regions"),
            ));
            return None;
        }
        if kind == LayoutKind::Grid {
            self.validate_grid_regions(&values, &regions, line);
        }
        if kind == LayoutKind::Overlay && !self.validate_overlay_regions(&regions, line) {
            return None;
        }
        if kind == LayoutKind::Aside && !self.validate_aside_regions(&regions, line) {
            return None;
        }
        Some(ContentBlock::Layout {
            kind,
            values,
            regions,
        })
    }

    fn parse_comparison_block(
        &mut self,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        if !attributes.trim().is_empty() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "comparison block does not accept layout attributes; use Grid for three or more regions",
            ));
            return None;
        }
        let parsed = parse_layout_region_sources(LayoutKind::Aside, body);
        if parsed
            .regions
            .iter()
            .any(|region| region.directive_offset.is_none())
        {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "comparison requires explicit four-colon primary and supporting regions",
            ));
            return None;
        }
        let regions = parsed
            .regions
            .into_iter()
            .map(|region| self.parse_layout_region(LayoutKind::Columns, region, line))
            .collect::<Vec<_>>();
        if !self.validate_comparison_regions(&regions, line) {
            return None;
        }
        Some(ContentBlock::Layout {
            kind: LayoutKind::Columns,
            values: LayoutValues {
                widths: Some(vec![
                    LayoutSize::Fraction { units: 1 },
                    LayoutSize::Fraction { units: 1 },
                ]),
                gap: Some(LayoutSize::Scale { step: 6 }),
                align: Some(LayoutAlign::Stretch),
                ..LayoutValues::default()
            },
            regions,
        })
    }

    fn parse_derivation_block(
        &mut self,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let pdf_policy = match directive_attribute(attributes, "pdf").as_deref() {
            None => StepPdfPolicy::FinalState,
            Some(value) => match parse_step_pdf_policy(value) {
                Some(policy) => policy,
                None => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("derivation pdf policy must be final or pages, found '{value}'"),
                    ));
                    return None;
                }
            },
        };
        let Some(parsed_regions) = parse_derivation_region_sources(body) else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "derivation requires explicit four-colon context and stage regions",
            ));
            return None;
        };
        let mut regions = parsed_regions
            .into_iter()
            .map(|region| self.parse_layout_region(LayoutKind::Stack, region, line))
            .collect::<Vec<_>>();
        if regions.len() < 2
            || regions[0].role != Some(LayoutRegionRole::Stable)
            || regions[1..]
                .iter()
                .any(|region| region.role != Some(LayoutRegionRole::Change))
            || regions.len() > 7
        {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "derivation requires one context followed by one to six stage regions",
            ));
            return None;
        }
        let mut valid = true;
        for region in &regions {
            if region.name.as_deref().is_none_or(str::is_empty) {
                self.diagnostics.push(Diagnostic::error(
                    region.source_span.clone(),
                    "derivation context and stages require non-empty labels",
                ));
                valid = false;
            }
            if region.blocks.iter().any(content_block_has_progression) {
                self.diagnostics.push(Diagnostic::error(
                    region.source_span.clone(),
                    "derivation regions cannot nest Steps, fragmented lists, or revealed code; each stage is already one semantic Step",
                ));
                valid = false;
            }
        }
        if !valid {
            return None;
        }
        for (index, region) in regions.iter_mut().skip(1).enumerate() {
            region.derivation_step = Some(index + 1);
        }
        Some(ContentBlock::Layout {
            kind: LayoutKind::Stack,
            values: LayoutValues {
                gap: Some(LayoutSize::Scale { step: 3 }),
                align: Some(LayoutAlign::Stretch),
                step_pdf_policy: Some(pdf_policy),
                ..LayoutValues::default()
            },
            regions,
        })
    }

    fn validate_comparison_regions(
        &mut self,
        regions: &[LayoutRegion],
        line: SourceLine<'_>,
    ) -> bool {
        let roles_valid = regions.len() == 2
            && regions[0].role == Some(LayoutRegionRole::Primary)
            && regions[1].role == Some(LayoutRegionRole::Supporting);
        if !roles_valid {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "comparison requires exactly one primary region followed by one supporting region",
            ));
            return false;
        }
        let mut valid = true;
        for region in regions {
            if region.name.as_deref().is_none_or(str::is_empty) {
                self.diagnostics.push(Diagnostic::error(
                    region
                        .source_span
                        .clone()
                        .or_else(|| self.span_for_line(line)),
                    "comparison regions require a non-empty label",
                ));
                valid = false;
            }
        }
        valid
    }

    fn normalize_comparison_pattern(
        &mut self,
        variant: Option<SlideVariant>,
        blocks: &mut [ContentBlock],
    ) {
        if variant != Some(SlideVariant::Comparison) {
            return;
        }
        let comparison_layouts = blocks
            .iter_mut()
            .filter_map(|block| match block {
                ContentBlock::Layout { kind, regions, .. }
                    if matches!(kind, LayoutKind::Columns | LayoutKind::Grid) =>
                {
                    Some((*kind, regions))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if comparison_layouts.len() != 1 {
            self.diagnostics.push(Diagnostic::error(
                None,
                "comparison Slide variant requires exactly one Columns or Grid Layout",
            ));
            return;
        }
        let (kind, regions) = comparison_layouts.into_iter().next().unwrap();
        if (kind == LayoutKind::Columns && regions.len() != 2) || regions.len() < 2 {
            self.diagnostics.push(Diagnostic::error(
                regions.first().and_then(|region| region.source_span.clone()),
                "comparison Columns require exactly two regions; use a labeled Grid for three or more",
            ));
            return;
        }
        for (index, region) in regions.iter_mut().enumerate() {
            if region.name.as_deref().is_none_or(str::is_empty) {
                self.diagnostics.push(Diagnostic::error(
                    region.source_span.clone(),
                    "comparison regions require a non-empty label",
                ));
            }
            let expected = if index == 0 {
                LayoutRegionRole::Primary
            } else {
                LayoutRegionRole::Supporting
            };
            if let Some(role) = region.role
                && role != expected
            {
                self.diagnostics.push(Diagnostic::error(
                    region.source_span.clone(),
                    "comparison region roles must be primary first, then supporting",
                ));
            } else {
                region.role = Some(expected);
            }
        }
    }

    fn parse_media_block(
        &mut self,
        name: &str,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let raw_kind = if name == "media" {
            directive_attribute(attributes, "kind").or_else(|| first_directive_word(attributes))
        } else {
            Some(name.to_string())
        };
        let kind = match raw_kind.as_deref().and_then(parse_media_kind) {
            Some(kind) => kind,
            None => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    "media directive must declare kind video, audio, or iframe",
                ));
                return None;
            }
        };
        let Some(raw_src) = directive_attribute(attributes, "src") else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "media directive is missing required src attribute",
            ));
            return None;
        };
        let (src, query_start_time) = self.parse_media_source(kind, &raw_src, line);
        let start_time = match directive_attribute(attributes, "start") {
            Some(value) => self.parse_media_start_time(&value, "media start", line),
            None => query_start_time,
        };
        let options = parse_figure_options_from_attributes(attributes);
        self.validate_visual_options_parse("media", &options, line);
        self.validate_media_asset("src", &src, line);
        let poster = directive_attribute(attributes, "poster");
        if let Some(poster) = &poster {
            self.validate_media_asset("poster", poster, line);
        }
        let caption = directive_attribute(attributes, "caption")
            .or_else(|| (!body.trim().is_empty()).then(|| body.trim().to_string()));
        let title = directive_attribute(attributes, "title");
        let alt = directive_attribute(attributes, "alt")
            .or_else(|| caption.clone())
            .or_else(|| title.clone())
            .unwrap_or_default();
        Some(ContentBlock::Media {
            kind,
            src,
            title,
            caption,
            poster,
            alt,
            start_time,
            options: options.options,
            autoplay: directive_bool(attributes, "autoplay").unwrap_or(false),
            controls: directive_bool(attributes, "controls").unwrap_or(true),
            loop_playback: directive_bool(attributes, "loop").unwrap_or(false),
            muted: directive_bool(attributes, "muted")
                .or_else(|| directive_bool(attributes, "mute"))
                .unwrap_or(false),
            autoadvance: directive_bool(attributes, "autoadvance").unwrap_or(false),
            visual_hidden: directive_bool(attributes, "hide").unwrap_or(false),
        })
    }

    fn parse_markdown_media(
        &mut self,
        trimmed: &str,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let media = parse_markdown_media(trimmed)?;
        self.validate_media_asset("src", &media.src, line);
        if let Some(poster) = &media.poster {
            self.validate_media_asset("poster", poster, line);
        }
        self.validate_visual_options_parse("media", &media.options, line);
        let start_time = match media.raw_start_time {
            Some(value) => self.parse_media_start_time(&value, "media start", line),
            None => media.start_time,
        };
        Some(ContentBlock::Media {
            kind: media.kind,
            src: media.src,
            title: media.title,
            caption: media.caption,
            poster: media.poster,
            alt: media.alt,
            start_time,
            options: media.options.options,
            autoplay: media.autoplay,
            controls: media.controls,
            loop_playback: media.loop_playback,
            muted: media.muted,
            autoadvance: media.autoadvance,
            visual_hidden: media.visual_hidden,
        })
    }

    fn parse_media_source(
        &mut self,
        kind: MediaKind,
        raw_src: &str,
        line: SourceLine<'_>,
    ) -> (String, Option<u32>) {
        if kind == MediaKind::Iframe {
            return embed_source_from_url(raw_src).unwrap_or_else(|| (raw_src.to_string(), None));
        }
        let Some((src, query)) = raw_src.split_once('?') else {
            return (raw_src.to_string(), None);
        };
        let start_time = query.split('&').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (key == "t").then(|| self.parse_media_start_time(value, "media start offset", line))?
        });
        (src.to_string(), start_time)
    }

    fn parse_media_start_time(
        &mut self,
        value: &str,
        label: &str,
        line: SourceLine<'_>,
    ) -> Option<u32> {
        match parse_media_start_time(value) {
            Some(seconds) => Some(seconds),
            None => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{label} '{value}' must be seconds, 90s, 1m30s, or 1h2m3s"),
                ));
                None
            }
        }
    }

    fn collect_inline_gallery<'b>(
        &mut self,
        lines: &[SourceLine<'b>],
        start: usize,
    ) -> (Option<ContentBlock>, usize) {
        let mut items = Vec::new();
        let mut columns = None;
        let mut index = start;
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.text.trim();
            if trimmed.is_empty() {
                break;
            }
            if self.parse_markdown_media(trimmed, line).is_some()
                || self
                    .parse_markdown_background_image(trimmed, line)
                    .is_some()
            {
                break;
            }
            let Some(figure) = parse_markdown_image(trimmed) else {
                break;
            };
            if !markdown_image_raw_alt_has_token(&figure.raw_alt, "inline") {
                break;
            }
            self.validate_figure_asset(&figure.src, line);
            if let Some(static_src) = &figure.static_src {
                self.validate_figure_asset(static_src, line);
            }
            if let Some(item_columns) = parse_gallery_columns(&figure.raw_alt) {
                columns = Some(item_columns);
            }
            if self.validate_figure_options_parse(&figure.options, line) {
                items.push(GalleryItem {
                    src: figure.src.clone(),
                    static_src: figure.static_src.clone(),
                    alt: inline_figure_alt_text(&figure),
                    caption: figure.caption.clone(),
                    options: self.figure_options_with_defaults(figure.options.options),
                });
            }
            index += 1;
        }
        let block = (!items.is_empty()).then_some(ContentBlock::Gallery { items, columns });
        (block, index)
    }

    fn figure_options_with_defaults(&self, mut options: FigureOptions) -> FigureOptions {
        if options.radius.is_none() {
            options.radius = self.image_corner_radius.clone();
        }
        options
    }

    fn parse_slide_background_block(
        &mut self,
        attributes: &str,
        body: &str,
        line: SourceLine<'_>,
    ) -> Option<DeckBackgroundImage> {
        self.validate_v1_background_attributes(
            attributes,
            &[
                "src",
                "intent",
                "alt",
                "description",
                "position",
                "fit",
                "split",
                "dim",
                "grayscale",
                "saturate",
                "blur",
            ],
            false,
            line,
        );
        let Some(src) = directive_attribute(attributes, "src") else {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "background directive is missing required src attribute",
            ));
            return None;
        };
        let raw_intent = directive_attribute(attributes, "intent");
        if raw_intent
            .as_deref()
            .is_some_and(|value| parse_background_image_intent(value).is_none())
        {
            self.theme_api_v1_diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "Theme API v1 Slide background intent must be 'decorative', 'contextual', or 'evidence'",
            ));
        }
        self.validate_background_asset("background", &src, line);
        let position = directive_attribute(attributes, "position")
            .unwrap_or_else(|| "center center".to_string());
        if !is_safe_css_token_list(&position) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("background position contains unsupported CSS tokens '{position}'"),
            ));
        }
        let raw_fit = directive_attribute(attributes, "fit");
        if raw_fit
            .as_deref()
            .is_some_and(|value| parse_background_image_fit(value).is_none())
        {
            self.theme_api_v1_diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "Theme API v1 Slide background fit must be 'cover' or 'contain'",
            ));
        }
        let fit = raw_fit
            .as_deref()
            .and_then(parse_background_image_fit)
            .unwrap_or(BackgroundImageFit::Cover);
        let raw_split = directive_attribute(attributes, "split");
        if raw_split
            .as_deref()
            .is_some_and(|value| parse_background_image_split(value).is_none())
        {
            self.theme_api_v1_diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "Theme API v1 Slide background split must be 'left' or 'right' with an optional 10%-90% size",
            ));
        }
        let dim = self.parse_background_directive_u8(
            attributes,
            "dim",
            DeckBackgroundImage::DEFAULT_DIM,
            100,
            line,
        );
        let grayscale = self.parse_background_directive_u8(
            attributes,
            "grayscale",
            DeckBackgroundImage::DEFAULT_GRAYSCALE,
            100,
            line,
        );
        let saturate = self.parse_background_directive_u8(
            attributes,
            "saturate",
            DeckBackgroundImage::DEFAULT_SATURATE,
            100,
            line,
        );
        let blur = self.parse_background_directive_u8(
            attributes,
            "blur",
            DeckBackgroundImage::DEFAULT_BLUR,
            24,
            line,
        );
        self.validate_background_treatment("background", dim, grayscale, saturate, blur, line);
        Some(DeckBackgroundImage {
            src,
            intent: raw_intent
                .as_deref()
                .and_then(parse_background_image_intent)
                .unwrap_or_default(),
            intent_explicit: raw_intent.is_some(),
            alt: directive_attribute(attributes, "alt").unwrap_or_default(),
            description: directive_attribute(attributes, "description")
                .or_else(|| (!body.trim().is_empty()).then(|| body.trim().to_string())),
            position,
            fit,
            split: raw_split.as_deref().and_then(parse_background_image_split),
            dim,
            grayscale,
            saturate,
            blur,
            splash: false,
            splash_explicit: false,
            title_application: BackgroundTitleApplication::Clean,
            source_span: self.span_for_line(line),
        })
    }

    fn parse_markdown_background_image(
        &mut self,
        trimmed: &str,
        line: SourceLine<'_>,
    ) -> Option<DeckBackgroundImage> {
        let figure = parse_markdown_image(trimmed)?;
        let raw_alt = figure.raw_alt.trim();
        let first_word = raw_alt.split_whitespace().next()?;
        if first_word != "bg" {
            return None;
        }
        self.validate_v1_background_attributes(
            raw_alt,
            &[
                "intent",
                "alt",
                "description",
                "position",
                "fit",
                "split",
                "dim",
                "grayscale",
                "saturate",
                "blur",
            ],
            true,
            line,
        );
        self.validate_background_asset("background", &figure.src, line);
        let raw_intent = directive_attribute(raw_alt, "intent");
        if raw_intent
            .as_deref()
            .is_some_and(|value| parse_background_image_intent(value).is_none())
        {
            self.theme_api_v1_diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "Theme API v1 Markdown background intent must be 'decorative', 'contextual', or 'evidence'",
            ));
        }
        let position =
            directive_attribute(raw_alt, "position").unwrap_or_else(|| "center center".to_string());
        if !is_safe_css_token_list(&position) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("background position contains unsupported CSS tokens '{position}'"),
            ));
        }
        let token_fit = raw_alt
            .split_whitespace()
            .skip(1)
            .find_map(parse_background_image_fit);
        let raw_split = directive_attribute(raw_alt, "split");
        if raw_split
            .as_deref()
            .is_some_and(|value| parse_background_image_split(value).is_none())
        {
            self.theme_api_v1_diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "Theme API v1 Markdown background split must be 'left' or 'right' with an optional 10%-90% size",
            ));
        }
        let split = raw_alt
            .split_whitespace()
            .skip(1)
            .find_map(parse_background_image_split)
            .or_else(|| raw_split.as_deref().and_then(parse_background_image_split));
        let raw_fit = directive_attribute(raw_alt, "fit");
        if raw_fit
            .as_deref()
            .is_some_and(|value| parse_background_image_fit(value).is_none())
        {
            self.theme_api_v1_diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                "Theme API v1 Markdown background fit must be 'cover' or 'contain'",
            ));
        }
        let fit = raw_fit
            .as_deref()
            .and_then(parse_background_image_fit)
            .or(token_fit)
            .unwrap_or(BackgroundImageFit::Cover);
        let dim = self.parse_background_directive_u8(
            raw_alt,
            "dim",
            DeckBackgroundImage::DEFAULT_DIM,
            100,
            line,
        );
        let grayscale = self.parse_background_directive_u8(
            raw_alt,
            "grayscale",
            DeckBackgroundImage::DEFAULT_GRAYSCALE,
            100,
            line,
        );
        let saturate = self.parse_background_directive_u8(
            raw_alt,
            "saturate",
            DeckBackgroundImage::DEFAULT_SATURATE,
            100,
            line,
        );
        let blur = self.parse_background_directive_u8(
            raw_alt,
            "blur",
            DeckBackgroundImage::DEFAULT_BLUR,
            24,
            line,
        );
        self.validate_background_treatment("background", dim, grayscale, saturate, blur, line);
        Some(DeckBackgroundImage {
            src: figure.src,
            intent: raw_intent
                .as_deref()
                .and_then(parse_background_image_intent)
                .unwrap_or_default(),
            intent_explicit: raw_intent.is_some(),
            alt: markdown_background_alt_text(raw_alt)
                .or(figure.caption)
                .unwrap_or_default(),
            description: directive_attribute(raw_alt, "description"),
            position,
            fit,
            split,
            dim,
            grayscale,
            saturate,
            blur,
            splash: false,
            splash_explicit: false,
            title_application: BackgroundTitleApplication::Clean,
            source_span: self.span_for_line(line),
        })
    }

    fn validate_v1_background_attributes(
        &mut self,
        attributes: &str,
        allowed: &[&str],
        markdown_shorthand: bool,
        line: SourceLine<'_>,
    ) {
        let mut seen = BTreeMap::<String, usize>::new();
        for (index, token) in split_directive_tokens(attributes).into_iter().enumerate() {
            let Some((name, _)) = token.split_once('=') else {
                let valid_shorthand = markdown_shorthand
                    && ((index == 0 && token == "bg")
                        || parse_background_image_fit(&token).is_some()
                        || parse_background_image_split(&token).is_some());
                if !valid_shorthand {
                    self.theme_api_v1_diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!(
                            "Theme API v1 Slide background has unknown bare token '{token}'; write alternative text as alt=\"...\""
                        ),
                    ));
                }
                continue;
            };
            if !allowed.contains(&name) {
                self.theme_api_v1_diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("Theme API v1 Slide background has unknown field '{name}'"),
                ));
            }
            let count = seen.entry(name.to_string()).or_default();
            *count += 1;
            if *count > 1 {
                self.theme_api_v1_diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("Theme API v1 Slide background repeats field '{name}'"),
                ));
            }
        }
    }

    fn parse_background_directive_u8(
        &mut self,
        attributes: &str,
        key: &str,
        default: u8,
        max: u8,
        line: SourceLine<'_>,
    ) -> u8 {
        let Some(raw_value) = directive_attribute(attributes, key) else {
            return default;
        };
        match raw_value.parse::<u16>() {
            Ok(value) if value <= u16::from(u8::MAX) => value as u8,
            Ok(value) => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("background {key} must be between 0 and {max}, found {value}"),
                ));
                default
            }
            Err(_) => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "background {key} must be a whole number between 0 and {max}, found '{raw_value}'"
                    ),
                ));
                default
            }
        }
    }

    fn parse_figure_options(
        &mut self,
        attributes: &str,
        line: SourceLine<'_>,
    ) -> Option<FigureOptions> {
        let parsed = parse_figure_options_from_attributes(attributes);
        self.validate_figure_options_parse(&parsed, line)
            .then_some(parsed.options)
    }

    fn parse_layout_values(
        &mut self,
        kind: LayoutKind,
        attributes: &str,
        line: SourceLine<'_>,
    ) -> Option<LayoutValues> {
        let mut values = LayoutValues::default();
        if kind == LayoutKind::Overlay {
            values.overlay_policy = Some(OverlayPolicy::EdgeOnly);
            if let Some(policy) = directive_attribute(attributes, "overlap")
                && policy != "edge-only"
            {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "Overlay overlap must be 'edge-only', found '{policy}'; unrestricted evidence overlap is not supported"
                    ),
                ));
                return None;
            }
        }
        if kind == LayoutKind::Aside {
            values.aside_width = Some(
                match directive_attribute(attributes, "supporting").as_deref() {
                    None | Some("standard") => AsideWidth::Standard,
                    Some("compact") => AsideWidth::Compact,
                    Some(value) => {
                        self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!(
                            "Aside supporting width must be 'compact' or 'standard', found '{value}'"
                        ),
                    ));
                        return None;
                    }
                },
            );
        }
        if let Some(widths) = directive_attribute(attributes, "widths") {
            if kind != LayoutKind::Columns {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{kind:?} does not accept widths; use tracks for Grid"),
                ));
                return None;
            }
            match parse_layout_widths(&widths) {
                Some(parsed) => values.widths = Some(parsed),
                None => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("invalid {kind:?} widths layout value '{widths}'"),
                    ));
                    return None;
                }
            }
        }
        if let Some(tracks) = directive_attribute(attributes, "tracks") {
            if kind != LayoutKind::Grid {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{kind:?} does not accept tracks"),
                ));
                return None;
            }
            match parse_layout_widths(&tracks) {
                Some(parsed) if (1..=6).contains(&parsed.len()) => values.tracks = Some(parsed),
                _ => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!(
                            "invalid Grid tracks layout value '{tracks}'; declare one to six checked track sizes separated by '/'"
                        ),
                    ));
                    return None;
                }
            }
        }
        if let Some(gap) = directive_attribute(attributes, "gap") {
            match parse_layout_size(&gap) {
                Some(parsed) => values.gap = Some(parsed),
                None => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("invalid {kind:?} gap layout value '{gap}'"),
                    ));
                    return None;
                }
            }
        }
        if let Some(align) = directive_attribute(attributes, "align") {
            match parse_layout_align(&align) {
                Some(parsed) => values.align = Some(parsed),
                None => {
                    self.diagnostics.push(Diagnostic::error(
                        self.span_for_line(line),
                        format!("invalid {kind:?} align layout value '{align}'"),
                    ));
                    return None;
                }
            }
        }
        Some(values)
    }

    fn parse_chart_block(
        &mut self,
        spec_json: &str,
        data_url_override: Option<String>,
        line: SourceLine<'_>,
    ) -> Option<ContentBlock> {
        let spec: serde_json::Value = match serde_json::from_str(spec_json) {
            Ok(spec) => spec,
            Err(error) => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("invalid Vega-Lite JSON: {error}"),
                ));
                return None;
            }
        };
        let data_url = data_url_override.or_else(|| chart_data_url(&spec));
        let data = data_url.and_then(|url| self.validate_chart_data_dependency(url, line));
        if let Err(reason) = crate::chart::validate(&spec, data.as_ref(), self.deck_root.as_deref())
        {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("unsupported Vega-Lite chart for HTML/PDF rendering: {reason}"),
            ));
        }
        Some(ContentBlock::Chart {
            format: ChartFormat::VegaLite,
            spec,
            data,
        })
    }

    fn parse_mermaid_block(&mut self, source: &str, line: SourceLine<'_>) -> Option<ContentBlock> {
        if let Some(reason) = mermaid_static_renderability_error(source) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("unsupported Mermaid diagram for HTML/PDF rendering: {reason}"),
            ));
            return None;
        }
        Some(ContentBlock::Diagram {
            language: DiagramLanguage::Mermaid,
            source: source.trim().to_string(),
        })
    }

    fn validate_figure_asset(&mut self, src: &str, line: SourceLine<'_>) {
        if looks_remote_or_fragment(src) {
            return;
        }
        if !is_safe_local_asset_reference(src) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("figure references unsupported local asset path '{src}'"),
            ));
            return;
        }
        let Some(deck_root) = &self.deck_root else {
            return;
        };
        if !deck_root.join(src).exists() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("figure references missing local asset '{src}'"),
            ));
        }
    }

    fn validate_figure_options_parse(
        &mut self,
        parsed: &FigureOptionsParse,
        line: SourceLine<'_>,
    ) -> bool {
        self.validate_visual_options_parse("figure", parsed, line)
    }

    fn validate_visual_options_parse(
        &mut self,
        label: &str,
        parsed: &FigureOptionsParse,
        line: SourceLine<'_>,
    ) -> bool {
        let mut valid = true;
        for (name, value) in [
            ("width", parsed.options.width.as_deref()),
            ("height", parsed.options.height.as_deref()),
            ("radius", parsed.options.radius.as_deref()),
        ] {
            let Some(value) = value else {
                continue;
            };
            if !is_safe_css_size(value) {
                valid = false;
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{label} {name} uses unsupported CSS size '{value}'"),
                ));
            }
        }
        if let Some(value) = &parsed.invalid_fit {
            valid = false;
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{label} fit must be contain, cover, or fill; found '{value}'"),
            ));
        }
        if let Some(value) = &parsed.invalid_align {
            valid = false;
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{label} align must be start, center, end, or stretch; found '{value}'"),
            ));
        }
        if let Some(value) = &parsed.invalid_radius {
            valid = false;
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{label} radius uses unsupported CSS size '{value}'"),
            ));
        }
        for invalid in &parsed.invalid_treatments {
            valid = false;
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!(
                    "{label} {} must be {}, found '{}'",
                    invalid.name, invalid.expected, invalid.value
                ),
            ));
        }
        valid
    }

    fn validate_media_asset(&mut self, key: &str, src: &str, line: SourceLine<'_>) {
        if looks_remote_or_fragment(src) {
            return;
        }
        if !is_safe_local_asset_reference(src) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("media {key} references unsupported local asset path '{src}'"),
            ));
            return;
        }
        let Some(deck_root) = &self.deck_root else {
            return;
        };
        if !deck_root.join(src).exists() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("media {key} references missing local asset '{src}'"),
            ));
        }
    }

    fn validate_background_asset(&mut self, name: &str, src: &str, line: SourceLine<'_>) {
        if looks_remote_or_fragment(src) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{name} references unsupported image URL '{src}'"),
            ));
            return;
        }
        if !is_safe_local_asset_reference(src) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{name} references unsupported local asset path '{src}'"),
            ));
            return;
        }
        let Some(deck_root) = &self.deck_root else {
            return;
        };
        if !deck_root.join(src).exists() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{name} references missing local asset '{src}'"),
            ));
        }
    }

    fn validate_background_treatment(
        &mut self,
        name: &str,
        dim: u8,
        grayscale: u8,
        saturate: u8,
        blur: u8,
        line: SourceLine<'_>,
    ) {
        for (field, value) in [
            ("dim", dim),
            ("grayscale", grayscale),
            ("saturate", saturate),
        ] {
            if value > 100 {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("{name} {field} must be between 0 and 100, found {value}"),
                ));
            }
        }
        if blur > 24 {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("{name} blur must be between 0 and 24 pixels, found {blur}"),
            ));
        }
    }

    fn parse_layout_region(
        &mut self,
        layout_kind: LayoutKind,
        region: ParsedLayoutRegionSource,
        directive_line: SourceLine<'_>,
    ) -> LayoutRegion {
        let start_line = directive_line.number + 1 + region.line_offset;
        if layout_kind == LayoutKind::Stack
            && region.attributes.as_deref().is_some_and(|attributes| {
                ["column", "span", "column-span", "row", "row-span"]
                    .iter()
                    .any(|name| directive_attribute(attributes, name).is_some())
            })
        {
            let attribute_line = SourceLine {
                number: directive_line.number + 1 + region.directive_offset.unwrap_or(0),
                text: region.attributes.as_deref().unwrap_or_default(),
            };
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(attribute_line),
                "Stack items follow source order and do not accept Grid placement attributes",
            ));
        }
        if layout_kind == LayoutKind::Overlay
            && region.role == Some(LayoutRegionRole::Base)
            && region.attributes.as_deref().is_some_and(|attributes| {
                ["anchor", "width"]
                    .iter()
                    .any(|name| directive_attribute(attributes, name).is_some())
            })
        {
            let attribute_line = SourceLine {
                number: directive_line.number + 1 + region.directive_offset.unwrap_or(0),
                text: region.attributes.as_deref().unwrap_or_default(),
            };
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(attribute_line),
                "Overlay base does not accept annotation anchor or width attributes",
            ));
        }
        let grid_placement = (layout_kind == LayoutKind::Grid).then(|| {
            let attribute_line = SourceLine {
                number: directive_line.number + 1 + region.directive_offset.unwrap_or(0),
                text: region.attributes.as_deref().unwrap_or_default(),
            };
            self.parse_grid_placement(
                region.attributes.as_deref().unwrap_or_default(),
                attribute_line,
            )
        });
        let overlay_placement = (layout_kind == LayoutKind::Overlay
            && region.role == Some(LayoutRegionRole::Annotation))
        .then(|| {
            let attribute_line = SourceLine {
                number: directive_line.number + 1 + region.directive_offset.unwrap_or(0),
                text: region.attributes.as_deref().unwrap_or_default(),
            };
            self.parse_overlay_placement(
                region.attributes.as_deref().unwrap_or_default(),
                attribute_line,
            )
        })
        .flatten();
        let owned_lines = region
            .markdown
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let source_lines = owned_lines
            .iter()
            .enumerate()
            .map(|(offset, text)| SourceLine {
                number: start_line + offset,
                text,
            })
            .collect::<Vec<_>>();
        let blocks = self
            .parse_slide_content(0, SlideRole::Main, 0, &source_lines, Some(layout_kind))
            .blocks;
        LayoutRegion {
            name: region.name,
            source_span: Some(SourceSpan {
                source_path: self.source_path.clone(),
                line: region
                    .directive_offset
                    .map_or(start_line, |offset| directive_line.number + 1 + offset),
                column: 1,
            }),
            grid_placement,
            role: region.role,
            overlay_placement,
            derivation_step: None,
            blocks,
        }
    }

    fn parse_overlay_placement(
        &mut self,
        attributes: &str,
        line: SourceLine<'_>,
    ) -> Option<OverlayPlacement> {
        let anchor = match directive_attribute(attributes, "anchor").as_deref() {
            Some("top-start") => OverlayAnchor::TopStart,
            Some("top-end") => OverlayAnchor::TopEnd,
            Some("bottom-start") => OverlayAnchor::BottomStart,
            Some("bottom-end") => OverlayAnchor::BottomEnd,
            Some(value) => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "Overlay annotation anchor must be top-start, top-end, bottom-start, or bottom-end, found '{value}'"
                    ),
                ));
                return None;
            }
            None => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    "Overlay annotations require an explicit edge anchor",
                ));
                return None;
            }
        };
        let width = match directive_attribute(attributes, "width").as_deref() {
            None | Some("compact") => OverlayWidth::Compact,
            Some("standard") => OverlayWidth::Standard,
            Some(value) => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "Overlay annotation width must be 'compact' or 'standard', found '{value}'"
                    ),
                ));
                return None;
            }
        };
        Some(OverlayPlacement { anchor, width })
    }

    fn parse_grid_placement(&mut self, attributes: &str, line: SourceLine<'_>) -> GridPlacement {
        let column = self.parse_grid_index(attributes, "column", line);
        let row = self.parse_grid_index(attributes, "row", line);
        let column_span = directive_attribute(attributes, "column-span")
            .or_else(|| directive_attribute(attributes, "span"))
            .map_or(1, |value| self.parse_grid_span("column span", &value, line));
        let row_span = directive_attribute(attributes, "row-span")
            .map_or(1, |value| self.parse_grid_span("row span", &value, line));
        GridPlacement {
            column,
            column_span,
            row,
            row_span,
        }
    }

    fn parse_grid_index(
        &mut self,
        attributes: &str,
        name: &str,
        line: SourceLine<'_>,
    ) -> Option<u8> {
        directive_attribute(attributes, name).and_then(|value| match value.parse::<u8>() {
            Ok(value @ 1..=12) => Some(value),
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!(
                        "Grid cell {name} must be a whole number from 1 to 12, found '{value}'"
                    ),
                ));
                None
            }
        })
    }

    fn parse_grid_span(&mut self, name: &str, value: &str, line: SourceLine<'_>) -> u8 {
        match value.parse::<u8>() {
            Ok(value @ 1..=6) => value,
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("Grid cell {name} must be a whole number from 1 to 6, found '{value}'"),
                ));
                1
            }
        }
    }

    fn validate_grid_regions(
        &mut self,
        values: &LayoutValues,
        regions: &[LayoutRegion],
        directive_line: SourceLine<'_>,
    ) {
        let track_count = values.tracks.as_ref().map_or(2, Vec::len);
        let mut occupied = BTreeMap::new();
        for (region_index, region) in regions.iter().enumerate() {
            let Some(placement) = &region.grid_placement else {
                continue;
            };
            let Some(column) = placement.column else {
                continue;
            };
            if usize::from(column) + usize::from(placement.column_span) - 1 > track_count {
                self.diagnostics.push(Diagnostic::error(
                    region
                        .source_span
                        .clone()
                        .or_else(|| self.span_for_line(directive_line)),
                    format!(
                        "Grid cell starting at column {column} with span {} exceeds the {track_count} declared tracks",
                        placement.column_span
                    ),
                ));
            }
            let Some(row) = placement.row else {
                continue;
            };
            for occupied_column in column..column.saturating_add(placement.column_span) {
                for occupied_row in row..row.saturating_add(placement.row_span) {
                    if let Some(previous_region) =
                        occupied.insert((occupied_column, occupied_row), region_index + 1)
                    {
                        self.diagnostics.push(Diagnostic::error(
                            region.source_span.clone(),
                            format!(
                                "Grid region {} overlaps region {previous_region} at column {occupied_column}, row {occupied_row}",
                                region_index + 1
                            ),
                        ));
                    }
                }
            }
        }
    }

    fn validate_overlay_regions(
        &mut self,
        regions: &[LayoutRegion],
        directive_line: SourceLine<'_>,
    ) -> bool {
        let mut valid = true;
        let base_count = regions
            .iter()
            .filter(|region| region.role == Some(LayoutRegionRole::Base))
            .count();
        let annotations = regions
            .iter()
            .filter(|region| region.role == Some(LayoutRegionRole::Annotation))
            .collect::<Vec<_>>();
        if base_count != 1 || !(1..=3).contains(&annotations.len()) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(directive_line),
                "Overlay requires exactly one base followed by one to three annotations",
            ));
            valid = false;
        }
        if regions.first().and_then(|region| region.role) != Some(LayoutRegionRole::Base) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(directive_line),
                "Overlay base must be the first region so DOM reading order remains meaningful",
            ));
            valid = false;
        }
        let mut anchors = BTreeMap::new();
        for (index, region) in regions.iter().enumerate() {
            if region.role == Some(LayoutRegionRole::Base) {
                continue;
            }
            let Some(placement) = region.overlay_placement else {
                valid = false;
                continue;
            };
            if let Some(previous) = anchors.insert(placement.anchor as u8, index + 1) {
                self.diagnostics.push(Diagnostic::error(
                    region.source_span.clone(),
                    format!(
                        "Overlay annotation {} reuses the edge anchor assigned to annotation {previous}",
                        index + 1
                    ),
                ));
                valid = false;
            }
        }
        valid
    }

    fn validate_aside_regions(
        &mut self,
        regions: &[LayoutRegion],
        directive_line: SourceLine<'_>,
    ) -> bool {
        let valid = regions.len() == 2
            && regions[0].role == Some(LayoutRegionRole::Primary)
            && regions[1].role == Some(LayoutRegionRole::Supporting);
        if !valid {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(directive_line),
                "Aside requires exactly one primary region followed by one supporting region",
            ));
        }
        valid
    }

    fn reject_region_content(&mut self, kind: LayoutKind, content: &str, line: SourceLine<'_>) {
        self.diagnostics.push(Diagnostic::error(
            self.span_for_line(line),
            format!(
                "{content} cannot be nested inside a {} Layout region",
                layout_kind_name(kind)
            ),
        ));
    }

    fn validate_chart_data_dependency(
        &mut self,
        url: String,
        line: SourceLine<'_>,
    ) -> Option<ChartData> {
        if looks_remote_or_fragment(&url) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("chart references unsupported data URL '{url}'"),
            ));
            return None;
        }
        if !is_safe_local_asset_reference(&url) {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("chart references unsupported local data path '{url}'"),
            ));
            return None;
        }
        let format = match Path::new(&url)
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("csv") => ChartDataFormat::Csv,
            Some("json") => ChartDataFormat::Json,
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    self.span_for_line(line),
                    format!("chart data dependency '{url}' must be a CSV or JSON file"),
                ));
                return None;
            }
        };
        let Some(deck_root) = &self.deck_root else {
            return Some(ChartData { url, format });
        };
        if !deck_root.join(&url).exists() {
            self.diagnostics.push(Diagnostic::error(
                self.span_for_line(line),
                format!("chart references missing local data dependency '{url}'"),
            ));
        }
        Some(ChartData { url, format })
    }
}

#[derive(Default)]
struct ParsedCodeFenceInfo {
    language: Option<String>,
    reveal: Option<CodeReveal>,
}

fn collect_local_dependency_references<'a>(
    blocks: &'a [ContentBlock],
    references: &mut Vec<&'a str>,
) {
    for block in blocks {
        match block {
            ContentBlock::Figure {
                src, static_src, ..
            } => {
                push_local_dependency_reference(src, references);
                if let Some(static_src) = static_src {
                    push_local_dependency_reference(static_src, references);
                }
            }
            ContentBlock::Gallery { items, .. } => {
                for item in items {
                    push_local_dependency_reference(&item.src, references);
                    if let Some(static_src) = &item.static_src {
                        push_local_dependency_reference(static_src, references);
                    }
                }
            }
            ContentBlock::Media { src, poster, .. } => {
                if !looks_remote_or_fragment(src) && is_safe_local_asset_reference(src) {
                    references.push(src);
                }
                if let Some(poster) = poster
                    && !looks_remote_or_fragment(poster)
                    && is_safe_local_asset_reference(poster)
                {
                    references.push(poster);
                }
            }
            ContentBlock::Chart {
                data: Some(data), ..
            } if is_safe_local_asset_reference(&data.url) => {
                references.push(&data.url);
            }
            ContentBlock::Layout { regions, .. } => {
                for region in regions {
                    collect_local_dependency_references(&region.blocks, references);
                }
            }
            _ => {}
        }
    }
}

fn push_local_dependency_reference<'a>(src: &'a str, references: &mut Vec<&'a str>) {
    if !looks_remote_or_fragment(src) && is_safe_local_asset_reference(src) {
        references.push(src);
    }
}

fn collect_slide_local_dependency_references<'a>(slide: &'a Slide, references: &mut Vec<&'a str>) {
    if let Some(background_image) = &slide.background_image
        && !looks_remote_or_fragment(&background_image.src)
        && is_safe_local_asset_reference(&background_image.src)
    {
        references.push(background_image.src.as_str());
    }
    collect_local_dependency_references(&slide.blocks, references);
}

pub(crate) fn mermaid_static_renderability_error(source: &str) -> Option<String> {
    match parse_mermaid_flowchart(source) {
        Some(_) => None,
        None => Some(
            "only simple Mermaid graph/flowchart diagrams with --> edges are supported in v1"
                .to_string(),
        ),
    }
}

pub(crate) fn parse_mermaid_flowchart(source: &str) -> Option<MermaidFlowchart> {
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("%%"));
    let header = lines.next()?;
    let direction = parse_mermaid_flowchart_header(header)?;
    let mut edges = Vec::new();
    for line in lines {
        edges.push(parse_mermaid_edge(line)?);
    }
    (!edges.is_empty()).then_some(MermaidFlowchart { direction, edges })
}

fn parse_mermaid_flowchart_header(header: &str) -> Option<MermaidDirection> {
    let direction = header
        .strip_prefix("flowchart ")
        .or_else(|| header.strip_prefix("graph "))?
        .trim();
    match direction {
        "TD" | "TB" => Some(MermaidDirection::TopDown),
        "BT" => Some(MermaidDirection::BottomTop),
        "LR" => Some(MermaidDirection::LeftRight),
        "RL" => Some(MermaidDirection::RightLeft),
        _ => None,
    }
}

fn parse_mermaid_edge(line: &str) -> Option<MermaidEdge> {
    let (from, to) = line.split_once("-->")?;
    let (label, to) = parse_mermaid_edge_label(to.trim());
    Some(MermaidEdge {
        from: parse_mermaid_node(from.trim())?,
        to: parse_mermaid_node(to.trim())?,
        label,
    })
}

fn parse_mermaid_edge_label(rest: &str) -> (Option<String>, &str) {
    let Some(rest) = rest.strip_prefix('|') else {
        return (None, rest);
    };
    let Some((label, to)) = rest.split_once('|') else {
        return (None, rest);
    };
    (Some(label.trim().to_string()), to.trim())
}

fn parse_mermaid_node(value: &str) -> Option<MermaidNode> {
    let value = value.trim();
    if let Some((id, label)) = parse_mermaid_node_label(value, '[', ']')
        .or_else(|| parse_mermaid_node_label(value, '(', ')'))
        .or_else(|| parse_mermaid_node_label(value, '{', '}'))
    {
        return Some(MermaidNode { id, label });
    }
    if is_mermaid_node_id(value) {
        return Some(MermaidNode {
            id: value.to_string(),
            label: value.to_string(),
        });
    }
    None
}

fn parse_mermaid_node_label(value: &str, open: char, close: char) -> Option<(String, String)> {
    let open_index = value.find(open)?;
    let id = value[..open_index].trim();
    let label = value[open_index + open.len_utf8()..]
        .strip_suffix(close)?
        .trim()
        .trim_matches('"');
    if is_mermaid_node_id(id) && !label.is_empty() {
        Some((id.to_string(), label.to_string()))
    } else {
        None
    }
}

fn is_mermaid_node_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let text = trimmed.get(level..)?.strip_prefix(' ')?;
    Some((level as u8, text.trim().to_string()))
}

fn code_fence_start(trimmed: &str) -> Option<Option<String>> {
    let rest = trimmed.strip_prefix("```")?;
    Some((!rest.trim().is_empty()).then(|| rest.trim().to_string()))
}

#[derive(Debug, Clone, Copy)]
enum LexicalFence {
    Code,
    Directive { fence_len: usize },
    HtmlComment,
}

// Source partitioning and directive collection must agree about where a directive ends.
fn advance_lexical_fences(fences: &mut Vec<LexicalFence>, line: &str) {
    let trimmed = line.trim();
    match fences.last().copied() {
        Some(LexicalFence::Code) => {
            if trimmed == "```" {
                fences.pop();
            }
        }
        Some(LexicalFence::HtmlComment) => {
            if line.contains("-->") {
                fences.pop();
            }
        }
        Some(LexicalFence::Directive { fence_len }) => {
            if directive_closing_fence(trimmed, fence_len) {
                fences.pop();
            } else if code_fence_start(trimmed).is_some() {
                fences.push(LexicalFence::Code);
            } else if html_comment_start(trimmed) && !trimmed.contains("-->") {
                fences.push(LexicalFence::HtmlComment);
            } else if let Some(directive) = directive_start(trimmed) {
                fences.push(LexicalFence::Directive {
                    fence_len: directive.fence_len,
                });
            }
        }
        None => {
            if code_fence_start(trimmed).is_some() {
                fences.push(LexicalFence::Code);
            } else if html_comment_start(trimmed) && !trimmed.contains("-->") {
                fences.push(LexicalFence::HtmlComment);
            } else if let Some(directive) = directive_start(trimmed) {
                fences.push(LexicalFence::Directive {
                    fence_len: directive.fence_len,
                });
            }
        }
    }
}

fn top_level_line_mask(lines: &[SourceLine<'_>]) -> Vec<bool> {
    let mut fences = Vec::<LexicalFence>::new();
    let mut top_level = Vec::with_capacity(lines.len());
    for line in lines {
        top_level.push(fences.is_empty());
        advance_lexical_fences(&mut fences, line.text);
    }
    top_level
}

fn parse_code_reveal(value: &str) -> Option<CodeReveal> {
    let mut groups = Vec::new();
    for (index, group) in value.split('|').enumerate() {
        let ranges = parse_code_reveal_group(group)?;
        groups.push(CodeRevealGroup {
            index: index + 1,
            ranges,
        });
    }
    (!groups.is_empty()).then_some(CodeReveal { groups })
}

fn parse_code_reveal_group(value: &str) -> Option<Vec<CodeLineRange>> {
    let mut ranges = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (parse_line_number(start)?, parse_line_number(end)?),
            None => {
                let line = parse_line_number(part)?;
                (line, line)
            }
        };
        if start > end {
            return None;
        }
        ranges.push(CodeLineRange { start, end });
    }
    (!ranges.is_empty()).then_some(ranges)
}

fn parse_line_number(value: &str) -> Option<usize> {
    let value = value.trim().parse().ok()?;
    (value > 0).then_some(value)
}

fn collect_code_fence(lines: &[SourceLine<'_>], mut index: usize) -> (String, usize, bool) {
    let mut code = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.text.trim() == "```" {
            return (code.join("\n"), index + 1, true);
        }
        code.push(line.text);
        index += 1;
    }
    (code.join("\n"), index, false)
}

fn collect_display_math(lines: &[SourceLine<'_>], mut index: usize) -> (String, usize, bool) {
    let mut latex = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.text.trim() == "$$" {
            return (latex.join("\n"), index + 1, true);
        }
        latex.push(line.text);
        index += 1;
    }
    (latex.join("\n"), index, false)
}

fn collect_table(lines: &[SourceLine<'_>], index: usize) -> Option<(ContentBlock, usize)> {
    let headers = table_cells(lines.get(index)?.text)?;
    let alignments = table_delimiter(lines.get(index + 1)?.text)?;
    if headers.is_empty() || alignments.len() != headers.len() {
        return None;
    }

    let mut rows = Vec::new();
    let mut next_index = index + 2;
    while next_index < lines.len() {
        let Some(cells) = table_cells(lines[next_index].text) else {
            break;
        };
        if cells.is_empty() {
            break;
        }
        rows.push(normalize_table_cells(cells, headers.len()));
        next_index += 1;
    }

    Some((
        ContentBlock::Table {
            headers,
            alignments,
            rows,
        },
        next_index,
    ))
}

fn table_cells(line: &str) -> Option<Vec<String>> {
    if !line.contains('|') {
        return None;
    }
    Some(
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect(),
    )
}

fn table_delimiter(line: &str) -> Option<Vec<TableAlignment>> {
    let cells = table_cells(line)?;
    let mut alignments = Vec::with_capacity(cells.len());
    for cell in cells {
        let trimmed = cell.trim();
        let marker = trimmed.trim_matches(':');
        if marker.len() < 3 || !marker.chars().all(|character| character == '-') {
            return None;
        }
        alignments.push(match (trimmed.starts_with(':'), trimmed.ends_with(':')) {
            (true, true) => TableAlignment::Center,
            (true, false) => TableAlignment::Left,
            (false, true) => TableAlignment::Right,
            (false, false) => TableAlignment::Default,
        });
    }
    Some(alignments)
}

fn normalize_table_cells(mut cells: Vec<String>, width: usize) -> Vec<String> {
    cells.truncate(width);
    cells.resize(width, String::new());
    cells
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FigureParts {
    src: String,
    alt: String,
    raw_alt: String,
    caption: Option<String>,
    static_src: Option<String>,
    options: FigureOptionsParse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FigureOptionsParse {
    options: FigureOptions,
    invalid_fit: Option<String>,
    invalid_align: Option<String>,
    invalid_treatments: Vec<InvalidFigureTreatment>,
    invalid_radius: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvalidFigureTreatment {
    name: &'static str,
    value: String,
    expected: &'static str,
}

fn parse_markdown_image(trimmed: &str) -> Option<FigureParts> {
    let rest = trimmed.strip_prefix("![")?;
    let (raw_alt, rest) = rest.split_once("](")?;
    let (target, trailing_attributes) = parse_markdown_image_target_and_trailing_attributes(rest)?;
    if target.is_empty() {
        return None;
    }
    let (src, caption) = parse_markdown_image_target(target);
    let attributes = combined_markdown_image_attributes(raw_alt, trailing_attributes.as_deref());
    let parsed_options = parse_figure_options_from_attributes(&attributes);
    let alt = directive_attribute(&attributes, "alt").unwrap_or_else(|| raw_alt.to_string());
    let caption = caption.or_else(|| directive_attribute(&attributes, "caption"));
    let static_src = directive_static_image_src(&attributes);
    Some(FigureParts {
        src,
        alt,
        raw_alt: attributes,
        caption,
        static_src,
        options: parsed_options,
    })
}

fn parse_markdown_image_target_and_trailing_attributes(
    rest: &str,
) -> Option<(&str, Option<String>)> {
    let rest = rest.trim();
    if rest.ends_with('}')
        && let Some(attribute_start) = rest.rfind('{')
    {
        let before_attributes = rest[..attribute_start].trim_end();
        if let Some(target) = before_attributes.strip_suffix(')') {
            let attributes = rest[attribute_start + 1..rest.len() - 1].trim();
            return Some((
                target.trim(),
                Some(normalize_pandoc_image_attributes(attributes)),
            ));
        }
    }
    Some((rest.strip_suffix(')')?.trim(), None))
}

fn combined_markdown_image_attributes(raw_alt: &str, trailing_attributes: Option<&str>) -> String {
    match trailing_attributes {
        Some(attributes) if !attributes.trim().is_empty() && !raw_alt.trim().is_empty() => {
            format!("{} {}", raw_alt.trim(), attributes.trim())
        }
        Some(attributes) if !attributes.trim().is_empty() => attributes.trim().to_string(),
        _ => raw_alt.to_string(),
    }
}

fn normalize_pandoc_image_attributes(attributes: &str) -> String {
    split_directive_tokens(attributes)
        .into_iter()
        .filter_map(|token| {
            if token.starts_with('.') || token.starts_with('#') {
                return None;
            }
            let Some((key, value)) = token.split_once('=') else {
                return Some(token);
            };
            let normalized_key = match key {
                "fig-align" => "align",
                "fig-alt" => "alt",
                "fig-cap" | "fig-caption" => "caption",
                "fig-fit" | "image-fit" => "fit",
                "fig-radius" | "corner-radius" => "radius",
                "out-width" => "width",
                "out-height" => "height",
                "fig-dim" => "dim",
                "fig-grayscale" => "grayscale",
                "fig-saturate" => "saturate",
                "fig-blur" => "blur",
                other => other,
            };
            Some(format!("{normalized_key}={value}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn directive_static_image_src(attributes: &str) -> Option<String> {
    directive_attribute(attributes, "pdf-src")
        .or_else(|| directive_attribute(attributes, "pdf_src"))
        .or_else(|| directive_attribute(attributes, "static-src"))
        .or_else(|| directive_attribute(attributes, "static_src"))
}

fn parse_implicit_figure_caption(trimmed: &str) -> Option<String> {
    let caption = if trimmed.starts_with('*')
        && trimmed.ends_with('*')
        && !trimmed.starts_with("**")
        && !trimmed.ends_with("**")
    {
        trimmed.strip_prefix('*')?.strip_suffix('*')?
    } else if trimmed.starts_with('_')
        && trimmed.ends_with('_')
        && !trimmed.starts_with("__")
        && !trimmed.ends_with("__")
    {
        trimmed.strip_prefix('_')?.strip_suffix('_')?
    } else {
        return None;
    }
    .trim();
    (!caption.is_empty()).then(|| caption.to_string())
}

fn markdown_image_is_inline(trimmed: &str) -> bool {
    parse_markdown_image(trimmed)
        .map(|figure| markdown_image_raw_alt_has_token(&figure.raw_alt, "inline"))
        .unwrap_or(false)
}

fn markdown_image_raw_alt_has_token(raw_alt: &str, token: &str) -> bool {
    split_directive_tokens(raw_alt)
        .into_iter()
        .any(|part| part == token)
}

fn parse_gallery_columns(raw_alt: &str) -> Option<u8> {
    directive_attribute(raw_alt, "columns")
        .as_deref()
        .and_then(parse_gallery_column_count)
        .or_else(|| {
            split_directive_tokens(raw_alt)
                .into_iter()
                .find_map(|token| parse_parenthesized_u8(&token, "columns"))
        })
}

fn parse_gallery_column_count(value: &str) -> Option<u8> {
    let count = value.parse::<u8>().ok()?;
    (1..=6).contains(&count).then_some(count)
}

fn parse_parenthesized_u8(token: &str, name: &str) -> Option<u8> {
    let rest = token.strip_prefix(name)?.strip_prefix('(')?;
    let value = rest.strip_suffix(')')?;
    parse_gallery_column_count(value)
}

fn inline_figure_alt_text(figure: &FigureParts) -> String {
    if let Some(alt) = directive_attribute(&figure.raw_alt, "alt") {
        return alt;
    }
    split_directive_tokens(&figure.raw_alt)
        .into_iter()
        .filter(|token| {
            !token.contains('=')
                && token != "inline"
                && !is_visual_option_token(token)
                && parse_gallery_columns(token).is_none()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn markdown_background_alt_text(raw_alt: &str) -> Option<String> {
    if let Some(alt) = directive_attribute(raw_alt, "alt") {
        return Some(alt);
    }
    let alt = split_directive_tokens(raw_alt)
        .into_iter()
        .skip(1)
        .filter(|token| {
            !token.contains('=') && parse_background_image_fit(token).is_none() && token != "bg"
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!alt.is_empty()).then_some(alt)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownMedia {
    kind: MediaKind,
    src: String,
    title: Option<String>,
    caption: Option<String>,
    poster: Option<String>,
    alt: String,
    start_time: Option<u32>,
    raw_start_time: Option<String>,
    options: FigureOptionsParse,
    autoplay: bool,
    controls: bool,
    loop_playback: bool,
    muted: bool,
    autoadvance: bool,
    visual_hidden: bool,
}

fn parse_markdown_media(trimmed: &str) -> Option<MarkdownMedia> {
    let figure = parse_markdown_image(trimmed)?;
    let raw_alt = figure.raw_alt.trim();
    let kind = directive_attribute(raw_alt, "kind")
        .as_deref()
        .and_then(parse_media_kind)
        .or_else(|| {
            first_directive_word(raw_alt)
                .as_deref()
                .and_then(parse_media_kind)
        })
        .or_else(|| infer_media_kind_from_source(&figure.src))?;
    let caption = directive_attribute(raw_alt, "caption").or(figure.caption);
    let title = directive_attribute(raw_alt, "title").or_else(|| caption.clone());
    let alt = directive_attribute(raw_alt, "alt")
        .or_else(|| markdown_media_alt_text(raw_alt))
        .or_else(|| caption.clone())
        .or_else(|| title.clone())
        .unwrap_or_default();
    let (src, query_start_time) = media_source_from_markdown(kind, &figure.src);
    let raw_start_time = directive_attribute(raw_alt, "start");
    let options = parse_figure_options_from_attributes(raw_alt);
    Some(MarkdownMedia {
        kind,
        src,
        title,
        caption,
        poster: directive_attribute(raw_alt, "poster"),
        alt,
        start_time: query_start_time,
        raw_start_time,
        options,
        autoplay: markdown_media_bool(raw_alt, "autoplay", false),
        controls: directive_bool(raw_alt, "controls").unwrap_or(true),
        loop_playback: markdown_media_bool(raw_alt, "loop", false),
        muted: markdown_media_bool(raw_alt, "muted", false)
            || markdown_media_bool(raw_alt, "mute", false),
        autoadvance: markdown_media_bool(raw_alt, "autoadvance", false),
        visual_hidden: markdown_media_bool(raw_alt, "hide", false),
    })
}

fn media_source_from_markdown(kind: MediaKind, raw_src: &str) -> (String, Option<u32>) {
    if kind == MediaKind::Iframe {
        return embed_source_from_url(raw_src).unwrap_or_else(|| (raw_src.to_string(), None));
    }
    let Some((src, query)) = raw_src.split_once('?') else {
        return (raw_src.to_string(), None);
    };
    let start_time = query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "t").then(|| parse_media_start_time(value))?
    });
    (src.to_string(), start_time)
}

fn parse_media_start_time(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(seconds) = value.parse::<u32>() {
        return Some(seconds);
    }

    let mut rest = value;
    let mut total = 0u32;
    let mut saw_unit = false;
    for unit in ['h', 'm', 's'] {
        let Some(index) = rest.find(unit) else {
            continue;
        };
        let (number, tail) = rest.split_at(index);
        if number.is_empty() || !number.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        let amount = number.parse::<u32>().ok()?;
        total = total.checked_add(match unit {
            'h' => amount.checked_mul(3600)?,
            'm' => amount.checked_mul(60)?,
            's' => amount,
            _ => unreachable!(),
        })?;
        rest = &tail[1..];
        saw_unit = true;
    }
    (saw_unit && rest.is_empty()).then_some(total)
}

fn embed_source_from_url(raw_src: &str) -> Option<(String, Option<u32>)> {
    youtube_embed_source(raw_src).or_else(|| vimeo_embed_source(raw_src))
}

fn youtube_embed_source(raw_src: &str) -> Option<(String, Option<u32>)> {
    let source = raw_src.trim();
    let lower = source.to_ascii_lowercase();
    let start_time = query_param(source, "t")
        .or_else(|| query_param(source, "start"))
        .and_then(parse_media_start_time);
    let id = if let Some(rest) = lower
        .strip_prefix("https://youtu.be/")
        .or_else(|| lower.strip_prefix("http://youtu.be/"))
    {
        source[source.len() - rest.len()..]
            .split(['?', '#', '/'])
            .next()
            .unwrap_or_default()
            .to_string()
    } else if lower.starts_with("https://www.youtube.com/watch?")
        || lower.starts_with("http://www.youtube.com/watch?")
        || lower.starts_with("https://youtube.com/watch?")
        || lower.starts_with("http://youtube.com/watch?")
    {
        query_param(source, "v")?.to_string()
    } else {
        let rest = strip_known_prefix(
            source,
            &lower,
            &[
                "https://www.youtube.com/embed/",
                "http://www.youtube.com/embed/",
                "https://youtube.com/embed/",
                "http://youtube.com/embed/",
                "https://www.youtube.com/shorts/",
                "http://www.youtube.com/shorts/",
                "https://youtube.com/shorts/",
                "http://youtube.com/shorts/",
            ],
        )?;
        rest.split(['?', '#', '/'])
            .next()
            .unwrap_or_default()
            .to_string()
    };
    is_safe_embed_id(&id).then(|| (format!("https://www.youtube.com/embed/{id}"), start_time))
}

fn vimeo_embed_source(raw_src: &str) -> Option<(String, Option<u32>)> {
    let source = raw_src.trim();
    let lower = source.to_ascii_lowercase();
    let rest = strip_known_prefix(
        source,
        &lower,
        &[
            "https://player.vimeo.com/video/",
            "http://player.vimeo.com/video/",
            "https://vimeo.com/",
            "http://vimeo.com/",
            "https://www.vimeo.com/",
            "http://www.vimeo.com/",
        ],
    )?;
    let id = rest
        .split(['?', '#', '/'])
        .next()
        .unwrap_or_default()
        .to_string();
    id.chars()
        .all(|character| character.is_ascii_digit())
        .then(|| (format!("https://player.vimeo.com/video/{id}"), None))
}

fn strip_known_prefix<'a>(source: &'a str, lower: &str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes
        .iter()
        .find(|prefix| lower.starts_with(**prefix))
        .map(|prefix| &source[prefix.len()..])
}

fn query_param<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let query = source
        .split_once('?')?
        .1
        .split('#')
        .next()
        .unwrap_or_default();
    query.split('&').find_map(|part| {
        let (param_key, value) = part.split_once('=')?;
        (param_key == key && !value.is_empty()).then_some(value)
    })
}

fn is_safe_embed_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn markdown_media_alt_text(raw_alt: &str) -> Option<String> {
    let alt = split_directive_tokens(raw_alt)
        .into_iter()
        .filter(|token| {
            !token.contains('=')
                && !matches!(
                    token.as_str(),
                    "media" | "video" | "audio" | "iframe" | "embed" | "youtube" | "vimeo"
                )
                && !is_media_control_token(token)
                && !is_visual_option_token(token)
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!alt.is_empty()).then_some(alt)
}

fn markdown_media_bool(raw_alt: &str, key: &str, default: bool) -> bool {
    directive_bool(raw_alt, key).unwrap_or_else(|| {
        default
            || split_directive_tokens(raw_alt)
                .into_iter()
                .any(|token| token == key)
    })
}

fn is_media_control_token(token: &str) -> bool {
    matches!(
        token,
        "autoplay" | "autoadvance" | "loop" | "mute" | "muted" | "controls" | "hide"
    )
}

fn is_visual_option_token(token: &str) -> bool {
    matches!(token, "fit" | "[fit]" | "fill" | "[fill]")
        || parse_visual_alignment_token(token).is_some()
        || is_percentage_size(token)
        || token.starts_with("corner-radius(")
        || token.starts_with("radius(")
        || token.starts_with("radius=")
        || token.starts_with("corner-radius=")
}

fn infer_media_kind_from_source(src: &str) -> Option<MediaKind> {
    if embed_source_from_url(src).is_some() {
        return Some(MediaKind::Iframe);
    }
    let path = src.split(['?', '#']).next().unwrap_or(src);
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "mp4" | "m4v" | "mov" | "webm" => Some(MediaKind::Video),
        "mp3" | "m4a" | "ogg" | "wav" | "flac" => Some(MediaKind::Audio),
        _ => None,
    }
}

fn parse_markdown_image_target(target: &str) -> (String, Option<String>) {
    if let Some((src, title)) = target.split_once(" \"")
        && let Some(title) = title.strip_suffix('"')
    {
        return (src.trim().to_string(), Some(title.to_string()));
    }
    (target.to_string(), None)
}

fn parse_figure_options_from_attributes(attributes: &str) -> FigureOptionsParse {
    let tokens = split_directive_tokens(attributes);
    let fit_shorthand = tokens
        .iter()
        .any(|token| *token == "fit" || *token == "[fit]");
    let fill_shorthand = tokens
        .iter()
        .any(|token| *token == "fill" || *token == "[fill]");
    let token_align = tokens
        .iter()
        .find_map(|token| parse_visual_alignment_token(token));
    let token_width = tokens
        .iter()
        .find(|token| is_percentage_size(token))
        .map(|token| (*token).to_string());
    let raw_fit = directive_attribute(attributes, "fit");
    let fit = raw_fit
        .as_deref()
        .and_then(parse_figure_fit)
        .or(fill_shorthand.then_some(FigureFit::Fill))
        .or(fit_shorthand.then_some(FigureFit::Contain));
    let invalid_fit = raw_fit.filter(|_| fit.is_none());
    let raw_align = directive_attribute(attributes, "align");
    let align = raw_align
        .as_deref()
        .and_then(parse_figure_align)
        .or(token_align)
        .or(fit_shorthand.then_some(FigureAlign::Center));
    let invalid_align = raw_align.filter(|_| align.is_none());
    let dim = parse_optional_u8_attribute(attributes, "dim", 100);
    let grayscale = parse_optional_u8_attribute(attributes, "grayscale", 100);
    let saturate = parse_optional_u8_attribute(attributes, "saturate", 100);
    let blur = parse_optional_u8_attribute(attributes, "blur", 24);
    let raw_radius = directive_attribute(attributes, "radius")
        .or_else(|| directive_attribute(attributes, "corner-radius"))
        .or_else(|| parenthesized_attribute(attributes, "corner-radius"))
        .or_else(|| parenthesized_attribute(attributes, "radius"));
    let radius = raw_radius
        .as_ref()
        .filter(|value| is_safe_css_size(value))
        .cloned();
    let invalid_radius = raw_radius.filter(|_| radius.is_none());
    let invalid_treatments = [
        invalid_figure_treatment("dim", "a whole number between 0 and 100", &dim),
        invalid_figure_treatment("grayscale", "a whole number between 0 and 100", &grayscale),
        invalid_figure_treatment("saturate", "a whole number between 0 and 100", &saturate),
        invalid_figure_treatment("blur", "a whole number between 0 and 24 pixels", &blur),
    ]
    .into_iter()
    .flatten()
    .collect();
    FigureOptionsParse {
        options: FigureOptions {
            width: directive_attribute(attributes, "width")
                .or(token_width)
                .or_else(|| fit_shorthand.then(|| "100%".to_string())),
            height: directive_attribute(attributes, "height")
                .or_else(|| fit_shorthand.then(|| "100%".to_string())),
            fit,
            align,
            dim: dim.value,
            grayscale: grayscale.value,
            saturate: saturate.value,
            blur: blur.value,
            radius,
        },
        invalid_fit,
        invalid_align,
        invalid_treatments,
        invalid_radius,
    }
}

fn parenthesized_attribute(attributes: &str, key: &str) -> Option<String> {
    for token in attributes.split_whitespace() {
        let Some(rest) = token
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('('))
        else {
            continue;
        };
        if let Some(value) = rest.strip_suffix(')').map(str::trim)
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

fn parse_visual_alignment_token(token: &str) -> Option<FigureAlign> {
    match token {
        "left" | "start" => Some(FigureAlign::Start),
        "center" => Some(FigureAlign::Center),
        "right" | "end" => Some(FigureAlign::End),
        "stretch" => Some(FigureAlign::Stretch),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OptionalU8Attribute {
    value: Option<u8>,
    invalid: Option<String>,
}

fn parse_optional_u8_attribute(
    attributes: &str,
    key: &'static str,
    max: u8,
) -> OptionalU8Attribute {
    let Some(raw_value) = directive_attribute(attributes, key) else {
        return OptionalU8Attribute {
            value: None,
            invalid: None,
        };
    };
    match raw_value.parse::<u16>() {
        Ok(value) if value <= u16::from(max) => OptionalU8Attribute {
            value: Some(value as u8),
            invalid: None,
        },
        _ => OptionalU8Attribute {
            value: None,
            invalid: Some(raw_value),
        },
    }
}

fn invalid_figure_treatment(
    name: &'static str,
    expected: &'static str,
    parsed: &OptionalU8Attribute,
) -> Option<InvalidFigureTreatment> {
    parsed.invalid.as_ref().map(|value| InvalidFigureTreatment {
        name,
        value: value.clone(),
        expected,
    })
}

fn chart_data_url(spec: &serde_json::Value) -> Option<String> {
    spec.get("data")?
        .get("url")?
        .as_str()
        .map(ToString::to_string)
}

/// A matched inline code span, including its opening/closing backtick runs.
pub(crate) fn inline_code_span(source: &str) -> Option<(&str, usize)> {
    let fence = source.bytes().take_while(|byte| *byte == b'`').count();
    if fence == 0 {
        return None;
    }
    let mut offset = fence;
    while offset < source.len() {
        let next = source[offset..].find('`')? + offset;
        let count = source[next..]
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count();
        if count == fence {
            return Some((&source[fence..next], next + count));
        }
        offset = next + count;
    }
    None
}

pub(crate) fn inline_math_segments(markdown: &str) -> Vec<String> {
    fn escaped_at(value: &str, index: usize) -> bool {
        value.as_bytes()[..index]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count()
            % 2
            == 1
    }

    let mut segments = Vec::new();
    let mut index = 0;
    while index < markdown.len() {
        if !escaped_at(markdown, index)
            && let Some((_, consumed)) = inline_code_span(&markdown[index..])
        {
            index += consumed;
            continue;
        }
        if markdown[index..].starts_with(r"\(") && !escaped_at(markdown, index) {
            let body_start = index + 2;
            if let Some(relative_end) = markdown[body_start..].find(r"\)") {
                let body_end = body_start + relative_end;
                segments.push(markdown[body_start..body_end].to_string());
                index = body_end + 2;
                continue;
            }
        } else if markdown.as_bytes()[index] == b'$'
            && !escaped_at(markdown, index)
            && markdown.as_bytes().get(index.wrapping_sub(1)) != Some(&b'$')
            && markdown.as_bytes().get(index + 1) != Some(&b'$')
        {
            let body_start = index + 1;
            let mut body_end = body_start;
            while body_end < markdown.len() {
                if markdown.as_bytes()[body_end] == b'$'
                    && !escaped_at(markdown, body_end)
                    && markdown.as_bytes().get(body_end + 1) != Some(&b'$')
                {
                    if body_end > body_start && !markdown[body_start..body_end].contains('\n') {
                        segments.push(markdown[body_start..body_end].to_string());
                        index = body_end + 1;
                        break;
                    }
                    index = body_end + 1;
                    break;
                }
                body_end += markdown[body_end..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
            }
            if body_end < markdown.len() {
                continue;
            }
        }
        index += markdown[index..].chars().next().map_or(1, char::len_utf8);
    }
    segments
}

fn collect_block_footnote_references(block: &ContentBlock, labels: &mut Vec<(String, usize)>) {
    match block {
        ContentBlock::Paragraph { markdown, .. }
        | ContentBlock::FitText { markdown, .. }
        | ContentBlock::Quote { markdown, .. } => {
            collect_markdown_footnote_references(markdown, labels);
        }
        ContentBlock::Callout {
            title, markdown, ..
        } => {
            if let Some(title) = title {
                collect_markdown_footnote_references(title, labels);
            }
            collect_markdown_footnote_references(markdown, labels);
        }
        ContentBlock::List { items, .. } => {
            for item in items {
                collect_markdown_footnote_references(&item.markdown, labels);
            }
        }
        ContentBlock::Layout { regions, .. } => {
            for region in regions {
                for block in &region.blocks {
                    collect_block_footnote_references(block, labels);
                }
            }
        }
        ContentBlock::Heading { text, .. } => collect_markdown_footnote_references(text, labels),
        ContentBlock::Figure {
            caption: Some(caption),
            ..
        }
        | ContentBlock::Media {
            caption: Some(caption),
            ..
        } => collect_markdown_footnote_references(caption, labels),
        ContentBlock::Gallery { items, .. } => {
            for item in items {
                if let Some(caption) = &item.caption {
                    collect_markdown_footnote_references(caption, labels);
                }
            }
        }
        ContentBlock::Steps { steps, .. } => {
            for step in steps {
                collect_markdown_footnote_references(&step.markdown, labels);
            }
        }
        ContentBlock::Table { headers, rows, .. } => {
            for cell in headers.iter().chain(rows.iter().flatten()) {
                collect_markdown_footnote_references(cell, labels);
            }
        }
        _ => {}
    }
}

fn collect_markdown_footnote_references(markdown: &str, labels: &mut Vec<(String, usize)>) {
    let math = inline_math_segments(markdown);
    let mut rest = markdown;
    while !rest.is_empty() {
        if let Some((_, consumed)) = inline_code_span(rest) {
            rest = &rest[consumed..];
            continue;
        }
        if let Some(consumed) = math.iter().find_map(|latex| {
            [format!(r"\({latex}\)"), format!("${latex}$")]
                .into_iter()
                .find(|spelling| rest.starts_with(spelling))
                .map(|spelling| spelling.len())
        }) {
            rest = &rest[consumed..];
            continue;
        }
        if let Some(escaped) = rest.strip_prefix('\\') {
            let consumed = escaped.chars().next().map_or(0, char::len_utf8);
            rest = &escaped[consumed..];
            continue;
        }
        if let Some(reference) = rest.strip_prefix("[^")
            && let Some(end) = reference.find(']')
        {
            let label = &reference[..end];
            if is_valid_footnote_label(label) {
                labels.push((label.to_string(), labels.len()));
            }
            rest = &reference[end + 1..];
            continue;
        }
        rest = &rest[rest.chars().next().unwrap().len_utf8()..];
    }
}

fn parse_footnote_definition_line(trimmed: &str) -> Option<(String, String)> {
    let rest = trimmed.strip_prefix("[^")?;
    let (label, markdown) = rest.split_once("]:")?;
    if !is_valid_footnote_label(label) {
        return None;
    }
    Some((label.to_string(), markdown.trim().to_string()))
}

fn parse_image_corner_radius_command(trimmed: &str) -> Option<String> {
    let value = trimmed
        .strip_prefix("image-corner-radius:")
        .or_else(|| trimmed.strip_prefix("image_corner_radius:"))?
        .trim()
        .trim_matches('"');
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_build_lists_command(trimmed: &str) -> Option<Option<BuildListsMode>> {
    let value = trimmed
        .strip_prefix("build-lists:")
        .or_else(|| trimmed.strip_prefix("build_lists:"))?
        .trim()
        .trim_matches('"');
    parse_build_lists_value(value)
}

fn parse_build_lists_value(value: &str) -> Option<Option<BuildListsMode>> {
    match value.trim() {
        "all" | "true" | "yes" | "on" | "1" => Some(Some(BuildListsMode::All)),
        "notFirst" | "not-first" | "not_first" => Some(Some(BuildListsMode::NotFirst)),
        "false" | "no" | "off" | "0" => Some(None),
        _ => None,
    }
}

fn is_valid_footnote_label(label: &str) -> bool {
    !label.is_empty()
        && label.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.')
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectiveStart<'a> {
    name: String,
    rest: &'a str,
    fence_len: usize,
}

fn directive_start(trimmed: &str) -> Option<DirectiveStart<'_>> {
    let fence_len = trimmed
        .chars()
        .take_while(|character| *character == ':')
        .count();
    if fence_len < 3 {
        return None;
    }
    let rest = trimmed[fence_len..].trim_start();
    if rest.is_empty() {
        return None;
    }
    let (name, remaining) = if let Some(after_open) = rest.strip_prefix('{') {
        let end = after_open.find('}')? + 2;
        (&rest[..end], rest[end..].trim())
    } else {
        rest.split_once(char::is_whitespace)
            .map_or((rest, ""), |(name, remaining)| (name, remaining.trim()))
    };
    let (name, remaining) = normalize_pandoc_fenced_div_start(name, remaining)?;
    Some(DirectiveStart {
        name,
        rest: remaining,
        fence_len,
    })
}

fn normalize_pandoc_fenced_div_start<'a>(
    name: &'a str,
    remaining: &'a str,
) -> Option<(String, &'a str)> {
    if let Some(attributes) = name
        .strip_prefix('{')
        .and_then(|name| name.strip_suffix('}'))
    {
        for class_name in [
            "columns",
            "column",
            "incremental",
            "nonincremental",
            "notes",
        ] {
            if pandoc_attribute_has_class(attributes, class_name) {
                return Some((class_name.to_string(), attributes));
            }
        }
    }
    Some((name.trim_matches(':').to_string(), remaining))
}

fn pandoc_attribute_has_class(attributes: &str, class_name: &str) -> bool {
    split_directive_tokens(attributes)
        .into_iter()
        .any(|token| token == format!(".{class_name}"))
}

fn directive_markdown(rest: &str, raw_body: &str) -> String {
    match (rest.trim(), raw_body.trim()) {
        ("", body) => body.to_string(),
        (rest, "") => rest.to_string(),
        (rest, body) => format!("{rest}\n{body}"),
    }
}

fn collect_directive_body(
    lines: &[SourceLine<'_>],
    mut index: usize,
    fence_len: usize,
) -> (String, usize, bool) {
    let mut body = Vec::new();
    let mut fences = vec![LexicalFence::Directive { fence_len }];
    while index < lines.len() {
        let line = lines[index];
        advance_lexical_fences(&mut fences, line.text);
        if fences.is_empty() {
            return (body.join("\n"), index + 1, true);
        }
        body.push(line.text);
        index += 1;
    }
    (body.join("\n"), index, false)
}

fn directive_closing_fence(trimmed: &str, fence_len: usize) -> bool {
    trimmed.len() == fence_len && trimmed.chars().all(|character| character == ':')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeparatorKind {
    Section,
    Detail,
}

fn separator_kind(line: &str) -> Option<SeparatorKind> {
    match line.trim() {
        "---" => Some(SeparatorKind::Section),
        "--" => Some(SeparatorKind::Detail),
        _ => None,
    }
}

fn is_malformed_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() > 1 && trimmed.chars().all(|character| character == '-')
}

fn slide_lines_are_empty(lines: &[SourceLine<'_>]) -> bool {
    lines.iter().all(|line| line.text.trim().is_empty())
}

fn parse_fit_text_line(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("[fit]")?.trim_start();
    (!rest.is_empty()).then_some(rest)
}

fn parse_slide_class_tokens(attributes: &str) -> Vec<String> {
    directive_attribute(attributes, "name")
        .or_else(|| directive_attribute(attributes, "class"))
        .or_else(|| directive_attribute(attributes, "classes"))
        .unwrap_or_else(|| attributes.to_string())
        .split(|character: char| character.is_whitespace() || character == ',')
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.contains('='))
        .map(ToString::to_string)
        .collect()
}

fn parse_slide_metadata_class_attributes(attributes: &str) -> Vec<String> {
    directive_attribute(attributes, "class")
        .or_else(|| directive_attribute(attributes, "classes"))
        .map(|classes| {
            classes
                .split(|character: char| character.is_whitespace() || character == ',')
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_theme_param_tokens(attributes: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for token in split_directive_tokens(attributes) {
        let Some((name, value)) = token.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim().trim_matches('"');
        if !name.is_empty() && !value.is_empty() {
            params.insert(name.to_string(), value.to_string());
        }
    }
    params
}

fn parse_slide_theme_shorthand(trimmed: &str) -> Option<(String, String)> {
    let command = trimmed.strip_prefix("[.")?.strip_suffix(']')?;
    let (name, value) = command.split_once(':')?;
    let parameter_name = slide_theme_shorthand_param_name(name.trim())?;
    let value = value.trim().trim_matches('"');
    (!value.is_empty()).then(|| (parameter_name.to_string(), value.to_string()))
}

fn parse_slide_preset_shorthand(trimmed: &str) -> Option<String> {
    let command = trimmed.strip_prefix("[.")?.strip_suffix(']')?;
    let (name, value) = command.split_once(':')?;
    match name.trim() {
        "preset" | "slide-preset" => {
            let value = value.trim().trim_matches('"');
            (!value.is_empty()).then(|| value.to_string())
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SlideFooterCommand {
    Content(String),
    Hide,
    SlideNumbers(bool),
}

fn parse_slide_footer_shorthand(trimmed: &str) -> Option<SlideFooterCommand> {
    let command = trimmed.strip_prefix("[.")?.strip_suffix(']')?.trim();
    match command {
        "hide-footer" | "hide_footer" => return Some(SlideFooterCommand::Hide),
        _ => {}
    }
    let (name, value) = command.split_once(':')?;
    let value = value.trim().trim_matches('"');
    match name.trim() {
        "footer" => (!value.is_empty()).then(|| SlideFooterCommand::Content(value.to_string())),
        "slide-number" | "slide-numbers" | "slidenumbers" | "paginate" => {
            parse_bool_token(value).map(SlideFooterCommand::SlideNumbers)
        }
        _ => None,
    }
}

fn parse_autoscale_shorthand(trimmed: &str) -> Option<bool> {
    let command = trimmed.strip_prefix("[.")?.strip_suffix(']')?.trim();
    let (name, value) = command.split_once(':')?;
    match name.trim() {
        "autoscale" => parse_bool_token(value.trim().trim_matches('"')),
        _ => None,
    }
}

fn parse_build_lists_shorthand(trimmed: &str) -> Option<Option<BuildListsMode>> {
    let command = trimmed.strip_prefix("[.")?.strip_suffix(']')?.trim();
    let (name, value) = command.split_once(':')?;
    match name.trim() {
        "build-lists" | "build_lists" => parse_build_lists_value(value.trim().trim_matches('"')),
        _ => None,
    }
}

fn parse_slide_transition_shorthand(trimmed: &str) -> Option<SlideTransition> {
    let command = trimmed.strip_prefix("[.")?.strip_suffix(']')?.trim();
    let (name, value) = command.split_once(':')?;
    match name.trim() {
        "transition" | "slide-transition" => parse_slide_transition(value.trim().trim_matches('"')),
        _ => None,
    }
}

fn parse_slide_transition(value: &str) -> Option<SlideTransition> {
    match value.trim() {
        "true" | "yes" | "on" | "1" => Some(SlideTransition::Fade),
        "false" | "no" | "off" | "0" => Some(SlideTransition::None),
        "none" => Some(SlideTransition::None),
        "fade" => Some(SlideTransition::Fade),
        "slide" | "slide-left" => Some(SlideTransition::Slide),
        "zoom" => Some(SlideTransition::Zoom),
        _ => None,
    }
}

fn parse_bool_token(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn slide_theme_shorthand_param_name(name: &str) -> Option<&'static str> {
    match name {
        "background-color" | "background" => Some("background"),
        "surface-color" | "surface" => Some("surface"),
        "text-color" | "text" => Some("text"),
        "accent-color" | "accent" => Some("accent"),
        "muted-color" | "muted" => Some("muted"),
        "rule-color" | "rule" => Some("rule"),
        _ => None,
    }
}

fn directive_shorthand_value(attributes: &str, body: &str) -> Option<String> {
    let value = if !body.trim().is_empty() {
        body.trim()
    } else {
        attributes.trim()
    }
    .trim_matches('"');
    (!value.is_empty()).then(|| value.to_string())
}

fn split_directive_tokens(attributes: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut in_quotes = false;
    for character in attributes.chars() {
        match character {
            '"' => {
                in_quotes = !in_quotes;
                token.push(character);
            }
            character if character.is_whitespace() && !in_quotes => {
                if !token.trim().is_empty() {
                    tokens.push(token.trim().to_string());
                    token.clear();
                }
            }
            _ => token.push(character),
        }
    }
    if !token.trim().is_empty() {
        tokens.push(token.trim().to_string());
    }
    tokens
}

fn yaml_string_list(value: &serde_yaml::Value) -> Vec<String> {
    match value {
        serde_yaml::Value::Sequence(values) => values.iter().filter_map(scalar_to_string).collect(),
        _ => scalar_to_string(value)
            .map(|value| {
                value
                    .split(|character: char| character.is_whitespace() || character == ',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn yaml_mapping_to_string_map(value: &serde_yaml::Value) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let serde_yaml::Value::Mapping(mapping) = value else {
        return values;
    };
    for (key, value) in mapping {
        let (Some(key), Some(value)) = (key.as_str(), scalar_to_string(value)) else {
            continue;
        };
        values.insert(key.to_string(), value);
    }
    values
}

fn is_valid_theme_param_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

fn is_valid_slide_class(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn is_valid_slide_preset(value: &str) -> bool {
    is_valid_slide_class(value)
}

fn collect_caret_notes(lines: &[SourceLine<'_>], start_index: usize) -> (String, usize) {
    let mut notes = Vec::new();
    let mut index = start_index;
    while index < lines.len() {
        let trimmed = lines[index].text.trim();
        if trimmed.is_empty() {
            break;
        }
        let Some(note) = parse_caret_note_line(trimmed) else {
            break;
        };
        notes.push(note.to_string());
        index += 1;
    }
    (notes.join("\n"), index)
}

fn parse_caret_note_line(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix('^')?.trim_start();
    Some(rest)
}

fn collect_reveal_notes(lines: &[SourceLine<'_>], start_index: usize) -> (String, usize) {
    let mut notes = Vec::new();
    if let Some(first_note) = parse_reveal_note_marker(lines[start_index].text.trim())
        && !first_note.trim().is_empty()
    {
        notes.push(first_note.trim().to_string());
    }
    let mut index = start_index + 1;
    while index < lines.len() {
        notes.push(lines[index].text.to_string());
        index += 1;
    }
    (notes.join("\n").trim().to_string(), index)
}

fn parse_reveal_note_marker(trimmed: &str) -> Option<&str> {
    let (marker, rest) = trimmed.split_once(':')?;
    matches!(marker, marker if marker.eq_ignore_ascii_case("note") || marker.eq_ignore_ascii_case("notes"))
        .then_some(rest.trim_start())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarpitCommentDirective {
    name: String,
    value: String,
    spot: bool,
}

fn parse_marpit_comment_directives(markdown: &str) -> Vec<MarpitCommentDirective> {
    let mut directives = Vec::new();
    for line in markdown.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((raw_name, raw_value)) = line.split_once(':') else {
            continue;
        };
        let Some((name, spot)) = normalize_marpit_directive_name(raw_name.trim()) else {
            continue;
        };
        let value = raw_value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        directives.push(MarpitCommentDirective { name, value, spot });
    }
    directives
}

fn normalize_marpit_directive_name(raw_name: &str) -> Option<(String, bool)> {
    let (raw_name, spot) = raw_name
        .strip_prefix('_')
        .map_or((raw_name, false), |name| (name, true));
    let name = raw_name.replace('_', "-");
    let normalized = match name.as_str() {
        "paginate" => "paginate",
        "footer" => "footer",
        "class" => "class",
        "backgroundColor" | "background-color" | "background" => "background-color",
        "backgroundImage" | "background-image" => "background-image",
        "backgroundPosition" | "background-position" => "background-position",
        "backgroundSize" | "background-size" => "background-size",
        "color" | "textColor" | "text-color" | "text" => "color",
        _ => return None,
    };
    Some((normalized.to_string(), spot))
}

fn marpit_background_image_src(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if let Some(inner) = value
        .strip_prefix("url(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let src = inner.trim().trim_matches('"').trim_matches('\'');
        return (!src.is_empty()).then(|| src.to_string());
    }
    (!value.is_empty()).then(|| value.to_string())
}

fn marpit_class_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| character.is_whitespace() || character == ',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn html_comment_start(trimmed: &str) -> bool {
    trimmed.starts_with("<!--")
}

fn collect_html_comment(lines: &[SourceLine<'_>], start_index: usize) -> (String, usize, bool) {
    let first = lines[start_index].text.trim();
    let Some(first_body) = first.strip_prefix("<!--") else {
        return (String::new(), start_index + 1, false);
    };
    if let Some(close_index) = first_body.find("-->") {
        return (
            first_body[..close_index].trim().to_string(),
            start_index + 1,
            true,
        );
    }

    let mut body = Vec::new();
    if !first_body.is_empty() {
        body.push(first_body.trim_start().to_string());
    }
    let mut index = start_index + 1;
    while index < lines.len() {
        let line = lines[index].text;
        if let Some(close_index) = line.find("-->") {
            body.push(line[..close_index].to_string());
            return (body.join("\n").trim().to_string(), index + 1, true);
        }
        body.push(line.to_string());
        index += 1;
    }
    (body.join("\n").trim().to_string(), index, false)
}

fn slidev_comment_is_speaker_note(markdown: &str) -> bool {
    let first_line = markdown
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or_default();
    !first_line.starts_with(".slide:") && !first_line.starts_with(".element:")
}

fn remaining_slide_lines_are_blank(lines: &[SourceLine<'_>], index: usize) -> bool {
    lines[index..]
        .iter()
        .all(|line| line.text.trim().is_empty())
}

fn looks_like_raw_html(trimmed: &str) -> bool {
    trimmed.starts_with('<') && trimmed.ends_with('>') && !trimmed.starts_with("<!--")
}

fn quote_line_markdown(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest).to_string())
}

fn parse_callout_quote(markdown: &str) -> Option<(CalloutKind, Option<String>, String)> {
    let mut lines = markdown.lines();
    let first = lines.next()?.trim();
    let (kind, title) = parse_callout_marker(first)?;
    let body = lines.collect::<Vec<_>>().join("\n");
    Some((kind, title, body.trim().to_string()))
}

fn parse_callout_marker(line: &str) -> Option<(CalloutKind, Option<String>)> {
    let rest = line.strip_prefix("[!")?;
    let (kind, title) = rest.split_once(']')?;
    let kind = parse_callout_kind(kind)?;
    let title = title.trim();
    Some((kind, (!title.is_empty()).then(|| title.to_string())))
}

fn parse_callout_kind(value: &str) -> Option<CalloutKind> {
    match value.to_ascii_lowercase().as_str() {
        "note" | "info" => Some(CalloutKind::Note),
        "tip" | "success" => Some(CalloutKind::Tip),
        "important" => Some(CalloutKind::Important),
        "warning" | "warn" => Some(CalloutKind::Warning),
        "caution" | "danger" => Some(CalloutKind::Caution),
        _ => None,
    }
}

fn first_nonspace_column(line: &str) -> usize {
    line.chars()
        .position(|character| !character.is_whitespace())
        .map_or(1, |index| index + 1)
}

fn directive_attribute(attributes: &str, key: &str) -> Option<String> {
    split_directive_tokens(attributes)
        .into_iter()
        .find_map(|token| {
            let (name, value) = token.split_once('=')?;
            (name == key).then(|| value.trim_matches('"').to_string())
        })
}

fn first_directive_word(attributes: &str) -> Option<String> {
    let word = attributes.split_whitespace().next()?;
    (!word.contains('=')).then(|| word.to_string())
}

fn parse_slide_variant(value: &str) -> Option<SlideVariant> {
    match value {
        "claim" => Some(SlideVariant::Claim),
        "figure" => Some(SlideVariant::Figure),
        "comparison" => Some(SlideVariant::Comparison),
        "derivation" => Some(SlideVariant::Derivation),
        "section-title" => Some(SlideVariant::SectionTitle),
        "dense" => Some(SlideVariant::Dense),
        _ => None,
    }
}

fn parse_media_kind(value: &str) -> Option<MediaKind> {
    match value {
        "video" => Some(MediaKind::Video),
        "audio" => Some(MediaKind::Audio),
        "iframe" | "embed" | "youtube" | "vimeo" => Some(MediaKind::Iframe),
        _ => None,
    }
}

fn parse_step_pdf_policy(value: &str) -> Option<StepPdfPolicy> {
    match value {
        "final" | "final-state" | "collapse" => Some(StepPdfPolicy::FinalState),
        "pages" | "one-page-per-step" => Some(StepPdfPolicy::OnePagePerStep),
        _ => None,
    }
}

fn directive_bool(attributes: &str, key: &str) -> Option<bool> {
    directive_attribute(attributes, key).and_then(|value| match value.as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    })
}

fn parse_ordered_step(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    let digit_count = trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    let (digits, rest) = trimmed.split_at(digit_count);
    let rest = rest.strip_prefix(". ")?;
    let index = digits.parse().ok()?;
    (!rest.trim().is_empty()).then(|| (index, rest.trim().to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListLineKind {
    Unordered,
    FragmentedUnordered,
    Ordered,
    FragmentedOrdered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListRevealMode {
    None,
    All,
    SkipFirst,
}

impl ListRevealMode {
    fn reveals(self) -> bool {
        self != Self::None
    }
}

fn list_line_kind(line: &str) -> Option<ListLineKind> {
    parse_list_line(line).map(|(kind, _)| kind)
}

fn parse_list_line(line: &str) -> Option<(ListLineKind, String)> {
    if let Some(rest) = line.strip_prefix("* ") {
        return Some((ListLineKind::FragmentedUnordered, rest.trim().to_string()));
    }

    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("+ ")) {
        return Some((ListLineKind::Unordered, rest.trim().to_string()));
    }

    let digit_count = line
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    let (digits, rest) = line.split_at(digit_count);
    let (kind, rest) = if let Some(rest) = rest.strip_prefix(". ") {
        (ListLineKind::Ordered, rest)
    } else {
        (ListLineKind::FragmentedOrdered, rest.strip_prefix(") ")?)
    };
    let index = digits.parse::<usize>().ok()?;
    (index > 0 && !rest.trim().is_empty()).then(|| (kind, rest.trim().to_string()))
}

fn list_line_kind_is_ordered(kind: ListLineKind) -> bool {
    matches!(
        kind,
        ListLineKind::Ordered | ListLineKind::FragmentedOrdered
    )
}

fn list_line_kind_is_fragmented(kind: ListLineKind) -> bool {
    matches!(
        kind,
        ListLineKind::FragmentedUnordered | ListLineKind::FragmentedOrdered
    )
}

fn list_line_reveal_mode(
    kind: ListLineKind,
    build_lists: Option<BuildListsMode>,
) -> ListRevealMode {
    if list_line_kind_is_fragmented(kind) {
        return ListRevealMode::All;
    }
    match build_lists {
        Some(BuildListsMode::All) => ListRevealMode::All,
        Some(BuildListsMode::NotFirst) => ListRevealMode::SkipFirst,
        None => ListRevealMode::None,
    }
}

fn parse_layout_widths(value: &str) -> Option<Vec<LayoutSize>> {
    let widths: Option<Vec<_>> = value.split('/').map(parse_layout_width).collect();
    let widths = widths?;
    (!widths.is_empty()).then_some(widths)
}

fn parse_layout_width(value: &str) -> Option<LayoutSize> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(units) = value.parse::<u8>() {
        return (units > 0).then_some(LayoutSize::Fraction { units });
    }
    parse_layout_size(value)
}

fn parse_layout_size(value: &str) -> Option<LayoutSize> {
    let value = value.trim();
    if let Some(arbitrary) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        return (!arbitrary.trim().is_empty()).then(|| LayoutSize::Arbitrary {
            value: arbitrary.trim().to_string(),
        });
    }
    match value {
        "0" => Some(LayoutSize::Scale { step: 0 }),
        "1" => Some(LayoutSize::Scale { step: 1 }),
        "2" => Some(LayoutSize::Scale { step: 2 }),
        "3" => Some(LayoutSize::Scale { step: 3 }),
        "4" => Some(LayoutSize::Scale { step: 4 }),
        "6" => Some(LayoutSize::Scale { step: 6 }),
        "8" => Some(LayoutSize::Scale { step: 8 }),
        "12" => Some(LayoutSize::Scale { step: 12 }),
        _ => None,
    }
}

fn parse_layout_align(value: &str) -> Option<LayoutAlign> {
    match value {
        "start" => Some(LayoutAlign::Start),
        "center" => Some(LayoutAlign::Center),
        "end" => Some(LayoutAlign::End),
        "stretch" => Some(LayoutAlign::Stretch),
        _ => None,
    }
}

fn parse_figure_fit(value: &str) -> Option<FigureFit> {
    match value {
        "contain" => Some(FigureFit::Contain),
        "cover" => Some(FigureFit::Cover),
        "fill" => Some(FigureFit::Fill),
        _ => None,
    }
}

fn parse_figure_align(value: &str) -> Option<FigureAlign> {
    match value {
        "left" | "start" => Some(FigureAlign::Start),
        "default" | "center" => Some(FigureAlign::Center),
        "right" | "end" => Some(FigureAlign::End),
        "stretch" => Some(FigureAlign::Stretch),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedLayoutRegions {
    regions: Vec<ParsedLayoutRegionSource>,
    widths: Option<Vec<LayoutSize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedLayoutRegionSource {
    name: Option<String>,
    role: Option<LayoutRegionRole>,
    attributes: Option<String>,
    directive_offset: Option<usize>,
    markdown: String,
    line_offset: usize,
}

fn parse_layout_region_sources(kind: LayoutKind, body: &str) -> ParsedLayoutRegions {
    if matches!(
        kind,
        LayoutKind::Grid | LayoutKind::Stack | LayoutKind::Overlay | LayoutKind::Aside
    ) && let Some(regions) = parse_structured_layout_regions(kind, body)
    {
        return ParsedLayoutRegions {
            regions,
            widths: None,
        };
    }
    if kind != LayoutKind::Columns {
        return ParsedLayoutRegions {
            regions: vec![ParsedLayoutRegionSource {
                name: None,
                role: None,
                attributes: None,
                directive_offset: None,
                markdown: body.trim().to_string(),
                line_offset: body
                    .lines()
                    .take_while(|line| line.trim().is_empty())
                    .count(),
            }],
            widths: None,
        };
    }

    if let Some(parsed) = parse_pandoc_column_regions(body) {
        return parsed;
    }

    let mut regions = Vec::new();
    let mut current_name = None;
    let mut current_lines = Vec::new();
    let mut current_start = 0usize;
    let mut fences = Vec::new();
    for (offset, line) in body.lines().enumerate() {
        let name = fences
            .is_empty()
            .then(|| layout_region_heading(line))
            .flatten();
        advance_lexical_fences(&mut fences, line);
        if let Some(name) = name {
            if current_name.is_some() || !current_lines.is_empty() {
                push_layout_region_source(
                    &mut regions,
                    current_name.take(),
                    &current_lines,
                    current_start,
                );
                current_lines.clear();
            }
            current_name = Some(name);
            current_start = offset + 1;
        } else {
            if current_lines.is_empty() {
                current_start = offset;
            }
            current_lines.push(line);
        }
    }
    if current_name.is_some() || !current_lines.is_empty() {
        push_layout_region_source(&mut regions, current_name, &current_lines, current_start);
    }
    ParsedLayoutRegions {
        regions,
        widths: None,
    }
}

fn push_layout_region_source(
    regions: &mut Vec<ParsedLayoutRegionSource>,
    name: Option<String>,
    lines: &[&str],
    start_offset: usize,
) {
    let leading_blank_lines = lines
        .iter()
        .take_while(|line| line.trim().is_empty())
        .count();
    let trailing_start = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(leading_blank_lines, |index| index + 1);
    let markdown = lines[leading_blank_lines..trailing_start].join("\n");
    regions.push(ParsedLayoutRegionSource {
        name,
        role: None,
        attributes: None,
        directive_offset: None,
        markdown,
        line_offset: start_offset + leading_blank_lines,
    });
}

fn parse_pandoc_column_regions(body: &str) -> Option<ParsedLayoutRegions> {
    let lines = body.lines().collect::<Vec<_>>();
    let mut regions = Vec::new();
    let mut widths = Vec::new();
    let mut index = 0usize;
    let mut saw_column = false;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        let directive = directive_start(trimmed)?;
        if directive.name != "column" {
            return None;
        }
        saw_column = true;
        let mut column_lines = Vec::new();
        index += 1;
        while index < lines.len() {
            let line = lines[index];
            if directive_closing_fence(line.trim(), directive.fence_len) {
                break;
            }
            column_lines.push(line);
            index += 1;
        }
        if index >= lines.len() {
            return None;
        }
        if let Some(width) = directive_attribute(directive.rest, "width")
            .or_else(|| directive_attribute(directive.rest, "data-width"))
            .and_then(|width| parse_pandoc_column_width(&width))
        {
            widths.push(width);
        }
        let name = directive_attribute(directive.rest, "name")
            .or_else(|| directive_attribute(directive.rest, "title"))
            .or_else(|| directive_attribute(directive.rest, "label"));
        let leading_blank_lines = column_lines
            .iter()
            .take_while(|line| line.trim().is_empty())
            .count();
        let content_start = index.saturating_sub(column_lines.len()) + leading_blank_lines;
        regions.push(ParsedLayoutRegionSource {
            name,
            role: None,
            attributes: None,
            directive_offset: None,
            markdown: column_lines.join("\n").trim().to_string(),
            line_offset: content_start,
        });
        index += 1;
    }
    if !saw_column || regions.is_empty() {
        return None;
    }
    let widths = (widths.len() == regions.len()).then_some(widths);
    Some(ParsedLayoutRegions { regions, widths })
}

fn parse_structured_layout_regions(
    kind: LayoutKind,
    body: &str,
) -> Option<Vec<ParsedLayoutRegionSource>> {
    let allowed_names: &[&str] = match kind {
        LayoutKind::Grid => &["cell"],
        LayoutKind::Stack => &["item"],
        LayoutKind::Overlay => &["base", "annotation"],
        LayoutKind::Aside => &["primary", "supporting"],
        _ => return None,
    };
    let lines = body.lines().collect::<Vec<_>>();
    let mut regions = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let directive_offset = index;
        let directive = directive_start(lines[index].trim())?;
        if !allowed_names.contains(&directive.name.as_str()) || directive.fence_len < 4 {
            return None;
        }
        let role = match directive.name.as_str() {
            "base" => Some(LayoutRegionRole::Base),
            "annotation" => Some(LayoutRegionRole::Annotation),
            "primary" => Some(LayoutRegionRole::Primary),
            "supporting" => Some(LayoutRegionRole::Supporting),
            _ => None,
        };
        let attributes = directive.rest.to_string();
        let name = directive_attribute(directive.rest, "name")
            .or_else(|| directive_attribute(directive.rest, "title"))
            .or_else(|| directive_attribute(directive.rest, "label"));
        index += 1;
        let content_start = index;
        let mut region_lines = Vec::new();
        while index < lines.len()
            && !directive_closing_fence(lines[index].trim(), directive.fence_len)
        {
            region_lines.push(lines[index]);
            index += 1;
        }
        if index >= lines.len() {
            return None;
        }
        let leading_blank_lines = region_lines
            .iter()
            .take_while(|line| line.trim().is_empty())
            .count();
        regions.push(ParsedLayoutRegionSource {
            name,
            role,
            attributes: Some(attributes),
            directive_offset: Some(directive_offset),
            markdown: region_lines.join("\n").trim().to_string(),
            line_offset: content_start + leading_blank_lines,
        });
        index += 1;
    }
    (!regions.is_empty()).then_some(regions)
}

fn parse_derivation_region_sources(body: &str) -> Option<Vec<ParsedLayoutRegionSource>> {
    let lines = body.lines().collect::<Vec<_>>();
    let mut regions = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let directive_offset = index;
        let directive = directive_start(lines[index].trim())?;
        let role = match directive.name.as_str() {
            "context" => LayoutRegionRole::Stable,
            "stage" => LayoutRegionRole::Change,
            _ => return None,
        };
        if directive.fence_len < 4 {
            return None;
        }
        let attributes = directive.rest.to_string();
        let name = directive_attribute(directive.rest, "label")
            .or_else(|| directive_attribute(directive.rest, "name"))
            .or_else(|| directive_attribute(directive.rest, "title"));
        index += 1;
        let content_start = index;
        let mut region_lines = Vec::new();
        while index < lines.len()
            && !directive_closing_fence(lines[index].trim(), directive.fence_len)
        {
            region_lines.push(lines[index]);
            index += 1;
        }
        if index >= lines.len() {
            return None;
        }
        let leading_blank_lines = region_lines
            .iter()
            .take_while(|line| line.trim().is_empty())
            .count();
        regions.push(ParsedLayoutRegionSource {
            name,
            role: Some(role),
            attributes: Some(attributes),
            directive_offset: Some(directive_offset),
            markdown: region_lines.join("\n").trim().to_string(),
            line_offset: content_start + leading_blank_lines,
        });
        index += 1;
    }
    (!regions.is_empty()).then_some(regions)
}

fn parse_pandoc_column_width(value: &str) -> Option<LayoutSize> {
    parse_layout_width(value).or_else(|| {
        is_safe_css_size(value).then(|| LayoutSize::Arbitrary {
            value: value.trim().to_string(),
        })
    })
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

fn layout_region_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let name = trimmed.strip_suffix(':')?;
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(" column")).then(|| name.trim_end_matches(" column").to_string())
}

fn looks_remote_or_fragment(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with('#')
}

fn is_safe_local_asset_reference(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_safe_css_token_list(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character.is_ascii_whitespace()
                || matches!(character, '-' | '.' | '%')
        })
}

fn is_safe_css_size(value: &str) -> bool {
    let value = value.trim();
    if value == "auto" {
        return true;
    }
    if value.is_empty()
        || value.len() > 32
        || value.starts_with('-')
        || value.chars().any(char::is_whitespace)
    {
        return false;
    }
    let numeric_len = value
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .count();
    if numeric_len == 0 {
        return false;
    }
    let (number, unit) = value.split_at(numeric_len);
    if number == "." || number.matches('.').count() > 1 {
        return false;
    }
    matches!(
        unit,
        "" | "%" | "px" | "rem" | "em" | "vh" | "vw" | "vmin" | "vmax" | "ch"
    )
}

fn slide_id(section_index: usize, role: SlideRole, detail_index: usize) -> String {
    match role {
        SlideRole::Main => format!("section-{section_index}-main"),
        SlideRole::Detail => format!("section-{section_index}-detail-{detail_index}"),
    }
}

fn yaml_location(error: &serde_yaml::Error) -> String {
    error.location().map_or_else(String::new, |location| {
        format!(":{}:{}", location.line(), location.column())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_context_rejects_metadata_but_preserves_literal_code() {
        let literal = "# Code\n\n:::: columns\nLeft column:\n```text\n::: notes\nRight column:\n^ literal note marker\n:::\n```\n::::\n";
        let deck = parse_source_text(literal, None).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout { regions, .. } = &deck.sections[0].main_slide.blocks[1] else {
            panic!("expected Columns")
        };
        assert_eq!(regions.len(), 1);
        assert!(
            matches!(&regions[0].blocks[0], ContentBlock::Code { code, .. } if code.contains("Right column:"))
        );

        for content in [
            "::::: derivation\n:::: context label=\"Context\"\nInvariant\n::::\n:::: stage label=\"Stage\"\nResult\n::::\n:::::",
            "[.footer: Unexpected]",
            "<!-- class: unexpected -->",
        ] {
            let source =
                format!("# Invalid nesting\n\n:::::: columns\nLeft column:\n{content}\n::::::\n");
            let deck = parse_source_text(&source, None).unwrap();
            assert!(
                deck.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.is_fatal()
                        && diagnostic.message.contains("cannot be nested")),
                "{content}: {:?}",
                deck.diagnostics
            );
        }
    }

    #[test]
    fn footnotes_follow_slide_reading_order_across_regions() {
        let source = "# Claim[^first]\n\n:::: columns\nLeft column:\nFirst[^second] and repeated[^first].\n\nRight column:\nOther[^third].\n::::\n\n[^first]: First citation\n[^second]: Second citation\n[^third]: Third citation\n";
        let deck = parse_source_text(source, None).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let blocks = &deck.sections[0].main_slide.blocks;
        let ContentBlock::Footnotes { notes } = blocks.last().unwrap() else {
            panic!("expected slide notes")
        };
        assert_eq!(
            notes
                .iter()
                .map(|note| (note.label.as_str(), note.number))
                .collect::<Vec<_>>(),
            vec![("first", 1), ("second", 2), ("third", 3)]
        );
        let ContentBlock::Layout { regions, .. } = &blocks[1] else {
            panic!("expected Columns")
        };
        assert!(regions.iter().all(|region| {
            region
                .blocks
                .iter()
                .all(|block| !matches!(block, ContentBlock::Footnotes { .. }))
        }));
    }

    #[test]
    fn directive_attributes_match_whole_names_outside_quoted_values() {
        assert_eq!(directive_attribute(r#"row-span="2""#, "span"), None);
        assert_eq!(
            directive_attribute(r#"row-span="2" span="3""#, "span"),
            Some("3".into())
        );
        for attributes in [
            r#"static-src="fallback.svg" src="live.svg""#,
            r#"src="live.svg" static-src="fallback.svg""#,
            r#"alt="Mention src=example.svg here" src="live.svg""#,
        ] {
            assert_eq!(
                directive_attribute(attributes, "src"),
                Some("live.svg".into())
            );
        }
        let deck = parse_source_text("# Grid\n\n::::: grid tracks=\"1/1\"\n:::: cell row-span=\"2\"\nA\n::::\n:::: cell\nB\n::::\n:::::\n", None).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout { regions, .. } = &deck.sections[0].main_slide.blocks[1] else {
            panic!("expected Grid")
        };
        let placement = regions[0].grid_placement.as_ref().unwrap();
        assert_eq!((placement.column_span, placement.row_span), (1, 2));
    }

    #[test]
    fn parses_three_sections_with_mixed_detail_slides() {
        let source = r#"---
title: "Search and symmetry"
author: "Zayenz"
theme: "debug"
aspect: "16:9"
---

# Main claim

Constraint models leak structure.

::: notes
Say this slowly.
:::

--

## Detail evidence

The toy model already shows it.

--

## Detail caveat

The example is small.

---

# Second claim

Propagation is not the whole story.

---

# Third claim

Search still decides the shape.

--

## Detail reference

Keep this for questions.
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        insta::assert_yaml_snapshot!("three_sections_mixed_details", deck);
    }

    #[test]
    fn pdf_linearization_is_main_then_details_per_section() {
        let source = r#"# A

--

## A detail

---

# B
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();
        let order: Vec<_> = deck
            .pdf_slide_order()
            .into_iter()
            .map(|slide| slide.id.as_str())
            .collect();

        assert_eq!(
            order,
            vec!["section-1-main", "section-1-detail-1", "section-2-main"]
        );
    }

    #[test]
    fn keeps_source_boundaries_inside_code_fences_in_one_slide() {
        let source = r#"# Code sample

```rust
fn main() {
    println!("---");
}
---
--
```

The explanation remains on the same slide.
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.sections.len(), 1);
        assert!(deck.sections[0].detail_slides.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Code {
                language: Some(language),
                code,
                ..
            } if language == "rust" && code.contains("\n---\n--")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Paragraph { markdown, .. }
                if markdown == "The explanation remains on the same slide."
        ));
    }

    #[test]
    fn keeps_source_boundaries_inside_directive_bodies_in_one_slide() {
        let source = r#"# Protected content

::: notes
Speaker reminder before the marker.
---
--
:::

::: html
<div>
---
--
</div>
:::

:::: {.columns}
::: {.column name="Left"}
---
--
:::
::: {.column name="Right"}
Nested directive content.
:::
::::

The explanation remains on the same slide.
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.sections.len(), 1);
        assert!(deck.sections[0].detail_slides.is_empty());
        assert!(deck.diagnostics.is_empty());
        assert!(
            deck.sections[0]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(
                    block,
                    ContentBlock::SpeakerNotes { markdown } if markdown.contains("\n---\n--")
                ))
        );
        assert!(
            deck.sections[0]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(
                    block,
                    ContentBlock::HtmlOnly { html } if html.contains("\n---\n--")
                ))
        );
        assert!(matches!(
            deck.sections[0].main_slide.blocks.last(),
            Some(ContentBlock::Paragraph { markdown, .. })
                if markdown == "The explanation remains on the same slide."
        ));
    }

    #[test]
    fn keeps_source_boundaries_inside_html_comments_hidden() {
        let source = r#"# First slide

<!--
Hidden author comment.
---
--
-->

Visible text.

---

# Second slide
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.sections.len(), 2);
        assert!(deck.sections[0].detail_slides.is_empty());
        assert!(deck.sections[1].detail_slides.is_empty());
        assert!(deck.diagnostics.is_empty());
        assert!(
            deck.sections[0]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(
                    block,
                    ContentBlock::Paragraph { markdown, .. } if markdown == "Visible text."
                ))
        );
    }

    #[test]
    fn ignores_deck_commands_and_footnote_definitions_inside_code_fences() {
        let source = r#"# Literal source

```text
image-corner-radius: definitely-not-css
build-lists: true
[^hidden]: This is example syntax, not a definition.
```

- First static point.
- Second static point.

The hidden definition must not resolve this reference[^hidden].
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.metadata.image_corner_radius, None);
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::List { reveal: false, .. }
        ));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("footnote reference '[^hidden]' has no matching definition")
        }));
        assert!(!deck.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("image-corner-radius uses unsupported CSS size")
        }));
    }

    #[test]
    fn nested_fences_do_not_close_an_outer_directive_early() {
        let source = r#"# Nested fences

:::: notes
Before nested content.
```text
::::
---
```
<!--
::::
--
-->
:::: aside
Nested directive body.
::::
After nested content.
::::

Visible after the notes.
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.sections.len(), 1);
        assert!(deck.sections[0].detail_slides.is_empty());
        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::SpeakerNotes { markdown }
                if markdown.contains("```text\n::::\n---\n```")
                    && markdown.contains("<!--\n::::\n--\n-->")
                    && markdown.contains(":::: aside\nNested directive body.\n::::")
                    && markdown.ends_with("After nested content.")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Visible after the notes."
        ));
    }

    #[test]
    fn only_top_level_commands_and_footnote_definitions_have_global_effect() {
        let source = r#"# Scoped pre-scan syntax

::: html
build-lists: true
image-corner-radius: invalid value;
[^shared]: Hidden in HTML.
:::

<!--
build-lists: true
image-corner-radius: invalid value;
[^shared]: Hidden in a comment.
-->

```text
build-lists: true
image-corner-radius: invalid value;
[^shared]: Hidden in code.
```

image-corner-radius: 12px

- First static point.
- Second static point.

The visible definition resolves this reference[^shared].

[^shared]: Visible top-level definition.
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.metadata.image_corner_radius.as_deref(), Some("12px"));
        assert!(deck.diagnostics.is_empty());
        assert!(
            deck.sections[0]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::List { reveal: false, .. }))
        );
        assert!(
            deck.sections[0]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(
                    block,
                    ContentBlock::Footnotes { notes }
                        if notes.len() == 1
                            && notes[0].label == "shared"
                            && notes[0].markdown == "Visible top-level definition."
                ))
        );
    }

    #[test]
    fn parses_markdown_blockquotes_as_quote_blocks() {
        let source = r#"# Quote slide

> The strongest useful local statement is \(p(D) \subseteq D\).
> It should remain visually distinct from ordinary paragraphs.

Back to the author's voice.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Quote {
                markdown,
                inline_math,
            } if markdown == "The strongest useful local statement is \\(p(D) \\subseteq D\\).\nIt should remain visually distinct from ordinary paragraphs."
                && inline_math == &vec!["p(D) \\subseteq D".to_string()]
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Back to the author's voice."
        ));
    }

    #[test]
    fn parses_single_dollar_inline_math_without_consuming_literals_or_display_math() {
        let source = r#"# Inline math

The bound is $z \leq 5$, the price is \$5, and $$display$$ stays literal here.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Paragraph {
                markdown,
                inline_math,
            } if markdown.contains("$z \\leq 5$")
                && inline_math == &vec!["z \\leq 5".to_string()]
        ));
    }

    #[test]
    fn lf_crlf_and_bom_sources_produce_the_same_deck() {
        let lf = r#"---
title: Equivalent source
theme: science
---

# First slide

```text
---
--
```

---

# Second slide
"#;
        let crlf = lf.replace('\n', "\r\n");
        let bom_lf = format!("\u{feff}{lf}");
        let bom_crlf = format!("\u{feff}{crlf}");

        let expected = parse_source_text(lf, None).unwrap();
        assert_eq!(parse_source_text(&crlf, None).unwrap(), expected);
        assert_eq!(parse_source_text(&bom_lf, None).unwrap(), expected);
        assert_eq!(parse_source_text(&bom_crlf, None).unwrap(), expected);

        let without_front_matter = "# Heading\n\nBody.\n";
        assert_eq!(
            parse_source_text(
                &format!("\u{feff}{}", without_front_matter.replace('\n', "\r\n")),
                None,
            )
            .unwrap(),
            parse_source_text(without_front_matter, None).unwrap()
        );
    }

    #[test]
    fn diagnoses_unterminated_front_matter_at_its_opening_delimiter() {
        let source = "\u{feff}---\r\ntheme: science\r\n# This is still front matter\r\n";

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic.message.contains("unterminated front matter")
                && diagnostic.span.as_ref().is_some_and(|span| {
                    span.source_path.as_deref() == Some(Path::new("talk.zp.md"))
                        && span.line == 1
                        && span.column == 1
                })
        }));
    }

    #[test]
    fn parses_alert_blockquotes_as_callout_blocks() {
        let source = r#"# Callout slide

> [!WARNING] Search caveat
> A greedy branching rule can hide \(2^n\) work.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Callout {
                kind: CalloutKind::Warning,
                title: Some(title),
                markdown,
                inline_math,
            } if title == "Search caveat"
                && markdown == "A greedy branching rule can hide \\(2^n\\) work."
                && inline_math == &vec!["2^n".to_string()]
        ));
    }

    #[test]
    fn parses_footnote_references_into_slide_local_footnotes() {
        let source = r#"# Footnote slide

The rule follows the paper[^paper] and the implementation note[^impl].

- The same paper can be cited again[^paper].

[^impl]: Implementation notes can include \(x_t\).

---

# Definition elsewhere

Another slide cites the same paper[^paper].

[^paper]: A compact citation-like source note.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        let first_slide = &deck.sections[0].main_slide;
        assert!(matches!(
            &first_slide.blocks[3],
            ContentBlock::Footnotes { notes }
            if notes.len() == 2
                && notes[0].label == "paper"
                && notes[0].number == 1
                && notes[0].markdown == "A compact citation-like source note."
                && notes[1].label == "impl"
                && notes[1].number == 2
                && notes[1].inline_math == vec!["x_t".to_string()]
        ));
        let second_slide = &deck.sections[1].main_slide;
        assert!(matches!(
            &second_slide.blocks[2],
            ContentBlock::Footnotes { notes }
            if notes.len() == 1
                && notes[0].label == "paper"
                && notes[0].number == 1
        ));
    }

    #[test]
    fn diagnoses_missing_and_duplicate_footnote_definitions() {
        let source = r#"# Footnote diagnostics

This cites a missing note[^missing].

[^dup]: First note.
[^dup]: Second note.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("footnote reference '[^missing]' has no matching definition")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("duplicate footnote definition '[^dup]'")
        }));
    }

    #[test]
    fn parses_markdown_lists_as_typed_list_blocks() {
        let source = r#"# Lists

- Model the state \(x_t\).
- Apply the branching rule.

1. Build the relaxation.
2. Check the bound.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: false, items, .. }
                if items.len() == 2
                    && items[0].markdown == "Model the state \\(x_t\\)."
                    && items[0].inline_math == vec!["x_t".to_string()]
                    && items[1].markdown == "Apply the branching rule."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::List { ordered: true, reveal: false, items, .. }
                if items.len() == 2
                    && items[0].markdown == "Build the relaxation."
                    && items[1].markdown == "Check the bound."
        ));
    }

    #[test]
    fn parses_asterisk_lists_as_fragmented_lists() {
        let source = r#"# Fragmented

* First point.
* Second point with \(x_t\).
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: true, items, .. }
                if items.len() == 2
                    && items[0].markdown == "First point."
                    && items[1].inline_math == vec!["x_t".to_string()]
        ));
    }

    #[test]
    fn parses_deckset_build_lists_command_as_fragmented_lists() {
        let source = r#"build-lists: true

# Built lists

- First point.
- Second point.

1. First ordered point.
2. Second ordered point.

::: nonincremental
- Static point.
:::
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.metadata.build_lists, None);
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: true, items, .. }
                if items.len() == 2 && items[0].markdown == "First point."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::List { ordered: true, reveal: true, items, .. }
                if items.len() == 2 && items[0].markdown == "First ordered point."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[3],
            ContentBlock::List { ordered: false, reveal: false, items, .. }
                if items.len() == 1 && items[0].markdown == "Static point."
        ));
    }

    #[test]
    fn parses_deckset_build_lists_slide_overrides() {
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

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: true, items, .. }
                if items.len() == 2 && items[0].markdown == "First point."
        ));
        assert!(matches!(
            &deck.sections[1].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: false, items, .. }
                if items.len() == 2 && items[0].markdown == "Static point."
        ));
        assert!(matches!(
            &deck.sections[2].main_slide.blocks[1],
            ContentBlock::List { ordered: true, reveal: true, items, .. }
                if items.len() == 2 && items[0].markdown == "First ordered point."
        ));
    }

    #[test]
    fn parses_build_lists_front_matter_as_fragmented_lists() {
        let source = r#"---
build-lists: all
---

# Built lists

- First point.
- Second point.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.metadata.build_lists, Some(BuildListsMode::All));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: true, items, .. }
                if items.len() == 2
        ));
    }

    #[test]
    fn parses_deckset_build_lists_not_first_mode() {
        let source = r#"build-lists: notFirst

# Built lists

- Already visible.
- First reveal.
- Second reveal.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List {
                ordered: false,
                reveal: true,
                reveal_skip_first: true,
                items,
            } if items.len() == 3 && items[0].markdown == "Already visible."
        ));
    }

    #[test]
    fn parses_parenthesized_ordered_lists_as_fragmented_lists() {
        let source = r#"# Ordered fragmented

1) First point.
2) Second point with \(x_t\).
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: true, reveal: true, items, .. }
                if items.len() == 2
                    && items[0].markdown == "First point."
                    && items[1].inline_math == vec!["x_t".to_string()]
        ));
    }

    #[test]
    fn parses_pandoc_incremental_list_divs_as_fragmented_lists() {
        let source = r#"# Incremental list

::: {.incremental}
- First point.
- Second point with \(x_t\).
:::

::: nonincremental
1. First ordered point.
2. Second ordered point.
:::
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::List { ordered: false, reveal: true, items, .. }
                if items.len() == 2
                    && items[0].markdown == "First point."
                    && items[1].inline_math == vec!["x_t".to_string()]
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::List { ordered: true, reveal: false, items, .. }
                if items.len() == 2
                    && items[0].markdown == "First ordered point."
                    && items[1].markdown == "Second ordered point."
        ));
    }

    #[test]
    fn parses_fit_text_shorthand_and_directive_blocks() {
        let source = r#"# Big statements

[fit] One strong sentence with \(x_t\).

::: fit
A second big statement.
:::
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::FitText {
                markdown,
                inline_math,
            } if markdown == "One strong sentence with \\(x_t\\)."
                && inline_math == &vec!["x_t".to_string()]
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::FitText {
                markdown,
                inline_math,
            } if markdown == "A second big statement." && inline_math.is_empty()
        ));
    }

    #[test]
    fn parses_caret_lines_as_speaker_notes() {
        let source = r#"# Presenter notes

Visible slide text.

^ Pause before the result.
^ Mention the backup slide.

Back to visible text.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Visible slide text."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::SpeakerNotes { markdown }
                if markdown == "Pause before the result.\nMention the backup slide."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[3],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Back to visible text."
        ));
    }

    #[test]
    fn parses_reveal_and_quarto_speaker_notes_as_private_notes() {
        let source = r#"# Reveal notes

Visible slide text.

Note:
Pause before the result.
Mention \(x_t\).

---

# Quarto notes

Visible slide text.

::: {.notes}
Keep this private.
:::
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Visible slide text."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::SpeakerNotes { markdown }
                if markdown == "Pause before the result.\nMention \\(x_t\\)."
        ));
        assert!(matches!(
            &deck.sections[1].main_slide.blocks[2],
            ContentBlock::SpeakerNotes { markdown }
                if markdown == "Keep this private."
        ));
    }

    #[test]
    fn parses_slidev_final_html_comments_as_speaker_notes() {
        let source = r#"# Slidev notes

Visible slide text.

<!-- This is a **note** -->

---

# Slidev multiline notes

Visible slide text.

<!--
This is _another_ note.
-->
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Visible slide text."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::SpeakerNotes { markdown } if markdown == "This is a **note**"
        ));
        assert!(matches!(
            &deck.sections[1].main_slide.blocks[2],
            ContentBlock::SpeakerNotes { markdown } if markdown == "This is _another_ note."
        ));
    }

    #[test]
    fn ignores_non_final_html_comments_and_reveal_attribute_comments() {
        let source = r##"# Not notes

<!-- This is not a note because it precedes content. -->

Visible content.

---

# Reveal attrs

Visible content.

<!-- .slide: data-background="#ff0000" -->
"##;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert_eq!(deck.sections[0].main_slide.blocks.len(), 2);
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Paragraph { markdown, .. } if markdown == "Visible content."
        ));
        assert!(
            !deck.sections[0]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::SpeakerNotes { .. }))
        );
        assert_eq!(deck.sections[1].main_slide.blocks.len(), 2);
        assert!(
            !deck.sections[1]
                .main_slide
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::SpeakerNotes { .. }))
        );
    }

    #[test]
    fn parses_marpit_comment_directives_as_inherited_slide_metadata() {
        let source = r##"<!--
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
_backgroundImage: url('assets/phase-space.svg')
_backgroundPosition: right bottom
_backgroundSize: cover
-->

# Second

---

# Third
"##;

        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        let first = &deck.sections[0].main_slide;
        assert_eq!(first.footer.slide_numbers, Some(true));
        assert_eq!(first.footer.content.as_deref(), Some("Global footer"));
        assert_eq!(first.classes, vec!["lead"]);
        assert_eq!(
            first.theme_params.get("background").map(String::as_str),
            Some("#f8fafc")
        );
        assert_eq!(
            first.theme_params.get("text").map(String::as_str),
            Some("#111827")
        );
        let first_background = first.background_image.as_ref().unwrap();
        assert_eq!(first_background.src, "assets/phase-space.svg");
        assert_eq!(first_background.position, "left top");
        assert_eq!(first_background.fit, BackgroundImageFit::Contain);
        assert!(!first_background.splash);

        let second = &deck.sections[1].main_slide;
        assert_eq!(second.footer.slide_numbers, Some(false));
        assert_eq!(second.footer.content.as_deref(), Some("Local footer"));
        assert_eq!(second.classes, vec!["lead", "result"]);
        assert_eq!(
            second.theme_params.get("background").map(String::as_str),
            Some("#222222")
        );
        assert_eq!(
            second.theme_params.get("text").map(String::as_str),
            Some("#111827")
        );
        let second_background = second.background_image.as_ref().unwrap();
        assert_eq!(second_background.src, "assets/phase-space.svg");
        assert_eq!(second_background.position, "right bottom");
        assert_eq!(second_background.fit, BackgroundImageFit::Cover);

        let third = &deck.sections[2].main_slide;
        assert_eq!(third.footer.slide_numbers, Some(true));
        assert_eq!(third.footer.content.as_deref(), Some("Global footer"));
        assert_eq!(third.classes, vec!["lead"]);
        assert_eq!(
            third.theme_params.get("background").map(String::as_str),
            Some("#f8fafc")
        );
        let third_background = third.background_image.as_ref().unwrap();
        assert_eq!(third_background.src, "assets/phase-space.svg");
        assert_eq!(third_background.position, "left top");
        assert_eq!(third_background.fit, BackgroundImageFit::Contain);
    }

    #[test]
    fn parses_slide_class_directives_as_checked_slide_metadata() {
        let source = r#"# Tagged slide

::: class lead result
:::

::: classes name="dense-table lead"
:::

Visible text.
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(
            deck.sections[0].main_slide.classes,
            vec!["lead", "result", "dense-table"]
        );
    }

    #[test]
    fn parses_slide_theme_params_as_checked_slide_metadata() {
        let source = r##"# Themed slide

::: theme mode=dark accent="#ff00ff" density=compact
:::

Visible text.
"##;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        let params = &deck.sections[0].main_slide.theme_params;
        assert_eq!(params.get("mode").map(String::as_str), Some("dark"));
        assert_eq!(params.get("accent").map(String::as_str), Some("#ff00ff"));
        assert_eq!(params.get("density").map(String::as_str), Some("compact"));
    }

    #[test]
    fn parses_slide_color_shorthands_as_theme_params() {
        let source = r##"# Color shorthand

[.background-color: #101820]
[.accent-color: "#ff00ff"]

::: text-color #f8fafc
:::

Visible text.
"##;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        let params = &deck.sections[0].main_slide.theme_params;
        assert_eq!(
            params.get("background").map(String::as_str),
            Some("#101820")
        );
        assert_eq!(params.get("accent").map(String::as_str), Some("#ff00ff"));
        assert_eq!(params.get("text").map(String::as_str), Some("#f8fafc"));
    }

    #[test]
    fn parses_slide_metadata_block_as_theme_and_layout_hooks() {
        let source = r##"# Metadata slide

::: slide
preset: spotlight
variant: claim
classes: [hero, branded]
theme:
  mode: dark
  accent: "#ff00ff"
colors:
  background: "#101820"
background:
  src: assets/phase-space.svg
  split: right:35%
  dim: 86
  grayscale: 20
:::

Visible text.
"##;

        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        assert!(deck.diagnostics.is_empty());
        let slide = &deck.sections[0].main_slide;
        assert_eq!(slide.preset.as_deref(), Some("spotlight"));
        assert_eq!(slide.variant, Some(SlideVariant::Claim));
        assert_eq!(slide.classes, vec!["hero", "branded"]);
        assert_eq!(slide.theme_params.get("mode").unwrap(), "dark");
        assert_eq!(slide.theme_params.get("accent").unwrap(), "#ff00ff");
        assert_eq!(slide.theme_params.get("background").unwrap(), "#101820");
        let background = slide.background_image.as_ref().unwrap();
        assert_eq!(background.src, "assets/phase-space.svg");
        assert_eq!(background.dim, 86);
        assert_eq!(background.grayscale, 20);
        assert_eq!(
            background.split,
            Some(BackgroundImageSplit {
                side: BackgroundImageSplitSide::Right,
                size: "35%".to_string()
            })
        );
        assert!(!background.splash);
    }

    #[test]
    fn parses_slide_preset_shorthand_and_directive() {
        let source = r#"# Preset one

[.preset: spotlight]

Content.

---

# Preset two

::: preset name=spotlight
:::
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(
            deck.sections[0].main_slide.preset.as_deref(),
            Some("spotlight")
        );
        assert_eq!(
            deck.sections[1].main_slide.preset.as_deref(),
            Some("spotlight")
        );
    }

    #[test]
    fn parses_slide_footer_commands_and_metadata() {
        let source = r##"---
footer: "Global footer"
slide_numbers: true
---

# First slide

[.footer: Local footer]
[.slidenumbers: false]

Visible text.

---

# Hidden footer

[.hide-footer]

No footer here.

---

# Metadata footer

::: slide
footer: "Metadata footer"
slide_numbers: false
:::

Visible text.
"##;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.metadata.footer.as_deref(), Some("Global footer"));
        assert_eq!(deck.metadata.slide_numbers, Some(true));
        let first = &deck.sections[0].main_slide.footer;
        assert_eq!(first.content.as_deref(), Some("Local footer"));
        assert_eq!(first.slide_numbers, Some(false));
        assert!(!first.hidden);
        assert!(deck.sections[1].main_slide.footer.hidden);
        let third = &deck.sections[2].main_slide.footer;
        assert_eq!(third.content.as_deref(), Some("Metadata footer"));
        assert_eq!(third.slide_numbers, Some(false));
    }

    #[test]
    fn parses_autoscale_metadata_and_slide_overrides() {
        let source = r##"---
autoscale: true
---

# Deck autoscale

Visible text.

---

# Disable autoscale

[.autoscale: false]

Visible text.

---

# Metadata autoscale

::: slide
autoscale: true
:::

Visible text.
"##;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.metadata.autoscale, Some(true));
        assert_eq!(deck.sections[0].main_slide.autoscale, None);
        assert_eq!(deck.sections[1].main_slide.autoscale, Some(false));
        assert_eq!(deck.sections[2].main_slide.autoscale, Some(true));
    }

    #[test]
    fn parses_transition_metadata_and_slide_overrides() {
        let source = r##"---
transition: fade
---

# Global transition

Visible text.

---

# Slide transition

[.transition: zoom]

Visible text.

---

# Disable transition

[.slide-transition: false]

Visible text.

---

# Metadata transition

::: slide
transition: slide
:::

Visible text.
"##;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.metadata.transition, Some(SlideTransition::Fade));
        assert_eq!(deck.sections[0].main_slide.transition, None);
        assert_eq!(
            deck.sections[1].main_slide.transition,
            Some(SlideTransition::Zoom)
        );
        assert_eq!(
            deck.sections[2].main_slide.transition,
            Some(SlideTransition::None)
        );
        assert_eq!(
            deck.sections[3].main_slide.transition,
            Some(SlideTransition::Slide)
        );
    }

    #[test]
    fn diagnoses_invalid_slide_theme_param_names() {
        let source = r#"# Bad theme param

::: theme 1bad=value ok-name=value ok_name=value
:::
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.diagnostics.len(), 1);
        assert!(
            deck.diagnostics[0]
                .message
                .contains("invalid theme parameter name '1bad'")
        );
        assert!(
            deck.sections[0]
                .main_slide
                .theme_params
                .contains_key("ok-name")
        );
        assert!(
            deck.sections[0]
                .main_slide
                .theme_params
                .contains_key("ok_name")
        );
    }

    #[test]
    fn diagnoses_invalid_slide_class_names() {
        let source = r#"# Bad class

::: class Lead ok bad_name
:::
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert_eq!(deck.diagnostics.len(), 2);
        assert!(
            deck.diagnostics[0]
                .message
                .contains("invalid slide class 'Lead'")
        );
        assert!(
            deck.diagnostics[1]
                .message
                .contains("invalid slide class 'bad_name'")
        );
        assert_eq!(deck.sections[0].main_slide.classes, vec!["ok"]);
    }

    #[test]
    fn default_step_policy_collapses_to_one_pdf_page() {
        let source = r#"# Claim

::: steps
1. First reveal.
2. Second reveal.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();
        let pages = deck.pdf_pages();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].step_state, PdfStepState::Final);
    }

    #[test]
    fn one_page_per_step_policy_expands_pdf_pages() {
        let source = r#"# Claim

::: steps pdf="pages"
1. First reveal.
2. Second reveal.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();
        let states: Vec<_> = deck
            .pdf_pages()
            .into_iter()
            .map(|page| page.step_state)
            .collect();

        assert_eq!(
            states,
            vec![
                PdfStepState::UpTo { step: 1 },
                PdfStepState::UpTo { step: 2 }
            ]
        );
    }

    #[test]
    fn parses_code_reveal_groups_from_fence_info() {
        let source = r#"# Algorithm

```pseudo reveal="1-2|4,6-7"
Input: active sites A
build conflict graph
compute matching
if bound fails
    lower z.max
return success
emit certificate
```
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Code {
                language: Some(language),
                reveal: Some(reveal),
                ..
            } if language == "pseudo"
                && reveal.groups.len() == 2
                && reveal.groups[0].ranges[0] == CodeLineRange { start: 1, end: 2 }
                && reveal.groups[1].ranges[0] == CodeLineRange { start: 4, end: 4 }
                && reveal.groups[1].ranges[1] == CodeLineRange { start: 6, end: 7 }
        ));
    }

    #[test]
    fn parses_simple_mermaid_flowchart_as_diagram_block() {
        let source = r#"# Diagram

```mermaid
flowchart LR
  A[Start] -->|choose| B{Branch}
  B --> C[Result]
```
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Diagram {
                language: DiagramLanguage::Mermaid,
                source,
            } if source.contains("flowchart LR")
                && source.contains("A[Start] -->|choose| B{Branch}")
        ));
    }

    #[test]
    fn diagnoses_unsupported_mermaid_diagrams() {
        let source = r#"# Diagram

```mermaid
sequenceDiagram
  Alice->>Bob: hello
```
"#;

        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("unsupported Mermaid diagram for HTML/PDF rendering")
                && diagnostic
                    .message
                    .contains("simple Mermaid graph/flowchart")
        }));
    }

    #[test]
    fn diagnoses_invalid_code_reveal_groups() {
        let source = r#"# Algorithm

```pseudo reveal="3-1"
return fail
```
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("invalid code reveal line groups")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_unknown_directives_and_raw_html() {
        let source = r#"# Diagnostics

::: columns
Left
:::

<div>raw</div>

::: html
<div>explicit</div>
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        insta::assert_yaml_snapshot!("diagnostics_for_unsupported_source", deck);
    }

    #[test]
    fn snapshots_malformed_separators() {
        let source = r#"--

### Detail without a main slide

----

# Main
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        insta::assert_yaml_snapshot!("malformed_separators", deck);
    }

    #[test]
    fn canonical_fixture_parses_into_stable_deck_model() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canonical");
        assert!(fixture_root.join("data/runtime.csv").exists());
        assert!(fixture_root.join("assets/phase-space.svg").exists());

        let source = std::fs::read_to_string(fixture_root.join("canonical.zp.md")).unwrap();
        let deck = parse_source_text(
            &source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        assert_eq!(deck.sections.len(), 8);
        assert_eq!(deck.pdf_slide_order().len(), 10);
        insta::assert_yaml_snapshot!("canonical_fixture_deck_model", deck);
    }

    #[test]
    fn rejects_non_zp_md_source_files() {
        let error = parse_source_file("talk.md").unwrap_err();

        assert!(matches!(error, ParseError::UnsupportedExtension { .. }));
    }

    #[test]
    fn diagnoses_missing_local_directive_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("missing-assets.zp.md");
        std::fs::write(
            &source_path,
            r#"# Missing asset

::: figure src="missing.svg"
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure references missing local asset 'missing.svg'")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn parses_markdown_image_as_figure_block() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("figure.zp.md");
        std::fs::write(
            &source_path,
            r#"# Figure

![Phase space](assets/plot.svg "Synthetic phase space")
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                options,
                ..
            } if src == "assets/plot.svg"
                && alt == "Phase space"
                && caption == "Synthetic phase space"
                && options.is_default()
        ));
    }

    #[test]
    fn parses_remote_markdown_figure_with_static_export_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("fallback.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("remote-figure.zp.md");
        std::fs::write(
            &source_path,
            r#"# Remote figure

![pdf-src="assets/fallback.svg" alt="Remote plot"](https://example.com/plot.png "Remote plot")
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                static_src: Some(static_src),
                ..
            } if src == "https://example.com/plot.png"
                && alt == "Remote plot"
                && caption == "Remote plot"
                && static_src == "assets/fallback.svg"
        ));
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/fallback.svg"]
        );
    }

    #[test]
    fn parses_implicit_markdown_figure_caption() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("figure-caption.zp.md");
        std::fs::write(
            &source_path,
            r#"# Figure

![Phase space](assets/plot.svg)
*Synthetic phase space*

After the figure.
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                ..
            } if src == "assets/plot.svg"
                && alt == "Phase space"
                && caption == "Synthetic phase space"
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Paragraph { markdown, .. } if markdown == "After the figure."
        ));
    }

    #[test]
    fn explicit_markdown_figure_caption_keeps_following_emphasis_as_text() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("explicit-caption.zp.md");
        std::fs::write(
            &source_path,
            r#"# Figure

![Phase space](assets/plot.svg "Explicit caption")
*Emphasized paragraph*
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure {
                caption: Some(caption),
                ..
            } if caption == "Explicit caption"
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Paragraph { markdown, .. } if markdown == "*Emphasized paragraph*"
        ));
    }

    #[test]
    fn parses_parenthesized_radius_visual_option() {
        let parsed = parse_figure_options_from_attributes("fit corner-radius(18)");

        assert_eq!(parsed.options.radius.as_deref(), Some("18"));
        assert!(parsed.invalid_radius.is_none());
    }

    #[test]
    fn applies_deck_image_corner_radius_default_to_figures_and_galleries() {
        let source = r#"image-corner-radius: 12

# Image defaults

![Default radius](assets/default.svg "Default")

![corner-radius(24) alt="Explicit radius"](assets/explicit.svg "Explicit")

![inline fill columns=2 alt="Gallery default"](assets/default.svg "Gallery default")
![inline fill corner-radius(30) alt="Gallery explicit"](assets/explicit.svg "Gallery explicit")
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert_eq!(deck.metadata.image_corner_radius.as_deref(), Some("12"));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure { options, .. }
                if options.radius.as_deref() == Some("12")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Figure { options, .. }
                if options.radius.as_deref() == Some("24")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[3],
            ContentBlock::Gallery { items, .. }
                if items[0].options.radius.as_deref() == Some("12")
                    && items[1].options.radius.as_deref() == Some("30")
        ));
    }

    #[test]
    fn parses_front_matter_image_corner_radius_default() {
        let source = r#"---
image_corner_radius: 0.75rem
---

# Image defaults

![Default radius](assets/default.svg)
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert_eq!(deck.diagnostics, Vec::new());
        assert_eq!(
            deck.metadata.image_corner_radius.as_deref(),
            Some("0.75rem")
        );
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure { options, .. }
                if options.radius.as_deref() == Some("0.75rem")
        ));
    }

    #[test]
    fn diagnoses_invalid_image_corner_radius_defaults() {
        let source = r#"---
image_corner_radius: "calc(1rem + 2px)"
---

image-corner-radius: calc(100% - 1rem)

# Bad radius

![No default](assets/default.svg)
"#;

        let deck = parse_source_text(source, None).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("image_corner_radius uses unsupported CSS size")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("image-corner-radius uses unsupported CSS size")
        }));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure { options, .. } if options.radius.is_none()
        ));
    }

    #[test]
    fn parses_figure_sizing_from_directives_and_markdown_images() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        std::fs::write(asset_dir.join("photo.jpg"), "jpg").unwrap();
        std::fs::write(asset_dir.join("overview.svg"), "<svg></svg>").unwrap();
        std::fs::write(asset_dir.join("quarto.png"), "png").unwrap();
        std::fs::write(asset_dir.join("quarto-static.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("sized-figures.zp.md");
        std::fs::write(
            &source_path,
            r#"# Sized figures

::: figure src="assets/plot.svg" alt="Runtime comparison" width="72%" height="48vh" fit="contain" align="center" dim="12" grayscale="20" saturate="85" blur="2" radius="12"
Centered runtime plot.
:::

![width=50% fit=cover align=end grayscale=35 saturate=72 radius=1rem alt="Cropped microscope photo"](assets/photo.jpg "Microscope")

![fit corner-radius(18) alt="Full slide overview"](assets/overview.svg "Overview")

![Quarto-style image](assets/quarto.png){out-width="64%" out-height="40vh" fig-align="left" fig-alt="Accessible Quarto plot" fig-cap="A Quarto-style image attribute block." pdf-src="assets/quarto-static.svg" fig-fit="contain" fig-radius="0.75rem" fig-dim="9" fig-grayscale="12" fig-saturate="88" fig-blur="1"}
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                options,
                ..
            } if src == "assets/plot.svg"
                && alt == "Runtime comparison"
                && caption == "Centered runtime plot."
                && options.width.as_deref() == Some("72%")
                && options.height.as_deref() == Some("48vh")
                && options.fit == Some(FigureFit::Contain)
                && options.align == Some(FigureAlign::Center)
                && options.dim == Some(12)
                && options.grayscale == Some(20)
                && options.saturate == Some(85)
                && options.blur == Some(2)
                && options.radius.as_deref() == Some("12")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                options,
                ..
            } if src == "assets/photo.jpg"
                && alt == "Cropped microscope photo"
                && caption == "Microscope"
                && options.width.as_deref() == Some("50%")
                && options.height.is_none()
                && options.fit == Some(FigureFit::Cover)
                && options.align == Some(FigureAlign::End)
                && options.dim.is_none()
                && options.grayscale == Some(35)
                && options.saturate == Some(72)
                && options.blur.is_none()
                && options.radius.as_deref() == Some("1rem")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[3],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                options,
                ..
            } if src == "assets/overview.svg"
                && alt == "Full slide overview"
                && caption == "Overview"
                && options.width.as_deref() == Some("100%")
                && options.height.as_deref() == Some("100%")
                && options.fit == Some(FigureFit::Contain)
                && options.align == Some(FigureAlign::Center)
                && options.dim.is_none()
                && options.grayscale.is_none()
                && options.saturate.is_none()
                && options.blur.is_none()
                && options.radius.as_deref() == Some("18")
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[4],
            ContentBlock::Figure {
                src,
                alt,
                caption: Some(caption),
                static_src: Some(static_src),
                options,
            } if src == "assets/quarto.png"
                && alt == "Accessible Quarto plot"
                && caption == "A Quarto-style image attribute block."
                && static_src == "assets/quarto-static.svg"
                && options.width.as_deref() == Some("64%")
                && options.height.as_deref() == Some("40vh")
                && options.fit == Some(FigureFit::Contain)
                && options.align == Some(FigureAlign::Start)
                && options.dim == Some(9)
                && options.grayscale == Some(12)
                && options.saturate == Some(88)
                && options.blur == Some(1)
                && options.radius.as_deref() == Some("0.75rem")
        ));
        assert_eq!(
            deck.local_dependency_references(),
            vec![
                "assets/overview.svg",
                "assets/photo.jpg",
                "assets/plot.svg",
                "assets/quarto-static.svg",
                "assets/quarto.png"
            ]
        );
    }

    #[test]
    fn diagnoses_invalid_figure_sizing_options() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("bad-figure-options.zp.md");
        std::fs::write(
            &source_path,
            r#"# Bad figure

::: figure src="assets/plot.svg" width="calc(100% - 1rem)" fit="squish" align="middle" dim="banana" grayscale="101" saturate="200" blur="25" radius="calc(1rem + 2px)"
:::

![Bad Pandoc attrs](assets/plot.svg){width="calc(100% - 1rem)" fig-align="middle" fig-fit="squish" fig-radius="calc(1rem + 2px)" fig-dim="banana" fig-grayscale="101" fig-saturate="200" fig-blur="25"}
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure width uses unsupported CSS size")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure fit must be contain, cover, or fill")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure align must be start, center, end, or stretch")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure dim must be a whole number between 0 and 100")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure grayscale must be a whole number between 0 and 100")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure saturate must be a whole number between 0 and 100")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure blur must be a whole number between 0 and 24 pixels")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure radius uses unsupported CSS size")
        }));
        assert!(
            deck.diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic
                        .message
                        .contains("figure width uses unsupported CSS size")
                })
                .count()
                >= 2
        );
    }

    #[test]
    fn parses_inline_image_gallery_and_local_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("before.png"), "before").unwrap();
        std::fs::write(asset_dir.join("after.png"), "after").unwrap();
        let source_path = temp.path().join("gallery.zp.md");
        std::fs::write(
            &source_path,
            r#"# Gallery

![inline fill columns=2 corner-radius(10) alt="Before state"](assets/before.png "Before")
![inline fit dim="18" radius=0.5rem alt="After state"](assets/after.png "After")
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Gallery {
                items,
                columns: Some(2)
            } if items.len() == 2
                && items[0].src == "assets/before.png"
                && items[0].alt == "Before state"
                && items[0].caption.as_deref() == Some("Before")
                && items[0].options.fit == Some(FigureFit::Fill)
                && items[0].options.radius.as_deref() == Some("10")
                && items[1].src == "assets/after.png"
                && items[1].alt == "After state"
                && items[1].caption.as_deref() == Some("After")
                && items[1].options.fit == Some(FigureFit::Contain)
                && items[1].options.dim == Some(18)
                && items[1].options.radius.as_deref() == Some("0.5rem")
        ));
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/after.png", "assets/before.png"]
        );
    }

    #[test]
    fn parses_media_blocks_and_local_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("clip.mp4"), "video").unwrap();
        std::fs::write(asset_dir.join("walkthrough.webm"), "video").unwrap();
        std::fs::write(asset_dir.join("poster.png"), "poster").unwrap();
        std::fs::write(asset_dir.join("voice.mp3"), "audio").unwrap();
        std::fs::write(asset_dir.join("ambient.wav"), "audio").unwrap();
        let source_path = temp.path().join("media.zp.md");
        std::fs::write(
            &source_path,
            r#"# Media

::: video src="assets/clip.mp4?t=1m30s" poster="assets/poster.png" title="Demo clip" width="62%" fit="cover" align="center" controls=true loop=true muted=true autoadvance=true
A short demo clip.
:::

::: audio src="assets/voice.mp3" title="Narration" start="45" loop=true mute=true hide=true
Listen to the short narration.
:::

::: iframe src="https://example.com/demo" title="Remote demo" poster="assets/poster.png"
Remote demo fallback.
:::

::: youtube src="https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m15s" poster="assets/poster.png" title="Keynote clip"
Watch the key moment.
:::

::: vimeo src="https://vimeo.com/123456" poster="assets/poster.png" title="Prototype demo"
Watch the prototype.
:::

![video right 50% fill loop mute hide autoadvance poster="assets/poster.png" title="Walkthrough" alt="Walkthrough poster"](assets/walkthrough.webm?t=2m3s "Walkthrough caption")

![title="Ambient track"](assets/ambient.wav "Ambient caption")

![iframe poster="assets/poster.png" title="Remote reference"](https://example.com/reference "Reference fallback")

![youtube poster="assets/poster.png" title="Conference talk"](https://youtu.be/dQw4w9WgXcQ?t=45s "Conference talk clip")

![poster="assets/poster.png" title="Vimeo demo"](https://vimeo.com/123456 "Vimeo demo")
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Media {
                kind: MediaKind::Video,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                start_time: Some(90),
                options,
                controls: true,
                loop_playback: true,
                muted: true,
                autoadvance: true,
                ..
            } if src == "assets/clip.mp4"
                && poster == "assets/poster.png"
                && title == "Demo clip"
                && caption == "A short demo clip."
                && options.width.as_deref() == Some("62%")
                && options.fit == Some(FigureFit::Cover)
                && options.align == Some(FigureAlign::Center)
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[2],
            ContentBlock::Media {
                kind: MediaKind::Audio,
                src,
                title: Some(title),
                caption: Some(caption),
                start_time: Some(45),
                loop_playback: true,
                muted: true,
                visual_hidden: true,
                ..
            } if src == "assets/voice.mp3"
                && title == "Narration"
                && caption == "Listen to the short narration."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[4],
            ContentBlock::Media {
                kind: MediaKind::Iframe,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                start_time: Some(75),
                ..
            } if src == "https://www.youtube.com/embed/dQw4w9WgXcQ"
                && poster == "assets/poster.png"
                && title == "Keynote clip"
                && caption == "Watch the key moment."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[5],
            ContentBlock::Media {
                kind: MediaKind::Iframe,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                ..
            } if src == "https://player.vimeo.com/video/123456"
                && poster == "assets/poster.png"
                && title == "Prototype demo"
                && caption == "Watch the prototype."
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[6],
            ContentBlock::Media {
                kind: MediaKind::Video,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                alt,
                start_time: Some(123),
                options,
                controls: true,
                loop_playback: true,
                muted: true,
                visual_hidden: true,
                ..
            } if src == "assets/walkthrough.webm"
                && poster == "assets/poster.png"
                && title == "Walkthrough"
                && caption == "Walkthrough caption"
                && alt == "Walkthrough poster"
                && options.width.as_deref() == Some("50%")
                && options.fit == Some(FigureFit::Fill)
                && options.align == Some(FigureAlign::End)
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[7],
            ContentBlock::Media {
                kind: MediaKind::Audio,
                src,
                title: Some(title),
                caption: Some(caption),
                ..
            } if src == "assets/ambient.wav"
                && title == "Ambient track"
                && caption == "Ambient caption"
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[8],
            ContentBlock::Media {
                kind: MediaKind::Iframe,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                ..
            } if src == "https://example.com/reference"
                && poster == "assets/poster.png"
                && title == "Remote reference"
                && caption == "Reference fallback"
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[9],
            ContentBlock::Media {
                kind: MediaKind::Iframe,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                start_time: Some(45),
                ..
            } if src == "https://www.youtube.com/embed/dQw4w9WgXcQ"
                && poster == "assets/poster.png"
                && title == "Conference talk"
                && caption == "Conference talk clip"
        ));
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[10],
            ContentBlock::Media {
                kind: MediaKind::Iframe,
                src,
                poster: Some(poster),
                title: Some(title),
                caption: Some(caption),
                ..
            } if src == "https://player.vimeo.com/video/123456"
                && poster == "assets/poster.png"
                && title == "Vimeo demo"
                && caption == "Vimeo demo"
        ));
        assert_eq!(
            deck.local_dependency_references(),
            vec![
                "assets/ambient.wav",
                "assets/clip.mp4",
                "assets/poster.png",
                "assets/voice.mp3",
                "assets/walkthrough.webm"
            ]
        );
    }

    #[test]
    fn diagnoses_missing_local_media_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("missing-media.zp.md");
        std::fs::write(
            &source_path,
            r#"# Missing media

::: video src="assets/missing.mp4" poster="assets/missing.png"
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("media src references missing local asset 'assets/missing.mp4'")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("media poster references missing local asset 'assets/missing.png'")
        }));
    }

    #[test]
    fn diagnoses_invalid_media_start_offsets() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("clip.mp4"), "video").unwrap();
        let source_path = temp.path().join("bad-media-start.zp.md");
        std::fs::write(
            &source_path,
            r#"# Bad media start

::: video src="assets/clip.mp4?t=1:30" width="calc(100% - 1rem)" fit="squish"
:::

![video start="soon"](assets/clip.mp4)
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal() && diagnostic.message.contains("media start offset '1:30'")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("media width uses unsupported CSS size")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("media fit must be contain, cover, or fill")
        }));
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal() && diagnostic.message.contains("media start 'soon'")
        }));
    }

    #[test]
    fn parses_background_image_metadata_as_local_dependency() {
        let source = r#"---
title: "Background deck"
background_image:
  src: "assets/phase-space.svg"
  alt: "Phase-space sketch"
  position: "center 42%"
  dim: 82
  grayscale: 40
  saturate: 65
---

# Title

---

# Content
"#;
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let background_image = deck.metadata.background_image.as_ref().unwrap();
        assert_eq!(background_image.src, "assets/phase-space.svg");
        assert_eq!(background_image.alt, "Phase-space sketch");
        assert_eq!(background_image.position, "center 42%");
        assert_eq!(background_image.dim, 82);
        assert_eq!(background_image.grayscale, 40);
        assert_eq!(background_image.saturate, 65);
        assert!(background_image.splash);
        assert!(deck.diagnostics.is_empty());
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/phase-space.svg"]
        );
    }

    #[test]
    fn diagnoses_invalid_background_image_metadata() {
        let source = r#"---
background_image:
  src: "../secret.png"
  position: "center; color:red"
  dim: 101
---

# Title
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("background_image references unsupported local asset path")
        }));
    }

    #[test]
    fn parses_slide_background_directive_as_checked_slide_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("room.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("talk.zp.md");
        std::fs::write(
            &source_path,
            r#"# Visual slide

::: background src="assets/room.svg" alt="Room" position="center 45%" dim="86" grayscale="20" saturate="80" blur="2"
:::

Text over a protected visual background.
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        let background = deck.sections[0]
            .main_slide
            .background_image
            .as_ref()
            .unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(background.src, "assets/room.svg");
        assert_eq!(background.alt, "Room");
        assert_eq!(background.position, "center 45%");
        assert_eq!(background.dim, 86);
        assert_eq!(background.grayscale, 20);
        assert_eq!(background.saturate, 80);
        assert_eq!(background.blur, 2);
        assert!(!background.splash);
        assert_eq!(deck.local_dependency_references(), vec!["assets/room.svg"]);
    }

    #[test]
    fn parses_markdown_bg_image_as_slide_background_shorthand() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("room.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("talk.zp.md");
        std::fs::write(
            &source_path,
            r#"# Visual slide

![bg contain position="center 42%" dim="86" grayscale="35" saturate="72" alt="Room detail"](assets/room.svg)

Text over a protected visual background.
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        let slide = &deck.sections[0].main_slide;
        let background = slide.background_image.as_ref().unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(background.src, "assets/room.svg");
        assert_eq!(background.alt, "Room detail");
        assert_eq!(background.position, "center 42%");
        assert_eq!(background.fit, BackgroundImageFit::Contain);
        assert_eq!(background.dim, 86);
        assert_eq!(background.grayscale, 35);
        assert_eq!(background.saturate, 72);
        assert_eq!(slide.blocks.len(), 2);
        assert!(!matches!(slide.blocks[1], ContentBlock::Figure { .. }));
        assert_eq!(deck.local_dependency_references(), vec!["assets/room.svg"]);
    }

    #[test]
    fn parses_marp_style_split_background_shorthand() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("portrait.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("talk.zp.md");
        std::fs::write(
            &source_path,
            r#"# About

![bg left:40% alt="Portrait"](assets/portrait.svg)

The text should sit beside the image.
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        let background = deck.sections[0]
            .main_slide
            .background_image
            .as_ref()
            .unwrap();
        let split = background.split.as_ref().unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(split.side, BackgroundImageSplitSide::Left);
        assert_eq!(split.size, "40%");
        assert_eq!(background.alt, "Portrait");
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/portrait.svg"]
        );
    }

    #[test]
    fn diagnoses_missing_slide_background_dependency() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("talk.zp.md");
        std::fs::write(
            &source_path,
            r#"# Visual slide

::: background src="assets/missing.svg"
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("background references missing local asset 'assets/missing.svg'")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_invalid_slide_background_treatment_values() {
        let temp = tempfile::tempdir().unwrap();
        let assets_dir = temp.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(assets_dir.join("room.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("talk.zp.md");
        std::fs::write(
            &source_path,
            r#"# Visual slide

::: background src="assets/room.svg" dim="banana" grayscale="300" saturate="101" blur="25"
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        let messages = deck
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();

        assert!(
            messages.iter().any(|message| message.contains(
                "background dim must be a whole number between 0 and 100, found 'banana'"
            ))
        );
        assert!(messages.iter().any(|message| {
            message.contains("background grayscale must be between 0 and 100, found 300")
        }));
        assert!(messages.iter().any(|message| {
            message.contains("background saturate must be between 0 and 100, found 101")
        }));
        assert!(
            messages
                .iter()
                .any(|message| message.contains("background blur must be between 0 and 24 pixels"))
        );
    }

    #[test]
    fn diagnoses_invalid_vega_lite_json_with_source_location() {
        let source = r#"# Chart

::: vega-lite
{ invalid json
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic.message.contains("invalid Vega-Lite JSON")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_missing_chart_data_dependency() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("chart.zp.md");
        std::fs::write(
            &source_path,
            r#"# Chart

::: vega-lite
{ "data": { "url": "data/missing.csv" }, "mark": "line" }
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("chart references missing local data dependency 'data/missing.csv'")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_unsupported_chart_renderer_shape() {
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

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic.message.contains(
                    "unsupported Vega-Lite chart for HTML/PDF rendering: only Vega-Lite line charts",
                )
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_invalid_layout_values_with_source_location() {
        let source = r#"# Layout

::: columns widths="wide/narrow"
Left
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("invalid Columns widths layout value")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn validates_layout_region_markdown_images_as_local_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("inset.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("layout.zp.md");
        std::fs::write(
            &source_path,
            r#"# Layout

::: columns widths="1/1"
Left column:
![width=50% alt="Inset"](assets/inset.svg "Inset figure")

Right column:
Text.
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.local_dependency_references(), vec!["assets/inset.svg"]);
    }

    #[test]
    fn parses_pandoc_fenced_div_columns_as_typed_layout() {
        let temp = tempfile::tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("pandoc-columns.zp.md");
        std::fs::write(
            &source_path,
            r#"# Pandoc columns

:::: {.columns}
::: {.column width="40%" name="Evidence"}
![Evidence plot](assets/plot.svg){out-width="85%" fig-align="center" fig-alt="Evidence plot"}
:::

::: {.column width="60%" name="Explanation"}
- first point
- second point
:::
::::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.is_empty());
        assert_eq!(deck.local_dependency_references(), vec!["assets/plot.svg"]);
        assert!(matches!(
            &deck.sections[0].main_slide.blocks[1],
            ContentBlock::Layout {
                kind: LayoutKind::Columns,
                values,
                regions,
            } if regions.len() == 2
                && regions[0].name.as_deref() == Some("Evidence")
                && regions[0].blocks.iter().any(|block| matches!(
                    block,
                    ContentBlock::Figure { src, .. } if src == "assets/plot.svg"
                ))
                && regions[0].source_span.as_ref().is_some_and(|span| span.line == 5)
                && regions[1].name.as_deref() == Some("Explanation")
                && regions[1].blocks.iter().any(|block| matches!(
                    block,
                    ContentBlock::List { items, .. }
                        if items.iter().any(|item| item.markdown == "second point")
                ))
                && regions[1].source_span.as_ref().is_some_and(|span| span.line == 9)
                && values.widths.as_ref().is_some_and(|widths| widths == &vec![
                    LayoutSize::Arbitrary { value: "40%".to_string() },
                    LayoutSize::Arbitrary { value: "60%".to_string() },
                ])
        ));
    }

    #[test]
    fn diagnoses_missing_layout_region_markdown_image_dependency() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("layout.zp.md");
        std::fs::write(
            &source_path,
            r#"# Layout

::: columns widths="1/1"
Left column:
![Missing](assets/missing.svg)
:::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("figure references missing local asset 'assets/missing.svg'")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 5 && span.column == 1)
        }));
    }

    #[test]
    fn columns_regions_use_the_normal_typed_content_and_dependency_pipeline() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("assets")).unwrap();
        std::fs::create_dir_all(temp.path().join("data")).unwrap();
        std::fs::write(temp.path().join("assets/clip.mp4"), "video").unwrap();
        std::fs::write(temp.path().join("assets/poster.svg"), "<svg></svg>").unwrap();
        std::fs::write(temp.path().join("data/runtime.csv"), "n,ms\n1,2\n").unwrap();
        let source_path = temp.path().join("typed-columns.zp.md");
        std::fs::write(
            &source_path,
            r#"# Typed columns

:::: columns widths="1/1"
Evidence column:
::: video src="assets/clip.mp4" poster="assets/poster.svg" caption="Run"
:::

::: vega-lite data="data/runtime.csv"
{ "mark": "line", "encoding": { "x": { "field": "n" }, "y": { "field": "ms" } } }
:::

Reasoning column:
> [!NOTE] Invariant
> Nested callouts retain their type.

1. First implication
2. Second implication

::: steps
1. Establish the model.
2. State the consequence.
:::
::::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout { regions, .. } = &deck.sections[0].main_slide.blocks[1] else {
            panic!("expected typed Columns block");
        };
        assert_eq!(regions.len(), 2);
        assert!(
            regions[0]
                .source_span
                .as_ref()
                .is_some_and(|span| span.line == 5)
        );
        assert!(
            regions[0]
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::Media { .. }))
        );
        assert!(
            regions[0]
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::Chart { data: Some(_), .. }))
        );
        assert!(
            regions[1]
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::Callout { .. }))
        );
        assert!(
            regions[1]
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::List { ordered: true, .. }))
        );
        assert!(
            regions[1]
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::Steps { .. }))
        );
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/clip.mp4", "assets/poster.svg", "data/runtime.csv"]
        );
        let serialized = serde_json::to_value(&deck).unwrap();
        let serialized_region = &serialized["sections"][0]["main_slide"]["blocks"][1]["regions"][0];
        assert!(serialized_region["blocks"].is_array());
        assert!(serialized_region.get("markdown").is_none());
    }

    #[test]
    fn diagnoses_slide_metadata_nested_inside_columns_at_its_source_line() {
        let source = r#"# Invalid nesting

:::: columns
Left column:
::: variant claim
:::
::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("directive 'variant' cannot be nested inside a columns Layout region")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 5 && span.column == 1)
        }));
    }

    #[test]
    fn grid_regions_parse_typed_content_tracks_and_source_ordered_placement() {
        let source = r#"# Typed grid

::::: grid tracks="2/1/1" gap="3" align="start"
:::: cell name="Primary" column="1" span="2"
> [!NOTE] Main evidence
> The primary cell spans two tracks.
::::

:::: cell name="Metric" column="3" row="1"
1. First value
2. Second value
::::
:::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("grid.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout {
            kind,
            values,
            regions,
        } = &deck.sections[0].main_slide.blocks[1]
        else {
            panic!("expected typed Grid block");
        };
        assert_eq!(*kind, LayoutKind::Grid);
        assert_eq!(values.tracks.as_ref().unwrap().len(), 3);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].name.as_deref(), Some("Primary"));
        assert_eq!(regions[0].source_span.as_ref().unwrap().line, 4);
        assert_eq!(regions[0].grid_placement.as_ref().unwrap().column, Some(1));
        assert_eq!(regions[0].grid_placement.as_ref().unwrap().column_span, 2);
        assert!(matches!(regions[0].blocks[0], ContentBlock::Callout { .. }));
        assert!(matches!(
            regions[1].blocks[0],
            ContentBlock::List { ordered: true, .. }
        ));

        let serialized = serde_json::to_value(&deck).unwrap();
        let layout = &serialized["sections"][0]["main_slide"]["blocks"][1];
        assert_eq!(layout["values"]["tracks"].as_array().unwrap().len(), 3);
        assert_eq!(layout["regions"][0]["grid_placement"]["column"], 1);
    }

    #[test]
    fn stack_regions_are_explicit_typed_items_without_grid_placement() {
        let source = r#"# Typed stack

::::: stack gap="4" align="stretch"
:::: item name="Question"
What must remain invariant?
::::

:::: item name="Answer"
::: steps
1. Preserve source order.
2. Preserve static output.
:::
::::
:::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("stack.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout { kind, regions, .. } = &deck.sections[0].main_slide.blocks[1]
        else {
            panic!("expected typed Stack block");
        };
        assert_eq!(*kind, LayoutKind::Stack);
        assert_eq!(regions.len(), 2);
        assert!(regions.iter().all(|region| region.grid_placement.is_none()));
        assert!(matches!(
            regions[0].blocks[0],
            ContentBlock::Paragraph { .. }
        ));
        assert!(matches!(regions[1].blocks[0], ContentBlock::Steps { .. }));
    }

    #[test]
    fn diagnoses_grid_spans_beyond_declared_tracks_at_the_cell() {
        let source = r#"# Invalid grid

::::: grid tracks="1/1"
:::: cell column="2" span="2"
Too wide.
::::

:::: cell
Second cell.
::::
:::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("grid.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic.message.contains("exceeds the 2 declared tracks")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 4 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_grid_and_stack_without_explicit_regions() {
        for (kind, region) in [("grid", "cell"), ("stack", "item")] {
            let source = format!("# Invalid {kind}\n\n::: {kind}\nLoose content.\n:::\n");
            let deck = parse_source_text(&source, Some(PathBuf::from("layout.zp.md"))).unwrap();
            assert!(deck.diagnostics.iter().any(|diagnostic| {
                diagnostic.is_fatal()
                    && diagnostic.message.contains(&format!(
                        "requires explicit four-colon ':::: {region}' regions"
                    ))
                    && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
            }));
        }
    }

    #[test]
    fn diagnoses_overlapping_grid_cells_and_grid_attributes_on_stack_items() {
        let grid = r#"# Overlap

::::: grid tracks="1/1"
:::: cell column="1" row="1"
First.
::::
:::: cell column="1" row="1"
Second.
::::
:::::
"#;
        let grid_deck = parse_source_text(grid, Some(PathBuf::from("grid.zp.md"))).unwrap();
        assert!(grid_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("Grid region 2 overlaps region 1 at column 1, row 1")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 7)
        }));

        let stack = r#"# Contradictory Stack

::::: stack
:::: item column="1"
First.
::::
:::: item
Second.
::::
:::::
"#;
        let stack_deck = parse_source_text(stack, Some(PathBuf::from("stack.zp.md"))).unwrap();
        assert!(stack_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("Stack items follow source order")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 4)
        }));
    }

    #[test]
    fn grid_and_stack_collect_dependencies_from_each_typed_region() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("assets")).unwrap();
        std::fs::write(temp.path().join("assets/grid.svg"), "<svg></svg>").unwrap();
        std::fs::write(temp.path().join("assets/stack.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("dependencies.zp.md");
        std::fs::write(
            &source_path,
            r#"# Dependencies

::::: grid tracks="1/1"
:::: cell
![Grid asset](assets/grid.svg)
::::
:::: cell
Grid explanation.
::::
:::::

---

# Stack dependencies

::::: stack
:::: item
![Stack asset](assets/stack.svg)
::::
:::: item
Stack explanation.
::::
:::::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/grid.svg", "assets/stack.svg"]
        );
    }

    #[test]
    fn overlay_and_aside_parse_explicit_roles_and_checked_geometry() {
        let source = r#"# Typed overlay

::::: overlay overlap="edge-only"
:::: base name="Evidence"
Base evidence.
::::
:::: annotation name="Result" anchor="top-end" width="standard"
The fixed point.
::::
:::::

---

# Typed aside

::::: aside supporting="compact" gap="4" align="start"
:::: primary name="Argument"
Primary evidence keeps the role floor.
::::
:::: supporting name="Context"
Short subordinate context.
::::
:::::
"#;
        let deck =
            parse_source_text(source, Some(PathBuf::from("semantic-layouts.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout {
            kind,
            values,
            regions,
        } = &deck.sections[0].main_slide.blocks[1]
        else {
            panic!("expected Overlay");
        };
        assert_eq!(*kind, LayoutKind::Overlay);
        assert_eq!(values.overlay_policy, Some(OverlayPolicy::EdgeOnly));
        assert_eq!(regions[0].role, Some(LayoutRegionRole::Base));
        assert_eq!(regions[1].role, Some(LayoutRegionRole::Annotation));
        assert_eq!(
            regions[1].overlay_placement,
            Some(OverlayPlacement {
                anchor: OverlayAnchor::TopEnd,
                width: OverlayWidth::Standard,
            })
        );

        let ContentBlock::Layout {
            kind,
            values,
            regions,
        } = &deck.sections[1].main_slide.blocks[1]
        else {
            panic!("expected Aside");
        };
        assert_eq!(*kind, LayoutKind::Aside);
        assert_eq!(values.aside_width, Some(AsideWidth::Compact));
        assert_eq!(regions[0].role, Some(LayoutRegionRole::Primary));
        assert_eq!(regions[1].role, Some(LayoutRegionRole::Supporting));

        let serialized = serde_json::to_value(&deck).unwrap();
        assert_eq!(
            serialized["sections"][0]["main_slide"]["blocks"][1]["regions"][1]["overlay_placement"]
                ["anchor"],
            "top_end"
        );
    }

    #[test]
    fn overlay_and_aside_collect_dependencies_through_typed_roles() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("assets")).unwrap();
        std::fs::write(temp.path().join("assets/base.svg"), "<svg></svg>").unwrap();
        std::fs::write(temp.path().join("assets/context.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("roles.zp.md");
        std::fs::write(
            &source_path,
            r#"# Overlay dependencies

::::: overlay
:::: base
![Base](assets/base.svg)
::::
:::: annotation anchor="top-end"
Result.
::::
:::::

---

# Aside dependencies

::::: aside
:::: primary
Argument.
::::
:::: supporting
![Context](assets/context.svg)
::::
:::::
"#,
        )
        .unwrap();

        let deck = parse_source_file(&source_path).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        assert_eq!(
            deck.local_dependency_references(),
            vec!["assets/base.svg", "assets/context.svg"]
        );
    }

    #[test]
    fn diagnoses_invalid_overlay_and_aside_semantics_at_the_authored_region() {
        let overlay = r#"# Invalid overlay

::::: overlay overlap="anything"
:::: annotation anchor="center"
Annotation first.
::::
:::: base anchor="top-start"
Base second.
::::
:::: annotation anchor="top-end"
Duplicate edge.
::::
:::: annotation anchor="top-end"
Duplicate edge again.
::::
:::::
"#;
        let overlay_deck =
            parse_source_text(overlay, Some(PathBuf::from("overlay.zp.md"))).unwrap();
        let messages = overlay_deck
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("overlap must be 'edge-only'"))
        );

        let overlay = overlay.replace(" overlap=\"anything\"", "");
        let overlay_deck =
            parse_source_text(&overlay, Some(PathBuf::from("overlay.zp.md"))).unwrap();
        assert!(overlay_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("anchor must be top-start")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 4)
        }));
        assert!(overlay_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("base must be the first region")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
        }));
        assert!(overlay_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("base does not accept annotation")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 7)
        }));
        assert!(overlay_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("reuses the edge anchor")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 13)
        }));

        let aside = r#"# Invalid aside

::::: aside supporting="half"
:::: supporting
Wrong order.
::::
:::: primary
Primary second.
::::
:::::
"#;
        let aside_deck = parse_source_text(aside, Some(PathBuf::from("aside.zp.md"))).unwrap();
        assert!(aside_deck.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("supporting width must be 'compact' or 'standard'")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
        }));
    }

    #[test]
    fn comparison_block_establishes_variant_labels_roles_and_typed_content() {
        let source = r#"# Solver comparison

::::: comparison
:::: primary label="Baseline"
> [!NOTE] Existing model
> Uses chronological branching.
::::

:::: supporting label="Candidate"
1. Stronger propagation
2. Smaller tree
::::
:::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("comparison.zp.md"))).unwrap();
        let slide = &deck.sections[0].main_slide;

        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        assert_eq!(slide.variant, Some(SlideVariant::Comparison));
        let ContentBlock::Layout {
            kind,
            values,
            regions,
        } = &slide.blocks[1]
        else {
            panic!("expected Comparison Columns");
        };
        assert_eq!(*kind, LayoutKind::Columns);
        assert_eq!(values.widths.as_ref().unwrap().len(), 2);
        assert_eq!(regions[0].name.as_deref(), Some("Baseline"));
        assert_eq!(regions[0].role, Some(LayoutRegionRole::Primary));
        assert_eq!(regions[1].name.as_deref(), Some("Candidate"));
        assert_eq!(regions[1].role, Some(LayoutRegionRole::Supporting));
        assert!(matches!(regions[0].blocks[0], ContentBlock::Callout { .. }));
        assert!(matches!(
            regions[1].blocks[0],
            ContentBlock::List { ordered: true, .. }
        ));
    }

    #[test]
    fn comparison_variant_promotes_labeled_columns_to_stable_roles() {
        let source = r#"# Comparison

::: variant comparison
:::

::: columns widths="1/1"
Baseline column:
Original model.

Candidate column:
Improved model.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("comparison.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let ContentBlock::Layout { regions, .. } = &deck.sections[0].main_slide.blocks[1] else {
            panic!("expected Columns");
        };
        assert_eq!(regions[0].role, Some(LayoutRegionRole::Primary));
        assert_eq!(regions[1].role, Some(LayoutRegionRole::Supporting));
    }

    #[test]
    fn diagnoses_unlabeled_or_misordered_comparison_regions() {
        let unlabeled = r#"# Unlabeled comparison

::::: comparison
:::: primary
Primary.
::::
:::: supporting label="Candidate"
Supporting.
::::
:::::
"#;
        let deck = parse_source_text(unlabeled, Some(PathBuf::from("comparison.zp.md"))).unwrap();
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("require a non-empty label")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 4)
        }));

        let misordered = r#"# Misordered comparison

::::: comparison
:::: supporting label="Candidate"
Supporting first.
::::
:::: primary label="Baseline"
Primary second.
::::
:::::
"#;
        let deck = parse_source_text(misordered, Some(PathBuf::from("comparison.zp.md"))).unwrap();
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("exactly one primary region followed by one supporting")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
        }));
    }

    #[test]
    fn parses_semantic_derivation_regions_and_final_pdf_policy() {
        let source = r#"# Derivation

::::: derivation
:::: context label="Invariant"
For all t, x_t is in D.
::::
:::: stage label="Substitute"
$$x_{t+1}=f(x_t,u_t)$$
::::
:::: stage label="Conclude"
The invariant is preserved.
::::
:::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("derivation.zp.md"))).unwrap();

        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let slide = &deck.sections[0].main_slide;
        assert_eq!(slide.variant, Some(SlideVariant::Derivation));
        assert_eq!(slide_screen_step_count(slide), 2);
        assert_eq!(slide_step_count(slide), 2);
        assert_eq!(
            slide_step_pdf_policy(slide),
            Some(StepPdfPolicy::FinalState)
        );
        let ContentBlock::Layout {
            kind,
            values,
            regions,
        } = &slide.blocks[1]
        else {
            panic!("expected Derivation layout");
        };
        assert_eq!(*kind, LayoutKind::Stack);
        assert_eq!(values.step_pdf_policy, Some(StepPdfPolicy::FinalState));
        assert_eq!(regions[0].role, Some(LayoutRegionRole::Stable));
        assert_eq!(regions[0].derivation_step, None);
        assert_eq!(regions[1].role, Some(LayoutRegionRole::Change));
        assert_eq!(regions[1].derivation_step, Some(1));
        assert_eq!(regions[2].derivation_step, Some(2));
        let serialized = serde_json::to_value(&deck).unwrap();
        assert_eq!(
            serialized["sections"][0]["main_slide"]["blocks"][1]["regions"][2]["derivation_step"],
            2
        );
    }

    #[test]
    fn derivation_pages_policy_produces_one_cumulative_page_per_stage() {
        let source = r#"# Process

::::: derivation pdf="pages"
:::: context label="Fixed"
Keep the input fixed.
::::
:::: stage label="First"
Measure the baseline.
::::
:::: stage label="Second"
Compare the result.
::::
:::::
"#;
        let deck = parse_source_text(source, None).unwrap();
        let slide = &deck.sections[0].main_slide;
        assert_eq!(
            slide_step_pdf_policy(slide),
            Some(StepPdfPolicy::OnePagePerStep)
        );
        assert_eq!(slide_step_count(slide), 2);
    }

    #[test]
    fn diagnoses_invalid_derivation_structure_and_nested_progression() {
        let source = r#"# Broken

::::: derivation
:::: stage label="Too early"
No context yet.
::::
:::: context
::: steps
1. Hidden progression.
:::
::::
:::::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("derivation.zp.md"))).unwrap();
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("one context followed by one to six stage regions")
        }));

        let nested = r#"# Nested

::::: derivation
:::: context label="Fixed"
::: steps
1. Nested.
:::
::::
:::: stage label="Change"
Done.
::::
:::::
"#;
        let deck = parse_source_text(nested, Some(PathBuf::from("derivation.zp.md"))).unwrap();
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("cannot nest Steps")
                && diagnostic.span.as_ref().is_some_and(|span| {
                    span.source_path.as_deref() == Some(Path::new("derivation.zp.md"))
                })
        }));
    }

    #[test]
    fn diagnoses_code_reveal_lines_beyond_the_block() {
        let source = "# Code\n\n```rust reveal=\"1|3\"\nlet x = 1;\nlet y = 2;\n```\n";
        let deck = parse_source_text(source, Some(PathBuf::from("code.zp.md"))).unwrap();
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("references line 3")
                && diagnostic.message.contains("2 line(s)")
                && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
        }));
    }

    #[test]
    fn diagnoses_invalid_step_entries_with_source_location() {
        let source = r#"# Claim

::: steps
- Not ordered.
:::
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic
                    .message
                    .contains("steps directive entries must be ordered list items")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn diagnoses_unsupported_display_math_with_source_location() {
        let source = r#"# Bad math

$$
\notarealcommand{x}
$$
"#;
        let deck = parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_fatal()
                && diagnostic.message.contains("unsupported KaTeX math")
                && diagnostic
                    .span
                    .as_ref()
                    .is_some_and(|span| span.line == 3 && span.column == 1)
        }));
    }

    #[test]
    fn retains_v1_background_authoring_presence_and_exact_front_matter_spans() {
        let deck = parse_source_text(
            r#"---
title: Background diagnostics
background_image:
  src: assets/field.svg
  position: "center; color:red"
  title: maybe
  splash: sometimes
  mystery: ignored
---

# Title
"#,
            Some(PathBuf::from("talk.zp.md")),
        )
        .unwrap();

        let background = deck.metadata.background_image.as_ref().unwrap();
        assert!(background.splash_explicit);
        assert_eq!(
            background.source_span.as_ref().map(|span| span.line),
            Some(4)
        );
        for (message, line) in [
            ("position contains unsupported CSS tokens", 5),
            ("title must be 'clean' or 'paint'", 6),
            ("splash must be true or false", 7),
            ("unknown field 'mystery'", 8),
        ] {
            assert!(
                deck.metadata
                    .theme_api_v1_diagnostics
                    .iter()
                    .any(|diagnostic| {
                        diagnostic.message.contains(message)
                            && diagnostic.span.as_ref().is_some_and(|span| {
                                span.source_path.as_deref() == Some(Path::new("talk.zp.md"))
                                    && span.line == line
                            })
                    }),
                "missing diagnostic '{message}' at line {line}"
            );
        }
    }

    #[test]
    fn retains_v1_context_errors_for_deck_only_slide_background_fields() {
        let deck = parse_source_text(
            r#"# Slide

::: slide
background:
  src: assets/local.svg
  title: paint
  splash: true
:::
"#,
            Some(PathBuf::from("talk.zp.md")),
        )
        .unwrap();

        for field in ["title", "splash"] {
            assert!(
                deck.metadata
                    .theme_api_v1_diagnostics
                    .iter()
                    .any(|diagnostic| {
                        diagnostic
                            .message
                            .contains(&format!("field '{field}' is Deck-only"))
                            && diagnostic.span.as_ref().is_some_and(|span| span.line == 3)
                    })
            );
        }
    }

    #[test]
    fn v1_slide_backgrounds_reject_bare_unknown_and_duplicate_fields() {
        let deck = parse_source_text(
            r#"# Directive

::: background src="assets/one.svg" bogus fit=cover fit=contain
:::

---

![bg conatin left:5%](assets/two.svg)

# Markdown
"#,
            Some(PathBuf::from("talk.zp.md")),
        )
        .unwrap();

        for message in [
            "unknown bare token 'bogus'",
            "repeats field 'fit'",
            "unknown bare token 'conatin'",
            "unknown bare token 'left:5%'",
        ] {
            assert!(
                deck.metadata
                    .theme_api_v1_diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(message)),
                "missing {message}"
            );
        }
    }
}
