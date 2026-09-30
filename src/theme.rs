use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::file_url::file_url;
use crate::{deck, html, pdf};

pub const SUPPORTED_THEME_API_VERSIONS: &[u32] = &[1];
pub const DEFAULT_THEME_NAME: &str = "debug";
pub const SUPPORTED_OUTPUT_TARGETS: &[&str] = &["html", "pdf"];
pub const SUPPORTED_THEME_MODULES: &[&str] = &["scientific-data"];
pub const SUPPORTED_SLIDE_VARIANTS: &[&str] = &[
    "claim",
    "figure",
    "comparison",
    "derivation",
    "section-title",
    "dense",
];

const BUILTIN_THEME_FILES: &[(&str, &[u8])] = &[
    (
        "dark-splash/theme.toml",
        include_bytes!("../themes/dark-splash/theme.toml"),
    ),
    (
        "dark-splash/theme.css.tmpl",
        include_bytes!("../themes/dark-splash/theme.css.tmpl"),
    ),
    (
        "dark-splash/print.css.tmpl",
        include_bytes!("../themes/dark-splash/print.css.tmpl"),
    ),
    (
        "debug/theme.toml",
        include_bytes!("../themes/debug/theme.toml"),
    ),
    (
        "debug/theme.css.tmpl",
        include_bytes!("../themes/debug/theme.css.tmpl"),
    ),
    (
        "debug/print.css.tmpl",
        include_bytes!("../themes/debug/print.css.tmpl"),
    ),
    (
        "debug/specimen.zp.md",
        include_bytes!("../themes/debug/specimen.zp.md"),
    ),
    (
        "debug/comparison-field.svg",
        include_bytes!("../themes/debug/comparison-field.svg"),
    ),
    (
        "paper-chalk/theme.toml",
        include_bytes!("../themes/paper-chalk/theme.toml"),
    ),
    (
        "paper-chalk/theme.css.tmpl",
        include_bytes!("../themes/paper-chalk/theme.css.tmpl"),
    ),
    (
        "paper-chalk/print.css.tmpl",
        include_bytes!("../themes/paper-chalk/print.css.tmpl"),
    ),
    (
        "science/theme.toml",
        include_bytes!("../themes/science/theme.toml"),
    ),
    (
        "science/theme.css.tmpl",
        include_bytes!("../themes/science/theme.css.tmpl"),
    ),
    (
        "science/print.css.tmpl",
        include_bytes!("../themes/science/print.css.tmpl"),
    ),
    (
        "science/assets/science-orbit-trace-dark.svg",
        include_bytes!("../themes/science/assets/science-orbit-trace-dark.svg"),
    ),
    (
        "science/specimen.zp.md",
        include_bytes!("../themes/science/specimen.zp.md"),
    ),
    (
        "science/assets/science-orbit.svg",
        include_bytes!("../themes/science/assets/science-orbit.svg"),
    ),
    ("sv/theme.toml", include_bytes!("../themes/sv/theme.toml")),
    (
        "sv/theme.css.tmpl",
        include_bytes!("../themes/sv/theme.css.tmpl"),
    ),
    (
        "sv/print.css.tmpl",
        include_bytes!("../themes/sv/print.css.tmpl"),
    ),
    (
        "reference/theme.toml",
        include_bytes!("../themes/reference/theme.toml"),
    ),
    (
        "reference/theme.css.tmpl",
        include_bytes!("../themes/reference/theme.css.tmpl"),
    ),
    (
        "reference/print.css.tmpl",
        include_bytes!("../themes/reference/print.css.tmpl"),
    ),
    (
        "reference/specimen.zp.md",
        include_bytes!("../themes/reference/specimen.zp.md"),
    ),
    (
        "reference/contract-field.svg",
        include_bytes!("../themes/reference/contract-field.svg"),
    ),
    (
        "wedding/theme.toml",
        include_bytes!("../themes/wedding/theme.toml"),
    ),
    (
        "wedding/theme.css.tmpl",
        include_bytes!("../themes/wedding/theme.css.tmpl"),
    ),
    (
        "wedding/print.css.tmpl",
        include_bytes!("../themes/wedding/print.css.tmpl"),
    ),
    (
        "wedding/assets/botanical.svg",
        include_bytes!("../themes/wedding/assets/botanical.svg"),
    ),
    (
        "wedding/assets/wildflower.svg",
        include_bytes!("../themes/wedding/assets/wildflower.svg"),
    ),
    (
        "wedding/assets/vine.svg",
        include_bytes!("../themes/wedding/assets/vine.svg"),
    ),
    (
        "wedding/assets/bow.svg",
        include_bytes!("../themes/wedding/assets/bow.svg"),
    ),
    (
        "wedding/assets/minimal.svg",
        include_bytes!("../themes/wedding/assets/minimal.svg"),
    ),
    (
        "__fixtures/theme-api-v1/sv-reference.zp.md",
        include_bytes!("../fixtures/theme-api-v1/sv-reference.zp.md"),
    ),
    (
        "__fixtures/theme-api-v1/paper-chalk-reference.zp.md",
        include_bytes!("../fixtures/theme-api-v1/paper-chalk-reference.zp.md"),
    ),
    (
        "__fixtures/theme-api-v1/wedding-reference.zp.md",
        include_bytes!("../fixtures/theme-api-v1/wedding-reference.zp.md"),
    ),
    (
        "__fixtures/theme-api-v1/dark-splash-reference.zp.md",
        include_bytes!("../fixtures/theme-api-v1/dark-splash-reference.zp.md"),
    ),
    (
        "__fixtures/theme-api-v1/assets/science-orbit.svg",
        include_bytes!("../fixtures/theme-api-v1/assets/science-orbit.svg"),
    ),
    (
        "__fixtures/theme-api-v1/assets/dark-splash-orbit.png",
        include_bytes!("../fixtures/theme-api-v1/assets/dark-splash-orbit.png"),
    ),
    (
        "__fixtures/theme-api-v1/data/columns-runtime.csv",
        include_bytes!("../fixtures/theme-api-v1/data/columns-runtime.csv"),
    ),
];
static BUILTIN_THEME_ROOT: OnceLock<PathBuf> = OnceLock::new();
pub const SUPPORTED_THEME_FEATURE_HOOKS: &[&str] = &[
    "autoscale",
    "background-image",
    "background-splash",
    "background-split",
    "background-treatment",
    "chart-local-data",
    "code-reveal",
    "detail-slides",
    "figure-align",
    "figure-fit",
    "figure-radius",
    "figure-size",
    "figure-treatment",
    "fit-text",
    "footer",
    "footnotes",
    "gallery-columns",
    "html-only",
    "image-gallery",
    "list-reveal",
    "media-align",
    "media-autoadvance",
    "media-fit",
    "media-hidden",
    "media-poster",
    "media-size",
    "media-start",
    "mermaid-diagram",
    "slide-classes",
    "slide-presets",
    "speaker-notes",
    "steps-final-state",
    "steps-pages",
    "transitions",
];
const THEME_API_V1_STABLE_REGION_SELECTORS: &[&str] = &[
    ".zpres-api-v1",
    ".zpres-section-stack",
    ".zpres-slide",
    ".zpres-slide-background",
    ".zpres-slide-frame",
    ".zpres-slide-header",
    ".zpres-slide-title",
    ".zpres-slide-body",
    ".zpres-slide-primary",
    ".zpres-slide-supporting",
    ".zpres-slide-sources",
    ".zpres-slide-footer",
    ".zpres-slide-footer-content",
    ".zpres-slide-footer-number",
    ".zpres-print-slide",
    ".zpres-slide-canvas",
    ".zpres-slide-content",
];
const STABLE_THEME_CONTENT_SELECTORS: &[&str] = &[
    ".zpres-block",
    ".zpres-block-label",
    ".zpres-block-heading",
    ".zpres-block-paragraph",
    ".zpres-block-fit-text",
    ".zpres-block-quote",
    ".zpres-block-callout",
    ".zpres-callout-title",
    ".zpres-block-list",
    ".zpres-block-math",
    ".zpres-block-code",
    ".zpres-block-table",
    ".zpres-block-figure",
    ".zpres-block-footnotes",
    ".zpres-footnote-ref",
    ".zpres-footnote-list",
    ".zpres-footnote",
    ".zpres-block-gallery",
    ".zpres-gallery-item",
    ".zpres-block-media",
    ".zpres-block-diagram",
    ".zpres-block-layout",
    ".zpres-layout-region",
    ".zpres-layout-region-title",
    ".zpres-block-chart",
    ".zpres-block-steps",
    ".zpres-step",
    ".zpres-code-line",
    ".zpres-math-inline",
    ".zpres-math-display",
    ".zpres-media-fallback",
    ".zpres-speaker-notes-source",
    ".zpres-speaker-notes-body",
];
const INTERNAL_THEME_SELECTORS: &[&str] = &[
    ".zpres-print-body",
    ".zpres-detail-indicator",
    ".zpres-slide-ornament",
    ".zpres-code-line-text",
    ".zpres-diagram-svg",
    ".zpres-diagram-surface",
    ".zpres-diagram-node",
    ".zpres-diagram-arrow",
    ".zpres-diagram-edge",
    ".zpres-diagram-edge-label",
    ".zpres-chart-svg",
    ".zpres-chart-grid",
    ".zpres-chart-axis",
    ".zpres-chart-axis-label",
    ".zpres-chart-legend",
    ".zpres-chart-line",
    ".zpres-chart-point",
    ".zpres-chart-mark",
    ".zpres-chart-annotation",
    ".zpres-chart-direct-annotation-point",
    ".zpres-speaker-notes-panel",
    ".zpres-speaker-notes-panel-body",
    ".zpres-speaker-notes-print-slide",
    ".zpres-speaker-notes-print-canvas",
    ".zpres-speaker-notes-print-block",
    ".zpres-speaker-notes-print-body",
];

pub fn supported_theme_semantic_selectors(
    _api: ThemeApiVersion,
) -> impl Iterator<Item = &'static str> {
    THEME_API_V1_STABLE_REGION_SELECTORS
        .iter()
        .chain(STABLE_THEME_CONTENT_SELECTORS)
        .chain(INTERNAL_THEME_SELECTORS)
        .copied()
}

pub fn stable_v1_theme_selectors() -> impl Iterator<Item = &'static str> {
    THEME_API_V1_STABLE_REGION_SELECTORS
        .iter()
        .chain(STABLE_THEME_CONTENT_SELECTORS)
        .copied()
}

pub fn internal_theme_selectors() -> impl Iterator<Item = &'static str> {
    INTERNAL_THEME_SELECTORS.iter().copied()
}
pub const STANDARD_COLOR_SLOTS: &[&str] = &[
    "background",
    "surface",
    "text",
    "muted",
    "accent",
    "accent_alt",
    "rule",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ThemeManifest {
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub style: ThemeStyle,
    pub fonts: Vec<String>,
    pub assets: Vec<String>,
    pub stylesheet: String,
    pub print_stylesheet: String,
    pub palette_parameter: String,
    pub output_targets: Vec<String>,
    pub modules: Vec<String>,
    pub slide_variants: Vec<String>,
    pub feature_hooks: Vec<String>,
    pub slide_presets: BTreeMap<String, ThemeSlidePreset>,
    pub color_variants: BTreeMap<String, ThemeColorVariant>,
    pub parameters: BTreeMap<String, ThemeParameter>,
}

impl ThemeManifest {
    pub fn api(&self) -> ThemeApiVersion {
        ThemeApiVersion::from_u32(self.api_version)
            .expect("ThemeManifest api_version is validated while loading")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeApiVersion {
    V1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDeckContractError {
    pub theme_name: String,
    pub api_version: u32,
    pub diagnostics: Vec<deck::Diagnostic>,
}

impl fmt::Display for ThemeDeckContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reasons = self
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        write!(
            formatter,
            "Deck is not supported by Theme API {} theme '{}': {reasons}",
            self.api_version, self.theme_name
        )
    }
}

impl std::error::Error for ThemeDeckContractError {}

impl ThemeApiVersion {
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::V1 => 1,
        }
    }

    const fn from_u32(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::V1),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RenderedTheme {
    pub manifest: ThemeManifest,
    pub params: BTreeMap<String, String>,
    pub screen_css: String,
    pub print_css: String,
    pub static_screen_css: String,
    pub static_print_css: String,
}

impl RenderedTheme {
    pub fn name(&self) -> &str {
        &self.manifest.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeCssDeclaration {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThemeCheckReport {
    pub manifest_path: PathBuf,
    pub name: String,
    pub version: String,
    pub checked_files: Vec<PathBuf>,
    pub theme_parameters: Vec<ThemeParameterCheckReport>,
    pub unused_theme_parameters: Vec<String>,
    pub rendered_variants: Vec<String>,
    pub declared_modules: Vec<String>,
    pub declared_feature_hooks: Vec<String>,
    pub unknown_selectors: Vec<String>,
    pub dead_selectors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture: Option<ThemeFixtureCheckReport>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThemeParameterCheckReport {
    pub name: String,
    pub parameter_type: String,
    pub default: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThemeFixtureCheckReport {
    pub source_path: PathBuf,
    pub sections: usize,
    pub expected_pdf_pages: usize,
    pub html_bytes: usize,
    pub print_html_bytes: usize,
    pub slide_variants: Vec<String>,
    pub missing_declared_slide_variants: Vec<String>,
    pub slide_presets: Vec<String>,
    pub slide_classes: Vec<String>,
    pub content_blocks: Vec<String>,
    pub layout_kinds: Vec<String>,
    pub media_kinds: Vec<String>,
    pub feature_hooks: Vec<String>,
    pub missing_declared_feature_hooks: Vec<String>,
    pub uncovered_feature_hooks: Vec<String>,
    pub dead_selectors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeCheckOptions {
    pub fixture: Option<PathBuf>,
    pub require_complete_fixture_feature_coverage: bool,
}

impl Default for ThemeCheckOptions {
    fn default() -> Self {
        Self {
            fixture: default_theme_check_fixture(),
            require_complete_fixture_feature_coverage: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ThemeStyle {
    pub family: Option<String>,
    pub inspiration: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThemeColorVariant {
    pub colors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ThemeSlidePreset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<deck::SlideVariant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub theme_params: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<deck::SlideTransition>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ThemeParameter {
    pub parameter_type: ThemeParameterType,
    pub default: Option<String>,
    pub required: bool,
    pub values: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeParameterType {
    String,
    Integer,
    Number,
    Boolean,
    Enum,
    Color,
    Font,
    Size,
}

impl ThemeParameterType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Enum => "enum",
            Self::Color => "color",
            Self::Font => "font",
            Self::Size => "size",
        }
    }
}

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("theme '{name}' was not found; searched: {searched}")]
    NotFound { name: String, searched: String },
    #[error("failed to read theme manifest {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to read theme template {path}: {source}")]
    ReadTemplate { path: PathBuf, source: io::Error },
    #[error("invalid theme manifest TOML in {path}{location}: {source}")]
    Toml {
        path: PathBuf,
        location: String,
        source: Box<toml::de::Error>,
    },
    #[error(
        "theme manifest {path}:{line}:{column} declares unsupported api_version {found}; supported api_version is 1"
    )]
    UnsupportedApiVersion {
        path: PathBuf,
        line: usize,
        column: usize,
        found: u32,
    },
    #[error("theme manifest {path} declares name '{found}', expected '{expected}'")]
    NameMismatch {
        path: PathBuf,
        found: String,
        expected: String,
    },
    #[error("theme '{name}' has invalid name; expected lowercase letters, digits, and hyphens")]
    InvalidName { name: String },
    #[error("theme '{theme}' declares unsupported output target '{target}'")]
    InvalidOutputTarget { theme: String, target: String },
    #[error("theme '{theme}' declares unsupported shared module '{module}'")]
    InvalidThemeModule { theme: String, module: String },
    #[error("theme '{theme}' declares shared module '{module}' more than once")]
    DuplicateThemeModule { theme: String, module: String },
    #[error("theme '{theme}' must declare output target '{target}'")]
    MissingOutputTarget { theme: String, target: String },
    #[error("theme '{theme}' declares unsupported slide variant '{variant}'")]
    InvalidSlideVariant { theme: String, variant: String },
    #[error(
        "theme '{theme}' declares invalid slide preset '{preset}'; expected lowercase letters, digits, and hyphens starting with a letter"
    )]
    InvalidSlidePreset { theme: String, preset: String },
    #[error("theme '{theme}' slide preset '{preset}' declares invalid class '{class}'")]
    InvalidSlidePresetClass {
        theme: String,
        preset: String,
        class: String,
    },
    #[error("theme '{theme}' slide preset '{preset}' declares unknown theme parameter '{name}'")]
    UnknownSlidePresetParameter {
        theme: String,
        preset: String,
        name: String,
    },
    #[error("theme '{theme}' slide preset '{preset}' declares unknown transition '{transition}'")]
    InvalidSlidePresetTransition {
        theme: String,
        preset: String,
        transition: String,
    },
    #[error("theme '{theme}' does not define slide preset '{preset}' used by slide '{slide_id}'")]
    UnknownSlidePreset {
        theme: String,
        slide_id: String,
        preset: String,
    },
    #[error("theme '{theme}' declares unsupported feature hook '{feature}'")]
    InvalidFeatureHook { theme: String, feature: String },
    #[error("theme parameter '{name}' is not declared by theme '{theme}'")]
    UnknownParameter { theme: String, name: String },
    #[error("theme parameter '{name}' is required by theme '{theme}'")]
    MissingParameter { theme: String, name: String },
    #[error("theme parameter '{name}' has invalid value '{value}': {reason}")]
    InvalidParameter {
        name: String,
        value: String,
        reason: String,
    },
    #[error("theme '{theme}' declares unsafe relative path '{path}'")]
    UnsafePath { theme: String, path: String },
    #[error(
        "theme '{theme}' declares dependency path '{path}' more than once across fonts and assets"
    )]
    DuplicateDependencyPath { theme: String, path: String },
    #[error(
        "theme '{theme}' dependency paths '{first}' and '{second}' collide on case-insensitive filesystems"
    )]
    PortableDependencyCollision {
        theme: String,
        first: String,
        second: String,
    },
    #[error("theme '{theme}' references missing file '{path}'")]
    MissingReferencedFile { theme: String, path: PathBuf },
    #[error("failed to resolve theme '{theme}' dependency '{path}': {source}")]
    ResolveDependency {
        theme: String,
        path: PathBuf,
        source: io::Error,
    },
    #[error(
        "theme '{theme}' dependency '{dependency}' resolves outside package root '{root}' as '{resolved}'"
    )]
    DependencyOutsideThemeRoot {
        theme: String,
        dependency: String,
        root: PathBuf,
        resolved: PathBuf,
    },
    #[error("failed to resolve theme '{theme}' manifest '{path}': {source}")]
    ResolveManifest {
        theme: String,
        path: PathBuf,
        source: io::Error,
    },
    #[error(
        "theme '{theme}' dependency '{dependency}' conflicts with renderer-owned asset '{asset_path}' for Theme API {api_version}"
    )]
    ReservedDependencyPath {
        theme: String,
        api_version: u32,
        dependency: String,
        asset_path: String,
    },
    #[error(
        "theme '{theme}' stylesheet '{stylesheet}':{line}:{column} has invalid CSS URL '{url}': {reason}"
    )]
    InvalidCssUrl {
        theme: String,
        stylesheet: String,
        line: usize,
        column: usize,
        url: Box<str>,
        reason: Box<str>,
    },
    #[error(
        "theme '{theme}' stylesheet '{stylesheet}':{line}:{column} uses unsupported @import; imported CSS bypasses the declared dependency and URL validation pipeline"
    )]
    UnsupportedCssImport {
        theme: String,
        stylesheet: String,
        line: usize,
        column: usize,
    },
    #[error(
        "theme '{theme}' stylesheet '{stylesheet}':{line}:{column} uses unsupported {function}(); use a declared local dependency through url() so every Output target can validate and rebase it"
    )]
    UnsupportedCssImageFunction {
        theme: String,
        stylesheet: String,
        line: usize,
        column: usize,
        function: &'static str,
    },
    #[error(
        "theme '{theme}' stylesheet '{stylesheet}' resolves outside package root '{root}' as '{resolved}'"
    )]
    StylesheetOutsideThemeRoot {
        theme: String,
        stylesheet: String,
        root: PathBuf,
        resolved: PathBuf,
    },
    #[error("theme '{theme}' failed fixture check for {path}: {reason}")]
    FixtureCheckFailed {
        theme: String,
        path: PathBuf,
        reason: String,
    },
    #[error("theme template {path} references unknown placeholder '{{{{{placeholder}}}}}'")]
    UnknownTemplatePlaceholder { path: PathBuf, placeholder: String },
    #[error("theme template {path} references missing parameter '{name}'")]
    MissingTemplateParameter { path: PathBuf, name: String },
    #[error(
        "theme '{theme}' color variant '{variant}' declares invalid color '{name}' = '{value}'; expected #rgb or #rrggbb"
    )]
    InvalidColorVariant {
        theme: String,
        variant: String,
        name: String,
        value: String,
    },
    #[error("theme '{theme}' color variant '{variant}' is missing color slot '{slot}'")]
    MissingColorSlot {
        theme: String,
        variant: String,
        slot: String,
    },
    #[error("theme '{theme}' color variant '{variant}' uses undeclared color parameter '{name}'")]
    UndeclaredColorSlot {
        theme: String,
        variant: String,
        name: String,
    },
}

