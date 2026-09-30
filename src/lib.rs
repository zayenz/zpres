use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

mod background_validation;
mod browser_validation;
mod chart;
mod chromium;
mod file_url;
mod native_fs;
mod output_ownership;
mod presentation_plan;
mod publication;
mod raster_publication;

pub mod deck;
pub mod html;
pub mod pdf;
pub mod room_profile;
pub mod server;
pub mod theme;
mod visual;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Parser)]
#[command(name = "zpres")]
#[command(about = "Build presentations from .zp.md source files.")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a starter .zp.md deck.
    Init(DeckInitCommand),
    /// Check a deck without writing output artifacts.
    Check(CheckCommand),
    /// Build an HTML presentation.
    Build(SourceCommand),
    /// Serve a source file while authoring.
    Serve(ServeCommand),
    /// Export a deck to PDF.
    Export(ExportCommand),
    /// Inspect or initialize zpres configuration.
    Config(ConfigCommand),
    /// Inspect or validate zpres themes.
    Theme(ThemeCommand),
}

#[derive(Debug, Args)]
struct DeckInitCommand {
    /// Deck source file to create. Must end in .zp.md.
    path: PathBuf,

    /// Title to write into the starter deck.
    #[arg(long)]
    title: Option<String>,

    /// Author to write into the starter deck.
    #[arg(long)]
    author: Option<String>,

    /// Overwrite the deck source if it already exists.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct CheckCommand {
    /// Source file to read.
    source: PathBuf,

    /// Include speaker-note pages in static readiness page counts.
    #[arg(long)]
    notes: bool,

    /// Treat warnings as fatal diagnostics.
    #[arg(long)]
    strict: bool,

    #[command(flatten)]
    config: CliConfigOverrides,
}

#[derive(Debug, Args)]
struct SourceCommand {
    /// Source file to read.
    source: PathBuf,

    /// Output directory for generated artifacts.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Treat warnings as fatal diagnostics.
    #[arg(long)]
    strict: bool,

    /// Exclude speaker notes and presenter-note controls from live HTML.
    #[arg(long = "exclude-speaker-notes")]
    exclude_speaker_notes: bool,

    #[command(flatten)]
    config: CliConfigOverrides,
}

#[derive(Debug, Args)]
struct ServeCommand {
    /// Source file to read.
    source: PathBuf,

    /// First port to try on 127.0.0.1. Use 0 to let the OS choose a free port.
    #[arg(long, default_value_t = 3000)]
    port: u16,

    /// Debounce delay for rebuilds after filesystem changes.
    #[arg(long, default_value_t = 200)]
    debounce_ms: u64,

    /// Show a floating theme and palette switcher in the live preview.
    #[arg(long = "theme-switcher")]
    theme_switcher: bool,

    #[command(flatten)]
    config: CliConfigOverrides,
}

#[derive(Debug, Args)]
struct ExportCommand {
    /// Source file to read.
    source: PathBuf,

    /// PDF export path.
    #[arg(long)]
    pdf: Option<PathBuf>,

    /// Directory for PNG page exports.
    #[arg(long)]
    png: Option<PathBuf>,

    /// Directory for JPEG page exports.
    #[arg(long)]
    jpg: Option<PathBuf>,

    /// Static page image size, written as WIDTHxHEIGHT.
    #[arg(long = "image-size", visible_alias = "png-size", value_parser = parse_image_size)]
    image_size: Option<pdf::PngViewport>,

    /// JPEG quality from 1 to 100.
    #[arg(long = "jpg-quality", value_parser = parse_jpeg_quality, default_value = "90")]
    jpg_quality: pdf::JpegQuality,

    /// PNG contact sheet path generated from exported PNG pages.
    #[arg(long = "png-contact-sheet")]
    png_contact_sheet: Option<PathBuf>,

    /// Include speaker-note pages after slides that have notes.
    #[arg(long)]
    notes: bool,

    /// Export speaker notes as a plain text script.
    #[arg(long = "notes-txt")]
    notes_txt: Option<PathBuf>,

    /// Export the static print HTML used for PDF/image rendering.
    #[arg(long = "print-html")]
    print_html: Option<PathBuf>,

    /// Treat warnings as fatal diagnostics.
    #[arg(long)]
    strict: bool,

    #[command(flatten)]
    config: CliConfigOverrides,
}

#[derive(Debug, Args)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Args)]
struct ThemeCommand {
    #[command(subcommand)]
    command: ThemeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ThemeSubcommand {
    /// Create a new editable theme package.
    Init(ThemeInitCommand),
    /// Validate a theme package and render its CSS templates.
    Check(ThemeCheckCommand),
}

#[derive(Debug, Args)]
struct ThemeInitCommand {
    /// Theme directory to create.
    path: PathBuf,

    /// Theme slug to write into theme.toml. Defaults to the directory name.
    #[arg(long)]
    name: Option<String>,

    /// Overwrite scaffold files if the directory already exists.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct ThemeCheckCommand {
    /// Theme directory or path to theme.toml.
    path: PathBuf,

    /// Fixture deck to render through the theme during validation.
    #[arg(long)]
    fixture: Option<PathBuf>,

    /// Write the checked fixture as an inspectable HTML theme specimen.
    #[arg(long = "write-specimen")]
    write_specimen: Option<PathBuf>,

    /// Write one specimen per rendered palette variant.
    #[arg(long = "all-variants")]
    all_variants: bool,

    /// Render the fixture in Chromium and write visual review artifacts.
    #[arg(long)]
    visual: bool,

    /// Capture the Debug Theme's contract bounds and orientation annotations.
    #[arg(long)]
    inspection: bool,

    /// Skip rendering a fixture deck during validation.
    #[arg(long)]
    no_fixture: bool,

    /// Room calibration profile used by the visual report.
    #[arg(long = "room-profile", default_value = "projected-room-default")]
    room_profile: String,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Print the global config path.
    Path,
    /// Create the default global config file.
    Init {
        /// Overwrite an existing global config file.
        #[arg(long)]
        force: bool,
    },
    /// Print the resolved configuration.
    Show(ConfigShowCommand),
}

#[derive(Debug, Args)]
struct ConfigShowCommand {
    /// Optional source file whose front matter and project config should be included.
    source: Option<PathBuf>,

    /// Explicit project config path. Defaults to the nearest zpres.toml above the deck root.
    #[arg(long)]
    project_config: Option<PathBuf>,

    #[command(flatten)]
    config: CliConfigOverrides,
}

#[derive(Debug, Args, Clone, Default)]
pub struct CliConfigOverrides {
    /// Theme name.
    #[arg(long)]
    theme: Option<String>,

    /// Theme search path. Repeat to add more paths.
    #[arg(long = "theme-dir")]
    theme_dirs: Vec<PathBuf>,

    /// Theme parameter override, written as key=value.
    #[arg(long = "theme-param")]
    theme_params: Vec<KeyValue>,
}

#[derive(Debug, Clone)]
struct KeyValue {
    key: String,
    value: String,
}

impl std::str::FromStr for KeyValue {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((key, value)) = value.split_once('=') else {
            return Err("expected key=value".to_string());
        };
        let key = key.trim();
        if key.is_empty() {
            return Err("theme parameter key cannot be empty".to_string());
        }
        Ok(Self {
            key: key.to_string(),
            value: value.to_string(),
        })
    }
}

fn parse_image_size(value: &str) -> Result<pdf::PngViewport, String> {
    let Some((width, height)) = value.split_once(['x', 'X']) else {
        return Err("expected WIDTHxHEIGHT, for example 1920x1080".to_string());
    };
    let width = width
        .parse::<u32>()
        .map_err(|_| "PNG width must be a whole number".to_string())?;
    let height = height
        .parse::<u32>()
        .map_err(|_| "PNG height must be a whole number".to_string())?;
    if !(320..=7680).contains(&width) {
        return Err("PNG width must be between 320 and 7680 pixels".to_string());
    }
    if !(180..=4320).contains(&height) {
        return Err("PNG height must be between 180 and 4320 pixels".to_string());
    }
    let viewport = pdf::PngViewport { width, height };
    pdf::validate_raster_output_viewport(viewport).map_err(|error| error.to_string())?;
    Ok(viewport)
}

fn parse_jpeg_quality(value: &str) -> Result<pdf::JpegQuality, String> {
    let quality = value
        .parse::<u8>()
        .map_err(|_| "JPEG quality must be a whole number from 1 to 100".to_string())?;
    if !(1..=100).contains(&quality) {
        return Err("JPEG quality must be between 1 and 100".to_string());
    }
    Ok(pdf::JpegQuality(quality))
}

#[derive(Debug, Error)]
pub enum ZpresError {
    #[error("{0}")]
    Config(#[from] ConfigError),
    #[error("{0}")]
    Parse(#[from] deck::ParseError),
    #[error("{0}")]
    Html(#[from] html::HtmlError),
    #[error("{0}")]
    Pdf(#[from] pdf::PdfError),
    #[error("{0}")]
    Server(#[from] server::ServerError),
    #[error("{0}")]
    Theme(#[from] theme::ThemeError),
    #[error("{0}")]
    RoomProfile(#[from] room_profile::RoomProfileError),
    #[error("cannot publish Output target at {path}: {source}")]
    OutputOwnership {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("cannot use raster Output target at {path}: {source}")]
    RasterOutputPreflight {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("browser-rendered visual review failed: {message}")]
    Visual { message: String },
    #[error("{summary}")]
    DiagnosticsFailed { summary: String },
    #[error("theme package already exists at {path}; pass --force to overwrite scaffold files")]
    ThemePackageExists { path: PathBuf },
    #[error("theme path {path} has no valid directory name; pass --name")]
    ThemePathMissingName { path: PathBuf },
    #[error("deck source already exists at {path}; pass --force to overwrite it")]
    DeckSourceExists { path: PathBuf },
    #[error("deck source path {path} must end with .zp.md")]
    InvalidDeckSourcePath { path: PathBuf },
    #[error("--png-contact-sheet requires --png so page images can be exported first")]
    PngContactSheetRequiresPng,
    #[error(
        "raster Output target {raster_flag} at {raster_path} is an exclusive directory and overlaps {other_label} at {other_path}; choose disjoint paths"
    )]
    RasterOutputTargetOverlap {
        raster_flag: &'static str,
        raster_path: PathBuf,
        other_label: &'static str,
        other_path: PathBuf,
    },
    #[error(
        "export Output target {output_label} at {output_path} overlaps {other_label} at {other_path}; choose disjoint paths"
    )]
    ExportOutputTargetOverlap {
        output_label: &'static str,
        output_path: PathBuf,
        other_label: &'static str,
        other_path: PathBuf,
    },
    #[error(
        "--write-specimen requires a fixture; remove --no-fixture or pass --fixture <deck.zp.md>"
    )]
    ThemeSpecimenRequiresFixture,
    #[error("--all-variants requires --write-specimen <dir>")]
    ThemeSpecimenVariantsRequireOutput,
    #[error("--visual requires --write-specimen <dir>")]
    ThemeVisualRequiresSpecimen,
    #[error("--inspection requires --visual and --write-specimen <dir>")]
    ThemeInspectionRequiresVisual,
    #[error(
        "browser-rendered visual gate failed for {failed_specimens} specimen(s) with {failures} objective failure(s); inspect the written visual reports"
    )]
    ThemeVisualGateFailed {
        failed_specimens: usize,
        failures: usize,
    },
    #[error("slide '{slide_id}' has invalid theme parameter override: {source}")]
    SlideThemeParams {
        slide_id: String,
        source: Box<theme::ThemeError>,
    },
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("failed to serialize resolved config: {0}")]
    SerializeToml(#[from] toml::ser::Error),
}