#[derive(Debug, Deserialize)]
struct RawThemeManifest {
    theme: RawThemeMetadata,
    #[serde(default)]
    style: RawThemeStyle,
    #[serde(default)]
    color_variants: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    presets: BTreeMap<String, RawThemeSlidePreset>,
    #[serde(default)]
    parameters: BTreeMap<String, RawThemeParameter>,
}

#[derive(Debug, Default, Deserialize)]
struct RawThemeStyle {
    family: Option<String>,
    #[serde(default)]
    inspiration: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawThemeMetadata {
    name: String,
    version: String,
    api_version: toml::Spanned<u32>,
    #[serde(default)]
    fonts: Vec<String>,
    #[serde(default)]
    assets: Vec<String>,
    #[serde(default = "default_theme_stylesheet")]
    stylesheet: String,
    #[serde(default = "default_theme_print_stylesheet")]
    print_stylesheet: String,
    #[serde(default = "default_palette_parameter")]
    palette_parameter: String,
    #[serde(default)]
    output_targets: Vec<String>,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    slide_variants: Vec<String>,
    #[serde(default)]
    feature_hooks: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawThemeParameter {
    #[serde(rename = "type")]
    parameter_type: ThemeParameterType,
    default: Option<toml::Value>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    values: Vec<String>,
    min: Option<f64>,
    max: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawThemeSlidePreset {
    variant: Option<String>,
    #[serde(default)]
    classes: Vec<String>,
    #[serde(default)]
    theme_params: BTreeMap<String, toml::Value>,
    autoscale: Option<bool>,
    transition: Option<String>,
}

pub fn load_named_theme(
    name: &str,
    deck_root: &Path,
    theme_search_paths: &[PathBuf],
) -> Result<ThemeManifest, ThemeError> {
    let candidates = theme_candidates(name, deck_root, theme_search_paths);
    for candidate in &candidates {
        if candidate.exists() {
            let manifest = load_theme_manifest(candidate)?;
            if manifest.name != name {
                return Err(ThemeError::NameMismatch {
                    path: candidate.clone(),
                    found: manifest.name,
                    expected: name.to_string(),
                });
            }
            return Ok(manifest);
        }
    }

    Err(ThemeError::NotFound {
        name: name.to_string(),
        searched: candidates
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

pub fn render_deck_theme(
    deck_theme_name: Option<&str>,
    deck_root: &Path,
    theme_search_paths: &[PathBuf],
    supplied_params: &BTreeMap<String, String>,
) -> Result<RenderedTheme, ThemeError> {
    let theme_name = deck_theme_name.unwrap_or(DEFAULT_THEME_NAME);
    let manifest = load_named_theme(theme_name, deck_root, theme_search_paths)?;
    render_theme(&manifest, supplied_params)
}

pub fn check_theme_package(path: &Path) -> Result<ThemeCheckReport, ThemeError> {
    let theme_dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or_else(|| Path::new("."))
    };
    let local_specimen = theme_dir.join("specimen.zp.md");
    let mut options = ThemeCheckOptions::default();
    if local_specimen.is_file() {
        options.fixture = Some(local_specimen);
    } else if let Some(name) = theme_dir.file_name().and_then(|name| name.to_str()) {
        let fixture = builtin_theme_search_path()
            .join("__fixtures/theme-api-v1")
            .join(format!("{name}-reference.zp.md"));
        if fixture.is_file() {
            options.fixture = Some(fixture);
        }
    }
    check_theme_package_with_options(path, &options)
}

pub fn check_theme_package_with_options(
    path: &Path,
    options: &ThemeCheckOptions,
) -> Result<ThemeCheckReport, ThemeError> {
    let manifest_path = if path.is_dir() {
        path.join("theme.toml")
    } else {
        path.to_path_buf()
    };
    let manifest = load_theme_manifest(&manifest_path)?;
    let mut checked_files = Vec::new();
    for relative_path in manifest_referenced_paths(&manifest) {
        let path = theme_relative_path(&manifest, relative_path)?;
        if !path.is_file() {
            return Err(ThemeError::MissingReferencedFile {
                theme: manifest.name.clone(),
                path,
            });
        }
        checked_files.push(path);
    }

    let mut rendered_variants = Vec::new();
    let mut warnings = Vec::new();
    let default_rendered = render_theme(&manifest, &BTreeMap::new())?;
    let authored_selectors = authored_zpres_selectors(
        &manifest,
        &default_rendered.screen_css,
        &default_rendered.print_css,
    );
    let unknown_selectors = unknown_v1_selectors(&manifest, &authored_selectors);
    rendered_variants.push("defaults".to_string());
    warnings.extend(theme_css_warnings(
        &manifest,
        &default_rendered.screen_css,
        &default_rendered.print_css,
    ));

    if manifest
        .parameters
        .contains_key(&manifest.palette_parameter)
    {
        for variant in manifest.color_variants.keys() {
            render_theme(
                &manifest,
                &BTreeMap::from([(manifest.palette_parameter.clone(), variant.clone())]),
            )?;
            rendered_variants.push(variant.clone());
        }
    } else if !manifest.color_variants.is_empty() {
        warnings.push(format!(
            "theme '{}' declares color variants but palette_parameter '{}' is not a declared parameter",
            manifest.name, manifest.palette_parameter
        ));
    }

    let fixture = options
        .fixture
        .as_ref()
        .map(|fixture| {
            check_theme_fixture(
                &manifest,
                &default_rendered,
                fixture,
                options.require_complete_fixture_feature_coverage,
            )
        })
        .transpose()?;
    if let Some(fixture) = &fixture {
        warnings.extend(
            fixture
                .warnings
                .iter()
                .map(|warning| format!("fixture '{}': {warning}", fixture.source_path.display())),
        );
    }
    if fixture.is_none()
        && (!manifest.feature_hooks.is_empty() || !manifest.slide_variants.is_empty())
    {
        warnings.push(format!(
            "theme '{}' declares slide variants or feature_hooks but no fixture was checked, so fixture coverage was not verified",
            manifest.name
        ));
    } else if let Some(fixture) = &fixture {
        if !fixture.missing_declared_slide_variants.is_empty() {
            warnings.push(format!(
                "fixture '{}' does not exercise declared slide variants: {}; this visual review covers only the variants used by the selected fixture",
                fixture.source_path.display(),
                fixture.missing_declared_slide_variants.join(", ")
            ));
        }
        if !fixture.missing_declared_feature_hooks.is_empty() {
            warnings.push(format!(
                "fixture '{}' does not exercise declared feature_hooks: {}; this visual review covers only the features used by the selected fixture",
                fixture.source_path.display(),
                fixture.missing_declared_feature_hooks.join(", ")
            ));
        }
    }

    let theme_parameters = theme_parameter_check_report(&manifest.parameters);
    let dead_selectors = fixture
        .as_ref()
        .map(|fixture| fixture.dead_selectors.clone())
        .unwrap_or_default();
    let unused_theme_parameters = unused_theme_parameters(&manifest)?;

    Ok(ThemeCheckReport {
        manifest_path: manifest.path.clone(),
        name: manifest.name,
        version: manifest.version,
        checked_files,
        theme_parameters,
        unused_theme_parameters,
        rendered_variants,
        declared_modules: manifest.modules,
        declared_feature_hooks: manifest.feature_hooks,
        unknown_selectors,
        dead_selectors,
        fixture,
        warnings,
    })
}

pub fn unused_theme_parameters(manifest: &ThemeManifest) -> Result<Vec<String>, ThemeError> {
    let screen_path = theme_relative_path(manifest, &manifest.stylesheet)?;
    let print_path = theme_relative_path(manifest, &manifest.print_stylesheet)?;
    let screen = fs::read_to_string(&screen_path).map_err(|source| ThemeError::ReadTemplate {
        path: screen_path,
        source,
    })?;
    let print = fs::read_to_string(&print_path).map_err(|source| ThemeError::ReadTemplate {
        path: print_path,
        source,
    })?;
    let authored_css = format!("{screen}\n{print}");
    let palette_consumed = !manifest.color_variants.is_empty()
        && manifest
            .parameters
            .contains_key(&manifest.palette_parameter);

    Ok(manifest
        .parameters
        .iter()
        .filter_map(|(name, parameter)| {
            if palette_consumed && name == &manifest.palette_parameter {
                return None;
            }
            let mut references = vec![
                format!("{{{{param.{name}}}}}"),
                format!("--zpres-param-{}", css_variable_suffix(name)),
            ];
            match parameter.parameter_type {
                ThemeParameterType::Color => {
                    references.push(format!("--zpres-color-{}", css_variable_suffix(name)))
                }
                ThemeParameterType::Font => {
                    references.push(format!("--zpres-font-{}", font_css_variable_suffix(name)))
                }
                ThemeParameterType::Size => {
                    references.push(format!("--zpres-size-{}", size_css_variable_suffix(name)))
                }
                _ => {}
            }
            (!references
                .iter()
                .any(|reference| authored_css.contains(reference)))
            .then(|| name.clone())
        })
        .collect())
}

fn theme_parameter_check_report(
    parameters: &BTreeMap<String, ThemeParameter>,
) -> Vec<ThemeParameterCheckReport> {
    parameters
        .iter()
        .map(|(name, parameter)| ThemeParameterCheckReport {
            name: name.clone(),
            parameter_type: parameter.parameter_type.as_str().to_string(),
            default: parameter.default.clone(),
            required: parameter.required,
        })
        .collect()
}

pub fn render_theme(
    manifest: &ThemeManifest,
    supplied_params: &BTreeMap<String, String>,
) -> Result<RenderedTheme, ThemeError> {
    let manifest = canonicalize_render_manifest(manifest)?;
    require_output_target(&manifest, "html")?;
    require_output_target(&manifest, "pdf")?;
    require_theme_dependencies(&manifest)?;
    let params = validate_theme_params(&manifest, supplied_params)?;
    let raw_screen_css = render_theme_css(&manifest, &manifest.stylesheet, &params)?;
    let raw_print_css = render_theme_css(&manifest, &manifest.print_stylesheet, &params)?;
    let screen_css = rewrite_theme_css_urls(
        &manifest,
        &manifest.stylesheet,
        &raw_screen_css,
        ThemeCssUrlOutput::Bundle,
    )?;
    let print_css = rewrite_theme_css_urls(
        &manifest,
        &manifest.print_stylesheet,
        &raw_print_css,
        ThemeCssUrlOutput::Bundle,
    )?;
    let static_screen_css = rewrite_theme_css_urls(
        &manifest,
        &manifest.stylesheet,
        &raw_screen_css,
        ThemeCssUrlOutput::Static,
    )?;
    let static_print_css = rewrite_theme_css_urls(
        &manifest,
        &manifest.print_stylesheet,
        &raw_print_css,
        ThemeCssUrlOutput::Static,
    )?;
    Ok(RenderedTheme {
        manifest,
        params,
        screen_css,
        print_css,
        static_screen_css,
        static_print_css,
    })
}

pub fn load_theme_manifest(path: &Path) -> Result<ThemeManifest, ThemeError> {
    let text = fs::read_to_string(path).map_err(|source| ThemeError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw: RawThemeManifest = toml::from_str(&text).map_err(|source| {
        let location = toml_location(&text, &source);
        ThemeError::Toml {
            path: path.to_path_buf(),
            location,
            source: Box::new(source),
        }
    })?;
    let api_version_span = raw.theme.api_version.span();
    let api_version = *raw.theme.api_version.get_ref();
    if ThemeApiVersion::from_u32(api_version).is_none() {
        let (line, column) = line_column_for_offset(&text, api_version_span.start);
        return Err(ThemeError::UnsupportedApiVersion {
            path: path.to_path_buf(),
            line,
            column,
            found: api_version,
        });
    }
    validate_theme_name(&raw.theme.name)?;
    validate_manifest_relative_path(&raw.theme.name, &raw.theme.stylesheet)?;
    validate_manifest_relative_path(&raw.theme.name, &raw.theme.print_stylesheet)?;
    validate_theme_dependency_declarations(&raw.theme.name, &raw.theme.fonts, &raw.theme.assets)?;
    for inspiration in &raw.style.inspiration {
        validate_manifest_relative_path(&raw.theme.name, inspiration)?;
    }
    let output_targets = normalize_output_targets(&raw.theme.name, raw.theme.output_targets)?;
    let modules = normalize_theme_modules(&raw.theme.name, raw.theme.modules)?;
    validate_slide_variants(&raw.theme.name, &raw.theme.slide_variants)?;
    let feature_hooks = normalize_feature_hooks(&raw.theme.name, raw.theme.feature_hooks)?;

    let mut parameters = BTreeMap::new();
    for (name, raw_parameter) in raw.parameters {
        let default = raw_parameter
            .default
            .as_ref()
            .map(theme_value_to_string)
            .transpose()
            .map_err(|reason| ThemeError::InvalidParameter {
                name: name.clone(),
                value: "<default>".to_string(),
                reason,
            })?;
        let parameter = ThemeParameter {
            parameter_type: raw_parameter.parameter_type,
            default,
            required: raw_parameter.required,
            values: raw_parameter.values,
            min: raw_parameter.min,
            max: raw_parameter.max,
        };
        validate_parameter_bounds(&parameter).map_err(|reason| ThemeError::InvalidParameter {
            name: name.clone(),
            value: "<bounds>".to_string(),
            reason,
        })?;
        if let Some(default) = &parameter.default {
            validate_parameter_value(&name, default, &parameter)?;
        }
        parameters.insert(name, parameter);
    }

    let slide_presets = normalize_slide_presets(
        &raw.theme.name,
        &raw.theme.slide_variants,
        &parameters,
        raw.presets,
    )?;

    let color_variants = raw
        .color_variants
        .into_iter()
        .map(|(variant, colors)| {
            for slot in STANDARD_COLOR_SLOTS {
                if !colors.contains_key(*slot) {
                    return Err(ThemeError::MissingColorSlot {
                        theme: raw.theme.name.clone(),
                        variant: variant.clone(),
                        slot: slot.to_string(),
                    });
                }
            }
            for (name, value) in &colors {
                if !is_hex_color(value) {
                    return Err(ThemeError::InvalidColorVariant {
                        theme: raw.theme.name.clone(),
                        variant: variant.clone(),
                        name: name.clone(),
                        value: value.clone(),
                    });
                }
                match parameters
                    .get(name)
                    .map(|parameter| parameter.parameter_type)
                {
                    Some(ThemeParameterType::Color) => {}
                    _ => {
                        return Err(ThemeError::UndeclaredColorSlot {
                            theme: raw.theme.name.clone(),
                            variant: variant.clone(),
                            name: name.clone(),
                        });
                    }
                }
            }
            Ok((variant, ThemeColorVariant { colors }))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    Ok(ThemeManifest {
        path: path.to_path_buf(),
        name: raw.theme.name,
        version: raw.theme.version,
        api_version,
        style: ThemeStyle {
            family: raw.style.family,
            inspiration: raw.style.inspiration,
        },
        fonts: raw.theme.fonts,
        assets: raw.theme.assets,
        stylesheet: raw.theme.stylesheet,
        print_stylesheet: raw.theme.print_stylesheet,
        palette_parameter: raw.theme.palette_parameter,
        output_targets,
        modules,
        slide_variants: raw.theme.slide_variants,
        feature_hooks,
        slide_presets,
        color_variants,
        parameters,
    })
}

pub fn prepare_deck_for_theme(
    deck: &mut deck::Deck,
    manifest: &ThemeManifest,
) -> Result<(), ThemeError> {
    resolve_theme_slide_presets(deck, manifest)?;
    let diagnostics = theme_deck_contract_diagnostics(deck, manifest);
    deck.diagnostics.extend(diagnostics);
    Ok(())
}

pub fn validate_deck_for_theme_contract(
    deck: &deck::Deck,
    manifest: &ThemeManifest,
) -> Result<(), ThemeDeckContractError> {
    let diagnostics = theme_deck_contract_diagnostics(deck, manifest)
        .into_iter()
        .filter(deck::Diagnostic::is_fatal)
        .collect::<Vec<_>>();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(ThemeDeckContractError {
            theme_name: manifest.name.clone(),
            api_version: manifest.api_version,
            diagnostics,
        })
    }
}

fn theme_deck_contract_diagnostics(
    deck: &deck::Deck,
    manifest: &ThemeManifest,
) -> Vec<deck::Diagnostic> {
    let source_path = deck
        .source_path
        .as_deref()
        .map_or_else(|| "<source>".to_string(), |path| path.display().to_string());
    let mut diagnostics = Vec::new();
    diagnostics.extend(deck.metadata.theme_api_v1_diagnostics.iter().cloned());

    if let Some(background) = deck.metadata.background_image.as_ref() {
        diagnostics.extend(v1_background_semantic_diagnostics(
            background,
            "Deck background",
        ));
    }
    for slide in deck.pdf_slide_order() {
        if let Some(background) = slide.background_image.as_ref() {
            diagnostics.extend(v1_background_semantic_diagnostics(
                background,
                &format!("Slide '{}' background", slide.id),
            ));
        }
    }

    if let Some(aspect) = deck.metadata.aspect.as_deref()
        && !is_sixteen_by_nine(aspect)
    {
        diagnostics.push(deck::Diagnostic::error(
            None,
            format!(
                "Source file '{source_path}' declares aspect '{aspect}' for Theme API v1 theme '{}'; Theme API v1 supports only 16:9 (equivalent ratios are accepted)",
                manifest.name
            ),
        ));
    }

    let plan = crate::presentation_plan::PresentationPlan::for_theme_api_v1(deck);
    if !diagnostics.iter().any(deck::Diagnostic::is_fatal)
        && plan.deck_background_is_dormant()
        && let Some(background) = deck.metadata.background_image.as_ref()
    {
        diagnostics.push(deck::Diagnostic::warning(
            background.source_span.clone(),
            "Theme API v1 keeps this Deck background as a validated dependency, but does not paint it: the title is clean, no Content slide inherits it, and no explicit Splash is enabled",
        ));
    }

    for section in &deck.sections {
        diagnostics.extend(v1_block_alternative_diagnostics(
            &section.main_slide.blocks,
            &section.main_slide.id,
        ));
        for slide in &section.detail_slides {
            diagnostics.extend(v1_block_alternative_diagnostics(&slide.blocks, &slide.id));
        }
        if section.main_slide.variant == Some(deck::SlideVariant::Dense) {
            diagnostics.push(deck::Diagnostic::warning(
                None,
                format!(
                    "Main slide '{}' uses the Dense variant; Dense is intended for compact Detail evidence, so confirm that this content belongs on the Main path and remains at the Technical type role or larger",
                    section.main_slide.title.as_deref().unwrap_or(&section.main_slide.id)
                ),
            ));
        }
    }

    diagnostics
}

fn v1_block_alternative_diagnostics(
    blocks: &[deck::ContentBlock],
    slide_id: &str,
) -> Vec<deck::Diagnostic> {
    let mut diagnostics = Vec::new();
    for block in blocks {
        match block {
            deck::ContentBlock::Figure { alt, caption, .. } => {
                diagnostics.extend(v1_figure_alternative_diagnostics(
                    "Figure",
                    slide_id,
                    alt,
                    caption.as_deref(),
                ));
            }
            deck::ContentBlock::Gallery { items, .. } => {
                for (index, item) in items.iter().enumerate() {
                    diagnostics.extend(v1_figure_alternative_diagnostics(
                        &format!("Gallery item {}", index + 1),
                        slide_id,
                        &item.alt,
                        item.caption.as_deref(),
                    ));
                }
            }
            deck::ContentBlock::Media {
                kind,
                alt,
                title,
                caption,
                visual_hidden,
                ..
            } => {
                let label = media_kind_name(*kind);
                let static_description = caption.as_deref().or(title.as_deref());
                if *visual_hidden {
                    if !alt.trim().is_empty()
                        || static_description.is_some_and(|text| !text.trim().is_empty())
                    {
                        diagnostics.push(deck::Diagnostic::error(
                            None,
                            format!(
                                "Slide '{slide_id}' hides meaningful {label} media, so its information disappears from static Output targets; provide a visible coherent fallback or make the media genuinely decorative"
                            ),
                        ));
                    }
                } else {
                    diagnostics.extend(v1_meaningful_image_diagnostics(
                        &format!("{} media", label),
                        slide_id,
                        alt,
                        static_description,
                    ));
                }
            }
            deck::ContentBlock::Layout { regions, .. } => {
                for region in regions {
                    diagnostics.extend(v1_block_alternative_diagnostics(&region.blocks, slide_id));
                }
            }
            _ => {}
        }
    }
    diagnostics
}

fn v1_figure_alternative_diagnostics(
    label: &str,
    slide_id: &str,
    alt: &str,
    caption: Option<&str>,
) -> Vec<deck::Diagnostic> {
    let alt = alt.trim();
    let caption = caption.map(str::trim);
    if alt.is_empty() {
        return if caption.is_some_and(|caption| !caption.is_empty()) {
            vec![deck::Diagnostic::error(
                None,
                format!(
                    "{label} on Slide '{slide_id}' has caption text but an empty alt; provide a short alternative or remove the caption when the image is decorative"
                ),
            )]
        } else {
            Vec::new()
        };
    }
    if alt.contains(['\n', '\r']) || alt.chars().count() > 160 {
        return vec![deck::Diagnostic::error(
            None,
            format!(
                "{label} on Slide '{slide_id}' needs a single short alt of at most 160 characters"
            ),
        )];
    }
    Vec::new()
}

fn v1_meaningful_image_diagnostics(
    label: &str,
    slide_id: &str,
    alt: &str,
    longer_description: Option<&str>,
) -> Vec<deck::Diagnostic> {
    let alt = alt.trim();
    let longer_description = longer_description.map(str::trim);
    if alt.is_empty() {
        return if longer_description.is_some_and(|description| !description.is_empty()) {
            vec![deck::Diagnostic::error(
                None,
                format!(
                    "{label} on Slide '{slide_id}' has descriptive text but an empty alt; provide a short alternative or remove the description when the image is decorative"
                ),
            )]
        } else {
            Vec::new()
        };
    }
    let mut diagnostics = Vec::new();
    if alt.contains(['\n', '\r']) || alt.chars().count() > 160 {
        diagnostics.push(deck::Diagnostic::error(
            None,
            format!(
                "{label} on Slide '{slide_id}' needs a single short alt of at most 160 characters"
            ),
        ));
    }
    if longer_description.is_none_or(str::is_empty) {
        diagnostics.push(deck::Diagnostic::error(
            None,
            format!(
                "{label} on Slide '{slide_id}' needs an adjacent caption or description route for static and assistive reading"
            ),
        ));
    }
    diagnostics
}

fn v1_background_semantic_diagnostics(
    background: &deck::DeckBackgroundImage,
    label: &str,
) -> Vec<deck::Diagnostic> {
    let mut diagnostics = Vec::new();
    if !background.intent_explicit {
        diagnostics.push(deck::Diagnostic::error(
            background.source_span.clone(),
            format!(
                "{label} requires explicit Theme API v1 intent: decorative, contextual, or evidence"
            ),
        ));
        return diagnostics;
    }

    let alt = background.alt.trim();
    let description = background.description.as_deref().map(str::trim);
    match background.intent {
        deck::BackgroundImageIntent::Decorative => {
            if !alt.is_empty() || description.is_some_and(|value| !value.is_empty()) {
                diagnostics.push(deck::Diagnostic::error(
                    background.source_span.clone(),
                    format!(
                        "{label} is decorative and must not provide alt or description text; change intent if the image carries information"
                    ),
                ));
            }
        }
        deck::BackgroundImageIntent::Contextual | deck::BackgroundImageIntent::Evidence => {
            if alt.is_empty() {
                diagnostics.push(deck::Diagnostic::error(
                    background.source_span.clone(),
                    format!("{label} with meaningful intent requires a non-empty short alt"),
                ));
            } else if alt.contains(['\n', '\r']) || alt.chars().count() > 160 {
                diagnostics.push(deck::Diagnostic::error(
                    background.source_span.clone(),
                    format!(
                        "{label} alt must be a single short description of at most 160 characters; move detail into description"
                    ),
                ));
            }
            if background.intent == deck::BackgroundImageIntent::Evidence
                && description.is_none_or(str::is_empty)
            {
                diagnostics.push(deck::Diagnostic::error(
                    background.source_span.clone(),
                    format!(
                        "{label} with evidence intent requires description text that explains the evidence"
                    ),
                ));
            }
        }
    }
    diagnostics
}

fn is_sixteen_by_nine(value: &str) -> bool {
    let Some((width, height)) = value.split_once([':', 'x', 'X']) else {
        return false;
    };
    let Ok(width) = width.trim().parse::<f64>() else {
        return false;
    };
    let Ok(height) = height.trim().parse::<f64>() else {
        return false;
    };
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return false;
    }

    let scaled_width = width * 9.0;
    let scaled_height = height * 16.0;
    let scale = scaled_width.abs().max(scaled_height.abs()).max(1.0);
    (scaled_width - scaled_height).abs() <= f64::EPSILON * 16.0 * scale
}

pub fn resolve_theme_slide_presets(
    deck: &mut deck::Deck,
    manifest: &ThemeManifest,
) -> Result<(), ThemeError> {
    for section in &mut deck.sections {
        resolve_slide_preset(&mut section.main_slide, manifest)?;
        for slide in &mut section.detail_slides {
            resolve_slide_preset(slide, manifest)?;
        }
    }
    Ok(())
}

fn resolve_slide_preset(
    slide: &mut deck::Slide,
    manifest: &ThemeManifest,
) -> Result<(), ThemeError> {
    let Some(preset_name) = slide.preset.as_deref() else {
        return Ok(());
    };
    let Some(preset) = manifest.slide_presets.get(preset_name) else {
        return Err(ThemeError::UnknownSlidePreset {
            theme: manifest.name.clone(),
            slide_id: slide.id.clone(),
            preset: preset_name.to_string(),
        });
    };
    if slide.variant.is_none() {
        slide.variant = preset.variant;
    }
    for class in &preset.classes {
        if !slide.classes.iter().any(|existing| existing == class) {
            slide.classes.push(class.clone());
        }
    }
    if !preset.theme_params.is_empty() {
        let explicit = slide.theme_params.clone();
        slide.theme_params = preset.theme_params.clone();
        slide.theme_params.extend(explicit);
    }
    if slide.autoscale.is_none() {
        slide.autoscale = preset.autoscale;
    }
    if slide.transition.is_none() {
        slide.transition = preset.transition;
    }
    Ok(())
}

pub fn validate_theme_params(
    manifest: &ThemeManifest,
    supplied: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ThemeError> {
    let mut resolved = BTreeMap::new();
    for (name, parameter) in &manifest.parameters {
        if let Some(default) = &parameter.default {
            resolved.insert(name.clone(), default.clone());
        }
    }

    for (name, value) in supplied {
        let Some(parameter) = manifest.parameters.get(name) else {
            return Err(ThemeError::UnknownParameter {
                theme: manifest.name.clone(),
                name: name.clone(),
            });
        };
        validate_parameter_value(name, value, parameter)?;
    }

    let variant_name = supplied
        .get(&manifest.palette_parameter)
        .or_else(|| resolved.get(&manifest.palette_parameter))
        .cloned();
    if let Some(variant_name) = variant_name
        && let Some(variant) = manifest.color_variants.get(&variant_name)
    {
        for (name, value) in &variant.colors {
            if manifest.parameters.contains_key(name) {
                resolved.insert(name.clone(), value.clone());
            }
        }
    }

    for (name, value) in supplied {
        resolved.insert(name.clone(), value.clone());
    }

    for (name, parameter) in &manifest.parameters {
        if parameter.required && !resolved.contains_key(name) {
            return Err(ThemeError::MissingParameter {
                theme: manifest.name.clone(),
                name: name.clone(),
            });
        }
    }

    Ok(resolved)
}

pub fn validate_theme_params_best_effort(
    manifest: &ThemeManifest,
    supplied: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    validate_theme_params_best_effort_with_warnings(manifest, supplied).0
}

pub fn validate_theme_params_best_effort_with_warnings(
    manifest: &ThemeManifest,
    supplied: &BTreeMap<String, String>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut warnings = Vec::new();
    let known = supplied
        .iter()
        .filter_map(|(name, value)| {
            let Some(parameter) = manifest.parameters.get(name) else {
                warnings.push(
                    ThemeError::UnknownParameter {
                        theme: manifest.name.clone(),
                        name: name.clone(),
                    }
                    .to_string(),
                );
                return None;
            };
            match validate_parameter_value(name, value, parameter) {
                Ok(()) => Some((name.clone(), value.clone())),
                Err(error) => {
                    warnings.push(error.to_string());
                    None
                }
            }
        })
        .collect();
    match validate_theme_params(manifest, &known) {
        Ok(resolved) => (resolved, warnings),
        Err(error) => {
            warnings.push(error.to_string());
            (known, warnings)
        }
    }
}

fn theme_candidates(name: &str, deck_root: &Path, theme_search_paths: &[PathBuf]) -> Vec<PathBuf> {
    // Configuration accumulates search paths from lowest to highest precedence.
    // Resolve them in reverse so CLI paths can override front matter and project
    // paths when two packages intentionally use the same Theme name.
    let mut candidates = Vec::new();
    for search_path in theme_search_paths.iter().rev() {
        candidates.push(search_path.join(name).join("theme.toml"));
        candidates.push(search_path.join("theme.toml"));
    }
    candidates.push(deck_root.join("themes").join(name).join("theme.toml"));
    candidates.push(deck_root.join(name).join("theme.toml"));
    let builtin_theme_path = builtin_theme_search_path();
    if !theme_search_paths
        .iter()
        .any(|path| path == &builtin_theme_path)
    {
        candidates.push(builtin_theme_path.join(name).join("theme.toml"));
    }
    candidates
}

pub fn builtin_theme_search_path() -> PathBuf {
    BUILTIN_THEME_ROOT
        .get_or_init(materialize_builtin_themes)
        .clone()
}

fn materialize_builtin_themes() -> PathBuf {
    let mut digest = Sha256::new();
    for (relative, bytes) in BUILTIN_THEME_FILES {
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(bytes);
    }
    let digest = format!("{:x}", digest.finalize());
    let cache_name = format!("{}-{}", env!("CARGO_PKG_VERSION"), &digest[..16]);
    let temporary_base = std::env::temp_dir().join("zpres").join("themes");
    let cache_base = dirs::cache_dir()
        .map(|base| base.join("zpres").join("themes"))
        .unwrap_or_else(|| temporary_base.clone());

    materialize_builtin_themes_with_fallback(&cache_base, &temporary_base, &cache_name)
        // Preserve the existing infallible lookup interface. If neither location
        // is writable, lookup reports a missing Theme at the temporary location.
        .unwrap_or_else(|_| temporary_base.join(cache_name))
}

fn materialize_builtin_themes_with_fallback(
    cache_base: &Path,
    temporary_base: &Path,
    cache_name: &str,
) -> io::Result<PathBuf> {
    materialize_builtin_themes_in(cache_base, cache_name)
        .or_else(|_| materialize_builtin_themes_in(temporary_base, cache_name))
}

fn materialize_builtin_themes_in(base: &Path, cache_name: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(base)?;
    // Existing entries are never edited or removed. The second stable name
    // makes a repaired cache reusable without trusting ownership of the first.
    for suffix in ["", "-recovered"] {
        let root = base.join(format!("{cache_name}{suffix}"));
        if builtin_theme_tree_matches(&root) {
            return Ok(root);
        }
        match create_builtin_theme_tree(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    // Bound recovery work even if both stable entries are damaged or occupied.
    // Exclusive creation also keeps simultaneous processes from sharing writes.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    // Concurrent threads can observe the same clock tick.
    static RECOVERY_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = RECOVERY_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = base.join(format!(
        "{cache_name}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    create_builtin_theme_tree(&root)?;
    Ok(root)
}

fn create_builtin_theme_tree(root: &Path) -> io::Result<()> {
    fs::create_dir(root)?;
    if let Err(error) = write_builtin_theme_tree(root) {
        // Only a directory exclusively created by this call may be removed.
        let _ = fs::remove_dir_all(root);
        return Err(error);
    }
    Ok(())
}

fn write_builtin_theme_tree(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    for (relative, bytes) in BUILTIN_THEME_FILES {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn builtin_theme_tree_matches(root: &Path) -> bool {
    BUILTIN_THEME_FILES.iter().all(|(relative, expected)| {
        fs::read(root.join(relative)).is_ok_and(|bytes| bytes == *expected)
    })
}

fn render_theme_template(
    manifest: &ThemeManifest,
    relative_path: &str,
    params: &BTreeMap<String, String>,
) -> Result<String, ThemeError> {
    let path = theme_relative_path(manifest, relative_path)?;
    let resolved = fs::canonicalize(&path).map_err(|source| ThemeError::ReadTemplate {
        path: path.clone(),
        source,
    })?;
    let theme_root = manifest.path.parent().unwrap_or_else(|| Path::new("."));
    if !resolved.starts_with(theme_root) {
        return Err(ThemeError::StylesheetOutsideThemeRoot {
            theme: manifest.name.clone(),
            stylesheet: relative_path.to_string(),
            root: theme_root.to_path_buf(),
            resolved,
        });
    }
    let template = fs::read_to_string(&resolved).map_err(|source| ThemeError::ReadTemplate {
        path: resolved.clone(),
        source,
    })?;
    render_template_text(manifest, &resolved, &template, params)
}

fn render_theme_css(
    manifest: &ThemeManifest,
    relative_path: &str,
    params: &BTreeMap<String, String>,
) -> Result<String, ThemeError> {
    let css = render_theme_template(manifest, relative_path, params)?;
    let variables = render_theme_variable_block(manifest, params);
    Ok(render_v1_theme_css(&css, &variables, "zpres-theme"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeCssUrlOutput {
    Bundle,
    Static,
}

#[derive(Debug, Clone, Copy)]
struct ParsedThemeCssUrl<'a> {
    value: &'a str,
    end: usize,
    has_escape: bool,
}

fn rewrite_theme_css_urls(
    manifest: &ThemeManifest,
    stylesheet: &str,
    css: &str,
    output: ThemeCssUrlOutput,
) -> Result<String, ThemeError> {
    let bytes = css.as_bytes();
    let mut rendered = String::with_capacity(css.len());
    let mut copy_from = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            index = skip_css_comment(bytes, index + 2);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"') {
            index = skip_css_string(bytes, index).map_err(|reason| {
                invalid_theme_css_url(
                    manifest,
                    stylesheet,
                    css,
                    index,
                    css_url_excerpt(css, index),
                    reason,
                )
            })?;
            continue;
        }
        if is_css_import_start(bytes, index) {
            let (line, column) = line_column_for_offset(css, index);
            return Err(ThemeError::UnsupportedCssImport {
                theme: manifest.name.clone(),
                stylesheet: stylesheet.to_string(),
                line,
                column,
            });
        }
        if let Some(function) = css_unsupported_image_function(bytes, index) {
            let (line, column) = line_column_for_offset(css, index);
            return Err(ThemeError::UnsupportedCssImageFunction {
                theme: manifest.name.clone(),
                stylesheet: stylesheet.to_string(),
                line,
                column,
                function,
            });
        }
        if bytes[index] == b'\\' {
            return Err(invalid_theme_css_url(
                manifest,
                stylesheet,
                css,
                index,
                css_url_excerpt(css, index),
                "CSS identifier escapes are unsupported because they can obscure url() references",
            ));
        }
        if !is_css_url_function_start(bytes, index) {
            index += 1;
            continue;
        }

        let parsed = parse_theme_css_url(css, index).map_err(|reason| {
            invalid_theme_css_url(
                manifest,
                stylesheet,
                css,
                index,
                css_url_excerpt(css, index),
                reason,
            )
        })?;
        if let Some(replacement) =
            rewrite_theme_css_url_value(manifest, parsed.value, parsed.has_escape, output).map_err(
                |reason| {
                    invalid_theme_css_url(
                        manifest,
                        stylesheet,
                        css,
                        index,
                        parsed.value.to_string(),
                        reason,
                    )
                },
            )?
        {
            rendered.push_str(&css[copy_from..index]);
            rendered.push_str(&replacement);
            copy_from = parsed.end;
        }
        index = parsed.end;
    }

    rendered.push_str(&css[copy_from..]);
    Ok(rendered)
}

fn is_css_url_function_start(bytes: &[u8], index: usize) -> bool {
    index + 4 <= bytes.len()
        && bytes[index..index + 3].eq_ignore_ascii_case(b"url")
        && bytes[index + 3] == b'('
        && (index == 0 || !is_css_identifier_byte(bytes[index - 1]))
}

fn is_css_import_start(bytes: &[u8], index: usize) -> bool {
    index + 7 <= bytes.len()
        && bytes[index] == b'@'
        && bytes[index + 1..index + 7].eq_ignore_ascii_case(b"import")
        && bytes
            .get(index + 7)
            .is_none_or(|byte| !is_css_identifier_byte(*byte))
}

fn css_unsupported_image_function(bytes: &[u8], index: usize) -> Option<&'static str> {
    if index > 0 && is_css_identifier_byte(bytes[index - 1]) {
        return None;
    }
    for (spelling, name) in [
        (b"image-set(".as_slice(), "image-set"),
        (b"-webkit-image-set(".as_slice(), "-webkit-image-set"),
        (b"image(".as_slice(), "image"),
    ] {
        if index + spelling.len() <= bytes.len()
            && bytes[index..index + spelling.len()].eq_ignore_ascii_case(spelling)
        {
            return Some(name);
        }
    }
    None
}

fn is_css_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80
}

fn skip_css_comment(bytes: &[u8], mut index: usize) -> usize {
    while index + 1 < bytes.len() {
        if bytes[index..].starts_with(b"*/") {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_css_string(bytes: &[u8], start: usize) -> Result<usize, String> {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 1 < bytes.len() => {
                index += if bytes[index + 1] == b'\r' && bytes.get(index + 2) == Some(&b'\n') {
                    3
                } else {
                    2
                };
            }
            b'\n' | b'\r' | 0x0C => {
                return Err("CSS strings cannot contain unescaped line breaks".to_string());
            }
            byte if byte == quote => return Ok(index + 1),
            _ => index += 1,
        }
    }
    Err("unterminated CSS string".to_string())
}

fn parse_theme_css_url(css: &str, start: usize) -> Result<ParsedThemeCssUrl<'_>, String> {
    let bytes = css.as_bytes();
    let mut index = start + 4;
    skip_css_whitespace(bytes, &mut index);
    if index >= bytes.len() {
        return Err("unterminated url() function".to_string());
    }

    if matches!(bytes[index], b'\'' | b'"') {
        let quote = bytes[index];
        let value_start = index + 1;
        index = value_start;
        let mut has_escape = false;
        loop {
            if index >= bytes.len() {
                return Err("unterminated quoted url() value".to_string());
            }
            match bytes[index] {
                b'\n' | b'\r' | 0x0C => {
                    return Err("quoted url() values cannot contain raw line breaks".to_string());
                }
                b'\\' => {
                    has_escape = true;
                    index += 1;
                    if index >= bytes.len() {
                        return Err("unterminated CSS escape in url() value".to_string());
                    }
                    index += 1;
                }
                byte if byte == quote => break,
                _ => index += 1,
            }
        }
        let value = &css[value_start..index];
        index += 1;
        skip_css_whitespace(bytes, &mut index);
        if bytes.get(index) != Some(&b')') {
            return Err("quoted url() value must be followed by ')'".to_string());
        }
        return Ok(ParsedThemeCssUrl {
            value,
            end: index + 1,
            has_escape,
        });
    }

    let value_start = index;
    let mut value_end = index;
    let mut has_escape = false;
    while index < bytes.len() {
        match bytes[index] {
            b')' => {
                return Ok(ParsedThemeCssUrl {
                    value: &css[value_start..value_end],
                    end: index + 1,
                    has_escape,
                });
            }
            byte if is_css_whitespace(byte) => {
                value_end = index;
                skip_css_whitespace(bytes, &mut index);
                if bytes.get(index) != Some(&b')') {
                    return Err(
                        "unquoted url() values cannot contain embedded whitespace".to_string()
                    );
                }
            }
            b'\'' | b'"' | b'(' => {
                return Err("unquoted url() value contains unsupported punctuation".to_string());
            }
            b'\\' => {
                has_escape = true;
                index += 1;
                if index >= bytes.len() {
                    return Err("unterminated CSS escape in url() value".to_string());
                }
                index += 1;
                value_end = index;
            }
            byte if byte < 0x20 => {
                return Err("url() value contains an unsupported control character".to_string());
            }
            _ => {
                index += 1;
                value_end = index;
            }
        }
    }
    Err("unterminated url() function".to_string())
}

fn skip_css_whitespace(bytes: &[u8], index: &mut usize) {
    while bytes
        .get(*index)
        .is_some_and(|byte| is_css_whitespace(*byte))
    {
        *index += 1;
    }
}

fn is_css_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

fn rewrite_theme_css_url_value(
    manifest: &ThemeManifest,
    value: &str,
    has_escape: bool,
    output: ThemeCssUrlOutput,
) -> Result<Option<String>, String> {
    if value.is_empty() {
        return Err("url() value is empty".to_string());
    }
    if value.starts_with('#')
        || value
            .get(..5)
            .is_some_and(|head| head.eq_ignore_ascii_case("data:"))
    {
        return Ok(None);
    }
    if has_escape {
        return Err("CSS escapes in local url() values are unsupported; percent-encode the URL path instead".to_string());
    }

    let (encoded_path, suffix) = split_url_path_suffix(value);
    if encoded_path.is_empty() {
        return Err("local url() path is empty".to_string());
    }
    let dependency = percent_decode_url_path(encoded_path)?;
    if dependency.contains('\\') {
        return Err("local url() paths must use '/' separators, not backslashes".to_string());
    }
    if let Some(scheme) = url_scheme(&dependency) {
        return Err(format!(
            "URL scheme '{scheme}' is not allowed; Themes must use declared local dependencies, data: URLs, or fragment references"
        ));
    }
    let dependency_path = Path::new(&dependency);
    if dependency_path.is_absolute() {
        return Err("absolute local URL paths are not allowed".to_string());
    }
    for component in dependency_path.components() {
        match component {
            Component::ParentDir => {
                return Err("parent traversal ('..') is not allowed in Theme CSS URLs".to_string());
            }
            Component::Normal(_) => {}
            _ => {
                return Err(
                    "Theme CSS URLs must be safe manifest-relative dependency paths".to_string(),
                );
            }
        }
    }
    if !theme_dependency_paths(manifest).any(|declared| declared == dependency) {
        return Err(format!(
            "'{dependency}' is not declared in the Theme manifest fonts or assets inventory"
        ));
    }

    let encoded_dependency = percent_encode_url_path(&dependency);
    let encoded_suffix = normalize_url_suffix(suffix)?;
    let resolved = match output {
        ThemeCssUrlOutput::Bundle => encoded_dependency,
        ThemeCssUrlOutput::Static => {
            let theme_root = manifest.path.parent().unwrap_or_else(|| Path::new("."));
            file_url(&theme_root.join(&dependency))
        }
    };
    Ok(Some(format!("url(\"{resolved}{encoded_suffix}\")")))
}

fn split_url_path_suffix(value: &str) -> (&str, &str) {
    value
        .char_indices()
        .find(|(_, character)| matches!(character, '?' | '#'))
        .map_or((value, ""), |(index, _)| value.split_at(index))
}

fn percent_decode_url_path(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err("URL path contains an incomplete percent escape".to_string());
        }
        let Some(high) = hex_value(bytes[index + 1]) else {
            return Err("URL path contains an invalid percent escape".to_string());
        };
        let Some(low) = hex_value(bytes[index + 2]) else {
            return Err("URL path contains an invalid percent escape".to_string());
        };
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded)
        .map_err(|_| "percent-decoded URL path is not valid UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn url_scheme(value: &str) -> Option<&str> {
    let colon = value.find(':')?;
    let candidate = &value[..colon];
    let mut characters = candidate.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
    {
        return None;
    }
    Some(candidate)
}

fn percent_encode_url_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn normalize_url_suffix(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut encoded = String::with_capacity(value.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            return Err("URL query and fragment suffixes cannot contain backslashes".to_string());
        }
        if byte == b'%' {
            if index + 2 >= bytes.len()
                || hex_value(bytes[index + 1]).is_none()
                || hex_value(bytes[index + 2]).is_none()
            {
                return Err("URL query or fragment contains an invalid percent escape".to_string());
            }
            encoded.push('%');
            encoded.push((bytes[index + 1] as char).to_ascii_uppercase());
            encoded.push((bytes[index + 2] as char).to_ascii_uppercase());
            index += 3;
            continue;
        }
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'?' | b'#'
                    | b'/'
                    | b':'
                    | b'@'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'~'
            )
        {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
        index += 1;
    }
    Ok(encoded)
}

fn invalid_theme_css_url(
    manifest: &ThemeManifest,
    stylesheet: &str,
    css: &str,
    offset: usize,
    url: String,
    reason: impl Into<String>,
) -> ThemeError {
    let (line, column) = line_column_for_offset(css, offset);
    ThemeError::InvalidCssUrl {
        theme: manifest.name.clone(),
        stylesheet: stylesheet.to_string(),
        line,
        column,
        url: url.into_boxed_str(),
        reason: reason.into().into_boxed_str(),
    }
}

fn css_url_excerpt(css: &str, offset: usize) -> String {
    let mut excerpt = css[offset..].chars().take(80).collect::<String>();
    if css[offset..].chars().count() > 80 {
        excerpt.push('…');
    }
    excerpt.replace(['\n', '\r'], " ")
}

fn render_v1_theme_css(css: &str, variables: &str, layer: &str) -> String {
    let mut rendered = format!("@layer {layer} {{\n");
    rendered.push_str(css);
    if !css.ends_with('\n') {
        rendered.push('\n');
    }
    if !variables.is_empty() {
        rendered.push('\n');
        rendered.push_str(variables);
    }
    rendered.push_str("}\n");
    rendered
}

fn render_theme_variable_block(
    manifest: &ThemeManifest,
    params: &BTreeMap<String, String>,
) -> String {
    if params.is_empty() {
        return String::new();
    }
    let mut css = format!(
        ":where(.zpres-api-v1.zpres-theme-{}, .zpres-api-v1 .{}-theme) {{\n",
        manifest.name, manifest.name
    );
    for declaration in theme_param_css_declarations(manifest, params) {
        css.push_str(&format!("  {}: {};\n", declaration.name, declaration.value));
    }
    css.push_str("}\n");
    css
}

pub fn theme_param_css_declarations(
    manifest: &ThemeManifest,
    params: &BTreeMap<String, String>,
) -> Vec<ThemeCssDeclaration> {
    let mut declarations = Vec::new();
    for (name, value) in params {
        let Some(parameter) = manifest.parameters.get(name) else {
            continue;
        };
        let css_value = template_param_value(value, parameter);
        declarations.push(ThemeCssDeclaration {
            name: format!("--zpres-param-{}", css_variable_suffix(name)),
            value: css_value.clone(),
        });
        if parameter.parameter_type == ThemeParameterType::Color {
            declarations.push(ThemeCssDeclaration {
                name: format!("--zpres-color-{}", css_variable_suffix(name)),
                value: css_value.clone(),
            });
        }
        if parameter.parameter_type == ThemeParameterType::Font {
            declarations.push(ThemeCssDeclaration {
                name: format!("--zpres-font-{}", font_css_variable_suffix(name)),
                value: css_value.clone(),
            });
        }
        if parameter.parameter_type == ThemeParameterType::Size {
            declarations.push(ThemeCssDeclaration {
                name: format!("--zpres-size-{}", size_css_variable_suffix(name)),
                value: css_value.clone(),
            });
        }
    }
    declarations
}

pub fn theme_param_data_name(name: &str) -> String {
    css_variable_suffix(name)
}

fn css_variable_suffix(name: &str) -> String {
    let mut suffix = String::with_capacity(name.len());
    let mut previous_hyphen = false;
    for character in name.chars() {
        let next = if character.is_ascii_alphanumeric() {
            previous_hyphen = false;
            character.to_ascii_lowercase()
        } else if character == '-' || character == '_' {
            if previous_hyphen {
                continue;
            }
            previous_hyphen = true;
            '-'
        } else {
            continue;
        };
        suffix.push(next);
    }
    let suffix = suffix.trim_matches('-').to_string();
    if suffix.is_empty() {
        "value".to_string()
    } else {
        suffix
    }
}

fn font_css_variable_suffix(name: &str) -> String {
    name.strip_prefix("font_")
        .or_else(|| name.strip_prefix("font-"))
        .map(css_variable_suffix)
        .unwrap_or_else(|| css_variable_suffix(name))
}

fn size_css_variable_suffix(name: &str) -> String {
    name.strip_prefix("size_")
        .or_else(|| name.strip_prefix("size-"))
        .map(css_variable_suffix)
        .unwrap_or_else(|| css_variable_suffix(name))
}

fn render_template_text(
    manifest: &ThemeManifest,
    path: &Path,
    template: &str,
    params: &BTreeMap<String, String>,
) -> Result<String, ThemeError> {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else {
            return Err(ThemeError::UnknownTemplatePlaceholder {
                path: path.to_path_buf(),
                placeholder: rest.trim().to_string(),
            });
        };
        let placeholder = rest[..end].trim();
        rendered.push_str(&template_value(manifest, path, placeholder, params)?);
        rest = &rest[end + 2..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

fn template_value(
    manifest: &ThemeManifest,
    path: &Path,
    placeholder: &str,
    params: &BTreeMap<String, String>,
) -> Result<String, ThemeError> {
    if placeholder == "theme.name" {
        return Ok(manifest.name.clone());
    }
    if placeholder == "theme.version" {
        return Ok(manifest.version.clone());
    }
    if let Some(name) = placeholder.strip_prefix("param.") {
        let value =
            params
                .get(name)
                .cloned()
                .ok_or_else(|| ThemeError::MissingTemplateParameter {
                    path: path.to_path_buf(),
                    name: name.to_string(),
                })?;
        let parameter =
            manifest
                .parameters
                .get(name)
                .ok_or_else(|| ThemeError::MissingTemplateParameter {
                    path: path.to_path_buf(),
                    name: name.to_string(),
                })?;
        return Ok(template_param_value(&value, parameter));
    }
    Err(ThemeError::UnknownTemplatePlaceholder {
        path: path.to_path_buf(),
        placeholder: placeholder.to_string(),
    })
}

fn template_param_value(value: &str, parameter: &ThemeParameter) -> String {
    match parameter.parameter_type {
        ThemeParameterType::String => css_string_literal(value),
        ThemeParameterType::Integer
        | ThemeParameterType::Number
        | ThemeParameterType::Boolean
        | ThemeParameterType::Enum
        | ThemeParameterType::Color
        | ThemeParameterType::Font
        | ThemeParameterType::Size => value.to_string(),
    }
}

fn css_string_literal(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r'], " ")
    )
}

fn theme_relative_path(manifest: &ThemeManifest, value: &str) -> Result<PathBuf, ThemeError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ThemeError::UnsafePath {
            theme: manifest.name.clone(),
            path: value.to_string(),
        });
    }
    Ok(manifest
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path))
}

/// Returns the Theme's declared runtime dependencies in stable lexical order.
///
/// Both fonts and other assets are served and bundled under the Theme asset
/// root. Manifests reject duplicate declarations, while the set keeps this
/// inventory deterministic for callers that receive a constructed manifest.
pub fn theme_dependency_paths(manifest: &ThemeManifest) -> impl Iterator<Item = &str> {
    manifest
        .fonts
        .iter()
        .chain(&manifest.assets)
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
}

fn validate_theme_dependency_declarations(
    theme_name: &str,
    fonts: &[String],
    assets: &[String],
) -> Result<(), ThemeError> {
    let mut paths = BTreeSet::new();
    let mut portable_paths = BTreeMap::<String, &str>::new();
    for path in fonts.iter().chain(assets) {
        validate_manifest_relative_path(theme_name, path)?;
        if path.contains('\\') {
            return Err(ThemeError::UnsafePath {
                theme: theme_name.to_string(),
                path: path.clone(),
            });
        }
        if !paths.insert(path.as_str()) {
            return Err(ThemeError::DuplicateDependencyPath {
                theme: theme_name.to_string(),
                path: path.clone(),
            });
        }
        let portable_key = portable_dependency_key(path);
        if let Some(first) = portable_paths.insert(portable_key, path) {
            return Err(ThemeError::PortableDependencyCollision {
                theme: theme_name.to_string(),
                first: first.to_string(),
                second: path.clone(),
            });
        }
    }
    Ok(())
}

fn portable_dependency_key(path: &str) -> String {
    path.split('/')
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("/")
}

fn canonicalize_render_manifest(manifest: &ThemeManifest) -> Result<ThemeManifest, ThemeError> {
    let canonical_path =
        fs::canonicalize(&manifest.path).map_err(|source| ThemeError::ResolveManifest {
            theme: manifest.name.clone(),
            path: manifest.path.clone(),
            source,
        })?;
    let mut canonical = manifest.clone();
    canonical.path = canonical_path;
    Ok(canonical)
}

fn require_unreserved_theme_dependency_paths(manifest: &ThemeManifest) -> Result<(), ThemeError> {
    // The live server routes every renderer asset before Theme dependencies,
    // independent of the active Theme API. Reserve the union so screen,
    // bundled, and static output cannot resolve the same path differently.
    let reserved = [
        "reveal.css",
        "zpres-theme-api-v1.css",
        "theme.css",
        "reveal.js",
    ];
    for dependency in theme_dependency_paths(manifest) {
        let portable_dependency = dependency.to_ascii_lowercase();
        if reserved.iter().any(|reserved| {
            portable_dependency == *reserved
                || portable_dependency
                    .strip_prefix(*reserved)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) {
            return Err(ThemeError::ReservedDependencyPath {
                theme: manifest.name.clone(),
                api_version: manifest.api_version,
                dependency: dependency.to_string(),
                asset_path: format!("assets/{dependency}"),
            });
        }
    }
    Ok(())
}

fn require_theme_dependencies(manifest: &ThemeManifest) -> Result<(), ThemeError> {
    require_unreserved_theme_dependency_paths(manifest)?;
    if manifest.fonts.is_empty() && manifest.assets.is_empty() {
        return Ok(());
    }
    let theme_root = manifest.path.parent().unwrap_or_else(|| Path::new("."));
    let canonical_root =
        fs::canonicalize(theme_root).map_err(|source| ThemeError::ResolveDependency {
            theme: manifest.name.clone(),
            path: theme_root.to_path_buf(),
            source,
        })?;
    for relative_path in theme_dependency_paths(manifest) {
        let path = theme_relative_path(manifest, relative_path)?;
        if !path.is_file() {
            return Err(ThemeError::MissingReferencedFile {
                theme: manifest.name.clone(),
                path,
            });
        }
        let resolved = fs::canonicalize(&path).map_err(|source| ThemeError::ResolveDependency {
            theme: manifest.name.clone(),
            path: path.clone(),
            source,
        })?;
        if !resolved.starts_with(&canonical_root) {
            return Err(ThemeError::DependencyOutsideThemeRoot {
                theme: manifest.name.clone(),
                dependency: relative_path.to_string(),
                root: canonical_root,
                resolved,
            });
        }
    }
    Ok(())
}

fn manifest_referenced_paths(manifest: &ThemeManifest) -> Vec<&str> {
    let mut paths = vec![
        manifest.stylesheet.as_str(),
        manifest.print_stylesheet.as_str(),
    ];
    paths.extend(theme_dependency_paths(manifest));
    paths.extend(manifest.style.inspiration.iter().map(String::as_str));
    paths
}

fn theme_css_warnings(manifest: &ThemeManifest, screen_css: &str, print_css: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    {
        for selector in [
            ".zpres-slide-frame",
            ".zpres-slide-header",
            ".zpres-slide-title",
            ".zpres-slide-body",
            ".zpres-slide-primary",
            ".zpres-slide-sources",
            ".zpres-slide-footer",
        ] {
            if !screen_css.contains(selector) && !print_css.contains(selector) {
                warnings.push(format!(
                    "theme '{}' CSS does not mention semantic selector {selector}",
                    manifest.name
                ));
            }
        }
    }
    if !print_css.contains(".zpres-print-slide") {
        warnings.push(format!(
            "theme '{}' print CSS does not mention semantic selector .zpres-print-slide",
            manifest.name
        ));
    }
    warnings
}

/// Extracts only simple `.zpres-*` class tokens from selector preludes.
///
/// This deliberately is not a general CSS parser. Comments and quoted strings
/// are blanked first, declarations are never treated as selectors, and element,
/// attribute, pseudo, and Theme-owned selectors are ignored. Compound selectors
/// still contribute their individual public class tokens.
fn authored_zpres_selectors(
    manifest: &ThemeManifest,
    screen_css: &str,
    print_css: &str,
) -> BTreeSet<String> {
    [screen_css, print_css]
        .into_iter()
        .flat_map(selector_prelude_zpres_classes)
        .filter(|selector| !is_theme_owned_zpres_selector(manifest, selector))
        .collect()
}

fn selector_prelude_zpres_classes(css: &str) -> BTreeSet<String> {
    let sanitized = css_without_comments_or_strings(css);
    let bytes = sanitized.as_bytes();
    let mut selectors = BTreeSet::new();
    let mut segment_start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'{' => {
                let prelude = sanitized[segment_start..index].trim();
                if !prelude.starts_with('@') {
                    collect_zpres_class_tokens(prelude, &mut selectors);
                }
                segment_start = index + 1;
            }
            b'}' | b';' => segment_start = index + 1,
            _ => {}
        }
    }
    selectors
}

fn css_without_comments_or_strings(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut output = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            output[index] = b' ';
            output[index + 1] = b' ';
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                output[index] = b' ';
                index += 1;
            }
            if index < bytes.len() {
                output[index] = b' ';
                index += 1;
            }
            if index < bytes.len() {
                output[index] = b' ';
                index += 1;
            }
        } else if bytes[index] == b'\'' || bytes[index] == b'"' {
            let quote = bytes[index];
            output[index] = b' ';
            index += 1;
            while index < bytes.len() {
                output[index] = b' ';
                if bytes[index] == b'\\' {
                    index += 1;
                    if index < bytes.len() {
                        output[index] = b' ';
                    }
                } else if bytes[index] == quote {
                    index += 1;
                    break;
                }
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    String::from_utf8(output).expect("blanking ASCII CSS syntax preserves UTF-8")
}

fn collect_zpres_class_tokens(prelude: &str, selectors: &mut BTreeSet<String>) {
    let bytes = prelude.as_bytes();
    let mut index = 0;
    while index + 7 <= bytes.len() {
        if &bytes[index..index + 7] == b".zpres-" {
            let start = index;
            index += 7;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric()
                    || bytes[index] == b'-'
                    || bytes[index] == b'_')
            {
                index += 1;
            }
            selectors.insert(prelude[start..index].to_string());
        } else {
            index += 1;
        }
    }
}

fn is_theme_owned_zpres_selector(manifest: &ThemeManifest, selector: &str) -> bool {
    selector == format!(".zpres-theme-{}", manifest.name)
        || selector.starts_with(".zpres-slide-class-")
}

fn unknown_v1_selectors(_manifest: &ThemeManifest, authored: &BTreeSet<String>) -> Vec<String> {
    let known = stable_v1_theme_selectors()
        .chain(internal_theme_selectors())
        .collect::<BTreeSet<_>>();
    authored
        .iter()
        .filter(|selector| !known.contains(selector.as_str()))
        .cloned()
        .collect()
}

pub fn unknown_authored_theme_selectors(
    manifest: &ThemeManifest,
    screen_css: &str,
    print_css: &str,
) -> Vec<String> {
    unknown_v1_selectors(
        manifest,
        &authored_zpres_selectors(manifest, screen_css, print_css),
    )
}

fn fixture_unmatched_stable_selectors(
    manifest: &ThemeManifest,
    rendered_theme: &RenderedTheme,
    html: &str,
    print_html: &str,
) -> Vec<String> {
    let stable = stable_v1_theme_selectors().collect::<BTreeSet<_>>();
    authored_zpres_selectors(
        manifest,
        &rendered_theme.screen_css,
        &rendered_theme.print_css,
    )
    .into_iter()
    .filter(|selector| stable.contains(selector.as_str()))
    .filter(|selector| {
        let class = selector.trim_start_matches('.');
        !html_has_class(html, class) && !html_has_class(print_html, class)
    })
    .collect()
}

fn html_has_class(html: &str, expected: &str) -> bool {
    for quote in ['"', '\''] {
        let marker = format!("class={quote}");
        let mut remainder = html;
        while let Some(start) = remainder.find(&marker) {
            remainder = &remainder[start + marker.len()..];
            let Some(end) = remainder.find(quote) else {
                break;
            };
            if remainder[..end]
                .split_ascii_whitespace()
                .any(|class| class == expected)
            {
                return true;
            }
            remainder = &remainder[end + quote.len_utf8()..];
        }
    }
    false
}

fn check_theme_fixture(
    manifest: &ThemeManifest,
    rendered_theme: &RenderedTheme,
    fixture: &Path,
    require_complete_feature_coverage: bool,
) -> Result<ThemeFixtureCheckReport, ThemeError> {
    let mut deck =
        deck::parse_source_file(fixture).map_err(|error| ThemeError::FixtureCheckFailed {
            theme: manifest.name.clone(),
            path: fixture.to_path_buf(),
            reason: error.to_string(),
        })?;
    deck.metadata.theme = Some(manifest.name.clone());
    deck.metadata.theme_params = rendered_theme.params.clone();
    prepare_deck_for_theme(&mut deck, manifest).map_err(|error| {
        ThemeError::FixtureCheckFailed {
            theme: manifest.name.clone(),
            path: fixture.to_path_buf(),
            reason: error.to_string(),
        }
    })?;
    let fatal_diagnostics = deck
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_fatal())
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    let fixture_warnings = deck
        .diagnostics
        .iter()
        .filter(|diagnostic| !diagnostic.is_fatal())
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    if !fatal_diagnostics.is_empty() {
        return Err(ThemeError::FixtureCheckFailed {
            theme: manifest.name.clone(),
            path: fixture.to_path_buf(),
            reason: fatal_diagnostics.join("; "),
        });
    }
    let unsupported_variants = unsupported_fixture_variants(manifest, &deck);
    if !unsupported_variants.is_empty() {
        return Err(ThemeError::FixtureCheckFailed {
            theme: manifest.name.clone(),
            path: fixture.to_path_buf(),
            reason: format!(
                "fixture uses slide variant(s) not declared by theme '{}': {}",
                manifest.name,
                unsupported_variants.join(", ")
            ),
        });
    }
    validate_fixture_slide_theme_params(manifest, rendered_theme, fixture, &deck)?;

    let readiness = pdf::check_pdf_readiness_for_theme_with_options(
        &deck,
        rendered_theme,
        html::StaticExportOptions::default(),
    )
    .map_err(|error| ThemeError::FixtureCheckFailed {
        theme: manifest.name.clone(),
        path: fixture.to_path_buf(),
        reason: error.to_string(),
    })?;
    let html = html::render_debug_html(&deck, rendered_theme);
    let print_html = html::render_debug_print_html(&deck, rendered_theme);
    let dead_selectors =
        fixture_unmatched_stable_selectors(manifest, rendered_theme, &html, &print_html);
    pdf::check_print_html_ready(&print_html, readiness.expected_pages).map_err(|error| {
        ThemeError::FixtureCheckFailed {
            theme: manifest.name.clone(),
            path: fixture.to_path_buf(),
            reason: error.to_string(),
        }
    })?;
    let feature_hooks = fixture_feature_hooks(&deck, manifest.api());
    let missing_feature_hooks = missing_declared_feature_hooks(manifest, &feature_hooks);
    let slide_variants = fixture_slide_variants(&deck);
    let missing_slide_variants = missing_declared_slide_variants(manifest, &slide_variants);
    if require_complete_feature_coverage
        && (!missing_feature_hooks.is_empty() || !missing_slide_variants.is_empty())
    {
        let mut missing = Vec::new();
        if !missing_slide_variants.is_empty() {
            missing.push(format!(
                "fixture does not exercise declared slide variants: {}",
                missing_slide_variants.join(", ")
            ));
        }
        if !missing_feature_hooks.is_empty() {
            missing.push(format!(
                "fixture does not exercise declared feature_hooks: {}",
                missing_feature_hooks.join(", ")
            ));
        }
        return Err(ThemeError::FixtureCheckFailed {
            theme: manifest.name.clone(),
            path: fixture.to_path_buf(),
            reason: missing.join("; "),
        });
    }

    Ok(ThemeFixtureCheckReport {
        source_path: fixture.to_path_buf(),
        sections: deck.sections.len(),
        expected_pdf_pages: readiness.expected_pages,
        html_bytes: html.len(),
        print_html_bytes: print_html.len(),
        slide_variants,
        missing_declared_slide_variants: missing_slide_variants,
        slide_presets: fixture_slide_presets(&deck),
        slide_classes: fixture_slide_classes(&deck),
        content_blocks: fixture_content_blocks(&deck),
        layout_kinds: fixture_layout_kinds(&deck),
        media_kinds: fixture_media_kinds(&deck),
        missing_declared_feature_hooks: missing_feature_hooks,
        uncovered_feature_hooks: uncovered_supported_feature_hooks(&feature_hooks),
        dead_selectors,
        feature_hooks,
        warnings: fixture_warnings,
    })
}

fn validate_fixture_slide_theme_params(
    manifest: &ThemeManifest,
    rendered_theme: &RenderedTheme,
    fixture: &Path,
    deck: &deck::Deck,
) -> Result<(), ThemeError> {
    for slide in deck.pdf_slide_order() {
        if slide.theme_params.is_empty() {
            continue;
        }
        let mut supplied = rendered_theme.params.clone();
        supplied.extend(slide.theme_params.clone());
        validate_theme_params(manifest, &supplied).map_err(|error| {
            ThemeError::FixtureCheckFailed {
                theme: manifest.name.clone(),
                path: fixture.to_path_buf(),
                reason: format!(
                    "slide '{}' has invalid theme parameter override: {error}",
                    slide.id
                ),
            }
        })?;
    }
    Ok(())
}

fn fixture_slide_variants(deck: &deck::Deck) -> Vec<String> {
    deck.pdf_slide_order()
        .into_iter()
        .filter_map(|slide| slide.variant.map(|variant| variant.as_str().to_string()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn missing_declared_slide_variants(
    manifest: &ThemeManifest,
    fixture_slide_variants: &[String],
) -> Vec<String> {
    manifest
        .slide_variants
        .iter()
        .filter(|variant| !fixture_slide_variants.contains(variant))
        .cloned()
        .collect()
}

fn fixture_slide_presets(deck: &deck::Deck) -> Vec<String> {
    deck.pdf_slide_order()
        .into_iter()
        .filter_map(|slide| slide.preset.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn fixture_slide_classes(deck: &deck::Deck) -> Vec<String> {
    deck.pdf_slide_order()
        .into_iter()
        .flat_map(|slide| slide.classes.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn fixture_content_blocks(deck: &deck::Deck) -> Vec<String> {
    let mut kinds = BTreeSet::new();
    for slide in deck.pdf_slide_order() {
        collect_content_block_kinds(&slide.blocks, &mut kinds);
    }
    kinds.into_iter().map(str::to_string).collect()
}

fn collect_content_block_kinds<'a>(
    blocks: &'a [deck::ContentBlock],
    kinds: &mut BTreeSet<&'a str>,
) {
    for block in blocks {
        kinds.insert(content_block_kind(block));
        if let deck::ContentBlock::Layout { regions, .. } = block {
            for region in regions {
                collect_content_block_kinds(&region.blocks, kinds);
            }
        }
    }
}

fn fixture_layout_kinds(deck: &deck::Deck) -> Vec<String> {
    let mut kinds = BTreeSet::new();
    for slide in deck.pdf_slide_order() {
        collect_layout_kinds(&slide.blocks, &mut kinds);
    }
    kinds.into_iter().map(str::to_string).collect()
}

fn collect_layout_kinds<'a>(blocks: &'a [deck::ContentBlock], kinds: &mut BTreeSet<&'a str>) {
    for block in blocks {
        if let deck::ContentBlock::Layout { kind, regions, .. } = block {
            kinds.insert(layout_kind_name(*kind));
            for region in regions {
                collect_layout_kinds(&region.blocks, kinds);
            }
        }
    }
}

fn fixture_media_kinds(deck: &deck::Deck) -> Vec<String> {
    let mut kinds = BTreeSet::new();
    for slide in deck.pdf_slide_order() {
        collect_media_kinds(&slide.blocks, &mut kinds);
    }
    kinds.into_iter().map(str::to_string).collect()
}

fn collect_media_kinds<'a>(blocks: &'a [deck::ContentBlock], kinds: &mut BTreeSet<&'a str>) {
    for block in blocks {
        match block {
            deck::ContentBlock::Media { kind, .. } => {
                kinds.insert(media_kind_name(*kind));
            }
            deck::ContentBlock::Layout { regions, .. } => {
                for region in regions {
                    collect_media_kinds(&region.blocks, kinds);
                }
            }
            _ => {}
        }
    }
}

fn fixture_feature_hooks(deck: &deck::Deck, api: ThemeApiVersion) -> Vec<String> {
    let mut features = BTreeSet::new();
    if let Some(background) = &deck.metadata.background_image {
        collect_background_features(background, api, &mut features);
    }
    for slide in deck.pdf_slide_order() {
        if deck.metadata.autoscale.is_some() || slide.autoscale.is_some() {
            features.insert("autoscale");
        }
        if deck.metadata.transition.is_some() || slide.transition.is_some() {
            features.insert("transitions");
        }
        if deck.metadata.theme_params.contains_key("footer")
            || deck.metadata.footer.is_some()
            || deck.metadata.slide_numbers.is_some()
            || !slide.footer.is_default()
        {
            features.insert("footer");
        }
        if slide.role == deck::SlideRole::Detail {
            features.insert("detail-slides");
        }
        if !slide.classes.is_empty() {
            features.insert("slide-classes");
        }
        if slide.preset.is_some() {
            features.insert("slide-presets");
        }
        if let Some(background) = &slide.background_image {
            collect_background_features(background, api, &mut features);
        }
        for block in &slide.blocks {
            collect_block_features(block, &mut features);
        }
    }
    features.into_iter().map(str::to_string).collect()
}

fn collect_background_features(
    background: &deck::DeckBackgroundImage,
    _api: ThemeApiVersion,
    features: &mut BTreeSet<&str>,
) {
    features.insert("background-image");
    if background.splash && background.splash_explicit {
        features.insert("background-splash");
    }
    if background.split.is_some() {
        features.insert("background-split");
    }
    if background.dim != deck::DeckBackgroundImage::DEFAULT_DIM
        || background.grayscale != deck::DeckBackgroundImage::DEFAULT_GRAYSCALE
        || background.saturate != deck::DeckBackgroundImage::DEFAULT_SATURATE
        || background.blur != deck::DeckBackgroundImage::DEFAULT_BLUR
    {
        features.insert("background-treatment");
    }
}

fn collect_block_features(block: &deck::ContentBlock, features: &mut BTreeSet<&str>) {
    match block {
        deck::ContentBlock::FitText { .. } => {
            features.insert("fit-text");
        }
        deck::ContentBlock::List { reveal: true, .. } => {
            features.insert("list-reveal");
        }
        deck::ContentBlock::Code {
            reveal: Some(_), ..
        } => {
            features.insert("code-reveal");
        }
        deck::ContentBlock::Figure { options, .. } => {
            if options.width.is_some() || options.height.is_some() {
                features.insert("figure-size");
            }
            if options.fit.is_some() {
                features.insert("figure-fit");
            }
            if options.align.is_some() {
                features.insert("figure-align");
            }
            if options.radius.is_some() {
                features.insert("figure-radius");
            }
            if options.dim.is_some()
                || options.grayscale.is_some()
                || options.saturate.is_some()
                || options.blur.is_some()
            {
                features.insert("figure-treatment");
            }
        }
        deck::ContentBlock::Gallery { items, columns } => {
            features.insert("image-gallery");
            if columns.is_some() {
                features.insert("gallery-columns");
            }
            if items.iter().any(|item| {
                item.options.dim.is_some()
                    || item.options.grayscale.is_some()
                    || item.options.saturate.is_some()
                    || item.options.blur.is_some()
            }) {
                features.insert("figure-treatment");
            }
            if items.iter().any(|item| item.options.radius.is_some()) {
                features.insert("figure-radius");
            }
        }
        deck::ContentBlock::Footnotes { .. } => {
            features.insert("footnotes");
        }
        deck::ContentBlock::Media {
            poster,
            start_time,
            options,
            visual_hidden,
            autoadvance,
            ..
        } => {
            if poster.is_some() {
                features.insert("media-poster");
            }
            if start_time.is_some() {
                features.insert("media-start");
            }
            if options.width.is_some() || options.height.is_some() {
                features.insert("media-size");
            }
            if options.fit.is_some() {
                features.insert("media-fit");
            }
            if options.align.is_some() {
                features.insert("media-align");
            }
            if *visual_hidden {
                features.insert("media-hidden");
            }
            if *autoadvance {
                features.insert("media-autoadvance");
            }
        }
        deck::ContentBlock::Diagram { language, .. } => match language {
            deck::DiagramLanguage::Mermaid => {
                features.insert("mermaid-diagram");
            }
        },
        deck::ContentBlock::Chart { data: Some(_), .. } => {
            features.insert("chart-local-data");
        }
        deck::ContentBlock::Steps { pdf_policy, .. } => match pdf_policy {
            deck::StepPdfPolicy::FinalState => {
                features.insert("steps-final-state");
            }
            deck::StepPdfPolicy::OnePagePerStep => {
                features.insert("steps-pages");
            }
        },
        deck::ContentBlock::SpeakerNotes { .. } => {
            features.insert("speaker-notes");
        }
        deck::ContentBlock::HtmlOnly { .. } => {
            features.insert("html-only");
        }
        deck::ContentBlock::Layout { regions, .. } => {
            for region in regions {
                for block in &region.blocks {
                    collect_block_features(block, features);
                }
            }
        }
        _ => {}
    }
}

fn content_block_kind(block: &deck::ContentBlock) -> &'static str {
    match block {
        deck::ContentBlock::Heading { .. } => "heading",
        deck::ContentBlock::Paragraph { .. } => "paragraph",
        deck::ContentBlock::FitText { .. } => "fit-text",
        deck::ContentBlock::Quote { .. } => "quote",
        deck::ContentBlock::Callout { .. } => "callout",
        deck::ContentBlock::List { .. } => "list",
        deck::ContentBlock::Math { .. } => "math",
        deck::ContentBlock::Code { .. } => "code",
        deck::ContentBlock::Table { .. } => "table",
        deck::ContentBlock::Figure { .. } => "figure",
        deck::ContentBlock::Footnotes { .. } => "footnotes",
        deck::ContentBlock::Gallery { .. } => "gallery",
        deck::ContentBlock::Media { .. } => "media",
        deck::ContentBlock::Diagram { .. } => "diagram",
        deck::ContentBlock::Chart { .. } => "chart",
        deck::ContentBlock::Steps { .. } => "steps",
        deck::ContentBlock::Layout { .. } => "layout",
        deck::ContentBlock::SpeakerNotes { .. } => "speaker-notes",
        deck::ContentBlock::HtmlOnly { .. } => "html-only",
        deck::ContentBlock::UnsupportedDirective { .. } => "unsupported-directive",
    }
}

fn layout_kind_name(kind: deck::LayoutKind) -> &'static str {
    match kind {
        deck::LayoutKind::Columns => "columns",
        deck::LayoutKind::Grid => "grid",
        deck::LayoutKind::Stack => "stack",
        deck::LayoutKind::Overlay => "overlay",
        deck::LayoutKind::Aside => "aside",
    }
}

fn media_kind_name(kind: deck::MediaKind) -> &'static str {
    match kind {
        deck::MediaKind::Video => "video",
        deck::MediaKind::Audio => "audio",
        deck::MediaKind::Iframe => "iframe",
    }
}

fn unsupported_fixture_variants(manifest: &ThemeManifest, deck: &deck::Deck) -> Vec<String> {
    let mut variants = Vec::new();
    for slide in deck.pdf_slide_order() {
        let Some(variant) = slide.variant else {
            continue;
        };
        let name = variant.as_str();
        if !manifest
            .slide_variants
            .iter()
            .any(|supported| supported == name)
        {
            variants.push(name.to_string());
        }
    }
    variants.sort();
    variants.dedup();
    variants
}

fn missing_declared_feature_hooks(
    manifest: &ThemeManifest,
    fixture_feature_hooks: &[String],
) -> Vec<String> {
    manifest
        .feature_hooks
        .iter()
        .filter(|feature| {
            !fixture_feature_hooks
                .iter()
                .any(|fixture_feature| fixture_feature == *feature)
        })
        .cloned()
        .collect()
}

fn uncovered_supported_feature_hooks(fixture_feature_hooks: &[String]) -> Vec<String> {
    SUPPORTED_THEME_FEATURE_HOOKS
        .iter()
        .filter(|feature| {
            !fixture_feature_hooks
                .iter()
                .any(|fixture_feature| fixture_feature == *feature)
        })
        .map(|feature| (*feature).to_string())
        .collect()
}

fn default_theme_check_fixture() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("canonical")
        .join("wide-sweep.zp.md");
    path.is_file().then_some(path)
}

fn validate_manifest_relative_path(theme: &str, value: &str) -> Result<(), ThemeError> {
    if is_safe_manifest_relative_path(value) {
        Ok(())
    } else {
        Err(ThemeError::UnsafePath {
            theme: theme.to_string(),
            path: value.to_string(),
        })
    }
}

fn is_safe_manifest_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

pub fn validate_theme_name(name: &str) -> Result<(), ThemeError> {
    if is_theme_slug(name) {
        Ok(())
    } else {
        Err(ThemeError::InvalidName {
            name: name.to_string(),
        })
    }
}

fn is_theme_slug(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) && !name.ends_with('-')
        && !name.contains("--")
}

fn normalize_output_targets(
    theme: &str,
    output_targets: Vec<String>,
) -> Result<Vec<String>, ThemeError> {
    let output_targets = if output_targets.is_empty() {
        SUPPORTED_OUTPUT_TARGETS
            .iter()
            .map(|target| (*target).to_string())
            .collect()
    } else {
        output_targets
    };
    for target in &output_targets {
        if !SUPPORTED_OUTPUT_TARGETS.contains(&target.as_str()) {
            return Err(ThemeError::InvalidOutputTarget {
                theme: theme.to_string(),
                target: target.clone(),
            });
        }
    }
    Ok(output_targets)
}

fn normalize_theme_modules(theme: &str, modules: Vec<String>) -> Result<Vec<String>, ThemeError> {
    let mut seen = BTreeSet::new();
    for module in &modules {
        if !SUPPORTED_THEME_MODULES.contains(&module.as_str()) {
            return Err(ThemeError::InvalidThemeModule {
                theme: theme.to_string(),
                module: module.clone(),
            });
        }
        if !seen.insert(module.clone()) {
            return Err(ThemeError::DuplicateThemeModule {
                theme: theme.to_string(),
                module: module.clone(),
            });
        }
    }
    Ok(modules)
}

fn require_output_target(manifest: &ThemeManifest, target: &str) -> Result<(), ThemeError> {
    if manifest
        .output_targets
        .iter()
        .any(|supported| supported == target)
    {
        Ok(())
    } else {
        Err(ThemeError::MissingOutputTarget {
            theme: manifest.name.clone(),
            target: target.to_string(),
        })
    }
}

fn validate_slide_variants(theme: &str, variants: &[String]) -> Result<(), ThemeError> {
    for variant in variants {
        if !SUPPORTED_SLIDE_VARIANTS.contains(&variant.as_str()) {
            return Err(ThemeError::InvalidSlideVariant {
                theme: theme.to_string(),
                variant: variant.clone(),
            });
        }
    }
    Ok(())
}

fn normalize_slide_presets(
    theme: &str,
    declared_variants: &[String],
    parameters: &BTreeMap<String, ThemeParameter>,
    presets: BTreeMap<String, RawThemeSlidePreset>,
) -> Result<BTreeMap<String, ThemeSlidePreset>, ThemeError> {
    let mut normalized = BTreeMap::new();
    for (name, preset) in presets {
        if !is_slide_preset_slug(&name) {
            return Err(ThemeError::InvalidSlidePreset {
                theme: theme.to_string(),
                preset: name,
            });
        }
        let variant = if let Some(raw_variant) = preset.variant {
            if !declared_variants
                .iter()
                .any(|variant| variant == &raw_variant)
            {
                return Err(ThemeError::InvalidSlideVariant {
                    theme: theme.to_string(),
                    variant: raw_variant,
                });
            }
            Some(parse_theme_slide_variant(&raw_variant).ok_or_else(|| {
                ThemeError::InvalidSlideVariant {
                    theme: theme.to_string(),
                    variant: raw_variant.clone(),
                }
            })?)
        } else {
            None
        };
        for class in &preset.classes {
            if !is_slide_preset_slug(class) {
                return Err(ThemeError::InvalidSlidePresetClass {
                    theme: theme.to_string(),
                    preset: name.clone(),
                    class: class.clone(),
                });
            }
        }
        let mut theme_params = BTreeMap::new();
        for (param_name, value) in preset.theme_params {
            let Some(parameter) = parameters.get(&param_name) else {
                return Err(ThemeError::UnknownSlidePresetParameter {
                    theme: theme.to_string(),
                    preset: name.clone(),
                    name: param_name,
                });
            };
            let value =
                theme_value_to_string(&value).map_err(|reason| ThemeError::InvalidParameter {
                    name: param_name.clone(),
                    value: "<preset>".to_string(),
                    reason,
                })?;
            validate_parameter_value(&param_name, &value, parameter)?;
            theme_params.insert(param_name, value);
        }
        let transition = if let Some(transition) = preset.transition {
            Some(parse_theme_slide_transition(&transition).ok_or_else(|| {
                ThemeError::InvalidSlidePresetTransition {
                    theme: theme.to_string(),
                    preset: name.clone(),
                    transition: transition.clone(),
                }
            })?)
        } else {
            None
        };
        normalized.insert(
            name,
            ThemeSlidePreset {
                variant,
                classes: preset.classes,
                theme_params,
                autoscale: preset.autoscale,
                transition,
            },
        );
    }
    Ok(normalized)
}

fn parse_theme_slide_variant(value: &str) -> Option<deck::SlideVariant> {
    match value {
        "claim" => Some(deck::SlideVariant::Claim),
        "figure" => Some(deck::SlideVariant::Figure),
        "comparison" => Some(deck::SlideVariant::Comparison),
        "derivation" => Some(deck::SlideVariant::Derivation),
        "section-title" => Some(deck::SlideVariant::SectionTitle),
        "dense" => Some(deck::SlideVariant::Dense),
        _ => None,
    }
}

fn parse_theme_slide_transition(value: &str) -> Option<deck::SlideTransition> {
    match value {
        "none" => Some(deck::SlideTransition::None),
        "fade" => Some(deck::SlideTransition::Fade),
        "slide" | "slide-left" => Some(deck::SlideTransition::Slide),
        "zoom" => Some(deck::SlideTransition::Zoom),
        _ => None,
    }
}

fn is_slide_preset_slug(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn normalize_feature_hooks(
    theme: &str,
    feature_hooks: Vec<String>,
) -> Result<Vec<String>, ThemeError> {
    let mut normalized = BTreeSet::new();
    for feature in feature_hooks {
        if !SUPPORTED_THEME_FEATURE_HOOKS.contains(&feature.as_str()) {
            return Err(ThemeError::InvalidFeatureHook {
                theme: theme.to_string(),
                feature,
            });
        }
        normalized.insert(feature);
    }
    Ok(normalized.into_iter().collect())
}

fn validate_parameter_value(
    name: &str,
    value: &str,
    parameter: &ThemeParameter,
) -> Result<(), ThemeError> {
    let invalid = |reason: String| ThemeError::InvalidParameter {
        name: name.to_string(),
        value: value.to_string(),
        reason,
    };

    match parameter.parameter_type {
        ThemeParameterType::String => {
            if let Some(min) = parameter.min
                && value.len() < min as usize
            {
                return Err(invalid(format!(
                    "must have length at least {}",
                    min as usize
                )));
            }
            if let Some(max) = parameter.max
                && value.len() > max as usize
            {
                return Err(invalid(format!(
                    "must have length at most {}",
                    max as usize
                )));
            }
            Ok(())
        }
        ThemeParameterType::Integer => {
            let parsed = value
                .parse::<i64>()
                .map_err(|_| invalid("must be an integer".to_string()))?;
            validate_numeric_bounds(parsed as f64, parameter).map_err(invalid)
        }
        ThemeParameterType::Number => {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| invalid("must be a number".to_string()))?;
            validate_numeric_bounds(parsed, parameter).map_err(invalid)
        }
        ThemeParameterType::Boolean => value
            .parse::<bool>()
            .map(|_| ())
            .map_err(|_| invalid("must be true or false".to_string())),
        ThemeParameterType::Enum => {
            if parameter.values.iter().any(|allowed| allowed == value) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "must be one of {}",
                    parameter.values.join(", ")
                )))
            }
        }
        ThemeParameterType::Color => {
            if is_hex_color(value) {
                Ok(())
            } else {
                Err(invalid("must be a #rgb or #rrggbb color".to_string()))
            }
        }
        ThemeParameterType::Font => {
            if is_safe_font_family_list(value) {
                Ok(())
            } else {
                Err(invalid(
                    "must be a safe CSS font-family list such as Inter, system-ui, sans-serif"
                        .to_string(),
                ))
            }
        }
        ThemeParameterType::Size => {
            if is_safe_css_size_token(value) {
                Ok(())
            } else {
                Err(invalid(
                    "must be a safe CSS size such as 1rem, 24px, or 50%".to_string(),
                ))
            }
        }
    }
}

fn validate_parameter_bounds(parameter: &ThemeParameter) -> Result<(), String> {
    if parameter
        .min
        .into_iter()
        .chain(parameter.max)
        .any(|bound| !bound.is_finite())
    {
        return Err("minimum and maximum must be finite".to_string());
    }
    if let (Some(min), Some(max)) = (parameter.min, parameter.max)
        && min > max
    {
        return Err("minimum cannot exceed maximum".to_string());
    }
    if parameter.parameter_type == ThemeParameterType::String
        && parameter
            .min
            .into_iter()
            .chain(parameter.max)
            .any(|bound| bound < 0.0 || bound.fract() != 0.0)
    {
        return Err("string length bounds must be nonnegative integers".to_string());
    }
    Ok(())
}

fn validate_numeric_bounds(value: f64, parameter: &ThemeParameter) -> Result<(), String> {
    if !value.is_finite() {
        return Err("must be a finite number".to_string());
    }
    if let Some(min) = parameter.min
        && value < min
    {
        return Err(format!("must be at least {min}"));
    }
    if let Some(max) = parameter.max
        && value > max
    {
        return Err(format!("must be at most {max}"));
    }
    Ok(())
}

fn is_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6) && hex.chars().all(|character| character.is_ascii_hexdigit())
}

fn is_safe_font_family_list(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 200
        && value.split(',').all(|family| {
            let family = family.trim();
            !family.is_empty()
                && family
                    .trim_matches(['"', '\''])
                    .chars()
                    .any(|character| character.is_ascii_alphanumeric())
                && family.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || character.is_ascii_whitespace()
                        || matches!(character, '-' | '_' | '"' | '\'' | '.')
                })
        })
}