impl From<visual::VisualError> for ZpresError {
    fn from(error: visual::VisualError) -> Self {
        Self::Visual {
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid TOML in {path}{location}: {source}")]
    Toml {
        path: PathBuf,
        location: String,
        source: Box<toml::de::Error>,
    },
    #[error("invalid source front matter in {path}{location}: {source}")]
    FrontMatter {
        path: PathBuf,
        location: String,
        source: Box<serde_yaml::Error>,
    },
    #[error(
        "unterminated source front matter in {path}:{line}:{column}; expected a closing --- delimiter"
    )]
    UnterminatedFrontMatter {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    #[error("unsupported schema_version {found} in {path}; expected schema_version = 1")]
    UnsupportedSchemaVersion { path: PathBuf, found: u32 },
    #[error("missing schema_version in {path}; expected schema_version = 1")]
    MissingSchemaVersion { path: PathBuf },
    #[error("global config already exists at {path}; pass --force to overwrite it")]
    ConfigExists { path: PathBuf },
    #[error("cannot initialize config at {path}: {source}")]
    OutputOwnership {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("unsupported setting '{setting}' in {path}: {advice}")]
    UnsupportedSetting {
        path: PathBuf,
        setting: &'static str,
        advice: &'static str,
    },
    #[error("could not determine a global config directory; set XDG_CONFIG_HOME")]
    NoConfigDirectory,
    #[error("{0}")]
    Theme(#[from] theme::ThemeError),
    #[error("{0}")]
    RoomProfile(#[from] room_profile::RoomProfileError),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedConfig {
    pub schema_version: u32,
    pub deck_root: PathBuf,
    pub global_config_path: PathBuf,
    pub project_config_path: Option<PathBuf>,
    pub theme: Option<String>,
    pub theme_manifest_path: Option<PathBuf>,
    pub output_dir: PathBuf,
    pub pdf_renderer: String,
    pub pdf_path: Option<PathBuf>,
    pub theme_search_paths: Vec<PathBuf>,
    pub theme_params: BTreeMap<String, String>,
}

impl ResolvedConfig {
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

#[derive(Debug, Clone)]
pub struct ResolveOptions {
    pub source: Option<PathBuf>,
    pub project_config: Option<PathBuf>,
    pub cli_theme: Option<String>,
    pub cli_output_dir: Option<PathBuf>,
    pub cli_pdf_path: Option<PathBuf>,
    pub cli_theme_dirs: Vec<PathBuf>,
    pub cli_theme_params: BTreeMap<String, String>,
    pub strict_theme_params: bool,
}

impl ResolveOptions {
    fn from_check_command(command: CheckCommand) -> Self {
        Self {
            source: Some(command.source),
            project_config: None,
            cli_theme: command.config.theme,
            cli_output_dir: None,
            cli_pdf_path: None,
            cli_theme_dirs: command.config.theme_dirs,
            cli_theme_params: theme_params_from_cli(command.config.theme_params),
            strict_theme_params: true,
        }
    }

    fn from_source_command(command: SourceCommand) -> Self {
        Self {
            source: Some(command.source),
            project_config: None,
            cli_theme: command.config.theme,
            cli_output_dir: command.out,
            cli_pdf_path: None,
            cli_theme_dirs: command.config.theme_dirs,
            cli_theme_params: theme_params_from_cli(command.config.theme_params),
            strict_theme_params: true,
        }
    }

    fn from_export_command(command: ExportCommand) -> Self {
        Self {
            source: Some(command.source),
            project_config: None,
            cli_theme: command.config.theme,
            cli_output_dir: None,
            cli_pdf_path: command.pdf,
            cli_theme_dirs: command.config.theme_dirs,
            cli_theme_params: theme_params_from_cli(command.config.theme_params),
            strict_theme_params: true,
        }
    }

    fn from_serve_command(command: &ServeCommand) -> Self {
        Self {
            source: Some(command.source.clone()),
            project_config: None,
            cli_theme: command.config.theme.clone(),
            cli_output_dir: None,
            cli_pdf_path: None,
            cli_theme_dirs: command.config.theme_dirs.clone(),
            cli_theme_params: theme_params_from_cli(command.config.theme_params.clone()),
            strict_theme_params: false,
        }
    }

    fn from_show_command(command: ConfigShowCommand) -> Self {
        Self {
            source: command.source,
            project_config: command.project_config,
            cli_theme: command.config.theme,
            cli_output_dir: None,
            cli_pdf_path: None,
            cli_theme_dirs: command.config.theme_dirs,
            cli_theme_params: theme_params_from_cli(command.config.theme_params),
            strict_theme_params: true,
        }
    }
}

fn theme_params_from_cli(params: Vec<KeyValue>) -> BTreeMap<String, String> {
    params
        .into_iter()
        .map(|param| (param.key, param.value))
        .collect()
}

#[derive(Debug, Clone)]
struct ConfigPaths {
    global_config: PathBuf,
    home_dir: Option<PathBuf>,
}

impl ConfigPaths {
    fn from_environment() -> Result<Self, ConfigError> {
        Ok(Self {
            global_config: default_global_config_path()?,
            home_dir: dirs::home_dir(),
        })
    }
}

pub fn run_cli() -> Result<(), ZpresError> {
    run(Cli::parse())
}

fn run(cli: Cli) -> Result<(), ZpresError> {
    match cli.command {
        Command::Init(command) => {
            let path = command.path;
            let title = command
                .title
                .unwrap_or_else(|| starter_deck_title_from_path(&path));
            let author = command.author.unwrap_or_else(|| "Your Name".to_string());
            init_deck_source(&path, &title, &author, command.force)?;
            println!("Created starter deck at {}", path.display());
            println!("Run `zpres check {}` to preflight it.", path.display());
        }
        Command::Check(command) => {
            let paths = ConfigPaths::from_environment()?;
            let source = command.source.clone();
            let source_display = source.display().to_string();
            let strict = command.strict;
            let static_export_options = html::StaticExportOptions {
                include_speaker_notes: command.notes,
            };
            let resolved = resolve_config(ResolveOptions::from_check_command(command), &paths)?;
            let mut deck = deck::parse_source_file(source)?;
            apply_resolved_theme_to_deck(&mut deck, &resolved);
            let rendered_theme = render_resolved_theme(&deck, &resolved)?;
            theme::prepare_deck_for_theme(&mut deck, &rendered_theme.manifest)?;
            validate_slide_theme_params(&deck, &rendered_theme)?;
            print_parse_diagnostics(&deck);
            enforce_output_diagnostics(&deck, strict)?;
            let readiness = pdf::check_pdf_readiness_for_theme_with_options(
                &deck,
                &rendered_theme,
                static_export_options,
            )?;
            let print_html = html::render_debug_print_html_with_options(
                &deck,
                &rendered_theme,
                static_export_options,
            );
            pdf::check_print_html_ready(&print_html, readiness.expected_pages)?;
            let browser_preflight = pdf::ChromiumPdfRenderer::discover()?
                .preflight_static_export_with_options(
                    &deck,
                    &rendered_theme,
                    static_export_options,
                )?;
            println!("Deck is ready");
            println!("source = {source_display}");
            println!("theme = {}", rendered_theme.name());
            println!("sections = {}", deck.sections.len());
            println!("static_pages = {}", browser_preflight.observed_pages);
            println!(
                "browser_validated_pages = {}",
                browser_preflight.validated_pages
            );
            println!("checked_math_blocks = {}", readiness.checked_math_blocks);
            println!("checked_chart_blocks = {}", readiness.checked_chart_blocks);
            println!("checked_image_blocks = {}", readiness.checked_image_blocks);
            println!("checked_media_blocks = {}", readiness.checked_media_blocks);
            for warning in &readiness.warnings {
                eprintln!("warning: {warning}");
            }
        }
        Command::Build(command) => {
            let paths = ConfigPaths::from_environment()?;
            let source = command.source.clone();
            let strict = command.strict;
            let html_options = html::LiveHtmlOptions {
                include_speaker_notes: !command.exclude_speaker_notes,
            };
            let resolved = resolve_config(ResolveOptions::from_source_command(command), &paths)?;
            let mut deck = deck::parse_source_file(source)?;
            apply_resolved_theme_to_deck(&mut deck, &resolved);
            let rendered_theme = render_resolved_theme(&deck, &resolved)?;
            theme::prepare_deck_for_theme(&mut deck, &rendered_theme.manifest)?;
            validate_slide_theme_params(&deck, &rendered_theme)?;
            print_parse_diagnostics(&deck);
            enforce_output_diagnostics(&deck, strict)?;
            let publication = html::write_debug_html_bundle_with_options(
                &deck,
                &rendered_theme,
                &resolved.output_dir,
                html_options,
            )?;
            for warning in &publication.warnings {
                eprintln!("warning: {warning}");
            }
            println!(
                "Built debug HTML presentation with {} section(s) at {}",
                deck.sections.len(),
                resolved.output_dir.join("index.html").display()
            );
        }
        Command::Serve(command) => {
            let paths = ConfigPaths::from_environment()?;
            let resolved = resolve_config(ResolveOptions::from_serve_command(&command), &paths)?;
            let cli_theme_params = theme_params_from_cli(command.config.theme_params.clone());
            server::serve(server::ServeOptions {
                source: command.source,
                theme_search_paths: resolved.theme_search_paths,
                fallback_theme: resolved.theme,
                fallback_theme_params: resolved.theme_params,
                cli_theme_override: command.config.theme,
                cli_theme_params,
                port: command.port,
                debounce: std::time::Duration::from_millis(command.debounce_ms),
                theme_switcher: command.theme_switcher,
            })?;
        }
        Command::Export(command) => {
            let paths = ConfigPaths::from_environment()?;
            let source = command.source.clone();
            let strict = command.strict;
            let png_dir = command.png.clone();
            let jpg_dir = command.jpg.clone();
            let image_viewport = command.image_size.unwrap_or_default();
            let jpg_quality = command.jpg_quality;
            let png_contact_sheet = command.png_contact_sheet.clone();
            let notes_txt = command.notes_txt.clone();
            let print_html = command.print_html.clone();
            if png_contact_sheet.is_some() && png_dir.is_none() {
                return Err(ZpresError::PngContactSheetRequiresPng);
            }
            let explicit_pdf = command.pdf.is_some();
            let explicit_image_export = png_dir.is_some() || jpg_dir.is_some();
            let explicit_notes_text_export = notes_txt.is_some();
            let explicit_print_html_export = print_html.is_some();
            let static_export_options = html::StaticExportOptions {
                include_speaker_notes: command.notes,
            };
            let resolved = resolve_config(ResolveOptions::from_export_command(command), &paths)?;
            let png_dir =
                png_dir.map(|path| resolve_export_path(path, &resolved.deck_root, &paths.home_dir));
            let jpg_dir =
                jpg_dir.map(|path| resolve_export_path(path, &resolved.deck_root, &paths.home_dir));
            let png_contact_sheet = png_contact_sheet
                .map(|path| resolve_export_path(path, &resolved.deck_root, &paths.home_dir));
            let notes_txt = notes_txt
                .map(|path| resolve_export_path(path, &resolved.deck_root, &paths.home_dir));
            let print_html = print_html
                .map(|path| resolve_export_path(path, &resolved.deck_root, &paths.home_dir));
            let should_export_pdf = explicit_pdf
                || (!explicit_image_export
                    && !explicit_notes_text_export
                    && !explicit_print_html_export)
                || resolved.pdf_path.is_some();
            let pdf_path = should_export_pdf.then(|| {
                let path = resolved
                    .pdf_path
                    .clone()
                    .unwrap_or_else(|| resolved.output_dir.join("deck.pdf"));
                lexically_normalize_absolute_path(&path)
            });
            let output_layout = ExportOutputLayout {
                png_dir: png_dir.as_deref(),
                jpg_dir: jpg_dir.as_deref(),
                png_contact_sheet: png_contact_sheet.as_deref(),
                notes_txt: notes_txt.as_deref(),
                print_html: print_html.as_deref(),
                pdf: pdf_path.as_deref(),
            };
            validate_export_output_layout(&source, &resolved, output_layout)?;
            let mut deck = deck::parse_source_file(source)?;
            apply_resolved_theme_to_deck(&mut deck, &resolved);
            let rendered_theme = render_resolved_theme(&deck, &resolved)?;
            theme::prepare_deck_for_theme(&mut deck, &rendered_theme.manifest)?;
            validate_export_dependency_layout(&deck, &rendered_theme, output_layout)?;
            preflight_export_output_ownership(output_layout)?;
            validate_slide_theme_params(&deck, &rendered_theme)?;
            print_parse_diagnostics(&deck);
            enforce_output_diagnostics(&deck, strict)?;
            let renderer = if should_export_pdf || explicit_image_export {
                Some(pdf::ChromiumPdfRenderer::discover()?)
            } else {
                None
            };
            if let Some(notes_txt) = notes_txt {
                write_speaker_notes_text(&deck, &notes_txt)?;
                println!("Exported speaker notes at {}", notes_txt.display());
            }
            if let Some(print_html) = print_html {
                let readiness = write_print_html_export(
                    &deck,
                    &rendered_theme,
                    &print_html,
                    static_export_options,
                )?;
                for warning in &readiness.warnings {
                    eprintln!("warning: {warning}");
                }
                println!(
                    "Exported print HTML with {} page(s) at {}",
                    readiness.expected_pages,
                    print_html.display()
                );
            }
            if should_export_pdf {
                let pdf_path = pdf_path
                    .as_deref()
                    .expect("PDF output path exists when PDF export is selected");
                let report = renderer.as_ref().unwrap().export_with_options(
                    &deck,
                    &rendered_theme,
                    pdf_path,
                    static_export_options,
                )?;
                for warning in &report.warnings {
                    eprintln!("warning: {warning}");
                }
                println!(
                    "Exported debug PDF with {} page(s), {} byte(s), at {}",
                    report.observed_pages,
                    report.bytes,
                    report.path.display()
                );
            }
            if let Some(png_dir) = png_dir {
                let report = renderer.as_ref().unwrap().export_png_pages_with_viewport(
                    &deck,
                    &rendered_theme,
                    &png_dir,
                    static_export_options,
                    image_viewport,
                )?;
                for warning in &report.warnings {
                    eprintln!("warning: {warning}");
                }
                println!(
                    "Exported debug PNG pages with {} page(s) at {} ({}x{})",
                    report.exported_pages,
                    report.output_dir.display(),
                    report.viewport.width,
                    report.viewport.height
                );
                if let Some(contact_sheet) = png_contact_sheet {
                    let contact_report = renderer
                        .as_ref()
                        .unwrap()
                        .export_png_contact_sheet_for_report(&report, &contact_sheet)?;
                    println!(
                        "Exported PNG contact sheet with {} page(s) at {}",
                        contact_report.page_count,
                        contact_report.path.display()
                    );
                }
            }
            if let Some(jpg_dir) = jpg_dir {
                let report = renderer.as_ref().unwrap().export_jpeg_pages_with_viewport(
                    &deck,
                    &rendered_theme,
                    &jpg_dir,
                    static_export_options,
                    image_viewport,
                    jpg_quality,
                )?;
                for warning in &report.warnings {
                    eprintln!("warning: {warning}");
                }
                println!(
                    "Exported debug JPEG pages with {} page(s) at {} ({}x{}, quality {})",
                    report.exported_pages,
                    report.output_dir.display(),
                    report.viewport.width,
                    report.viewport.height,
                    report.quality.0
                );
            }
        }
        Command::Config(command) => {
            let paths = ConfigPaths::from_environment()?;
            run_config_command(command, &paths)?;
        }
        Command::Theme(command) => run_theme_command(command)?,
    }
    Ok(())
}

fn apply_resolved_theme_to_deck(deck: &mut deck::Deck, resolved: &ResolvedConfig) {
    if resolved.theme.is_some() {
        deck.metadata.theme = resolved.theme.clone();
    }
    deck.metadata.theme_params = resolved.theme_params.clone();
}

fn render_resolved_theme(
    deck: &deck::Deck,
    resolved: &ResolvedConfig,
) -> Result<theme::RenderedTheme, theme::ThemeError> {
    let deck_root = deck.deck_root().unwrap_or(&resolved.deck_root);
    theme::render_deck_theme(
        deck.metadata.theme.as_deref(),
        deck_root,
        &resolved.theme_search_paths,
        &deck.metadata.theme_params,
    )
}

fn validate_slide_theme_params(
    deck: &deck::Deck,
    rendered_theme: &theme::RenderedTheme,
) -> Result<(), ZpresError> {
    for slide in deck.pdf_slide_order() {
        if slide.theme_params.is_empty() {
            continue;
        }
        let mut supplied = rendered_theme.params.clone();
        supplied.extend(slide.theme_params.clone());
        theme::validate_theme_params(&rendered_theme.manifest, &supplied).map_err(|source| {
            ZpresError::SlideThemeParams {
                slide_id: slide.id.clone(),
                source: Box::new(source),
            }
        })?;
    }
    Ok(())
}

fn print_parse_diagnostics(deck: &deck::Deck) {
    for diagnostic in &deck.diagnostics {
        eprintln!(
            "{}: {:?}: {}",
            diagnostic_location(diagnostic),
            diagnostic.severity,
            diagnostic.message
        );
    }
}

fn enforce_output_diagnostics(deck: &deck::Deck, strict: bool) -> Result<(), ZpresError> {
    let fatal_count = deck
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_fatal() || strict)
        .count();
    if fatal_count == 0 {
        return Ok(());
    }

    let warning_suffix = if strict {
        " (--strict treats warnings as fatal)"
    } else {
        ""
    };
    Err(ZpresError::DiagnosticsFailed {
        summary: format!("{fatal_count} fatal diagnostic(s){warning_suffix}; output not written"),
    })
}

fn diagnostic_location(diagnostic: &deck::Diagnostic) -> String {
    match &diagnostic.span {
        Some(span) => {
            let path = span
                .source_path
                .as_ref()
                .map_or_else(|| "<source>".to_string(), |path| path.display().to_string());
            format!("{}:{}:{}", path, span.line, span.column)
        }
        None => "<unknown>".to_string(),
    }
}

fn run_config_command(command: ConfigCommand, paths: &ConfigPaths) -> Result<(), ZpresError> {
    match command.command {
        ConfigSubcommand::Path => {
            println!("{}", paths.global_config.display());
        }
        ConfigSubcommand::Init { force } => {
            init_global_config(&paths.global_config, force)?;
            println!("Created {}", paths.global_config.display());
        }
        ConfigSubcommand::Show(command) => {
            let resolved = resolve_config(ResolveOptions::from_show_command(command), paths)?;
            print!("{}", resolved.to_toml_string()?);
        }
    }
    Ok(())
}

fn run_theme_command(command: ThemeCommand) -> Result<(), ZpresError> {
    match command.command {
        ThemeSubcommand::Init(command) => {
            let path = command.path;
            let name = match command.name {
                Some(name) => name,
                None => path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| ZpresError::ThemePathMissingName { path: path.clone() })?
                    .to_string(),
            };
            init_theme_package(&path, &name, command.force)?;
            println!("Created theme '{}' at {}", name, path.display());
            println!(
                "Run `zpres theme check {}` before using it.",
                path.display()
            );
        }
        ThemeSubcommand::Check(command) => {
            if command.no_fixture && command.write_specimen.is_some() {
                return Err(ZpresError::ThemeSpecimenRequiresFixture);
            }
            if command.all_variants && command.write_specimen.is_none() {
                return Err(ZpresError::ThemeSpecimenVariantsRequireOutput);
            }
            if command.visual && command.write_specimen.is_none() {
                return Err(ZpresError::ThemeVisualRequiresSpecimen);
            }
            if command.inspection && (!command.visual || command.write_specimen.is_none()) {
                return Err(ZpresError::ThemeInspectionRequiresVisual);
            }
            let explicitly_selected_fixture = command.fixture.is_some();
            let room_profile = room_profile::resolve(&command.room_profile, &env::current_dir()?)?;
            let fixture = if command.no_fixture {
                None
            } else {
                command
                    .fixture
                    .or_else(|| local_theme_specimen_path(&command.path))
                    .or_else(|| theme::ThemeCheckOptions::default().fixture)
            };
            let write_specimen = command.write_specimen.clone();
            let report = theme::check_theme_package_with_options(
                &command.path,
                &theme::ThemeCheckOptions {
                    fixture: fixture.clone(),
                    require_complete_fixture_feature_coverage: !(command.visual
                        && explicitly_selected_fixture),
                },
            )?;
            println!("Theme '{}' {} is valid", report.name, report.version);
            println!("contract_status = valid");
            if !command.visual {
                println!("visual_status = not-run");
            }
            println!("manifest = {}", report.manifest_path.display());
            println!("checked_files = {}", report.checked_files.len());
            println!(
                "theme_parameters = {}",
                theme_parameter_list_or_none(&report.theme_parameters)
            );
            println!(
                "unused_theme_parameters = {}",
                comma_list_or_none(&report.unused_theme_parameters)
            );
            println!(
                "rendered_variants = {}",
                report.rendered_variants.join(", ")
            );
            println!(
                "declared_feature_hooks = {}",
                comma_list_or_none(&report.declared_feature_hooks)
            );
            println!(
                "declared_modules = {}",
                comma_list_or_none(&report.declared_modules)
            );
            println!(
                "unknown_selectors = {}",
                comma_list_or_none(&report.unknown_selectors)
            );
            println!(
                "dead_selectors = {}",
                comma_list_or_none(&report.dead_selectors)
            );
            if let Some(fixture) = &report.fixture {
                println!(
                    "fixture = {} ({} section(s), {} pdf page(s))",
                    fixture.source_path.display(),
                    fixture.sections,
                    fixture.expected_pdf_pages
                );
                println!(
                    "fixture_variants = {}",
                    comma_list_or_none(&fixture.slide_variants)
                );
                println!(
                    "fixture_missing_declared_variants = {}",
                    comma_list_or_none(&fixture.missing_declared_slide_variants)
                );
                println!(
                    "fixture_presets = {}",
                    comma_list_or_none(&fixture.slide_presets)
                );
                println!(
                    "fixture_classes = {}",
                    comma_list_or_none(&fixture.slide_classes)
                );
                println!(
                    "fixture_blocks = {}",
                    comma_list_or_none(&fixture.content_blocks)
                );
                println!(
                    "fixture_layouts = {}",
                    comma_list_or_none(&fixture.layout_kinds)
                );
                println!(
                    "fixture_media = {}",
                    comma_list_or_none(&fixture.media_kinds)
                );
                println!(
                    "fixture_features = {}",
                    comma_list_or_none(&fixture.feature_hooks)
                );
                println!(
                    "fixture_feature_coverage = {}",
                    if fixture.missing_declared_slide_variants.is_empty()
                        && fixture.missing_declared_feature_hooks.is_empty()
                    {
                        "complete"
                    } else {
                        "incomplete"
                    }
                );
                println!(
                    "fixture_missing_declared_features = {}",
                    comma_list_or_none(&fixture.missing_declared_feature_hooks)
                );
                println!(
                    "fixture_uncovered_features = {}",
                    comma_list_or_none(&fixture.uncovered_feature_hooks)
                );
                println!(
                    "fixture_dead_selectors = {}",
                    comma_list_or_none(&fixture.dead_selectors)
                );
            }
            let mut failed_visual_specimens = 0usize;
            let mut visual_failures = 0usize;
            if let Some(output_dir) = write_specimen {
                let Some(fixture_path) = fixture.as_deref() else {
                    return Err(ZpresError::ThemeSpecimenRequiresFixture);
                };
                let Some(fixture_report) = report.fixture.as_ref() else {
                    return Err(ZpresError::ThemeSpecimenRequiresFixture);
                };
                if command.all_variants {
                    let specimens = write_theme_specimens_for_variants(
                        &command.path,
                        fixture_path,
                        fixture_report,
                        &output_dir,
                        command.visual,
                        command.inspection,
                        &room_profile,
                    )?;
                    println!(
                        "specimens = {} ({})",
                        output_dir.display(),
                        specimens
                            .iter()
                            .map(|specimen| specimen.label.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    for specimen in specimens {
                        println!(
                            "specimen_variant = {} at {} (review.html, index.html, print.html, print-notes.html, speaker-notes.txt, theme-check.txt, theme-api.txt, {} pdf page(s), {} notes page(s))",
                            specimen.label,
                            specimen.output_dir.display(),
                            specimen.expected_pdf_pages,
                            specimen.expected_notes_pdf_pages
                        );
                        if let Some(visual_report) = &specimen.visual_report {
                            print_theme_visual_status(&specimen.output_dir, visual_report);
                            if visual_report.visual_status.is_failed() {
                                failed_visual_specimens += 1;
                                visual_failures += visual_report.failures.len();
                            }
                        }
                    }
                } else {
                    let specimen = write_theme_specimen(
                        &command.path,
                        fixture_path,
                        fixture_report,
                        &output_dir,
                        command.visual,
                        command.inspection,
                        &room_profile,
                    )?;
                    println!(
                        "specimen = {} (review.html, index.html, print.html, print-notes.html, speaker-notes.txt, theme-check.txt, theme-api.txt, {} pdf page(s), {} notes page(s))",
                        specimen.output_dir.display(),
                        specimen.expected_pdf_pages,
                        specimen.expected_notes_pdf_pages
                    );
                    if let Some(visual_report) = &specimen.visual_report {
                        print_theme_visual_status(&specimen.output_dir, visual_report);
                        if visual_report.visual_status.is_failed() {
                            failed_visual_specimens += 1;
                            visual_failures += visual_report.failures.len();
                        }
                    }
                }
            }
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
            if failed_visual_specimens > 0 {
                return Err(ZpresError::ThemeVisualGateFailed {
                    failed_specimens: failed_visual_specimens,
                    failures: visual_failures,
                });
            }
        }
    }
    Ok(())
}

fn local_theme_specimen_path(theme_path: &Path) -> Option<PathBuf> {
    let package_dir = if theme_path.is_dir() {
        theme_path
    } else {
        theme_path.parent()?
    };
    let specimen = package_dir.join("specimen.zp.md");
    specimen.is_file().then_some(specimen)
}

fn comma_list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn theme_parameter_list_or_none(parameters: &[theme::ThemeParameterCheckReport]) -> String {
    if parameters.is_empty() {
        return "none".to_string();
    }
    parameters
        .iter()
        .map(|parameter| {
            let value = parameter.default.as_deref().unwrap_or({
                if parameter.required {
                    "required"
                } else {
                    "optional"
                }
            });
            format!("{}:{}={}", parameter.name, parameter.parameter_type, value)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn print_theme_visual_status(output_dir: &Path, report: &visual::VisualReport) {
    let status = |value: &visual::VisualStatus| match value {
        visual::VisualStatus::Passed => "passed",
        visual::VisualStatus::Failed => "failed",
    };
    let visual_status = status(&report.visual_status);
    let release_status = match &report.release_status {
        visual::ReleaseStatus::PendingReview => "pending-review",
        visual::ReleaseStatus::Blocked => "blocked",
    };
    println!(
        "objective_gate_status = {}",
        status(&report.objective_gate_status)
    );
    println!("visual_status = {visual_status}");
    println!("screen_status = {}", status(&report.screen_status));
    println!("print_status = {}", status(&report.print_status));
    println!("composition_review_status = required");
    println!("human_review = required");
    println!("final_source_release_approval = {release_status}");
    println!("release_status = {release_status}");
    println!(
        "visual_screen_slides = {}/{}",
        report.captured_screen_slides, report.expected_screen_slides
    );
    println!(
        "visual_screen_states = {}/{}",
        report.captured_screen_states, report.expected_screen_states
    );
    println!(
        "visual_pages = {}/{}",
        report.captured_pages, report.expected_pages
    );
    println!("visual_warnings = {}", report.warnings.len());
    println!("visual_failures = {}", report.failures.len());
    println!("visual_artifacts = {}", output_dir.display());
}

fn init_deck_source(path: &Path, title: &str, author: &str, force: bool) -> Result<(), ZpresError> {
    if !is_zp_markdown_path(path) {
        return Err(ZpresError::InvalidDeckSourcePath {
            path: path.to_path_buf(),
        });
    }
    if path.exists() && !force {
        return Err(ZpresError::DeckSourceExists {
            path: path.to_path_buf(),
        });
    }

    let namespace = output_ownership::OutputNamespaceGuard::acquire(path).map_err(|source| {
        ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    if path.exists() && !force {
        return Err(ZpresError::DeckSourceExists {
            path: path.to_path_buf(),
        });
    }
    let publication = namespace
        .publish_file(path, starter_deck_source(title, author).as_bytes())
        .map_err(|source| ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    for warning in publication.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn is_zp_markdown_path(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.ends_with(".zp.md"))
}

fn starter_deck_title_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_suffix(".zp.md"))
        .map(humanize_deck_title)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Untitled zpres deck".to_string())
}

fn humanize_deck_title(slug: &str) -> String {
    slug.replace(['-', '_'], " ")
        .split_whitespace()
        .map(capitalize_ascii_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_ascii_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut result = first.to_ascii_uppercase().to_string();
    result.push_str(chars.as_str());
    result
}

fn starter_deck_source(title: &str, author: &str) -> String {
    DECK_SCAFFOLD_SOURCE
        .replace("__TITLE_YAML__", &yaml_double_quote(title))
        .replace("__AUTHOR_YAML__", &yaml_double_quote(author))
        .replace("__TITLE__", title)
}

fn yaml_double_quote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn init_theme_package(path: &Path, name: &str, force: bool) -> Result<(), ZpresError> {
    theme::validate_theme_name(name)?;
    if path.exists() && !force {
        return Err(ZpresError::ThemePackageExists {
            path: path.to_path_buf(),
        });
    }

    let manifest = theme_scaffold_manifest(name).into_bytes();
    let specimen = theme_specimen_source(name).into_bytes();
    let files = vec![
        (path.join("theme.toml"), manifest),
        (
            path.join("theme.css.tmpl"),
            theme_scaffold_css(THEME_SCAFFOLD_SCREEN_CSS, name).into_bytes(),
        ),
        (
            path.join("print.css.tmpl"),
            theme_scaffold_css(THEME_SCAFFOLD_PRINT_CSS, name).into_bytes(),
        ),
        (path.join("specimen.zp.md"), specimen),
        (
            path.join("assets").join("specimen-visual.svg"),
            THEME_SPECIMEN_VISUAL_SVG.as_bytes().to_vec(),
        ),
        (
            path.join("assets").join("specimen-clip.mp4"),
            THEME_SPECIMEN_CLIP_MP4.to_vec(),
        ),
        (
            path.join("assets").join("specimen-voice.mp3"),
            THEME_SPECIMEN_VOICE_MP3.to_vec(),
        ),
        (
            path.join("data").join("specimen-runtime.csv"),
            THEME_SPECIMEN_RUNTIME_CSV.as_bytes().to_vec(),
        ),
    ];
    let namespace = output_ownership::OutputNamespaceGuard::acquire(path).map_err(|source| {
        ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    if path.exists() && !force {
        return Err(ZpresError::ThemePackageExists {
            path: path.to_path_buf(),
        });
    }
    namespace
        .ensure_peer_output_allowed(path, output_ownership::OutputTargetKind::Directory)
        .map_err(|source| ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    for (target, _) in &files {
        namespace
            .ensure_peer_output_allowed(target, output_ownership::OutputTargetKind::File)
            .map_err(|source| ZpresError::OutputOwnership {
                path: target.clone(),
                source: Box::new(source),
            })?;
    }
    fs::create_dir_all(path)?;
    for (target, bytes) in files {
        let publication = namespace.publish_file(&target, &bytes).map_err(|source| {
            ZpresError::OutputOwnership {
                path: target.clone(),
                source: Box::new(source),
            }
        })?;
        for warning in publication.warnings {
            eprintln!("warning: {warning}");
        }
    }
    Ok(())
}

fn theme_scaffold_manifest(name: &str) -> String {
    THEME_SCAFFOLD_MANIFEST.replace("__THEME_NAME__", name)
}

fn theme_scaffold_css(template: &str, name: &str) -> String {
    template.replace("__THEME_NAME__", name)
}

fn theme_specimen_source(name: &str) -> String {
    THEME_SPECIMEN_SOURCE.replace("__THEME_NAME__", name)
}

const DECK_SCAFFOLD_SOURCE: &str = r#"---
title: "__TITLE_YAML__"
author: "__AUTHOR_YAML__"
aspect: "16:9"
---

# __TITLE__

::: variant section-title
:::

A short zpres deck generated for theme and talk authoring.

::: notes
Replace this with your opening presenter note.
:::

---

# Main claim

::: variant claim
:::

::: class lead
:::

::: slide
autoscale: true
:::

[fit] Strong presentations usually make one claim per slide.

* Keep each slide focused.
* Let the theme handle visual hierarchy.
* Use typed blocks when the content has intent.

^ Presenter notes can use this caret shorthand when you want Deckset-style authoring.

---

# Reveal the reasoning

::: steps pdf="pages"
1. Start with the model.
2. Add the constraint that changes the outcome.
3. Show the consequence.
:::

$$
x_{t+1} = f(x_t, u_t)
$$

---

# Compare alternatives

::::: comparison
:::: primary label="Baseline"
Clear and familiar.

Tradeoff: less tailored.
::::

:::: supporting label="Alternative"
Better fit.

Tradeoff: more moving parts.
::::
:::::

---

# Closing

The next step is to replace this scaffold with your actual story.
"#;

const THEME_SCAFFOLD_MANIFEST: &str = r##"[theme]
name = "__THEME_NAME__"
version = "0.1.0"
api_version = 1
fonts = []
assets = []
stylesheet = "theme.css.tmpl"
print_stylesheet = "print.css.tmpl"
palette_parameter = "mode"
output_targets = ["html", "pdf"]
modules = ["scientific-data"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
feature_hooks = [
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
]

[presets.spotlight]
variant = "claim"
classes = ["lead"]
autoscale = true
transition = "fade"

[presets.spotlight.theme_params]
accent = "#256f6c"

[style]
family = "editorial"
inspiration = []

[parameters.mode]
type = "enum"
default = "light"
values = ["light", "dark"]

[parameters.background]
type = "color"
default = "#f3f4ef"

[parameters.surface]
type = "color"
default = "#fffdf7"

[parameters.text]
type = "color"
default = "#17211d"

[parameters.muted]
type = "color"
default = "#5f6f68"

[parameters.accent]
type = "color"
default = "#256f6c"

[parameters.accent_alt]
type = "color"
default = "#b15f3b"

[parameters.rule]
type = "color"
default = "#ccd7cf"

[parameters.font_body]
type = "font"
default = 'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'

[parameters.font_heading]
type = "font"
default = 'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'

[parameters.font_mono]
type = "font"
default = 'ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace'

[color_variants.light]
background = "#f3f4ef"
surface = "#fffdf7"
text = "#17211d"
muted = "#5f6f68"
accent = "#256f6c"
accent_alt = "#b15f3b"
rule = "#ccd7cf"

[color_variants.dark]
background = "#111816"
surface = "#18221f"
text = "#f4f7f2"
muted = "#a8bab2"
accent = "#69d5c7"
accent_alt = "#f1a06c"
rule = "#334640"
"##;

const THEME_SPECIMEN_SOURCE: &str = r##"---
title: "Theme specimen"
author: "zpres"
theme: "__THEME_NAME__"
aspect: "16:9"
autoscale: true
transition: "fade"
footer: "Generated theme specimen"
slide_numbers: true
background_image:
  src: "assets/specimen-visual.svg"
  intent: contextual
  alt: "Generated abstract background"
  position: "center 48%"
  dim: 90
  grayscale: 18
  saturate: 78
  splash: true
theme_params:
  mode: "light"
---

image-corner-radius: 18

# Theme specimen

::: variant section-title
:::

This local deck is generated with the theme package. Use it to inspect the
screen view, print view, and semantic coverage before styling a real talk.

::: notes
Start here when checking whether the title slide and notes styling feel right.
:::

---

# One strong claim

::: preset spotlight
:::

Common authoring shapes should feel natural[^intent].

* Use lists for staged emphasis.
* Keep the claim visually dominant.
* Let the theme carry rhythm and contrast.

::: steps
1. Read the claim.
2. Notice the supporting evidence.
3. Decide what should be memorable.
:::

[^intent]: Footnotes are part of the theme surface, not an afterthought.

--

## Detail: Voice and contract

[fit] One memorable line.

> Change the voice, not the Source file.

> [!NOTE] Theme contract
> Fit text, quotes, callouts, classes, footnotes, and notes belong to one contract.

---

# Derivation and code

::: variant derivation
:::

```rust reveal="1-2|3"
fn score(signal: f64) -> f64 {
    signal.max(0.0).sqrt()
}
```

```mermaid
flowchart LR
  Idea[Intent] --> Theme[Theme hooks]
  Theme --> Export[Static export]
```

--

## Detail: Dense comparison

::: variant dense
:::

| Element | Theme question | Hook |
| --- | --- | --- |
| Table | Is dense data readable? | `.zpres-block-table` |
| Code | Is monospace contrast clear? | `.zpres-block-code` |
| Notes | Are private notes hidden? | `.zpres-speaker-notes-source` |

$$
H = \text{structure} + \text{contrast} + \text{rhythm}
$$

---

# Runtime chart

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/specimen-runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "size", "type": "quantitative" },
    "y": { "field": "runtime_ms", "type": "quantitative" },
    "color": { "field": "model", "type": "nominal" }
  }
}
:::

---

# Visual treatment

::: variant figure
:::

::: background src="assets/specimen-visual.svg" intent="contextual" alt="Abstract specimen background" position="center 44%" dim="84" grayscale="24" saturate="82"
:::

::: figure src="assets/specimen-visual.svg" alt="Abstract specimen visual" caption="A generated local SVG keeps the starter theme self-contained." width="72%" height="44vh" fit="contain" align="center" dim="8" grayscale="12" saturate="96" blur="1" radius="18"
:::

---

# Gallery and media

::: variant figure
:::

![inline fill columns=2 corner-radius(12) alt="Specimen before state"](assets/specimen-visual.svg "Before")
![inline fit dim="18" grayscale="25" radius=0.75rem alt="Specimen after state"](assets/specimen-visual.svg "After")

![video right 48% fill mute autoadvance poster="assets/specimen-visual.svg" title="Generated clip" alt="Generated clip poster"](assets/specimen-clip.mp4?t=2s "A local placeholder video checks poster and static media fallbacks.")

![audio hide](assets/specimen-voice.mp3?t=4s)

---

# Side-by-side story

![bg right:38% intent="evidence" alt="Split specimen visual" description="The abstract split visual anchors the structural comparison on the left against its supporting evidence on the right."](assets/specimen-visual.svg)

::::: comparison
:::: primary label="Structure"

- structure
- contrast
- rhythm
::::

:::: supporting label="Purpose"
A theme should keep related ideas visually related without asking authors to
write custom CSS inside their talk.
::::
:::::

--

## Detail: Grid matrix

::::: grid tracks="2/1/1" gap="4" align="start"
:::: cell name="Primary" column="1" span="2"
The main result occupies the wider evidence region.
::::
:::: cell name="Boundary" column="3"
State the condition beside it.
::::
:::::

--

## Detail: Stack sequence

::::: stack gap="3" align="stretch"
:::: item name="Question"
What must the audience understand first?
::::
:::: item name="Answer"
The consequence follows in source order.
::::
:::::

--

## Detail: Overlay annotation

::::: overlay overlap="edge-only"
:::: base name="Evidence"
The base region keeps the complete evidence visible.
::::
:::: annotation name="Result" anchor="top-end" width="compact"
The annotation marks the result without covering it.
::::
:::::

--

## Detail: Aside context

::::: aside supporting="compact" gap="4" align="start"
:::: primary name="Argument"
Primary evidence retains the dominant reading path.
::::
:::: supporting name="Context"
Short context remains visibly subordinate.
::::
:::::

--

## Detail: export-only HTML

This detail slide checks that backup slides and HTML-only blocks remain visible
to the theme contract.

::: html
<aside class="fixture-callout">HTML-only specimen block</aside>
:::

--

## Detail: step pages

This detail slide checks one-page-per-step export policy.

::: steps pdf="pages"
1. First export page.
2. Second export page.
:::
"##;

const THEME_SPECIMEN_RUNTIME_CSV: &str = r#"size,model,runtime_ms
10,baseline,42
20,baseline,95
30,baseline,180
10,theme-aware,35
20,theme-aware,71
30,theme-aware,128
"#;

const THEME_SPECIMEN_VISUAL_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" role="img" aria-labelledby="title desc">
  <title id="title">Theme specimen visual</title>
  <desc id="desc">A compact abstract illustration used by generated zpres theme packages.</desc>
  <rect width="640" height="360" fill="#f8fafc"/>
  <rect x="56" y="46" width="528" height="268" rx="28" fill="#fff7ed" stroke="#0f766e" stroke-width="4"/>
  <path d="M92 258 C162 178 218 118 304 158 C390 198 426 118 548 82" fill="none" stroke="#256f6c" stroke-width="10" stroke-linecap="round"/>
  <path d="M104 286 L196 224 L286 238 L398 174 L532 132" fill="none" stroke="#b15f3b" stroke-width="5" stroke-dasharray="14 10" stroke-linecap="round"/>
  <circle cx="304" cy="158" r="24" fill="#facc15" stroke="#a16207" stroke-width="4"/>
  <circle cx="398" cy="174" r="18" fill="#fb7185" stroke="#be123c" stroke-width="4"/>
  <text x="92" y="102" font-family="Arial, sans-serif" font-size="26" fill="#17211d">theme surface</text>
  <text x="366" y="292" font-family="Arial, sans-serif" font-size="22" fill="#17211d">visual rhythm</text>
</svg>
"##;

const THEME_SPECIMEN_CLIP_MP4: &[u8] = b"zpres theme specimen placeholder video";
const THEME_SPECIMEN_VOICE_MP3: &[u8] = b"zpres theme specimen placeholder audio";

const THEME_SCAFFOLD_SCREEN_CSS: &str = r#"/*
 * Theme API v1 supplies its layout and technical-block foundation as compiled,
 * offline CSS. Keep this package as a small visual overlay: no Tailwind runtime,
 * CDN, or copied foundation stylesheet is needed.
 */
.zpres-theme-__THEME_NAME__ {
  --starter-background: {{param.background}};
  --starter-surface: {{param.surface}};
  --starter-text: {{param.text}};
  --starter-muted: {{param.muted}};
  --starter-accent: {{param.accent}};
  --starter-accent-alt: {{param.accent_alt}};
  --starter-rule: {{param.rule}};
  color: var(--starter-text);
  background: var(--starter-background);
}

.zpres-theme-__THEME_NAME__ .zpres-slide-frame {
  color: var(--starter-text);
  background: var(--starter-surface);
  box-shadow:
    inset 7px 0 0 var(--starter-accent),
    inset 0 0 0 1px var(--starter-rule),
    0 24px 70px rgb(23 33 29 / 0.14);
}

.zpres-theme-__THEME_NAME__ .zpres-slide-header {
  padding-block-end: var(--spacing-slide-4);
  border-block-end: 1px solid var(--starter-rule);
}

.zpres-theme-__THEME_NAME__ .zpres-slide-title {
  max-width: 24ch;
  color: var(--starter-text);
  font-family: {{param.font_heading}};
  font-weight: 720;
  letter-spacing: -0.025em;
}

.zpres-theme-__THEME_NAME__ :where(.zpres-slide-body, .zpres-slide-primary) {
  color: var(--starter-text);
  font-family: {{param.font_body}};
}

.zpres-theme-__THEME_NAME__ :where(.zpres-slide-sources, .zpres-slide-footer) {
  color: var(--starter-muted);
  border-color: var(--starter-rule);
}

.zpres-theme-__THEME_NAME__ .zpres-slide-footer-number {
  color: var(--starter-accent);
  font-weight: 750;
}

.zpres-theme-__THEME_NAME__ :where(.zpres-block-code, .zpres-block-math, .zpres-block-table) {
  font-family: {{param.font_mono}};
}

.zpres-theme-__THEME_NAME__ .zpres-block-code pre {
  color: #f4f7f2;
  background: #101815;
  box-shadow: inset 4px 0 0 var(--starter-accent);
}

.zpres-theme-__THEME_NAME__ :where(.zpres-block-callout, .zpres-block-quote blockquote) {
  border-inline-start-color: var(--starter-accent);
}

.zpres-theme-__THEME_NAME__ .zpres-slide[data-slide-variant="claim"] .zpres-slide-primary {
  border-inline-start-color: var(--starter-accent-alt);
}

.zpres-theme-__THEME_NAME__ .zpres-slide[data-slide-variant="section-title"] .zpres-slide-content {
  align-content: center;
}

.zpres-theme-__THEME_NAME__ .zpres-slide[data-slide-variant="section-title"] .zpres-slide-header {
  border-block-end: 0;
}

.zpres-theme-__THEME_NAME__ .zpres-slide[data-slide-role="detail"] .zpres-slide-frame {
  background: color-mix(in srgb, var(--starter-surface) 94%, var(--starter-background));
}
"#;

const THEME_SCAFFOLD_PRINT_CSS: &str = r#"/* Print is a bounded Theme overlay; v1 owns 16:9 page geometry and pagination. */
.zpres-theme-__THEME_NAME__ .zpres-print-slide .zpres-slide-frame {
  background: var(--starter-surface);
  box-shadow:
    inset 7px 0 0 var(--starter-accent),
    inset 0 0 0 1px var(--starter-rule);
}

.zpres-theme-__THEME_NAME__ .zpres-print-slide :where(.zpres-slide-sources, .zpres-slide-footer) {
  color: var(--starter-muted);
  border-color: var(--starter-rule);
}
"#;

pub fn default_global_config_path() -> Result<PathBuf, ConfigError> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(config_home).join("zpres").join("zpres.toml"));
    }

    let Some(config_dir) = dirs::config_dir() else {
        return Err(ConfigError::NoConfigDirectory);
    };
    Ok(config_dir.join("zpres").join("zpres.toml"))
}

pub fn init_global_config(path: &Path, force: bool) -> Result<(), ConfigError> {
    if path.exists() && !force {
        return Err(ConfigError::ConfigExists {
            path: path.to_path_buf(),
        });
    }

    let namespace = output_ownership::OutputNamespaceGuard::acquire(path).map_err(|source| {
        ConfigError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    if path.exists() && !force {
        return Err(ConfigError::ConfigExists {
            path: path.to_path_buf(),
        });
    }
    let publication = namespace
        .publish_file(path, DEFAULT_GLOBAL_CONFIG.as_bytes())
        .map_err(|source| ConfigError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    for warning in publication.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

const DEFAULT_GLOBAL_CONFIG: &str = r#"schema_version = 1

[paths]
theme_dirs = []
"#;

fn resolve_config(
    options: ResolveOptions,
    paths: &ConfigPaths,
) -> Result<ResolvedConfig, ConfigError> {
    let cwd = env::current_dir().map_err(|source| ConfigError::Read {
        path: PathBuf::from("."),
        source,
    })?;
    let source = options.source.clone().map(|path| absolutize(&cwd, path));
    let deck_root = source
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| cwd.clone());

    let mut state = ConfigState {
        output_dir: deck_root.join("dist"),
        pdf_renderer: "chromium".to_string(),
        ..ConfigState::default()
    };

    if paths.global_config.exists() {
        let layer = load_toml_layer(&paths.global_config, &paths.global_config, &paths.home_dir)?;
        state.apply_low_precedence(layer);
    }

    let project_config_path = options
        .project_config
        .clone()
        .map(|path| absolutize(&cwd, path))
        .or_else(|| find_project_config(&deck_root));

    if let Some(project_config) = &project_config_path {
        let layer = load_toml_layer(project_config, project_config, &paths.home_dir)?;
        state.apply_low_precedence(layer);
    }

    if let Some(source_path) = &source
        && source_path.exists()
        && let Some(layer) = load_front_matter_layer(source_path, &deck_root, &paths.home_dir)?
    {
        state.apply_low_precedence(layer);
    }

    let strict_theme_params = options.strict_theme_params;
    state.apply_cli(options, &deck_root, &paths.home_dir);
    let theme_manifest_path = if let Some(theme_name) = state.theme.as_deref() {
        let manifest = theme::load_named_theme(theme_name, &deck_root, &state.theme_dirs)?;
        if strict_theme_params {
            state.theme_params = theme::validate_theme_params(&manifest, &state.theme_params)?;
        }
        Some(manifest.path)
    } else {
        None
    };

    Ok(ResolvedConfig {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        deck_root,
        global_config_path: paths.global_config.clone(),
        project_config_path,
        theme: state.theme,
        theme_manifest_path,
        output_dir: state.output_dir,
        pdf_renderer: state.pdf_renderer,
        pdf_path: state.pdf_path,
        theme_search_paths: state.theme_dirs,
        theme_params: state.theme_params,
    })
}

#[derive(Debug, Default)]
struct ConfigState {
    theme: Option<String>,
    output_dir: PathBuf,
    pdf_renderer: String,
    pdf_path: Option<PathBuf>,
    theme_dirs: Vec<PathBuf>,
    theme_params: BTreeMap<String, String>,
}

impl ConfigState {
    fn apply_low_precedence(&mut self, layer: ConfigLayer) {
        if let Some(theme) = layer.theme {
            self.theme = Some(theme);
        }
        if let Some(output_dir) = layer.output_dir {
            self.output_dir = output_dir;
        }
        if let Some(pdf_renderer) = layer.pdf_renderer {
            self.pdf_renderer = pdf_renderer;
        }
        if let Some(pdf_path) = layer.pdf_path {
            self.pdf_path = Some(pdf_path);
        }
        if !layer.theme_dirs.is_empty() {
            self.theme_dirs.extend(layer.theme_dirs);
        }
        self.theme_params.extend(layer.theme_params);
    }

    fn apply_cli(&mut self, options: ResolveOptions, deck_root: &Path, home_dir: &Option<PathBuf>) {
        if let Some(theme) = options.cli_theme {
            if self.theme.as_deref() != Some(theme.as_str()) {
                self.theme_params.clear();
            }
            self.theme = Some(theme);
        }
        if let Some(output_dir) = options.cli_output_dir {
            self.output_dir = resolve_path(output_dir, deck_root, home_dir);
        }
        if let Some(pdf_path) = options.cli_pdf_path {
            self.pdf_path = Some(resolve_path(pdf_path, deck_root, home_dir));
        }
        if !options.cli_theme_dirs.is_empty() {
            let theme_dirs = options
                .cli_theme_dirs
                .into_iter()
                .map(|path| resolve_path(path, deck_root, home_dir));
            self.theme_dirs.extend(theme_dirs);
        }
        self.theme_params.extend(options.cli_theme_params);
    }
}

#[derive(Debug, Default)]
struct ConfigLayer {
    theme: Option<String>,
    output_dir: Option<PathBuf>,
    pdf_renderer: Option<String>,
    pdf_path: Option<PathBuf>,
    theme_dirs: Vec<PathBuf>,
    theme_params: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    schema_version: Option<u32>,
    deck: Option<RawDeckConfig>,
    paths: Option<RawPathsConfig>,
    pdf: Option<RawPdfConfig>,
    theme: Option<RawThemeConfig>,
}

#[derive(Debug, Deserialize)]
struct RawDeckConfig {
    theme: Option<String>,
    output_dir: Option<PathBuf>,
    room_profile: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPathsConfig {
    #[serde(default)]
    theme_dirs: Vec<PathBuf>,
    output_root: Option<PathBuf>,
    cache_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct RawPdfConfig {
    renderer: Option<String>,
    path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct RawThemeConfig {
    #[serde(default)]
    params: BTreeMap<String, ConfigValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

impl std::fmt::Display for ConfigValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigValue::String(value) => value.fmt(formatter),
            ConfigValue::Integer(value) => value.fmt(formatter),
            ConfigValue::Float(value) => value.fmt(formatter),
            ConfigValue::Boolean(value) => value.fmt(formatter),
        }
    }
}

fn load_toml_layer(
    path: &Path,
    display_path: &Path,
    home_dir: &Option<PathBuf>,
) -> Result<ConfigLayer, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: display_path.to_path_buf(),
        source,
    })?;
    let raw: RawConfig = toml::from_str(&text).map_err(|source| {
        let location = toml_location(&text, &source);
        ConfigError::Toml {
            path: display_path.to_path_buf(),
            location,
            source: Box::new(source),
        }
    })?;
    validate_schema(raw.schema_version, display_path)?;
    validate_config_settings(
        raw.paths.as_ref(),
        raw.pdf.as_ref(),
        raw.deck.as_ref(),
        None,
        display_path,
    )?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(layer_from_raw_config(raw, base, home_dir))
}

fn validate_schema(schema_version: Option<u32>, path: &Path) -> Result<(), ConfigError> {
    match schema_version {
        Some(SUPPORTED_SCHEMA_VERSION) => Ok(()),
        Some(found) => Err(ConfigError::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            found,
        }),
        None => Err(ConfigError::MissingSchemaVersion {
            path: path.to_path_buf(),
        }),
    }
}

fn validate_config_settings(
    paths: Option<&RawPathsConfig>,
    pdf: Option<&RawPdfConfig>,
    deck: Option<&RawDeckConfig>,
    source_room_profile: Option<&str>,
    path: &Path,
) -> Result<(), ConfigError> {
    let unsupported = |setting, advice| ConfigError::UnsupportedSetting {
        path: path.to_path_buf(),
        setting,
        advice,
    };
    if let Some(paths) = paths {
        if paths.output_root.is_some() {
            return Err(unsupported(
                "paths.output_root",
                "use deck.output_dir or --out",
            ));
        }
        if paths.cache_root.is_some() {
            return Err(unsupported(
                "paths.cache_root",
                "the embedded Theme cache uses the platform cache directory",
            ));
        }
    }
    if pdf
        .and_then(|pdf| pdf.renderer.as_deref())
        .is_some_and(|renderer| renderer != "chromium")
    {
        return Err(unsupported(
            "pdf.renderer",
            "the supported renderer is chromium",
        ));
    }
    if source_room_profile.is_some() || deck.is_some_and(|deck| deck.room_profile.is_some()) {
        return Err(unsupported(
            "room_profile",
            "select a room profile with theme check --visual --room-profile",
        ));
    }
    Ok(())
}

fn layer_from_raw_config(raw: RawConfig, base: &Path, home_dir: &Option<PathBuf>) -> ConfigLayer {
    let mut layer = ConfigLayer::default();

    if let Some(deck) = raw.deck {
        layer.theme = deck.theme;
        layer.output_dir = deck
            .output_dir
            .map(|path| resolve_path(path, base, home_dir));
    }

    if let Some(paths) = raw.paths {
        layer.theme_dirs = paths
            .theme_dirs
            .into_iter()
            .map(|path| resolve_path(path, base, home_dir))
            .collect();
    }

    if let Some(pdf) = raw.pdf {
        layer.pdf_renderer = pdf.renderer;
        layer.pdf_path = pdf.path.map(|path| resolve_path(path, base, home_dir));
    }

    if let Some(theme) = raw.theme {
        layer.theme_params = theme
            .params
            .into_iter()
            .map(|(key, value)| (key, value.to_string()))
            .collect();
    }

    layer
}

#[derive(Debug, Deserialize)]
struct RawFrontMatter {
    theme: Option<String>,
    room_profile: Option<String>,
    output_dir: Option<PathBuf>,
    #[serde(default)]
    theme_dirs: Vec<PathBuf>,
    #[serde(default)]
    theme_params: BTreeMap<String, ConfigValue>,
    deck: Option<RawDeckConfig>,
    paths: Option<RawPathsConfig>,
    pdf: Option<RawPdfConfig>,
}

fn load_front_matter_layer(
    path: &Path,
    deck_root: &Path,
    home_dir: &Option<PathBuf>,
) -> Result<Option<ConfigLayer>, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let front_matter = match deck::split_front_matter(&text) {
        deck::FrontMatterSplit::Valid { front_matter, .. } => front_matter,
        deck::FrontMatterSplit::Absent { .. } => return Ok(None),
        deck::FrontMatterSplit::Unterminated => {
            return Err(ConfigError::UnterminatedFrontMatter {
                path: path.to_path_buf(),
                line: 1,
                column: 1,
            });
        }
    };

    let raw: RawFrontMatter = serde_yaml::from_str(front_matter).map_err(|source| {
        let location = yaml_location(&source);
        ConfigError::FrontMatter {
            path: path.to_path_buf(),
            location,
            source: Box::new(source),
        }
    })?;

    validate_config_settings(
        raw.paths.as_ref(),
        raw.pdf.as_ref(),
        raw.deck.as_ref(),
        raw.room_profile.as_deref(),
        path,
    )?;

    let mut layer = ConfigLayer {
        theme: raw.theme,
        output_dir: raw
            .output_dir
            .map(|path| resolve_path(path, deck_root, home_dir)),
        theme_dirs: raw
            .theme_dirs
            .into_iter()
            .map(|path| resolve_path(path, deck_root, home_dir))
            .collect(),
        theme_params: raw
            .theme_params
            .into_iter()
            .map(|(key, value)| (key, value.to_string()))
            .collect(),
        ..ConfigLayer::default()
    };

    if let Some(deck) = raw.deck {
        if deck.theme.is_some() {
            layer.theme = deck.theme;
        }
        if let Some(output_dir) = deck.output_dir {
            layer.output_dir = Some(resolve_path(output_dir, deck_root, home_dir));
        }
    }

    if let Some(paths) = raw.paths {
        layer.theme_dirs.extend(
            paths
                .theme_dirs
                .into_iter()
                .map(|path| resolve_path(path, deck_root, home_dir)),
        );
    }

    if let Some(pdf) = raw.pdf {
        layer.pdf_renderer = pdf.renderer;
        layer.pdf_path = pdf.path.map(|path| resolve_path(path, deck_root, home_dir));
    }

    Ok(Some(layer))
}

#[cfg(test)]
fn extract_yaml_front_matter(text: &str) -> Option<&str> {
    match deck::split_front_matter(text) {
        deck::FrontMatterSplit::Valid { front_matter, .. } => Some(front_matter),
        deck::FrontMatterSplit::Absent { .. } | deck::FrontMatterSplit::Unterminated => None,
    }
}

fn toml_location(text: &str, error: &toml::de::Error) -> String {
    error.span().map_or_else(String::new, |span| {
        let (line, column) = line_column_for_offset(text, span.start);
        format!(":{line}:{column}")
    })
}

fn yaml_location(error: &serde_yaml::Error) -> String {
    error.location().map_or_else(String::new, |location| {
        format!(":{}:{}", location.line(), location.column())
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

fn find_project_config(deck_root: &Path) -> Option<PathBuf> {
    let mut current = deck_root.to_path_buf();
    loop {
        let candidate = current.join("zpres.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn resolve_path(path: PathBuf, base: &Path, home_dir: &Option<PathBuf>) -> PathBuf {
    if let Some(expanded) = expand_tilde(&path, home_dir) {
        return expanded;
    }
    if path.is_absolute() {
        return path;
    }
    base.join(path)
}

fn resolve_export_path(path: PathBuf, base: &Path, home_dir: &Option<PathBuf>) -> PathBuf {
    lexically_normalize_absolute_path(&resolve_path(path, base, home_dir))
}

#[derive(Debug, Clone, Copy)]
struct ExportOutputLayout<'a> {
    png_dir: Option<&'a Path>,
    jpg_dir: Option<&'a Path>,
    png_contact_sheet: Option<&'a Path>,
    notes_txt: Option<&'a Path>,
    print_html: Option<&'a Path>,
    pdf: Option<&'a Path>,
}

#[derive(Debug)]
struct ComparablePath {
    label: &'static str,
    path: PathBuf,
    identity: ComparableFileSystemPath,
}

impl ComparablePath {
    fn new(label: &'static str, path: &Path) -> Result<Self, ZpresError> {
        Ok(Self {
            label,
            path: path.to_path_buf(),
            identity: comparable_output_path(path)?,
        })
    }
}

fn validate_export_output_layout(
    source: &Path,
    resolved: &ResolvedConfig,
    layout: ExportOutputLayout<'_>,
) -> Result<(), ZpresError> {
    let (raster_directories, file_outputs) = comparable_export_targets(layout)?;
    let mut protected_inputs = vec![ComparablePath::new("the Deck source", source)?];
    let mut raster_only_inputs = Vec::new();
    if let Some(path) = resolved.project_config_path.as_deref() {
        protected_inputs.push(ComparablePath::new("the project configuration", path)?);
    }
    if let Some(path) = resolved.theme_manifest_path.as_deref() {
        protected_inputs.push(ComparablePath::new("the Theme manifest", path)?);
        raster_only_inputs.push(ComparablePath::new(
            "the Theme package",
            path.parent().unwrap_or(path),
        )?);
    }
    protected_inputs.push(ComparablePath::new(
        "the global configuration",
        &resolved.global_config_path,
    )?);

    validate_output_target_collections(
        &raster_directories,
        &file_outputs,
        &protected_inputs,
        &raster_only_inputs,
    )
}

fn preflight_export_output_ownership(layout: ExportOutputLayout<'_>) -> Result<(), ZpresError> {
    let requested = layout
        .png_dir
        .or(layout.jpg_dir)
        .or(layout.png_contact_sheet)
        .or(layout.notes_txt)
        .or(layout.print_html)
        .or(layout.pdf)
        .expect("an export command always selects at least one Output target");
    let namespace =
        output_ownership::OutputNamespaceGuard::acquire(requested).map_err(|source| {
            ZpresError::OutputOwnership {
                path: requested.to_path_buf(),
                source: Box::new(source),
            }
        })?;
    for path in [
        layout.png_contact_sheet,
        layout.notes_txt,
        layout.print_html,
        layout.pdf,
    ]
    .into_iter()
    .flatten()
    {
        namespace
            .ensure_peer_output_allowed(path, output_ownership::OutputTargetKind::File)
            .map_err(|source| ZpresError::OutputOwnership {
                path: path.to_path_buf(),
                source: Box::new(source),
            })?;
    }
    for (path, format) in [
        layout
            .png_dir
            .map(|path| (path, raster_publication::RasterFormat::Png)),
        layout
            .jpg_dir
            .map(|path| (path, raster_publication::RasterFormat::Jpeg)),
    ]
    .into_iter()
    .flatten()
    {
        raster_publication::preflight_raster_page_set_destination_under_namespace(
            &namespace, path, format,
        )
        .map_err(|source| ZpresError::RasterOutputPreflight {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    }
    Ok(())
}

fn validate_export_dependency_layout(
    deck: &deck::Deck,
    rendered_theme: &theme::RenderedTheme,
    layout: ExportOutputLayout<'_>,
) -> Result<(), ZpresError> {
    let (raster_directories, file_outputs) = comparable_export_targets(layout)?;
    let mut dependencies = Vec::new();
    if let Some(deck_root) = deck.deck_root() {
        for dependency in deck.local_dependency_references() {
            dependencies.push(ComparablePath::new(
                "a Deck dependency",
                &lexically_normalize_absolute_path(&deck_root.join(dependency)),
            )?);
        }
    }
    if let Some(theme_root) = rendered_theme.manifest.path.parent() {
        let mut theme_dependencies = vec![
            rendered_theme.manifest.path.clone(),
            theme_root.join(&rendered_theme.manifest.stylesheet),
            theme_root.join(&rendered_theme.manifest.print_stylesheet),
        ];
        theme_dependencies.extend(
            theme::theme_dependency_paths(&rendered_theme.manifest)
                .map(|dependency| theme_root.join(dependency)),
        );
        for dependency in theme_dependencies {
            dependencies.push(ComparablePath::new("a Theme dependency", &dependency)?);
        }
        let theme_package = ComparablePath::new("the Theme package", theme_root)?;
        for raster in &raster_directories {
            reject_raster_overlap(raster, &theme_package)?;
        }
    }
    validate_outputs_against_protected(&raster_directories, &file_outputs, &dependencies)
}

fn comparable_export_targets(
    layout: ExportOutputLayout<'_>,
) -> Result<(Vec<ComparablePath>, Vec<ComparablePath>), ZpresError> {
    let mut raster_directories = Vec::new();
    if let Some(path) = layout.png_dir {
        raster_directories.push(ComparablePath::new("--png", path)?);
    }
    if let Some(path) = layout.jpg_dir {
        raster_directories.push(ComparablePath::new("--jpg", path)?);
    }

    let mut file_outputs = Vec::new();
    for (label, path) in [
        ("--png-contact-sheet", layout.png_contact_sheet),
        ("--notes-txt", layout.notes_txt),
        ("--print-html", layout.print_html),
        ("--pdf", layout.pdf),
    ] {
        if let Some(path) = path {
            file_outputs.push(ComparablePath::new(label, path)?);
        }
    }
    Ok((raster_directories, file_outputs))
}

fn validate_output_target_collections(
    raster_directories: &[ComparablePath],
    file_outputs: &[ComparablePath],
    protected_inputs: &[ComparablePath],
    raster_only_inputs: &[ComparablePath],
) -> Result<(), ZpresError> {
    for (index, output) in file_outputs.iter().enumerate() {
        for other_output in file_outputs.iter().skip(index + 1) {
            reject_export_output_overlap(output, other_output)?;
        }
    }

    validate_outputs_against_protected(raster_directories, file_outputs, protected_inputs)?;
    for raster in raster_directories {
        for protected in raster_only_inputs {
            reject_raster_overlap(raster, protected)?;
        }
    }

    for (index, raster) in raster_directories.iter().enumerate() {
        for other_raster in raster_directories.iter().skip(index + 1) {
            reject_raster_overlap(raster, other_raster)?;
        }
        for file_output in file_outputs {
            reject_raster_overlap(raster, file_output)?;
        }
    }
    Ok(())
}

fn validate_outputs_against_protected(
    raster_directories: &[ComparablePath],
    file_outputs: &[ComparablePath],
    protected_inputs: &[ComparablePath],
) -> Result<(), ZpresError> {
    for raster in raster_directories {
        for protected in protected_inputs {
            reject_raster_overlap(raster, protected)?;
        }
    }
    for output in file_outputs {
        for protected in protected_inputs {
            reject_export_output_overlap(output, protected)?;
        }
    }
    Ok(())
}

fn reject_raster_overlap(
    raster: &ComparablePath,
    other: &ComparablePath,
) -> Result<(), ZpresError> {
    if paths_overlap(&raster.identity, &other.identity) {
        return Err(ZpresError::RasterOutputTargetOverlap {
            raster_flag: raster.label,
            raster_path: raster.path.clone(),
            other_label: other.label,
            other_path: other.path.clone(),
        });
    }
    Ok(())
}

fn reject_export_output_overlap(
    output: &ComparablePath,
    other: &ComparablePath,
) -> Result<(), ZpresError> {
    if paths_overlap(&output.identity, &other.identity) {
        return Err(ZpresError::ExportOutputTargetOverlap {
            output_label: output.label,
            output_path: output.path.clone(),
            other_label: other.label,
            other_path: other.path.clone(),
        });
    }
    Ok(())
}

fn paths_overlap(left: &ComparableFileSystemPath, right: &ComparableFileSystemPath) -> bool {
    comparable_path_is_equal_or_within(left, right)
        || comparable_path_is_equal_or_within(right, left)
}

pub(crate) fn path_is_equal_or_within(path: &Path, directory: &Path) -> bool {
    match (
        comparable_output_path(path),
        comparable_output_path(directory),
    ) {
        (Ok(path), Ok(directory)) => comparable_path_is_equal_or_within(&path, &directory),
        _ => true,
    }
}

fn portable_path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| portable_path_component(component.as_os_str()))
        .collect()
}

fn portable_path_component(component: &OsStr) -> String {
    component
        .to_string_lossy()
        .nfkc()
        .case_fold()
        .nfkc()
        .collect()
}

#[derive(Debug, Clone)]
struct ComparableFileSystemPath {
    normalized: PathBuf,
    anchors: Vec<PhysicalPathAnchor>,
}

#[derive(Debug, Clone)]
struct PhysicalPathAnchor {
    object: PhysicalObjectIdentity,
    relative: Vec<String>,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhysicalObjectIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PhysicalObjectIdentity {
    canonical_path: PathBuf,
}

fn comparable_path_is_equal_or_within(
    path: &ComparableFileSystemPath,
    directory: &ComparableFileSystemPath,
) -> bool {
    if path.normalized.starts_with(&directory.normalized) {
        return true;
    }
    let path_components = portable_path_components(&path.normalized);
    let directory_components = portable_path_components(&directory.normalized);
    if path_components.starts_with(&directory_components) {
        return true;
    }
    path.anchors.iter().any(|path_anchor| {
        directory.anchors.iter().any(|directory_anchor| {
            path_anchor.object == directory_anchor.object
                && path_anchor.relative.starts_with(&directory_anchor.relative)
        })
    })
}

fn comparable_output_path(path: &Path) -> io::Result<ComparableFileSystemPath> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    let normalized = lexically_normalize_absolute_path(&absolute);
    let mut ancestor = normalized.clone();
    let mut suffix = Vec::<OsString>::new();

    loop {
        match fs::canonicalize(&ancestor) {
            Ok(canonical) => {
                let relative = suffix.into_iter().rev().collect::<Vec<_>>();
                let mut normalized = canonical.clone();
                for component in &relative {
                    normalized.push(component);
                }
                return Ok(ComparableFileSystemPath {
                    normalized: lexically_normalize_absolute_path(&normalized),
                    anchors: physical_path_anchors(&canonical, &relative)?,
                });
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                let Some(component) = ancestor.file_name().map(OsStr::to_os_string) else {
                    return Err(error);
                };
                suffix.push(component);
                if !ancestor.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn physical_path_anchors(
    existing_path: &Path,
    unresolved_suffix: &[OsString],
) -> io::Result<Vec<PhysicalPathAnchor>> {
    let mut current = existing_path.to_path_buf();
    let mut relative = unresolved_suffix
        .iter()
        .map(|component| portable_path_component(component))
        .collect::<Vec<_>>();
    let mut anchors = Vec::new();
    loop {
        let metadata = fs::metadata(&current)?;
        anchors.push(PhysicalPathAnchor {
            object: physical_object_identity(&metadata, &current),
            relative: relative.clone(),
        });
        let Some(name) = current.file_name().map(OsStr::to_os_string) else {
            break;
        };
        if !current.pop() {
            break;
        }
        relative.insert(0, portable_path_component(&name));
    }
    Ok(anchors)
}

#[cfg(unix)]
fn physical_object_identity(metadata: &fs::Metadata, _path: &Path) -> PhysicalObjectIdentity {
    use std::os::unix::fs::MetadataExt;

    PhysicalObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn physical_object_identity(_metadata: &fs::Metadata, path: &Path) -> PhysicalObjectIdentity {
    PhysicalObjectIdentity {
        canonical_path: path.to_path_buf(),
    }
}

fn lexically_normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                }
            }
            std::path::Component::Normal(component) => normalized.push(component),
        }
    }
    normalized
}

#[derive(Debug)]
struct ThemeSpecimenExportReport {
    label: String,
    output_dir: PathBuf,
    expected_pdf_pages: usize,
    expected_notes_pdf_pages: usize,
    visual_report: Option<visual::VisualReport>,
}

struct ThemeSpecimenOptions<'a> {
    label: &'a str,
    params: &'a BTreeMap<String, String>,
    visual: bool,
    inspection: bool,
    room_profile: &'a room_profile::ResolvedRoomProfile,
}

fn write_theme_specimen(
    theme_path: &Path,
    fixture_path: &Path,
    fixture_report: &theme::ThemeFixtureCheckReport,
    output_dir: &Path,
    visual: bool,
    inspection: bool,
    room_profile: &room_profile::ResolvedRoomProfile,
) -> Result<ThemeSpecimenExportReport, ZpresError> {
    let manifest_path = if theme_path.is_dir() {
        theme_path.join("theme.toml")
    } else {
        theme_path.to_path_buf()
    };
    let manifest = theme::load_theme_manifest(&manifest_path)?;
    write_theme_specimen_with_params(
        &manifest,
        fixture_path,
        fixture_report,
        output_dir,
        ThemeSpecimenOptions {
            label: "defaults",
            params: &BTreeMap::new(),
            visual,
            inspection,
            room_profile,
        },
    )
}

fn write_theme_specimens_for_variants(
    theme_path: &Path,
    fixture_path: &Path,
    fixture_report: &theme::ThemeFixtureCheckReport,
    output_dir: &Path,
    visual: bool,
    inspection: bool,
    room_profile: &room_profile::ResolvedRoomProfile,
) -> Result<Vec<ThemeSpecimenExportReport>, ZpresError> {
    let manifest_path = if theme_path.is_dir() {
        theme_path.join("theme.toml")
    } else {
        theme_path.to_path_buf()
    };
    let manifest = theme::load_theme_manifest(&manifest_path)?;
    let mut reports = Vec::new();
    reports.push(write_theme_specimen_with_params(
        &manifest,
        fixture_path,
        fixture_report,
        &output_dir.join("defaults"),
        ThemeSpecimenOptions {
            label: "defaults",
            params: &BTreeMap::new(),
            visual,
            inspection,
            room_profile,
        },
    )?);
    if manifest
        .parameters
        .contains_key(&manifest.palette_parameter)
    {
        for variant in manifest.color_variants.keys() {
            let params = BTreeMap::from([(manifest.palette_parameter.clone(), variant.clone())]);
            reports.push(write_theme_specimen_with_params(
                &manifest,
                fixture_path,
                fixture_report,
                &output_dir.join(safe_specimen_path_segment(variant)),
                ThemeSpecimenOptions {
                    label: variant,
                    params: &params,
                    visual,
                    inspection,
                    room_profile,
                },
            )?);
        }
    }
    Ok(reports)
}

fn write_theme_specimen_with_params(
    manifest: &theme::ThemeManifest,
    fixture_path: &Path,
    fixture_report: &theme::ThemeFixtureCheckReport,
    output_dir: &Path,
    options: ThemeSpecimenOptions<'_>,
) -> Result<ThemeSpecimenExportReport, ZpresError> {
    let rendered_theme = theme::render_theme(manifest, options.params)?;
    let mut deck = deck::parse_source_file(fixture_path)?;
    deck.metadata.theme = Some(rendered_theme.name().to_string());
    deck.metadata.theme_params = rendered_theme.params.clone();
    theme::prepare_deck_for_theme(&mut deck, &rendered_theme.manifest)?;
    validate_slide_theme_params(&deck, &rendered_theme)?;
    enforce_output_diagnostics(&deck, false)?;

    let publication = html::write_debug_html_bundle(&deck, &rendered_theme, output_dir)?;
    for warning in &publication.warnings {
        eprintln!("warning: {warning}");
    }
    let readiness = write_print_html_export(
        &deck,
        &rendered_theme,
        &output_dir.join("print.html"),
        html::StaticExportOptions::default(),
    )?;
    let notes_readiness = write_print_html_export(
        &deck,
        &rendered_theme,
        &output_dir.join("print-notes.html"),
        html::StaticExportOptions {
            include_speaker_notes: true,
        },
    )?;
    write_speaker_notes_text(&deck, &output_dir.join("speaker-notes.txt"))?;
    write_theme_specimen_check_report(
        output_dir,
        options.label,
        fixture_report,
        &rendered_theme,
        &publication,
        (readiness.expected_pages, notes_readiness.expected_pages),
        options.visual,
    )?;
    let visual_report = if options.visual {
        Some(visual::write_theme_visual_review(
            &deck,
            &rendered_theme,
            fixture_path,
            output_dir,
            visual::VisualReviewOptions {
                inspection: options.inspection,
                room_profile: options.room_profile.clone(),
                ..visual::VisualReviewOptions::default()
            },
        )?)
    } else {
        None
    };
    Ok(ThemeSpecimenExportReport {
        label: options.label.to_string(),
        output_dir: output_dir.to_path_buf(),
        expected_pdf_pages: readiness.expected_pages,
        expected_notes_pdf_pages: notes_readiness.expected_pages,
        visual_report,
    })
}

fn write_theme_specimen_check_report(
    output_dir: &Path,
    label: &str,
    fixture: &theme::ThemeFixtureCheckReport,
    rendered_theme: &theme::RenderedTheme,
    publication: &html::HtmlBundlePublicationReport,
    page_counts: (usize, usize),
    visual: bool,
) -> Result<(), ZpresError> {
    let (expected_pdf_pages, expected_notes_pdf_pages) = page_counts;
    let manifest = &rendered_theme.manifest;
    let mut report = String::new();
    report.push_str("# zpres theme specimen check\n\n");
    report.push_str(&format!("specimen_variant = {label}\n"));
    report.push_str(&format!("fixture = {}\n", fixture.source_path.display()));
    report.push_str(&format!("sections = {}\n", fixture.sections));
    report.push_str(&format!("pdf_pages = {expected_pdf_pages}\n"));
    report.push_str(&format!("notes_pdf_pages = {expected_notes_pdf_pages}\n"));
    report.push_str(&format!("html_bytes = {}\n", fixture.html_bytes));
    report.push_str(&format!(
        "print_html_bytes = {}\n",
        fixture.print_html_bytes
    ));
    report.push_str(&format!(
        "fixture_variants = {}\n",
        comma_list_or_none(&fixture.slide_variants)
    ));
    report.push_str(&format!(
        "fixture_missing_declared_variants = {}\n",
        comma_list_or_none(&fixture.missing_declared_slide_variants)
    ));
    report.push_str(&format!(
        "fixture_presets = {}\n",
        comma_list_or_none(&fixture.slide_presets)
    ));
    report.push_str(&format!(
        "fixture_classes = {}\n",
        comma_list_or_none(&fixture.slide_classes)
    ));
    report.push_str(&format!(
        "fixture_blocks = {}\n",
        comma_list_or_none(&fixture.content_blocks)
    ));
    report.push_str(&format!(
        "fixture_layouts = {}\n",
        comma_list_or_none(&fixture.layout_kinds)
    ));
    report.push_str(&format!(
        "fixture_media = {}\n",
        comma_list_or_none(&fixture.media_kinds)
    ));
    report.push_str(&format!(
        "fixture_features = {}\n",
        comma_list_or_none(&fixture.feature_hooks)
    ));
    report.push_str(&format!(
        "fixture_feature_coverage = {}\n",
        if fixture.missing_declared_slide_variants.is_empty()
            && fixture.missing_declared_feature_hooks.is_empty()
        {
            "complete"
        } else {
            "incomplete"
        }
    ));
    report.push_str(&format!(
        "fixture_missing_declared_features = {}\n",
        comma_list_or_none(&fixture.missing_declared_feature_hooks)
    ));
    report.push_str(&format!(
        "fixture_uncovered_features = {}\n",
        comma_list_or_none(&fixture.uncovered_feature_hooks)
    ));
    report.push_str(&format!(
        "unknown_selectors = {}\n",
        comma_list_or_none(&theme::unknown_authored_theme_selectors(
            manifest,
            &rendered_theme.screen_css,
            &rendered_theme.print_css,
        ))
    ));
    report.push_str(&format!(
        "fixture_dead_selectors = {}\n",
        comma_list_or_none(&fixture.dead_selectors)
    ));
    let unused_theme_parameters = theme::unused_theme_parameters(manifest)?;
    report.push_str(&format!(
        "unused_theme_parameters = {}\n",
        comma_list_or_none(&unused_theme_parameters)
    ));
    let authoring_reference = render_theme_authoring_reference(
        label,
        fixture,
        manifest,
        rendered_theme,
        expected_pdf_pages,
        expected_notes_pdf_pages,
        &unused_theme_parameters,
    );
    let review = render_theme_specimen_review_html(
        label,
        expected_pdf_pages,
        expected_notes_pdf_pages,
        &report,
        &authoring_reference,
        &format!(
            "{}/{}/assets/theme.css",
            publication::HTML_GENERATIONS_DIRECTORY,
            publication.generation
        ),
        visual,
    );
    let namespace =
        output_ownership::OutputNamespaceGuard::acquire(output_dir).map_err(|source| {
            ZpresError::OutputOwnership {
                path: output_dir.to_path_buf(),
                source: Box::new(source),
            }
        })?;
    namespace
        .ensure_peer_output_allowed(output_dir, output_ownership::OutputTargetKind::Directory)
        .map_err(|source| ZpresError::OutputOwnership {
            path: output_dir.to_path_buf(),
            source: Box::new(source),
        })?;
    for (path, bytes) in [
        (output_dir.join("theme-check.txt"), report.as_bytes()),
        (
            output_dir.join("theme-api.txt"),
            authoring_reference.as_bytes(),
        ),
        (output_dir.join("review.html"), review.as_bytes()),
    ] {
        let published =
            namespace
                .publish_file(&path, bytes)
                .map_err(|source| ZpresError::OutputOwnership {
                    path: path.clone(),
                    source: Box::new(source),
                })?;
        for warning in published.warnings {
            eprintln!("warning: {warning}");
        }
    }
    Ok(())
}

fn render_theme_authoring_reference(
    label: &str,
    fixture: &theme::ThemeFixtureCheckReport,
    manifest: &theme::ThemeManifest,
    rendered_theme: &theme::RenderedTheme,
    expected_pdf_pages: usize,
    expected_notes_pdf_pages: usize,
    unused_theme_parameters: &[String],
) -> String {
    let mut reference = String::new();
    reference.push_str("# zpres theme authoring reference\n\n");
    reference.push_str(&format!("specimen_variant = {label}\n"));
    reference.push_str(&format!("theme = {}\n", manifest.name));
    reference.push_str(&format!("version = {}\n", manifest.version));
    reference.push_str(&format!("api_version = {}\n", manifest.api_version));
    reference.push_str(&format!("fixture = {}\n", fixture.source_path.display()));
    reference.push_str(&format!("pdf_pages = {expected_pdf_pages}\n"));
    reference.push_str(&format!("notes_pdf_pages = {expected_notes_pdf_pages}\n\n"));
    reference.push_str(&format!(
        "unused_theme_parameters = {}\n\n",
        comma_list_or_none(unused_theme_parameters)
    ));
    reference.push_str(&format!(
        "unknown_selectors = {}\n",
        comma_list_or_none(&theme::unknown_authored_theme_selectors(
            manifest,
            &rendered_theme.screen_css,
            &rendered_theme.print_css,
        ))
    ));
    reference.push_str(&format!(
        "fixture_dead_selectors = {}\n\n",
        comma_list_or_none(&fixture.dead_selectors)
    ));

    reference.push_str("## Resolved CSS variables\n\n");
    for declaration in theme::theme_param_css_declarations(manifest, &rendered_theme.params) {
        reference.push_str(&format!("{}: {};\n", declaration.name, declaration.value));
    }
    if rendered_theme.params.is_empty() {
        reference.push_str("none\n");
    }

    reference.push_str("\n## Slide-level hooks covered by this specimen\n\n");
    reference.push_str(&format!(
        "variants = {}\n",
        comma_list_or_none(&fixture.slide_variants)
    ));
    reference.push_str(&format!(
        "missing_declared_variants = {}\n",
        comma_list_or_none(&fixture.missing_declared_slide_variants)
    ));
    reference.push_str(&format!(
        "presets = {}\n",
        comma_list_or_none(&fixture.slide_presets)
    ));
    reference.push_str(&format!(
        "classes = {}\n",
        comma_list_or_none(&fixture.slide_classes)
    ));

    reference.push_str("\n## Content hooks covered by this specimen\n\n");
    reference.push_str(&format!(
        "blocks = {}\n",
        comma_list_or_none(&fixture.content_blocks)
    ));
    reference.push_str(&format!(
        "layouts = {}\n",
        comma_list_or_none(&fixture.layout_kinds)
    ));
    reference.push_str(&format!(
        "media = {}\n",
        comma_list_or_none(&fixture.media_kinds)
    ));
    reference.push_str(&format!(
        "features = {}\n",
        comma_list_or_none(&fixture.feature_hooks)
    ));
    reference.push_str(&format!(
        "declared_feature_coverage = {}\n",
        if fixture.missing_declared_feature_hooks.is_empty() {
            "complete"
        } else {
            "incomplete"
        }
    ));
    reference.push_str(&format!(
        "missing_declared_features = {}\n",
        comma_list_or_none(&fixture.missing_declared_feature_hooks)
    ));
    reference.push_str(&format!(
        "uncovered_supported_features = {}\n",
        comma_list_or_none(&fixture.uncovered_feature_hooks)
    ));

    reference.push_str("\n## Declared theme contract\n\n");
    reference.push_str(&format!(
        "output_targets = {}\n",
        comma_list_or_none(&manifest.output_targets)
    ));
    reference.push_str(&format!(
        "slide_variants = {}\n",
        comma_list_or_none(&manifest.slide_variants)
    ));
    reference.push_str(&format!(
        "feature_hooks = {}\n",
        comma_list_or_none(&manifest.feature_hooks)
    ));
    let presets = manifest.slide_presets.keys().cloned().collect::<Vec<_>>();
    reference.push_str(&format!("presets = {}\n", comma_list_or_none(&presets)));

    reference.push_str("\n## Stable Theme API v1 selectors\n\n");
    for selector in theme::stable_v1_theme_selectors() {
        reference.push_str(selector);
        reference.push('\n');
    }

    reference.push_str("\n## Internal selectors (not a Theme API contract)\n\n");
    for selector in theme::internal_theme_selectors() {
        reference.push_str(selector);
        reference.push('\n');
    }

    reference.push_str("\n## Supported feature hook names\n\n");
    for feature in theme::SUPPORTED_THEME_FEATURE_HOOKS {
        reference.push_str(feature);
        reference.push('\n');
    }

    reference
}

fn render_theme_specimen_review_html(
    label: &str,
    expected_pdf_pages: usize,
    expected_notes_pdf_pages: usize,
    report: &str,
    authoring_reference: &str,
    theme_css_href: &str,
    visual: bool,
) -> String {
    let visual_links = if visual {
        r#"
      <a href="contact-sheet.png">Contact sheet</a>
      <a href="visual-report.txt">Visual report</a>
      <a href="visual-report.json">Visual report JSON</a>
      <a href="provenance.json">Provenance</a>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>zpres theme specimen review - {label}</title>
  <style>
    :root {{ color-scheme: light dark; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
    body {{ margin: 0; background: #f5f5f0; color: #17211d; }}
    main {{ max-width: 1180px; margin: 0 auto; padding: 32px; }}
    h1 {{ margin: 0 0 8px; font-size: 2rem; }}
    p {{ margin: 0 0 20px; color: #51635b; }}
    nav {{ display: flex; flex-wrap: wrap; gap: 10px; margin: 20px 0 28px; }}
    a {{ color: #0f766e; font-weight: 700; }}
    nav a {{ border: 1px solid #cbd7d0; border-radius: 6px; padding: 8px 10px; background: #fffdf7; text-decoration: none; }}
    .previews {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 18px; }}
    section {{ min-width: 0; }}
    h2 {{ font-size: 1rem; margin: 0 0 8px; }}
    iframe {{ width: 100%; aspect-ratio: 16 / 9; border: 1px solid #cbd7d0; border-radius: 6px; background: #fff; }}
    pre {{ overflow: auto; border: 1px solid #cbd7d0; border-radius: 6px; background: #fffdf7; padding: 14px; line-height: 1.45; }}
    @media (prefers-color-scheme: dark) {{
      body {{ background: #111816; color: #f4f7f2; }}
      p {{ color: #a8bab2; }}
      nav a, pre {{ background: #18221f; border-color: #334640; }}
      a {{ color: #69d5c7; }}
      iframe {{ border-color: #334640; background: #18221f; }}
    }}
  </style>
</head>
<body>
  <main>
    <h1>zpres theme specimen review</h1>
    <p>Variant <strong>{label}</strong>. Slide pages: {expected_pdf_pages}. Notes pages: {expected_notes_pdf_pages}.</p>
    <nav aria-label="specimen artifacts">
      <a href="index.html">Live deck</a>
      <a href="print.html">Print HTML</a>
      <a href="print-notes.html">Print HTML with notes</a>
      <a href="speaker-notes.txt">Speaker notes text</a>
      <a href="theme-check.txt">Coverage report</a>
      <a href="theme-api.txt">Theme API</a>
      <a href="{theme_css_href}">Resolved theme CSS</a>
      {visual_links}
    </nav>
    <div class="previews">
      <section>
        <h2>Live deck</h2>
        <iframe src="index.html" title="Live deck preview"></iframe>
      </section>
      <section>
        <h2>Print HTML</h2>
        <iframe src="print.html" title="Print HTML preview"></iframe>
      </section>
      <section>
        <h2>Print HTML with notes</h2>
        <iframe src="print-notes.html" title="Print HTML with notes preview"></iframe>
      </section>
    </div>
    <h2>Coverage report</h2>
    <pre>{report}</pre>
    <h2>Theme API</h2>
    <pre>{authoring_reference}</pre>
  </main>
</body>
</html>
"#,
        label = escape_html_text(label),
        expected_pdf_pages = expected_pdf_pages,
        expected_notes_pdf_pages = expected_notes_pdf_pages,
        report = escape_html_text(report),
        authoring_reference = escape_html_text(authoring_reference),
        theme_css_href = escape_html_text(theme_css_href),
        visual_links = visual_links,
    )
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn safe_specimen_path_segment(value: &str) -> String {
    let segment = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if segment.is_empty() {
        "variant".to_string()
    } else {
        segment
    }
}

fn write_print_html_export(
    deck: &deck::Deck,
    theme: &theme::RenderedTheme,
    path: &Path,
    options: html::StaticExportOptions,
) -> Result<pdf::PdfReadinessReport, ZpresError> {
    let mut readiness = pdf::check_pdf_readiness_for_theme_with_options(deck, theme, options)?;
    let print_html = html::render_debug_print_html_with_options(deck, theme, options);
    pdf::check_print_html_ready(&print_html, readiness.expected_pages)?;
    let namespace = output_ownership::OutputNamespaceGuard::acquire(path).map_err(|source| {
        ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    let publication = namespace
        .publish_file(path, print_html.as_bytes())
        .map_err(|source| ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    readiness.warnings.extend(publication.warnings);
    Ok(readiness)
}

fn write_speaker_notes_text(deck: &deck::Deck, path: &Path) -> Result<(), ZpresError> {
    let namespace = output_ownership::OutputNamespaceGuard::acquire(path).map_err(|source| {
        ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    let publication = namespace
        .publish_file(path, render_speaker_notes_text(deck).as_bytes())
        .map_err(|source| ZpresError::OutputOwnership {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    for warning in publication.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn render_speaker_notes_text(deck: &deck::Deck) -> String {
    let mut text = String::from("# Speaker Notes\n\n");
    let mut note_index = 1usize;
    let mut last_slide_id = None::<&str>;
    for slide in deck.pdf_slide_order() {
        if last_slide_id == Some(slide.id.as_str()) {
            continue;
        }
        last_slide_id = Some(slide.id.as_str());
        let Some(notes) = slide_speaker_notes_markdown(slide) else {
            continue;
        };
        let title = slide.title.as_deref().unwrap_or("Untitled slide");
        let notes = speaker_notes_text_without_click_markers(notes.trim());
        text.push_str(&format!(
            "## {note_index}. {title} ({})\n\n{}\n\n",
            slide.id, notes
        ));
        note_index += 1;
    }
    if note_index == 1 {
        text.push_str("_No speaker notes._\n");
    }
    text
}

fn slide_speaker_notes_markdown(slide: &deck::Slide) -> Option<String> {
    let notes = slide
        .blocks
        .iter()
        .filter_map(|block| match block {
            deck::ContentBlock::SpeakerNotes { markdown } if !markdown.trim().is_empty() => {
                Some(markdown.trim())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!notes.is_empty()).then_some(notes)
}

fn speaker_notes_text_without_click_markers(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| {
            parse_speaker_note_click_marker(line.trim_start())
                .map(|body| body.to_string())
                .unwrap_or_else(|| line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_speaker_note_click_marker(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("[click")?;
    let after_marker = if let Some(after_marker) = rest.strip_prefix(']') {
        after_marker
    } else {
        let after_colon = rest.strip_prefix(':')?;
        let (raw_click, after_marker) = after_colon.split_once(']')?;
        raw_click.trim().parse::<usize>().ok()?;
        after_marker
    };
    Some(after_marker.trim_start())
}

fn expand_tilde(path: &Path, home_dir: &Option<PathBuf>) -> Option<PathBuf> {
    let mut components = path.components();
    let first = components.next()?;
    if first.as_os_str() != OsStr::new("~") {
        return None;
    }
    let home_dir = home_dir.as_ref()?;
    let mut expanded = home_dir.clone();
    expanded.extend(components);
    Some(expanded)
}

fn absolutize(cwd: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_paths(root: &Path) -> ConfigPaths {
        ConfigPaths {
            global_config: root.join("global").join("zpres.toml"),
            home_dir: Some(root.join("home")),
        }
    }

    fn write_theme_manifest(root: &Path, name: &str) {
        let theme_dir = root.join(name);
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            format!(
                r##"[theme]
name = "{name}"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "comparison"]

[parameters.accent]
type = "color"
default = "#0891b2"

[parameters.density]
type = "enum"
default = "normal"
values = ["compact", "normal", "spacious"]

[parameters.show_slide_ids]
type = "boolean"
default = true
"##
            ),
        )
        .unwrap();
    }

    fn write_minimum_distance_theme_manifests(root: &Path) {
        let dark_splash = root.join("dark-splash");
        fs::create_dir_all(&dark_splash).unwrap();
        fs::write(
            dark_splash.join("theme.toml"),
            r##"[theme]
name = "dark-splash"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "comparison"]
palette_parameter = "variant"

[parameters.variant]
type = "enum"
default = "violet"
values = ["violet", "cyan"]

[parameters.density]
type = "enum"
default = "normal"
values = ["compact", "normal", "spacious"]

[parameters.footer]
type = "enum"
default = "slide-number"
values = ["none", "slide-number", "section-progress"]
"##,
        )
        .unwrap();

        let sv = root.join("sv");
        fs::create_dir_all(&sv).unwrap();
        fs::write(
            sv.join("theme.toml"),
            r##"[theme]
name = "sv"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "comparison"]
palette_parameter = "mode"

[parameters.mode]
type = "enum"
default = "light"
values = ["light", "dark"]

[parameters.density]
type = "enum"
default = "normal"
values = ["compact", "normal", "spacious"]

[parameters.footer]
type = "enum"
default = "slide-number"
values = ["none", "slide-number", "section-progress"]
"##,
        )
        .unwrap();
    }

    fn write_minimum_distance_source(deck_root: &Path) -> PathBuf {
        let source = deck_root.join("example-deck.zp.md");
        fs::write(
            &source,
            r#"---
theme: "dark-splash"
theme_params:
  variant: "cyan"
  density: "compact"
  footer: "section-progress"
aspect: "16:9"
---

# Example Deck
"#,
        )
        .unwrap();
        source
    }

    #[test]
    fn parses_static_image_cli_values() {
        assert_eq!(
            parse_image_size("1920x1080").unwrap(),
            pdf::PngViewport {
                width: 1920,
                height: 1080,
            }
        );
        assert_eq!(
            parse_image_size("2560X1440").unwrap(),
            pdf::PngViewport {
                width: 2560,
                height: 1440,
            }
        );
        assert!(
            parse_image_size("wide")
                .unwrap_err()
                .contains("WIDTHxHEIGHT")
        );
        assert!(parse_image_size("100x100").unwrap_err().contains("width"));
        assert!(parse_image_size("1280x100").unwrap_err().contains("height"));
        assert!(
            parse_image_size("1024x768")
                .unwrap_err()
                .contains("exact 16:9")
        );
        assert_eq!(parse_jpeg_quality("85").unwrap(), pdf::JpegQuality(85));
        assert!(
            parse_jpeg_quality("0")
                .unwrap_err()
                .contains("between 1 and 100")
        );
        assert!(
            parse_jpeg_quality("high")
                .unwrap_err()
                .contains("whole number")
        );
    }

    #[test]
    fn speaker_notes_text_follows_pdf_slide_order_without_step_duplicates() {
        let source = r#"# Main

::: notes
Opening note.
[click] First reveal note.
[click:3] Third reveal note.
:::

::: steps pdf="pages"
1. First reveal
2. Second reveal
:::

--

## Detail

^ Detail note.

---

# No notes
"#;
        let deck = deck::parse_source_text(source, Some(PathBuf::from("talk.zp.md"))).unwrap();

        let text = render_speaker_notes_text(&deck);

        assert!(text.starts_with("# Speaker Notes"));
        assert!(text.contains("## 1. Main (section-1-main)"));
        assert!(text.contains("Opening note."));
        assert!(text.contains("First reveal note."));
        assert!(text.contains("Third reveal note."));
        assert!(!text.contains("[click]"));
        assert!(!text.contains("[click:3]"));
        assert!(text.contains("## 2. Detail (section-1-detail-1)"));
        assert!(text.contains("Detail note."));
        assert!(!text.contains("## 3."));
    }

    #[test]
    fn resolves_config_precedence_and_paths() {
        let temp = tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_config,
            r##"schema_version = 1

[deck]
theme = "global"
output_dir = "global-dist"

[paths]
theme_dirs = ["~/shared-themes"]

[pdf]
renderer = "chromium"

[theme.params]
accent = "blue"
"##,
        )
        .unwrap();

        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        fs::write(
            deck_root.join("zpres.toml"),
            r##"schema_version = 1

[deck]
theme = "project"
output_dir = "project-dist"

[paths]
theme_dirs = ["./themes"]

[theme.params]
accent = "green"
density = "normal"
"##,
        )
        .unwrap();
        write_theme_manifest(&deck_root.join("cli-themes"), "cli");
        fs::write(
            deck_root.join("talk.zp.md"),
            r#"---
theme: front
output_dir: front-dist
theme_dirs:
  - front-themes
theme_params:
  accent: cyan
---

# Talk
"#,
        )
        .unwrap();

        let resolved = resolve_config(
            ResolveOptions {
                source: Some(deck_root.join("talk.zp.md")),
                project_config: None,
                cli_theme: Some("cli".to_string()),
                cli_output_dir: Some(PathBuf::from("cli-dist")),
                cli_pdf_path: Some(PathBuf::from("talk.pdf")),
                cli_theme_dirs: vec![PathBuf::from("cli-themes")],
                cli_theme_params: BTreeMap::from([("accent".to_string(), "#ff00ff".to_string())]),
                strict_theme_params: true,
            },
            &paths,
        )
        .unwrap();

        assert_eq!(resolved.theme.as_deref(), Some("cli"));
        assert_eq!(
            resolved.theme_manifest_path,
            Some(deck_root.join("cli-themes").join("cli").join("theme.toml"))
        );
        assert_eq!(resolved.output_dir, deck_root.join("cli-dist"));
        assert_eq!(resolved.pdf_path, Some(deck_root.join("talk.pdf")));
        assert_eq!(resolved.pdf_renderer, "chromium");
        assert_eq!(
            resolved.theme_search_paths,
            vec![
                temp.path().join("home").join("shared-themes"),
                deck_root.join("themes"),
                deck_root.join("front-themes"),
                deck_root.join("cli-themes"),
            ]
        );
        assert_eq!(resolved.theme_params.get("accent").unwrap(), "#ff00ff");
        assert_eq!(resolved.theme_params.get("density").unwrap(), "normal");
        assert_eq!(resolved.theme_params.get("show_slide_ids").unwrap(), "true");
    }

    #[test]
    fn unsupported_config_settings_fail_with_actionable_diagnostics() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("zpres.toml");
        for (settings, expected) in [
            ("[paths]\noutput_root = \"dist\"", "paths.output_root"),
            ("[paths]\ncache_root = \"cache\"", "paths.cache_root"),
            ("[pdf]\nrenderer = \"other\"", "pdf.renderer"),
            ("[deck]\nroom_profile = \"room.toml\"", "room_profile"),
        ] {
            fs::write(&path, format!("schema_version = 1\n{settings}\n")).unwrap();
            let error = load_toml_layer(&path, &path, &None).unwrap_err();
            assert!(
                matches!(error, ConfigError::UnsupportedSetting { setting, .. } if setting == expected)
            );
        }
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "---\nroom_profile: room.toml\n---\n# Talk\n").unwrap();
        assert!(matches!(
            load_front_matter_layer(&source, temp.path(), &None),
            Err(ConfigError::UnsupportedSetting {
                setting: "room_profile",
                ..
            })
        ));
    }

    #[test]
    fn explicit_cli_theme_change_resets_inherited_theme_params_before_cli_params() {
        let temp = tempdir().unwrap();
        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        write_minimum_distance_theme_manifests(&deck_root.join("themes"));
        let source = write_minimum_distance_source(&deck_root);

        let resolved = resolve_config(
            ResolveOptions {
                source: Some(source),
                project_config: None,
                cli_theme: Some("sv".to_string()),
                cli_output_dir: None,
                cli_pdf_path: None,
                cli_theme_dirs: Vec::new(),
                cli_theme_params: BTreeMap::from([("mode".to_string(), "dark".to_string())]),
                strict_theme_params: true,
            },
            &ConfigPaths {
                global_config: temp.path().join("missing.toml"),
                home_dir: None,
            },
        )
        .unwrap();

        assert_eq!(resolved.theme.as_deref(), Some("sv"));
        assert_eq!(
            resolved.theme_params.get("mode").map(String::as_str),
            Some("dark")
        );
        assert_eq!(
            resolved.theme_params.get("density").map(String::as_str),
            Some("normal")
        );
        assert_eq!(
            resolved.theme_params.get("footer").map(String::as_str),
            Some("slide-number")
        );
        assert!(!resolved.theme_params.contains_key("variant"));
    }

    #[test]
    fn explicit_same_theme_preserves_inherited_params_then_applies_cli_params() {
        let temp = tempdir().unwrap();
        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        write_minimum_distance_theme_manifests(&deck_root.join("themes"));
        let source = write_minimum_distance_source(&deck_root);

        let resolved = resolve_config(
            ResolveOptions {
                source: Some(source),
                project_config: None,
                cli_theme: Some("dark-splash".to_string()),
                cli_output_dir: None,
                cli_pdf_path: None,
                cli_theme_dirs: Vec::new(),
                cli_theme_params: BTreeMap::from([("density".to_string(), "spacious".to_string())]),
                strict_theme_params: true,
            },
            &ConfigPaths {
                global_config: temp.path().join("missing.toml"),
                home_dir: None,
            },
        )
        .unwrap();

        assert_eq!(resolved.theme.as_deref(), Some("dark-splash"));
        assert_eq!(
            resolved.theme_params.get("variant").map(String::as_str),
            Some("cyan")
        );
        assert_eq!(
            resolved.theme_params.get("density").map(String::as_str),
            Some("spacious")
        );
        assert_eq!(
            resolved.theme_params.get("footer").map(String::as_str),
            Some("section-progress")
        );
    }

    #[test]
    fn no_cli_theme_override_preserves_inherited_theme_params() {
        let temp = tempdir().unwrap();
        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        write_minimum_distance_theme_manifests(&deck_root.join("themes"));
        let source = write_minimum_distance_source(&deck_root);

        let resolved = resolve_config(
            ResolveOptions {
                source: Some(source),
                project_config: None,
                cli_theme: None,
                cli_output_dir: None,
                cli_pdf_path: None,
                cli_theme_dirs: Vec::new(),
                cli_theme_params: BTreeMap::new(),
                strict_theme_params: true,
            },
            &ConfigPaths {
                global_config: temp.path().join("missing.toml"),
                home_dir: None,
            },
        )
        .unwrap();

        assert_eq!(resolved.theme.as_deref(), Some("dark-splash"));
        assert_eq!(
            resolved.theme_params.get("variant").map(String::as_str),
            Some("cyan")
        );
        assert_eq!(
            resolved.theme_params.get("density").map(String::as_str),
            Some("compact")
        );
        assert_eq!(
            resolved.theme_params.get("footer").map(String::as_str),
            Some("section-progress")
        );
    }

    #[test]
    fn discovers_theme_from_shared_theme_directory() {
        let temp = tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        let shared_themes = temp.path().join("shared-themes");
        write_theme_manifest(&shared_themes, "science");
        fs::write(
            &paths.global_config,
            format!(
                r#"schema_version = 1

[paths]
theme_dirs = ["{}"]
"#,
                shared_themes.display()
            ),
        )
        .unwrap();

        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        fs::write(
            deck_root.join("talk.zp.md"),
            r#"---
theme: science
---

# Talk
"#,
        )
        .unwrap();

        let resolved = resolve_config(
            ResolveOptions {
                source: Some(deck_root.join("talk.zp.md")),
                project_config: None,
                cli_theme: None,
                cli_output_dir: None,
                cli_pdf_path: None,
                cli_theme_dirs: Vec::new(),
                cli_theme_params: BTreeMap::new(),
                strict_theme_params: true,
            },
            &paths,
        )
        .unwrap();

        assert_eq!(
            resolved.theme_manifest_path,
            Some(shared_themes.join("science").join("theme.toml"))
        );
        assert_eq!(resolved.theme_params.get("accent").unwrap(), "#0891b2");
    }

    #[test]
    fn rejects_unknown_theme_parameters() {
        let temp = tempdir().unwrap();
        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        write_theme_manifest(&deck_root.join("themes"), "debug");

        let error = resolve_config(
            ResolveOptions {
                source: Some(deck_root.join("talk.zp.md")),
                project_config: None,
                cli_theme: Some("debug".to_string()),
                cli_output_dir: None,
                cli_pdf_path: None,
                cli_theme_dirs: Vec::new(),
                cli_theme_params: BTreeMap::from([("unknown".to_string(), "value".to_string())]),
                strict_theme_params: true,
            },
            &ConfigPaths {
                global_config: temp.path().join("missing.toml"),
                home_dir: None,
            },
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("theme parameter 'unknown' is not declared")
        );
    }

    #[test]
    fn rejects_invalid_theme_parameter_values() {
        let temp = tempdir().unwrap();
        let paths = test_paths(temp.path());
        let deck_root = temp.path().join("deck");
        fs::create_dir_all(&deck_root).unwrap();
        write_theme_manifest(&deck_root.join("themes"), "debug");

        let error = resolve_config(
            ResolveOptions {
                source: Some(deck_root.join("talk.zp.md")),
                project_config: None,
                cli_theme: Some("debug".to_string()),
                cli_output_dir: None,
                cli_pdf_path: None,
                cli_theme_dirs: Vec::new(),
                cli_theme_params: BTreeMap::from([("density".to_string(), "cramped".to_string())]),
                strict_theme_params: true,
            },
            &paths,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("theme parameter 'density' has invalid value 'cramped'")
        );
    }

    #[test]
    fn rejects_unsupported_schema_version() {
        let temp = tempdir().unwrap();
        let config = temp.path().join("zpres.toml");
        fs::write(&config, "schema_version = 2\n").unwrap();

        let error = load_toml_layer(&config, &config, &None).unwrap_err();

        assert!(matches!(
            error,
            ConfigError::UnsupportedSchemaVersion { found: 2, .. }
        ));
    }

    #[test]
    fn rejects_wrong_toml_value_type() {
        let temp = tempdir().unwrap();
        let config = temp.path().join("zpres.toml");
        fs::write(&config, "schema_version = 1\n[paths]\ntheme_dirs = true\n").unwrap();

        let error = load_toml_layer(&config, &config, &None).unwrap_err();

        assert!(matches!(error, ConfigError::Toml { .. }));
        assert!(error.to_string().contains("zpres.toml:"));
    }

    #[test]
    fn config_init_does_not_overwrite_without_force() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("config").join("zpres.toml");

        init_global_config(&path, false).unwrap();
        fs::write(&path, "custom").unwrap();

        let error = init_global_config(&path, false).unwrap_err();
        assert!(matches!(error, ConfigError::ConfigExists { .. }));

        init_global_config(&path, true).unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("schema_version = 1"));
    }

    #[test]
    fn concurrent_config_initializers_do_not_overwrite_the_winner() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let temp = tempdir().unwrap();
        let path = temp.path().join("config").join("zpres.toml");
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let barrier = Arc::clone(&barrier);
            let path = path.clone();
            workers.push(thread::spawn(move || {
                barrier.wait();
                init_global_config(&path, false)
            }));
        }
        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(ConfigError::ConfigExists { .. })))
                .count(),
            1
        );
        assert_eq!(fs::read_to_string(path).unwrap(), DEFAULT_GLOBAL_CONFIG);
    }

    #[test]
    fn extracts_yaml_front_matter_only_at_file_start() {
        let text = "---\ntheme: debug\n---\n\n# Title\n---\n# Next section";
        assert_eq!(
            extract_yaml_front_matter(text).unwrap().trim(),
            "theme: debug"
        );

        assert!(extract_yaml_front_matter("# Title\n---\n").is_none());
        assert_eq!(
            extract_yaml_front_matter("\u{feff}---\r\ntheme: debug\r\n---\r\n# Title")
                .unwrap()
                .trim(),
            "theme: debug"
        );
    }

    #[test]
    fn config_resolution_rejects_unterminated_source_front_matter() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            "\u{feff}---\r\ntheme: debug\r\n# Missing close\r\n",
        )
        .unwrap();

        let error = load_front_matter_layer(&source, temp.path(), &None).unwrap_err();

        assert!(matches!(
            &error,
            ConfigError::UnterminatedFrontMatter {
                line: 1,
                column: 1,
                ..
            }
        ));
        assert!(error.to_string().contains("talk.zp.md:1:1"));
    }
}