fn is_safe_css_size_token(value: &str) -> bool {
    let value = value.trim();
    if value == "auto" || value == "0" {
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
        "" | "%"
            | "px"
            | "rem"
            | "em"
            | "vh"
            | "vw"
            | "vmin"
            | "vmax"
            | "ch"
            | "pt"
            | "pc"
            | "cm"
            | "mm"
            | "in"
            | "Q"
            | "q"
    )
}

fn theme_value_to_string(value: &toml::Value) -> Result<String, String> {
    match value {
        toml::Value::String(value) => Ok(value.clone()),
        toml::Value::Integer(value) => Ok(value.to_string()),
        toml::Value::Float(value) => Ok(value.to_string()),
        toml::Value::Boolean(value) => Ok(value.to_string()),
        _ => Err("default must be a string, integer, number, or boolean".to_string()),
    }
}

fn default_theme_stylesheet() -> String {
    "theme.css.tmpl".to_string()
}

fn default_theme_print_stylesheet() -> String {
    "print.css.tmpl".to_string()
}

fn default_palette_parameter() -> String {
    "variant".to_string()
}

fn toml_location(text: &str, error: &toml::de::Error) -> String {
    error.span().map_or_else(String::new, |span| {
        let (line, column) = line_column_for_offset(text, span.start);
        format!(":{line}:{column}")
    })
}

fn line_column_for_offset(text: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, character) in text.char_indices() {
        if index >= offset {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn built_in_theme_names() -> [&'static str; 6] {
        [
            "debug",
            "science",
            "paper-chalk",
            "dark-splash",
            "wedding",
            "sv",
        ]
    }

    fn write_checkable_theme(root: &Path) -> PathBuf {
        let theme_dir = root.join("checkable");
        fs::create_dir_all(theme_dir.join("assets")).unwrap();
        fs::write(theme_dir.join("assets/paper.png"), "png").unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r##"[theme]
name = "checkable"
version = "0.1.0"
api_version = 1
assets = ["assets/paper.png"]
output_targets = ["html", "pdf"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
palette_parameter = "mode"

[presets.spotlight]
variant = "claim"
classes = ["lead"]
autoscale = true
transition = "fade"

[presets.spotlight.theme_params]
accent = "#0f766e"

[parameters.mode]
type = "enum"
default = "light"
values = ["light", "dark"]

[parameters.background]
type = "color"

[parameters.surface]
type = "color"

[parameters.text]
type = "color"

[parameters.muted]
type = "color"

[parameters.accent]
type = "color"

[parameters.accent_alt]
type = "color"

[parameters.rule]
type = "color"

[color_variants.light]
background = "#ffffff"
surface = "#f8fafc"
text = "#0f172a"
muted = "#64748b"
accent = "#0f766e"
accent_alt = "#be123c"
rule = "#cbd5e1"

[color_variants.dark]
background = "#020617"
surface = "#111827"
text = "#f8fafc"
muted = "#94a3b8"
accent = "#22d3ee"
accent_alt = "#fb7185"
rule = "#334155"
"##,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            r#".zpres-slide { color: {{param.text}}; }
.zpres-slide-canvas { background: {{param.surface}}; }
.zpres-block { border-color: {{param.rule}}; }
.zpres-slide-frame, .zpres-slide-header, .zpres-slide-title, .zpres-slide-body, .zpres-slide-primary, .zpres-slide-sources, .zpres-slide-footer { color: {{param.text}}; }
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            r#".zpres-print-slide { color: {{param.text}}; }
.zpres-slide { background: {{param.background}}; }
.zpres-slide-canvas { background: {{param.surface}}; }
.zpres-block { border-color: {{param.rule}}; }
.zpres-slide-frame, .zpres-slide-header, .zpres-slide-title, .zpres-slide-body, .zpres-slide-primary, .zpres-slide-sources, .zpres-slide-footer { color: {{param.text}}; }
"#,
        )
        .unwrap();
        theme_dir
    }

    fn write_versioned_theme(
        root: &Path,
        name: &str,
        api_version: u32,
        screen_css: &str,
        print_css: &str,
    ) -> PathBuf {
        let theme_dir = root.join(name);
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            format!(
                r##"[theme]
name = "{name}"
version = "0.1.0"
api_version = {api_version}
output_targets = ["html", "pdf"]

[parameters.accent]
type = "color"
default = "#0f766e"
"##
            ),
        )
        .unwrap();
        fs::write(theme_dir.join("theme.css.tmpl"), screen_css).unwrap();
        fs::write(theme_dir.join("print.css.tmpl"), print_css).unwrap();
        theme_dir
    }

    #[test]
    fn numeric_parameters_reject_nonfinite_values_and_preserve_bounds() {
        let parameter = ThemeParameter {
            parameter_type: ThemeParameterType::Number,
            default: None,
            required: false,
            values: Vec::new(),
            min: Some(0.88),
            max: Some(0.96),
        };
        for (value, valid) in [
            ("0.88", true),
            ("0.92", true),
            ("0.96", true),
            ("0.87", false),
            ("0.97", false),
            ("NaN", false),
            ("inf", false),
            ("-inf", false),
        ] {
            assert_eq!(
                validate_parameter_value("scale", value, &parameter).is_ok(),
                valid,
                "{value}"
            );
        }
        let unbounded = ThemeParameter {
            min: None,
            max: None,
            ..parameter
        };
        for value in ["NaN", "inf", "-inf"] {
            assert!(validate_parameter_value("scale", value, &unbounded).is_err());
        }
    }

    #[test]
    fn manifest_rejects_invalid_parameter_bounds_without_a_default() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "bounds", 1, "", "");
        let path = theme_dir.join("theme.toml");
        let manifest = fs::read_to_string(&path).unwrap();
        for (parameter_type, bounds, valid) in [
            ("number", "min = 0.88\nmax = 0.96", true),
            ("number", "min = 1.0\nmax = 1.0", true),
            ("number", "min = nan", false),
            ("number", "max = inf", false),
            ("number", "min = -inf", false),
            ("number", "min = 1.0\nmax = 0.0", false),
            ("string", "min = -1.0", false),
            ("string", "max = 1.5", false),
            ("string", "min = 0\nmax = 10", true),
        ] {
            fs::write(
                &path,
                format!("{manifest}\n[parameters.value]\ntype = \"{parameter_type}\"\n{bounds}\n"),
            )
            .unwrap();
            assert_eq!(
                load_theme_manifest(&path).is_ok(),
                valid,
                "{parameter_type}: {bounds}"
            );
        }
    }

    #[test]
    fn loads_supported_api_version_through_typed_accessor() {
        let temp = tempdir().unwrap();
        let (number, expected) = (1, ThemeApiVersion::V1);
        let name = format!("version-{number}");
        let theme_dir = write_versioned_theme(temp.path(), &name, number, "", "");

        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

        assert_eq!(manifest.api_version, number);
        assert_eq!(manifest.api(), expected);
        assert_eq!(expected.as_u32(), number);
        assert_eq!(
            serde_json::to_value(&manifest).unwrap()["api_version"],
            serde_json::json!(number)
        );
    }

    #[test]
    fn v1_scientific_data_module_is_typed_and_validated() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "module-consumer", 1, "", "");
        let manifest_path = theme_dir.join("theme.toml");
        let manifest_text = fs::read_to_string(&manifest_path).unwrap().replace(
            "output_targets = [\"html\", \"pdf\"]",
            "output_targets = [\"html\", \"pdf\"]\nmodules = [\"scientific-data\"]",
        );
        fs::write(&manifest_path, manifest_text).unwrap();

        let manifest = load_theme_manifest(&manifest_path).unwrap();

        assert_eq!(manifest.modules, vec!["scientific-data"]);
        assert_eq!(
            serde_json::to_value(&manifest).unwrap()["modules"],
            serde_json::json!(["scientific-data"])
        );
    }

    #[test]
    fn shared_modules_reject_unknown_and_duplicate_declarations() {
        let cases = [
            (1, "[\"unknown\"]", "unsupported shared module 'unknown'"),
            (
                1,
                "[\"scientific-data\", \"scientific-data\"]",
                "shared module 'scientific-data' more than once",
            ),
        ];
        for (index, (api_version, modules, expected)) in cases.into_iter().enumerate() {
            let temp = tempdir().unwrap();
            let theme_dir = write_versioned_theme(
                temp.path(),
                &format!("module-invalid-{index}"),
                api_version,
                "",
                "",
            );
            let manifest_path = theme_dir.join("theme.toml");
            let manifest_text = fs::read_to_string(&manifest_path).unwrap().replace(
                "output_targets = [\"html\", \"pdf\"]",
                &format!("output_targets = [\"html\", \"pdf\"]\nmodules = {modules}"),
            );
            fs::write(&manifest_path, manifest_text).unwrap();

            let error = load_theme_manifest(&manifest_path).unwrap_err();

            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn rejects_unknown_api_version_at_exact_manifest_location() {
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("future-theme.toml");
        fs::write(
            &manifest_path,
            r#"[theme]
name = "future"
version = "0.1.0"
api_version = 2
"#,
        )
        .unwrap();

        let error = load_theme_manifest(&manifest_path).unwrap_err();

        assert!(matches!(
            &error,
            ThemeError::UnsupportedApiVersion {
                path,
                line: 4,
                column: 15,
                found: 2,
            } if path == &manifest_path
        ));
        let message = error.to_string();
        assert!(message.contains(&format!("{}:4:15", manifest_path.display())));
        assert!(message.contains("supported api_version is 1"));
    }

    #[test]
    fn v1_theme_css_uses_layer_and_namespaced_theme_ownership() {
        let temp = tempdir().unwrap();
        let regions = ".zpres-slide-frame {}\n.zpres-slide-header {}\n.zpres-slide-title {}\n.zpres-slide-body {}\n.zpres-slide-primary {}\n.zpres-slide-sources {}\n.zpres-slide-footer {}\n";
        let theme_dir = write_versioned_theme(
            temp.path(),
            "layered",
            1,
            regions,
            ".zpres-print-slide {}\n",
        );
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

        let rendered = render_theme(&manifest, &BTreeMap::new()).unwrap();

        assert!(rendered.screen_css.starts_with("@layer zpres-theme {\n"));
        assert!(rendered.print_css.starts_with("@layer zpres-theme {\n"));
        assert!(!rendered.screen_css.contains("@scope (.zpres-api-v1)"));
        assert!(!rendered.print_css.contains("@scope (.zpres-api-v1)"));
        for css in [&rendered.screen_css, &rendered.print_css] {
            assert!(css.contains(
                ":where(.zpres-api-v1.zpres-theme-layered, .zpres-api-v1 .layered-theme)"
            ));
            assert!(css.contains("--zpres-param-accent: #0f766e"));
            assert!(!css.contains(":root"));
        }
        assert!(
            theme_css_warnings(&manifest, &rendered.screen_css, &rendered.print_css).is_empty()
        );
    }

    #[test]
    fn theme_selector_registries_separate_stable_and_internal_hooks() {
        let stable = stable_v1_theme_selectors().collect::<BTreeSet<_>>();
        let internal = internal_theme_selectors().collect::<BTreeSet<_>>();

        assert!(stable.contains(".zpres-slide-frame"));
        assert!(stable.contains(".zpres-block-chart"));
        assert!(stable.contains(".zpres-block-layout"));
        assert!(!stable.contains(".zpres-chart-svg"));
        assert!(internal.contains(".zpres-chart-svg"));
        assert!(internal.contains(".zpres-speaker-notes-panel"));
        assert!(stable.is_disjoint(&internal));
    }

    #[test]
    fn v1_selector_warnings_and_authoring_registry_use_semantic_regions() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "region-check", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

        let warnings = theme_css_warnings(&manifest, "", ".zpres-print-slide {}");

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains(".zpres-slide-frame"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains(".zpres-slide-primary"))
        );
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.contains(".zpres-block"))
        );
        let selectors = supported_theme_semantic_selectors(ThemeApiVersion::V1).collect::<Vec<_>>();
        for selector in [
            ".zpres-slide-frame",
            ".zpres-slide-header",
            ".zpres-slide-title",
            ".zpres-slide-body",
            ".zpres-slide-primary",
            ".zpres-slide-supporting",
            ".zpres-slide-sources",
            ".zpres-slide-footer",
        ] {
            assert!(selectors.contains(&selector));
        }
        assert!(!selectors.contains(&".zpres-slide-meta"));
        assert!(selectors.contains(&".zpres-block-layout"));
    }

    #[test]
    fn prepare_deck_for_v1_accepts_missing_and_equivalent_sixteen_by_nine_aspects() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-aspect", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        assert!(is_sixteen_by_nine("16:9"));
        assert!(is_sixteen_by_nine("16x9"));
        assert!(is_sixteen_by_nine("1920:1080"));
        assert!(is_sixteen_by_nine(" 32 X 18 "));

        for (index, aspect) in [None, Some("16:9"), Some("1920x1080")]
            .into_iter()
            .enumerate()
        {
            let front_matter = aspect.map_or_else(String::new, |aspect| {
                format!("---\naspect: \"{aspect}\"\n---\n\n")
            });
            let mut deck = deck::parse_source_text(
                &format!("{front_matter}# Supported aspect\n\nBody\n"),
                Some(PathBuf::from(format!("supported-{index}.zp.md"))),
            )
            .unwrap();

            prepare_deck_for_theme(&mut deck, &manifest).unwrap();

            assert!(
                deck.diagnostics.iter().all(|diagnostic| !diagnostic
                    .message
                    .contains("Theme API v1 supports only 16:9")),
                "unexpected diagnostics for {aspect:?}: {:?}",
                deck.diagnostics
            );
        }
    }

    #[test]
    fn v1_background_authoring_errors_are_fatal_at_the_authored_field() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-background", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let mut deck = deck::parse_source_text(
            r#"---
background_image:
  src: assets/field.svg
  title: maybe
---

# Title
"#,
            Some(PathBuf::from("background.zp.md")),
        )
        .unwrap();

        prepare_deck_for_theme(&mut deck, &manifest).unwrap();

        let diagnostic = deck
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("title must be 'clean' or 'paint'")
            })
            .unwrap();
        assert!(diagnostic.is_fatal());
        assert!(diagnostic.span.as_ref().is_some_and(|span| {
            span.source_path.as_deref() == Some(Path::new("background.zp.md")) && span.line == 4
        }));
        assert!(validate_deck_for_theme_contract(&deck, &manifest).is_err());
        assert!(
            deck.diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("does not paint it"))
        );
    }

    #[test]
    fn v1_background_intent_requires_structural_alternatives_without_judging_quality() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-background-intent", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        fs::create_dir_all(temp.path().join("assets")).unwrap();
        fs::write(temp.path().join("assets/field.svg"), "<svg></svg>").unwrap();

        for (name, background, expected) in [
            (
                "implicit",
                "src: assets/field.svg",
                "requires explicit Theme API v1 intent",
            ),
            (
                "contextual-empty",
                "src: assets/field.svg\n  intent: contextual",
                "requires a non-empty short alt",
            ),
            (
                "decorative-labeled",
                "src: assets/field.svg\n  intent: decorative\n  alt: Information",
                "is decorative and must not provide alt or description",
            ),
            (
                "evidence-undescribed",
                "src: assets/field.svg\n  intent: evidence\n  alt: Evidence field",
                "requires description text",
            ),
        ] {
            let source = format!("---\nbackground_image:\n  {background}\n---\n\n# {name}\n");
            let mut deck =
                deck::parse_source_text(&source, Some(temp.path().join(format!("{name}.zp.md"))))
                    .unwrap();
            prepare_deck_for_theme(&mut deck, &manifest).unwrap();
            assert!(deck.diagnostics.iter().any(|diagnostic| {
                diagnostic.is_fatal() && diagnostic.message.contains(expected)
            }));
        }

        let short_alt = "x".repeat(160);
        let source = format!(
            "---\nbackground_image:\n  src: assets/field.svg\n  intent: evidence\n  alt: '{short_alt}'\n  description: A detailed scientific explanation that remains available in reading order.\n---\n\n# Valid evidence\n"
        );
        let mut deck =
            deck::parse_source_text(&source, Some(temp.path().join("valid.zp.md"))).unwrap();
        prepare_deck_for_theme(&mut deck, &manifest).unwrap();
        assert!(
            deck.diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.is_fatal())
        );

        let too_long = "x".repeat(161);
        let source = format!(
            "---\nbackground_image:\n  src: assets/field.svg\n  intent: contextual\n  alt: '{too_long}'\n---\n\n# Long alt\n"
        );
        let mut deck =
            deck::parse_source_text(&source, Some(temp.path().join("long.zp.md"))).unwrap();
        prepare_deck_for_theme(&mut deck, &manifest).unwrap();
        assert!(
            deck.diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("at most 160 characters"))
        );
    }

    #[test]
    fn v1_figures_accept_alt_only_while_media_require_static_description_routes() {
        let source = r#"# Alternatives

::: figure src="assets/figure.svg" alt="Short figure alternative"
:::

![inline alt="First gallery alternative"](assets/first.svg)
![inline alt="Second gallery alternative"](assets/second.svg)

![video poster="assets/poster.svg" alt="Short video alternative"](assets/clip.mp4)
"#;
        let deck =
            deck::parse_source_text(source, Some(PathBuf::from("alternatives.zp.md"))).unwrap();
        let diagnostics = v1_block_alternative_diagnostics(
            &deck.sections[0].main_slide.blocks,
            &deck.sections[0].main_slide.id,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics.iter().all(deck::Diagnostic::is_fatal));
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("video media")
                    && diagnostic.message.contains("caption or description route"))
        );

        let decorative = deck::parse_source_text(
            "# Decorative\n\n::: figure src=\"assets/figure.svg\" alt=\"\"\n:::\n",
            Some(PathBuf::from("decorative.zp.md")),
        )
        .unwrap();
        assert!(
            v1_block_alternative_diagnostics(
                &decorative.sections[0].main_slide.blocks,
                &decorative.sections[0].main_slide.id,
            )
            .is_empty()
        );

        let caption_without_alt = deck::parse_source_text(
            "# Invalid\n\n::: figure src=\"assets/figure.svg\" alt=\"\" caption=\"Visible caption\"\n:::\n",
            Some(PathBuf::from("caption-without-alt.zp.md")),
        )
        .unwrap();
        let diagnostics = v1_block_alternative_diagnostics(
            &caption_without_alt.sections[0].main_slide.blocks,
            &caption_without_alt.sections[0].main_slide.id,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0]
                .message
                .contains("caption text but an empty alt")
        );

        let too_long = "x".repeat(161);
        let source =
            format!("# Invalid\n\n::: figure src=\"assets/figure.svg\" alt=\"{too_long}\"\n:::\n");
        let too_long =
            deck::parse_source_text(&source, Some(PathBuf::from("too-long-figure-alt.zp.md")))
                .unwrap();
        let diagnostics = v1_block_alternative_diagnostics(
            &too_long.sections[0].main_slide.blocks,
            &too_long.sections[0].main_slide.id,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("at most 160 characters"));
    }

    #[test]
    fn a_dormant_v1_deck_background_warns_without_invalidating_the_contract() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-background", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let mut deck = deck::parse_source_text(
            r#"---
background_image:
  src: assets/dormant.svg
  intent: decorative
---

# Clean title only
"#,
            None,
        )
        .unwrap();

        prepare_deck_for_theme(&mut deck, &manifest).unwrap();

        let warnings = deck
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.is_fatal())
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("does not paint it"));
        assert!(validate_deck_for_theme_contract(&deck, &manifest).is_ok());
    }

    #[test]
    fn v1_theme_check_surfaces_dormant_fixture_background_warnings() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-background", 1, "", "");
        let assets = temp.path().join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("dormant.svg"), "<svg></svg>").unwrap();
        let fixture = temp.path().join("dormant.zp.md");
        fs::write(
            &fixture,
            r#"---
background_image:
  src: assets/dormant.svg
  intent: decorative
---

# Clean title only
"#,
        )
        .unwrap();

        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: Some(fixture),
                require_complete_fixture_feature_coverage: false,
            },
        )
        .unwrap();

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("does not paint it"))
        );
        assert!(report.fixture.as_ref().is_some_and(|fixture| {
            fixture
                .warnings
                .iter()
                .any(|warning| warning.contains("does not paint it"))
        }));
    }

    #[test]
    fn prepare_deck_for_v1_adds_fatal_source_diagnostic_for_unsupported_aspect() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-aspect", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let source_path = PathBuf::from("four-three.zp.md");
        let mut deck = deck::parse_source_text(
            "---\naspect: \"4:3\"\n---\n\n# Unsupported aspect\n\nBody\n",
            Some(source_path.clone()),
        )
        .unwrap();

        prepare_deck_for_theme(&mut deck, &manifest).unwrap();

        let diagnostics = deck
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("supports only 16:9"))
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].is_fatal());
        assert_eq!(diagnostics[0].span, None);
        assert!(
            diagnostics[0]
                .message
                .contains("Source file 'four-three.zp.md'")
        );
        assert!(diagnostics[0].message.contains("aspect '4:3'"));
    }

    #[test]
    fn prepare_deck_for_v1_accepts_all_typed_layout_kinds() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-layout", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let source_path = PathBuf::from("layouts.zp.md");
        let mut deck = deck::parse_source_text(
            "---\naspect: \"16:9\"\n---\n\n# Layout diagnostics\n\nBody\n",
            Some(source_path.clone()),
        )
        .unwrap();
        for kind in [
            deck::LayoutKind::Columns,
            deck::LayoutKind::Grid,
            deck::LayoutKind::Stack,
            deck::LayoutKind::Overlay,
            deck::LayoutKind::Aside,
        ] {
            deck.sections[0]
                .main_slide
                .blocks
                .push(deck::ContentBlock::Layout {
                    kind,
                    values: deck::LayoutValues::default(),
                    regions: Vec::new(),
                });
        }
        prepare_deck_for_theme(&mut deck, &manifest).unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
    }

    #[test]
    fn v1_warns_when_dense_is_used_on_a_main_slide_but_not_a_detail_slide() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "v1-dense", 1, "", "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let mut deck = deck::parse_source_text(
            r#"# Dense main

::: variant dense
:::

Main evidence.

--

## Dense detail

::: variant dense
:::

Detail evidence.
"#,
            Some(PathBuf::from("dense.zp.md")),
        )
        .unwrap();

        prepare_deck_for_theme(&mut deck, &manifest).unwrap();

        let warnings = deck
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("uses the Dense variant"))
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("Dense main"));
        assert!(!warnings[0].is_fatal());
        assert!(validate_deck_for_theme_contract(&deck, &manifest).is_ok());
    }

    #[test]
    fn direct_v1_theme_check_uses_the_package_local_specimen() {
        let theme_dir = builtin_theme_search_path().join("reference");

        let report = check_theme_package(&theme_dir).unwrap();

        assert_eq!(report.name, "reference");
        assert!(report.warnings.is_empty());
        assert!(
            report
                .fixture
                .unwrap()
                .source_path
                .ends_with("reference/specimen.zp.md")
        );
    }

    #[test]
    fn higher_precedence_theme_search_paths_win_name_collisions() {
        let deck_root = tempdir().unwrap();
        let low_precedence = tempdir().unwrap();
        let high_precedence = tempdir().unwrap();
        let low = write_versioned_theme(low_precedence.path(), "collision", 1, "", "");
        let high = write_versioned_theme(high_precedence.path(), "collision", 1, "", "");

        let manifest = load_named_theme(
            "collision",
            deck_root.path(),
            &[
                low_precedence.path().to_path_buf(),
                high_precedence.path().to_path_buf(),
            ],
        )
        .unwrap();

        assert_eq!(manifest.path, high.join("theme.toml"));
        assert_ne!(manifest.path, low.join("theme.toml"));
        assert_eq!(manifest.api_version, 1);
    }

    #[test]
    fn embedded_cache_recovers_missing_and_corrupt_files_without_touching_existing_entries() {
        let temp = tempdir().unwrap();
        let base = temp.path().join("cache");
        let original = materialize_builtin_themes_in(&base, "test").unwrap();
        fs::remove_file(original.join("debug/theme.toml")).unwrap();
        fs::write(original.join("user-notes.txt"), "keep this").unwrap();

        let recovered = materialize_builtin_themes_in(&base, "test").unwrap();
        assert_ne!(recovered, original);
        assert!(builtin_theme_tree_matches(&recovered));
        assert!(!original.join("debug/theme.toml").exists());
        assert_eq!(
            fs::read_to_string(original.join("user-notes.txt")).unwrap(),
            "keep this"
        );
        assert_eq!(
            materialize_builtin_themes_in(&base, "test").unwrap(),
            recovered
        );

        fs::write(recovered.join("debug/theme.toml"), "corrupt").unwrap();
        let fresh = materialize_builtin_themes_in(&base, "test").unwrap();
        assert!(builtin_theme_tree_matches(&fresh));
        assert_ne!(fresh, original);
        assert_ne!(fresh, recovered);
        assert_eq!(
            fs::read_to_string(recovered.join("debug/theme.toml")).unwrap(),
            "corrupt"
        );
    }

    #[test]
    fn embedded_cache_preserves_unowned_entries_and_falls_back_from_an_unavailable_cache() {
        let temp = tempdir().unwrap();
        let base = temp.path().join("cache");
        fs::create_dir_all(base.join("test")).unwrap();
        fs::write(base.join("test-recovered"), "unowned file").unwrap();
        let root = materialize_builtin_themes_in(&base, "test").unwrap();
        assert!(builtin_theme_tree_matches(&root));
        assert_eq!(fs::read_dir(base.join("test")).unwrap().count(), 0);
        assert_eq!(
            fs::read_to_string(base.join("test-recovered")).unwrap(),
            "unowned file"
        );

        let blocked = temp.path().join("not-a-directory");
        fs::write(&blocked, "keep this too").unwrap();
        let fallback = temp.path().join("temporary");
        let root = materialize_builtin_themes_with_fallback(&blocked, &fallback, "test").unwrap();
        assert!(root.starts_with(&fallback));
        assert!(builtin_theme_tree_matches(&root));
        assert_eq!(fs::read_to_string(&blocked).unwrap(), "keep this too");
        assert!(materialize_builtin_themes_with_fallback(&blocked, &blocked, "test").is_err());
    }

    #[test]
    fn concurrent_embedded_cache_creators_only_return_complete_trees() {
        let temp = tempdir().unwrap();
        let base = temp.path().join("cache");
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            let handles = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        materialize_builtin_themes_in(&base, "test").unwrap()
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                assert!(builtin_theme_tree_matches(&handle.join().unwrap()));
            }
        });
        let root = materialize_builtin_themes_in(&base, "test").unwrap();
        assert_eq!(root, base.join("test"));
        assert!(builtin_theme_tree_matches(&root));
    }

    #[test]
    fn built_in_theme_manifests_reference_files_inside_theme_folders() {
        let root = builtin_theme_search_path();
        for name in built_in_theme_names() {
            let manifest = load_theme_manifest(&root.join(name).join("theme.toml")).unwrap();
            for relative_path in [
                manifest.stylesheet.as_str(),
                manifest.print_stylesheet.as_str(),
            ] {
                let path = theme_relative_path(&manifest, relative_path).unwrap();
                assert!(
                    path.is_file(),
                    "{} should reference an existing file",
                    path.display()
                );
            }
            for dependency in theme_dependency_paths(&manifest) {
                let path = theme_relative_path(&manifest, dependency).unwrap();
                assert!(
                    path.is_file(),
                    "{} should reference an existing file",
                    path.display()
                );
            }
            for inspiration in &manifest.style.inspiration {
                let path = theme_relative_path(&manifest, inspiration).unwrap();
                assert!(
                    path.is_file(),
                    "{} should reference an existing file",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn built_in_themes_pass_theme_check_without_warnings() {
        let root = builtin_theme_search_path();
        for name in built_in_theme_names() {
            let report = check_theme_package(&root.join(name)).unwrap();
            assert!(
                report.warnings.is_empty(),
                "{name} theme check warnings: {:?}",
                report.warnings
            );
        }
    }

    #[test]
    fn rendered_theme_css_comes_from_templates_without_unresolved_placeholders() {
        let root = builtin_theme_search_path();
        let manifest = load_theme_manifest(&root.join("paper-chalk").join("theme.toml")).unwrap();
        let rendered = render_theme(
            &manifest,
            &BTreeMap::from([("mode".to_string(), "dark".to_string())]),
        )
        .unwrap();

        assert!(
            rendered
                .screen_css
                .contains("--zpres-color-background: #101a22")
        );
        assert!(rendered.print_css.contains("--zpres-param-mode: dark"));
        assert!(!rendered.screen_css.contains("{{"));
        assert!(!rendered.print_css.contains("{{"));
    }

    #[test]
    fn rendered_theme_css_exposes_resolved_params_as_standard_variables() {
        let root = builtin_theme_search_path();
        let manifest = load_theme_manifest(&root.join("science").join("theme.toml")).unwrap();
        let rendered = render_theme(
            &manifest,
            &BTreeMap::from([
                ("accent".to_string(), "#ff00ff".to_string()),
                ("footer".to_string(), "none".to_string()),
            ]),
        )
        .unwrap();

        assert!(rendered.screen_css.contains(".zpres-theme-science {"));
        assert!(
            rendered
                .screen_css
                .contains("--zpres-param-accent: #ff00ff;")
        );
        assert!(
            rendered
                .screen_css
                .contains("--zpres-color-accent: #ff00ff;")
        );
        assert!(rendered.screen_css.contains("--zpres-param-footer: none;"));
        assert!(
            rendered
                .print_css
                .contains("--zpres-param-accent: #ff00ff;")
        );
    }

    #[test]
    fn theme_coverage_recurses_through_typed_columns_blocks() {
        let deck = deck::parse_source_text(
            r#"# Nested coverage

:::: columns
Evidence column:
![fit=contain radius=10 alt="Evidence"](assets/phase-space.svg)

Measurement column:
::: vega-lite data="data/runtime.csv"
{ "mark": "line", "encoding": { "x": { "field": "size" }, "y": { "field": "runtime_ms" } } }
:::

::: steps pdf="pages"
1. First state.
2. Second state.
:::
::::
"#,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);

        let blocks = fixture_content_blocks(&deck);
        for kind in ["layout", "figure", "chart", "steps"] {
            assert!(blocks.iter().any(|found| found == kind), "missing {kind}");
        }
        let features = fixture_feature_hooks(&deck, ThemeApiVersion::V1);
        for feature in [
            "figure-fit",
            "figure-radius",
            "chart-local-data",
            "steps-pages",
        ] {
            assert!(
                features.iter().any(|found| found == feature),
                "missing {feature}"
            );
        }
    }

    #[test]
    fn palette_variants_override_color_defaults_before_explicit_overrides() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

        let resolved = validate_theme_params(
            &manifest,
            &BTreeMap::from([
                ("mode".to_string(), "dark".to_string()),
                ("accent".to_string(), "#ff00ff".to_string()),
            ]),
        )
        .unwrap();

        assert_eq!(resolved.get("mode").unwrap(), "dark");
        assert_eq!(resolved.get("background").unwrap(), "#020617");
        assert_eq!(resolved.get("text").unwrap(), "#f8fafc");
        assert_eq!(resolved.get("accent").unwrap(), "#ff00ff");
    }

    #[test]
    fn font_parameters_render_as_safe_unquoted_theme_variables() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("typeful");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "typeful"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]

[parameters.font_body]
type = "font"
default = 'Inter, system-ui, sans-serif'

[parameters.font_mono]
type = "font"
default = '"IBM Plex Mono", ui-monospace, monospace'
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            ".zpres-slide { font-family: var(--zpres-font-body); }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide {}\n.zpres-slide { font-family: var(--zpres-font-body); }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();

        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = render_theme(
            &manifest,
            &BTreeMap::from([(
                "font_body".to_string(),
                r#""Aptos Display", ui-sans-serif, sans-serif"#.to_string(),
            )]),
        )
        .unwrap();

        assert!(
            rendered.screen_css.contains(
                r#"--zpres-param-font-body: "Aptos Display", ui-sans-serif, sans-serif;"#
            )
        );
        assert!(
            rendered
                .screen_css
                .contains(r#"--zpres-font-body: "Aptos Display", ui-sans-serif, sans-serif;"#)
        );
        assert!(
            rendered
                .screen_css
                .contains(r#"--zpres-font-mono: "IBM Plex Mono", ui-monospace, monospace;"#)
        );
    }

    #[test]
    fn font_parameters_reject_css_injection_values() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("unsafe-font");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "unsafe-font"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]

[parameters.font_body]
type = "font"
default = "Inter; color: red"
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            ".zpres-slide {}\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide {}\n.zpres-slide {}\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();

        let error = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap_err();

        assert!(error.to_string().contains("safe CSS font-family list"));
    }

    #[test]
    fn size_parameters_render_as_safe_unquoted_theme_variables() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("sized");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "sized"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]

[parameters.slide_padding]
type = "size"
default = "5rem"

[parameters.size_canvas_radius]
type = "size"
default = "1.25rem"
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            ".zpres-slide { padding: var(--zpres-size-slide-padding); }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide {}\n.zpres-slide { padding: var(--zpres-size-slide-padding); }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();

        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = render_theme(
            &manifest,
            &BTreeMap::from([("slide_padding".to_string(), "6.5rem".to_string())]),
        )
        .unwrap();

        assert!(
            rendered
                .screen_css
                .contains("--zpres-param-slide-padding: 6.5rem;")
        );
        assert!(
            rendered
                .screen_css
                .contains("--zpres-size-slide-padding: 6.5rem;")
        );
        assert!(
            rendered
                .screen_css
                .contains("--zpres-size-canvas-radius: 1.25rem;")
        );
    }

    #[test]
    fn size_parameters_reject_css_injection_values() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("unsafe-size");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "unsafe-size"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]

[parameters.slide_padding]
type = "size"
default = "1rem; color: red"
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            ".zpres-slide {}\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide {}\n.zpres-slide {}\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();

        let error = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap_err();

        assert!(error.to_string().contains("safe CSS size"));
    }

    #[test]
    fn theme_check_renders_templates_assets_and_palette_variants() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());

        let report = check_theme_package(&theme_dir).unwrap();

        assert_eq!(report.name, "checkable");
        assert_eq!(report.version, "0.1.0");
        assert_eq!(report.manifest_path, theme_dir.join("theme.toml"));
        assert!(report.declared_feature_hooks.is_empty());
        assert!(report.theme_parameters.iter().any(|parameter| {
            parameter.name == "mode"
                && parameter.parameter_type == "enum"
                && parameter.default.as_deref() == Some("light")
        }));
        assert!(report.theme_parameters.iter().any(|parameter| {
            parameter.name == "background"
                && parameter.parameter_type == "color"
                && parameter.default.is_none()
        }));
        assert!(
            report
                .checked_files
                .contains(&theme_dir.join("theme.css.tmpl"))
        );
        assert!(
            report
                .checked_files
                .contains(&theme_dir.join("print.css.tmpl"))
        );
        assert!(
            report
                .checked_files
                .contains(&theme_dir.join("assets/paper.png"))
        );
        assert_eq!(report.rendered_variants, vec!["defaults", "dark", "light"]);
        let fixture = report.fixture.as_ref().expect("fixture should render");
        assert!(
            fixture
                .source_path
                .ends_with("fixtures/canonical/wide-sweep.zp.md")
        );
        assert_eq!(fixture.sections, 12);
        assert_eq!(fixture.expected_pdf_pages, 18);
        assert!(fixture.html_bytes > 1000);
        assert!(fixture.print_html_bytes > 1000);
        assert_eq!(
            fixture.slide_variants,
            vec![
                "claim",
                "comparison",
                "dense",
                "derivation",
                "figure",
                "section-title"
            ]
        );
        assert!(fixture.slide_presets.is_empty());
        assert_eq!(fixture.slide_classes, vec!["hero", "lead", "result"]);
        assert_eq!(
            fixture.content_blocks,
            vec![
                "callout",
                "chart",
                "code",
                "diagram",
                "figure",
                "fit-text",
                "footnotes",
                "gallery",
                "heading",
                "html-only",
                "layout",
                "list",
                "math",
                "media",
                "paragraph",
                "quote",
                "speaker-notes",
                "steps",
                "table"
            ]
        );
        assert_eq!(fixture.layout_kinds, vec!["columns", "stack"]);
        assert_eq!(fixture.media_kinds, vec!["audio", "iframe", "video"]);
        assert_eq!(
            fixture.feature_hooks,
            vec![
                "autoscale",
                "background-image",
                "background-splash",
                "background-split",
                "background-treatment",
                "chart-local-data",
                "code-reveal",
                "detail-slides",
                "figure-align",
                "figure-fit",
                "figure-radius",
                "figure-size",
                "figure-treatment",
                "fit-text",
                "footer",
                "footnotes",
                "gallery-columns",
                "html-only",
                "image-gallery",
                "list-reveal",
                "media-align",
                "media-autoadvance",
                "media-fit",
                "media-hidden",
                "media-poster",
                "media-size",
                "media-start",
                "mermaid-diagram",
                "slide-classes",
                "speaker-notes",
                "steps-final-state",
                "steps-pages",
                "transitions"
            ]
        );
        assert_eq!(fixture.uncovered_feature_hooks, vec!["slide-presets"]);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn theme_check_reports_parameters_unused_by_both_authored_templates() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("parameter-usage");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r##"[theme]
name = "parameter-usage"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
palette_parameter = "mode"

[parameters.mode]
type = "enum"
default = "light"
values = ["light", "dark"]

[parameters.screen_only]
type = "string"
default = "screen"

[parameters.accent]
type = "color"
default = "#0f766e"

[parameters.background]
type = "color"
default = "#ffffff"

[parameters.surface]
type = "color"
default = "#f8fafc"

[parameters.text]
type = "color"
default = "#0f172a"

[parameters.muted]
type = "color"
default = "#64748b"

[parameters.accent_alt]
type = "color"
default = "#be123c"

[parameters.rule]
type = "color"
default = "#cbd5e1"

[parameters.font_body]
type = "font"
default = "system-ui"

[parameters.slide_padding]
type = "size"
default = "4rem"

[parameters.enabled]
type = "boolean"
default = true

[parameters.unused]
type = "integer"
default = 7

[color_variants.light]
background = "#ffffff"
surface = "#f8fafc"
text = "#0f172a"
muted = "#64748b"
accent = "#0f766e"
accent_alt = "#be123c"
rule = "#cbd5e1"

[color_variants.dark]
background = "#020617"
surface = "#111827"
text = "#f8fafc"
muted = "#94a3b8"
accent = "#5eead4"
accent_alt = "#fb7185"
rule = "#334155"
"##,
        )
        .unwrap();
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            r#".zpres-slide {
  color: {{param.screen_only}};
  background: var(--zpres-color-background);
  outline-color: var(--zpres-color-surface);
  text-decoration-color: var(--zpres-color-text);
  caret-color: var(--zpres-color-muted);
  border-color: var(--zpres-color-accent);
  column-rule-color: var(--zpres-color-accent-alt);
  fill: var(--zpres-color-rule);
  font-family: var(--zpres-font-body);
  padding: var(--zpres-size-slide-padding);
  opacity: var(--zpres-param-enabled);
}
.zpres-slide-canvas {}
.zpres-block {}
"#,
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide {}\n.zpres-slide {}\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();

        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: None,
                require_complete_fixture_feature_coverage: false,
            },
        )
        .unwrap();

        assert_eq!(report.unused_theme_parameters, vec!["unused"]);
    }

    #[test]
    fn fixture_layout_coverage_collects_layouts_nested_inside_regions() {
        let nested = deck::ContentBlock::Layout {
            kind: deck::LayoutKind::Stack,
            values: deck::LayoutValues::default(),
            regions: Vec::new(),
        };
        let blocks = vec![deck::ContentBlock::Layout {
            kind: deck::LayoutKind::Aside,
            values: deck::LayoutValues::default(),
            regions: vec![deck::LayoutRegion {
                name: Some("Argument".to_string()),
                source_span: None,
                grid_placement: None,
                role: Some(deck::LayoutRegionRole::Primary),
                overlay_placement: None,
                derivation_step: None,
                blocks: vec![nested],
            }],
        }];
        let mut kinds = BTreeSet::new();

        collect_layout_kinds(&blocks, &mut kinds);

        assert_eq!(kinds, BTreeSet::from(["aside", "stack"]));
    }

    #[test]
    fn theme_check_accepts_minimal_templates_that_use_generated_variables() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            ".zpres-slide { color: var(--zpres-color-text); }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();
        fs::write(
            theme_dir.join("print.css.tmpl"),
            ".zpres-print-slide { color: var(--zpres-color-text); }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
        )
        .unwrap();

        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = render_theme(
            &manifest,
            &BTreeMap::from([("mode".to_string(), "dark".to_string())]),
        )
        .unwrap();
        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: None,
                ..ThemeCheckOptions::default()
            },
        )
        .unwrap();

        assert_eq!(report.rendered_variants, vec!["defaults", "dark", "light"]);
        assert!(rendered.screen_css.contains("--zpres-color-text: #f8fafc;"));
        assert!(
            rendered
                .screen_css
                .contains("--zpres-param-accent-alt: #fb7185;")
        );
    }

    #[test]
    fn theme_check_rejects_missing_referenced_assets() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        fs::remove_file(theme_dir.join("assets/paper.png")).unwrap();

        let error = check_theme_package(&theme_dir).unwrap_err();

        assert!(matches!(error, ThemeError::MissingReferencedFile { .. }));
        assert!(error.to_string().contains("assets/paper.png"));
    }

    #[test]
    fn theme_check_rejects_template_errors() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            ".zpres-slide { color: {{param.unknown}}; }",
        )
        .unwrap();

        let error = check_theme_package(&theme_dir).unwrap_err();

        assert!(matches!(error, ThemeError::MissingTemplateParameter { .. }));
    }

    #[test]
    fn theme_check_rejects_fixture_variants_the_theme_does_not_declare() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        fs::write(
            theme_dir.join("theme.toml"),
            fs::read_to_string(theme_dir.join("theme.toml"))
                .unwrap()
                .replace(
                    r#"slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]"#,
                    r#"slide_variants = ["claim", "figure"]"#,
                ),
        )
        .unwrap();

        let error = check_theme_package(&theme_dir).unwrap_err();

        assert!(matches!(error, ThemeError::FixtureCheckFailed { .. }));
        assert!(error.to_string().contains("comparison"));
        assert!(error.to_string().contains("not declared"));
    }

    #[test]
    fn strict_theme_check_rejects_declared_variants_missing_from_fixture() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        let fixture = temp.path().join("claim-only.zp.md");
        fs::write(
            &fixture,
            r#"---
title: "Claim only"
---

# Claim

::: variant claim
:::

One covered variant.
"#,
        )
        .unwrap();

        let error = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: Some(fixture),
                require_complete_fixture_feature_coverage: true,
            },
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("fixture does not exercise declared slide variants")
        );
        assert!(error.to_string().contains("comparison"));
        assert!(!error.to_string().contains("claim, "));

        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: Some(temp.path().join("claim-only.zp.md")),
                require_complete_fixture_feature_coverage: false,
            },
        )
        .unwrap();
        let fixture = report.fixture.unwrap();
        assert_eq!(
            fixture.missing_declared_slide_variants,
            vec![
                "figure",
                "comparison",
                "derivation",
                "section-title",
                "dense"
            ]
        );
        assert!(report.warnings.iter().any(|warning| {
            warning.contains(
                "this visual review covers only the variants used by the selected fixture",
            )
        }));
    }

    #[test]
    fn theme_check_rejects_declared_feature_hooks_not_covered_by_fixture() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        fs::write(
            theme_dir.join("theme.toml"),
            fs::read_to_string(theme_dir.join("theme.toml"))
                .unwrap()
                .replace(
                    r#"slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]"#,
                    r#"slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
feature_hooks = ["fit-text", "media-poster"]"#,
                ),
        )
        .unwrap();

        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("canonical")
            .join("canonical.zp.md");
        let error = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: Some(fixture),
                ..ThemeCheckOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(error, ThemeError::FixtureCheckFailed { .. }));
        assert!(error.to_string().contains("media-poster"));
        assert!(
            error
                .to_string()
                .contains("fixture does not exercise declared feature_hooks")
        );
    }

    #[test]
    fn theme_check_can_report_incomplete_fixture_coverage_without_rejecting_it() {
        let temp = tempdir().unwrap();
        let theme_dir = write_checkable_theme(temp.path());
        fs::write(
            theme_dir.join("theme.toml"),
            fs::read_to_string(theme_dir.join("theme.toml"))
                .unwrap()
                .replace(
                    r#"slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]"#,
                    r#"slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
feature_hooks = ["fit-text", "media-poster"]"#,
                ),
        )
        .unwrap();

        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("canonical")
            .join("canonical.zp.md");
        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: Some(fixture),
                require_complete_fixture_feature_coverage: false,
            },
        )
        .unwrap();

        let fixture = report.fixture.unwrap();
        assert!(fixture.missing_declared_slide_variants.is_empty());
        assert_eq!(fixture.missing_declared_feature_hooks, vec!["media-poster"]);
        assert!(report.warnings.iter().any(|warning| {
            warning.contains(
                "this visual review covers only the features used by the selected fixture",
            )
        }));
    }

    #[test]
    fn rejects_theme_names_that_are_not_css_safe_slugs() {
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("theme.toml");
        fs::write(
            &manifest_path,
            r#"[theme]
name = "bad theme"
version = "0.1.0"
api_version = 1
"#,
        )
        .unwrap();

        let error = load_theme_manifest(&manifest_path).unwrap_err();

        assert!(matches!(error, ThemeError::InvalidName { .. }));
    }

    #[test]
    fn selector_diagnostics_ignore_comments_strings_and_valid_compounds() {
        let temp = tempdir().unwrap();
        let css = r#"
/* .zpres-comment-typo {} */
.zpres-slide-frame, .zpres-slide-header, .zpres-slide-title,
.zpres-slide-body, .zpres-slide-primary, .zpres-slide-sources,
.zpres-slide-footer {}
.zpres-slide:is([data-slide-role="main"], :hover) .zpres-block-code::before {
  content: ".zpres-string-typo";
}
.zpres-theme-selector-check .zpres-slide-class-local {}
.zpres-slide-titel {}
.zpres-slide-meta {}
"#;
        let theme_dir = write_versioned_theme(
            temp.path(),
            "selector-check",
            1,
            css,
            ".zpres-print-slide:hover {}\n",
        );
        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: None,
                require_complete_fixture_feature_coverage: false,
            },
        )
        .unwrap();

        assert_eq!(
            report.unknown_selectors,
            vec![".zpres-slide-meta", ".zpres-slide-titel"]
        );
    }

    #[test]
    fn selector_diagnostics_report_fixture_unmatched_but_not_print_only_classes() {
        let temp = tempdir().unwrap();
        let css = r#"
.zpres-slide-frame, .zpres-slide-header, .zpres-slide-title,
.zpres-slide-body, .zpres-slide-primary,
.zpres-slide-footer {}
.zpres-block-paragraph:hover {}
.zpres-block-chart[data-chart-kind="line"] {}
"#;
        let theme_dir = write_versioned_theme(
            temp.path(),
            "dead-selector-check",
            1,
            css,
            ".zpres-print-slide {}\n",
        );
        let fixture = temp.path().join("ordinary.zp.md");
        fs::write(
            &fixture,
            "---\ntitle: Ordinary\n---\n\n# An ordinary slide\n\nNo chart.\n",
        )
        .unwrap();
        let report = check_theme_package_with_options(
            &theme_dir,
            &ThemeCheckOptions {
                fixture: Some(fixture),
                require_complete_fixture_feature_coverage: false,
            },
        )
        .unwrap();

        assert_eq!(report.dead_selectors, vec![".zpres-block-chart"]);
        assert_eq!(
            report.fixture.unwrap().dead_selectors,
            vec![".zpres-block-chart"]
        );
    }

    #[test]
    fn rejects_unsafe_manifest_paths_and_unknown_manifest_contract_values() {
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("theme.toml");
        fs::write(
            &manifest_path,
            r#"[theme]
name = "safe-theme"
version = "0.1.0"
api_version = 1
stylesheet = "../theme.css.tmpl"
"#,
        )
        .unwrap();
        assert!(matches!(
            load_theme_manifest(&manifest_path).unwrap_err(),
            ThemeError::UnsafePath { .. }
        ));

        fs::write(
            &manifest_path,
            r#"[theme]
name = "safe-theme"
version = "0.1.0"
api_version = 1
fonts = ["../outside.woff2"]
"#,
        )
        .unwrap();
        assert!(matches!(
            load_theme_manifest(&manifest_path).unwrap_err(),
            ThemeError::UnsafePath { path, .. } if path == "../outside.woff2"
        ));

        fs::write(
            &manifest_path,
            r#"[theme]
name = "safe-theme"
version = "0.1.0"
api_version = 1
output_targets = ["html", "keynote"]
"#,
        )
        .unwrap();
        assert!(matches!(
            load_theme_manifest(&manifest_path).unwrap_err(),
            ThemeError::InvalidOutputTarget { .. }
        ));

        fs::write(
            &manifest_path,
            r#"[theme]
name = "safe-theme"
version = "0.1.0"
api_version = 1
slide_variants = ["hero"]
"#,
        )
        .unwrap();
        assert!(matches!(
            load_theme_manifest(&manifest_path).unwrap_err(),
            ThemeError::InvalidSlideVariant { .. }
        ));

        fs::write(
            &manifest_path,
            r#"[theme]
name = "safe-theme"
version = "0.1.0"
api_version = 1
feature_hooks = ["lasers"]
"#,
        )
        .unwrap();
        assert!(matches!(
            load_theme_manifest(&manifest_path).unwrap_err(),
            ThemeError::InvalidFeatureHook { .. }
        ));
    }

    #[test]
    fn theme_dependency_inventory_is_sorted_and_rejects_duplicate_declarations() {
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("theme.toml");
        fs::write(
            &manifest_path,
            r#"[theme]
name = "dependency-theme"
version = "0.1.0"
api_version = 1
fonts = ["fonts/zeta.woff2", "fonts/alpha.woff2"]
assets = ["textures/paper.png"]
"#,
        )
        .unwrap();

        let manifest = load_theme_manifest(&manifest_path).unwrap();

        assert_eq!(
            theme_dependency_paths(&manifest).collect::<Vec<_>>(),
            vec![
                "fonts/alpha.woff2",
                "fonts/zeta.woff2",
                "textures/paper.png"
            ]
        );

        fs::write(
            &manifest_path,
            r#"[theme]
name = "dependency-theme"
version = "0.1.0"
api_version = 1
fonts = ["shared/file.woff2"]
assets = ["shared/file.woff2"]
"#,
        )
        .unwrap();

        let error = load_theme_manifest(&manifest_path).unwrap_err();
        assert!(matches!(
            error,
            ThemeError::DuplicateDependencyPath { path, .. }
                if path == "shared/file.woff2"
        ));

        fs::write(
            &manifest_path,
            r#"[theme]
name = "dependency-theme"
version = "0.1.0"
api_version = 1
fonts = ["Fonts/Body.woff2"]
assets = ["fonts/body.woff2"]
"#,
        )
        .unwrap();

        let error = load_theme_manifest(&manifest_path).unwrap_err();
        assert!(matches!(
            error,
            ThemeError::PortableDependencyCollision { first, second, .. }
                if first == "Fonts/Body.woff2" && second == "fonts/body.woff2"
        ));
    }

    #[test]
    fn theme_css_preserves_data_fragments_comments_and_strings() {
        let temp = tempdir().unwrap();
        let screen_css = ".slide { mask: url(#clip); background: url(\"data:image/svg+xml,%3Csvg/%3E\"); }\n/* url(missing.svg) */\n.slide::after { content: \"url(also-missing.svg)\"; }\n";
        let theme_dir = write_versioned_theme(temp.path(), "inline-urls", 1, screen_css, "");
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

        let rendered = render_theme(&manifest, &BTreeMap::new()).unwrap();

        assert!(rendered.screen_css.contains("url(#clip)"));
        assert!(
            rendered
                .screen_css
                .contains("url(\"data:image/svg+xml,%3Csvg/%3E\")")
        );
        assert!(rendered.screen_css.contains("/* url(missing.svg) */"));
        assert!(
            rendered
                .screen_css
                .contains("content: \"url(also-missing.svg)\"")
        );
        assert_eq!(rendered.screen_css, rendered.static_screen_css);
        assert_eq!(
            rendered.manifest.path,
            fs::canonicalize(theme_dir.join("theme.toml")).unwrap()
        );
    }

    #[test]
    fn theme_css_rejects_undeclared_traversal_remote_absolute_and_malformed_urls() {
        let temp = tempdir().unwrap();
        for (name, css, expected_reason) in [
            (
                "undeclared-url",
                ".slide { background: url(textures/missing.svg); }",
                "not declared",
            ),
            (
                "traversal-url",
                ".slide { background: url('../outside.svg'); }",
                "parent traversal",
            ),
            (
                "remote-url",
                ".slide { background: url(https://example.com/paper.svg); }",
                "URL scheme 'https'",
            ),
            (
                "absolute-url",
                ".slide { background: url('/tmp/paper.svg'); }",
                "absolute local URL",
            ),
            (
                "malformed-url",
                ".slide { background: url('textures/paper.svg'; }",
                "followed by ')'",
            ),
            (
                "invalid-percent-url",
                ".slide { background: url('textures/paper%XX.svg'); }",
                "invalid percent escape",
            ),
            (
                "string-newline-remote-recovery",
                ".slide::before { content: \"unterminated\n} .slide { background: url(https://example.com/paper.svg); }",
                "unescaped line breaks",
            ),
            (
                "string-newline-traversal-recovery",
                ".slide::before { content: \"unterminated\n} .slide { background: url(../paper.svg); }",
                "unescaped line breaks",
            ),
            (
                "string-newline-undeclared-recovery",
                ".slide::before { content: \"unterminated\n} .slide { background: url(paper.svg); }",
                "unescaped line breaks",
            ),
            (
                "unterminated-string",
                ".slide::before { content: \"unterminated",
                "unescaped line breaks",
            ),
        ] {
            let theme_dir = write_versioned_theme(temp.path(), name, 1, css, "");
            let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

            let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

            assert!(matches!(&error, ThemeError::InvalidCssUrl { .. }));
            assert!(
                error.to_string().contains(expected_reason),
                "unexpected {name} error: {error}"
            );
        }
        assert_eq!(
            skip_css_string(b"\"unterminated", 0).unwrap_err(),
            "unterminated CSS string"
        );
        let escaped_crlf = b"\"continued\\\r\nstring\"";
        assert_eq!(
            skip_css_string(escaped_crlf, 0).unwrap(),
            escaped_crlf.len()
        );
    }

    #[test]
    fn theme_css_rejects_imports_before_they_bypass_url_validation() {
        let temp = tempdir().unwrap();
        for (name, import) in [
            ("local-import", "@import 'extra.css';"),
            (
                "remote-import",
                "@IMPORT url(https://example.com/theme.css);",
            ),
        ] {
            let theme_dir = write_versioned_theme(temp.path(), name, 1, import, "");
            let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

            let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

            assert!(matches!(&error, ThemeError::UnsupportedCssImport { .. }));
            assert!(
                error
                    .to_string()
                    .contains("bypasses the declared dependency")
            );
        }
    }

    #[test]
    fn theme_css_rejects_image_functions_that_can_hide_string_urls() {
        let temp = tempdir().unwrap();
        for (name, function, css) in [
            (
                "image-set-string",
                "image-set",
                ".slide { background: image-set(\"paper.png\" 1x); }",
            ),
            (
                "webkit-image-set-string",
                "-webkit-image-set",
                ".slide { background: -webkit-image-set(\"paper.png\" 1x); }",
            ),
            (
                "image-string",
                "image",
                ".slide { background: image(\"paper.png\"); }",
            ),
        ] {
            let theme_dir = write_versioned_theme(temp.path(), name, 1, css, "");
            let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

            let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

            assert!(matches!(
                &error,
                ThemeError::UnsupportedCssImageFunction {
                    function: found,
                    ..
                } if *found == function
            ));
            assert!(error.to_string().contains("through url()"));
        }
    }

    #[test]
    fn render_validation_rejects_renderer_owned_dependency_paths_and_prefixes() {
        let temp = tempdir().unwrap();
        for (name, api_version, dependency) in [
            ("v1-reserved", 1, "Reveal.CSS/nested.svg"),
            ("v1-reserved", 1, "ZPRES-THEME-API-V1.CSS/nested.svg"),
            ("reserved-foundation", 1, "REVEAL.CSS"),
            ("v1-cross-api-reserved", 1, "zpres-theme-api-v1.css"),
            ("shared-reserved", 1, "Theme.CSS"),
            ("runtime-reserved", 1, "REVEAL.JS/module.js"),
        ] {
            let theme_dir = write_versioned_theme(temp.path(), name, api_version, "", "");
            fs::write(
                theme_dir.join("theme.toml"),
                format!(
                    "[theme]\nname = \"{name}\"\nversion = \"0.1.0\"\napi_version = {api_version}\nassets = [\"{dependency}\"]\noutput_targets = [\"html\", \"pdf\"]\n"
                ),
            )
            .unwrap();
            let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

            let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

            assert!(matches!(
                &error,
                ThemeError::ReservedDependencyPath {
                    dependency: found,
                    ..
                } if found == dependency
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn render_rejects_screen_and_print_stylesheet_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        for (name, escaped_stylesheet) in [
            ("escaped-screen", "theme.css.tmpl"),
            ("escaped-print", "print.css.tmpl"),
        ] {
            let theme_dir = write_versioned_theme(temp.path(), name, 1, "", "");
            let outside = temp.path().join(format!("{name}-outside.css"));
            fs::write(&outside, ".slide {}").unwrap();
            fs::remove_file(theme_dir.join(escaped_stylesheet)).unwrap();
            symlink(&outside, theme_dir.join(escaped_stylesheet)).unwrap();
            let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();

            let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

            assert!(matches!(
                &error,
                ThemeError::StylesheetOutsideThemeRoot { stylesheet, .. }
                    if stylesheet == escaped_stylesheet
            ));
        }
    }

    #[test]
    fn ordinary_render_and_theme_check_require_declared_fonts() {
        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "font-theme", 1, "", "");
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "font-theme"
version = "0.1.0"
api_version = 1
fonts = ["fonts/body.woff2"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let canonical_theme_dir = fs::canonicalize(&theme_dir).unwrap();

        let render_error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();
        assert!(matches!(
            &render_error,
            ThemeError::MissingReferencedFile { path, .. }
                if path == &canonical_theme_dir.join("fonts/body.woff2")
        ));
        let check_options = ThemeCheckOptions {
            fixture: None,
            ..ThemeCheckOptions::default()
        };
        let check_error = check_theme_package_with_options(&theme_dir, &check_options).unwrap_err();
        assert!(matches!(
            &check_error,
            ThemeError::MissingReferencedFile { path, .. }
                if path == &theme_dir.join("fonts/body.woff2")
        ));

        fs::create_dir_all(theme_dir.join("fonts")).unwrap();
        fs::write(theme_dir.join("fonts/body.woff2"), b"test font").unwrap();

        render_theme(&manifest, &BTreeMap::new()).unwrap();
        let report = check_theme_package_with_options(&theme_dir, &check_options).unwrap();
        assert!(
            report
                .checked_files
                .contains(&theme_dir.join("fonts/body.woff2"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_render_rejects_dependency_symlinks_outside_the_theme_root() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let theme_dir = write_versioned_theme(temp.path(), "contained-theme", 1, "", "");
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "contained-theme"
version = "0.1.0"
api_version = 1
assets = ["assets/paper.png"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::create_dir_all(theme_dir.join("assets")).unwrap();
        let outside = temp.path().join("outside.png");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, theme_dir.join("assets/paper.png")).unwrap();
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let canonical_outside = fs::canonicalize(&outside).unwrap();

        let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

        assert!(matches!(
            &error,
            ThemeError::DependencyOutsideThemeRoot {
                dependency,
                resolved,
                ..
            } if dependency == "assets/paper.png" && resolved == &canonical_outside
        ));
        assert!(error.to_string().contains("outside package root"));
    }

    #[test]
    fn render_theme_requires_current_peer_output_targets() {
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("theme.toml");
        fs::write(
            &manifest_path,
            r#"[theme]
name = "html-only"
version = "0.1.0"
api_version = 1
output_targets = ["html"]
"#,
        )
        .unwrap();
        fs::write(temp.path().join("theme.css.tmpl"), "").unwrap();
        fs::write(temp.path().join("print.css.tmpl"), "").unwrap();
        let manifest = load_theme_manifest(&manifest_path).unwrap();

        let error = render_theme(&manifest, &BTreeMap::new()).unwrap_err();

        assert!(matches!(
            error,
            ThemeError::MissingOutputTarget { target, .. } if target == "pdf"
        ));
    }

    #[test]
    fn best_effort_theme_params_report_discarded_values() {
        let root = builtin_theme_search_path();
        let manifest = load_theme_manifest(&root.join("science").join("theme.toml")).unwrap();

        let (resolved, warnings) = validate_theme_params_best_effort_with_warnings(
            &manifest,
            &BTreeMap::from([
                ("accent".to_string(), "teal".to_string()),
                ("unknown".to_string(), "value".to_string()),
            ]),
        );

        assert_eq!(resolved.get("accent").unwrap(), "#0f766e");
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|warning| warning.contains("accent")));
        assert!(warnings.iter().any(|warning| warning.contains("unknown")));
    }
}
