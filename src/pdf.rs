use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::ImageFormat;
use image::codecs::jpeg::JpegEncoder;
use lopdf::Document;
use thiserror::Error;

use crate::background_validation::{
    expected_print_backgrounds_for_plan, validate_authored_background,
};
use crate::browser_validation::{
    BrowserFinding, validate_browser_page, validate_browser_page_count,
};
use crate::chromium::{
    ChromiumError, ChromiumSession, ChromiumSessionOptions, STATIC_READINESS_TIMEOUT,
    discover_chromium_executable,
};
use crate::deck::{
    ChartData, ContentBlock, Deck, DiagramLanguage, MediaKind, inline_math_segments,
    mermaid_static_renderability_error,
};
use crate::file_url::file_url;
use crate::html;
use crate::output_ownership::{OutputNamespaceGuard, OutputTargetKind};
use crate::presentation_plan::PresentationPlan;
use crate::raster_publication::{self, RasterFormat};
use crate::theme::{self, RenderedTheme, ThemeApiVersion};

pub trait PdfRenderer {
    fn export(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_pdf: &Path,
    ) -> Result<PdfSmokeReport, PdfError>;
}

#[derive(Debug, Clone)]
pub struct ChromiumPdfRenderer {
    executable: PathBuf,
}

impl ChromiumPdfRenderer {
    pub fn discover() -> Result<Self, PdfError> {
        let discovery = discover_chromium_executable();
        let executable = discovery.executable.ok_or(PdfError::MissingChromium {
            requested: discovery.requested,
            searched: discovery.searched,
        })?;
        Ok(Self { executable })
    }

    #[cfg(test)]
    fn from_executable(executable: PathBuf) -> Self {
        Self { executable }
    }

    pub fn export_png_pages(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_dir: &Path,
    ) -> Result<PngExportReport, PdfError> {
        self.export_png_pages_with_options(
            deck,
            theme,
            output_dir,
            html::StaticExportOptions::default(),
        )
    }

    pub fn export_png_pages_with_options(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_dir: &Path,
        options: html::StaticExportOptions,
    ) -> Result<PngExportReport, PdfError> {
        self.export_png_pages_with_viewport(
            deck,
            theme,
            output_dir,
            options,
            PngViewport::default(),
        )
    }

    pub fn export_png_pages_with_viewport(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_dir: &Path,
        options: html::StaticExportOptions,
        viewport: PngViewport,
    ) -> Result<PngExportReport, PdfError> {
        theme::validate_deck_for_theme_contract(deck, &theme.manifest)?;
        validate_raster_output_viewport(viewport)?;
        let namespace = OutputNamespaceGuard::acquire(output_dir)
            .map_err(|source| map_output_ownership_error(output_dir, source))?;
        raster_publication::preflight_raster_page_set_destination_under_namespace(
            &namespace,
            output_dir,
            RasterFormat::Png,
        )
        .map_err(|source| map_raster_publication_error(output_dir, source))?;
        let readiness = check_pdf_readiness_for_theme_with_options(deck, theme, options)?;
        let capture_scratch = raster_capture_scratch_directory(output_dir);
        let mut loaded = self.load_print_document(
            deck,
            theme,
            options,
            PngViewport::default(),
            capture_scratch,
            readiness.expected_pages,
        )?;

        let mut captures = Vec::with_capacity(readiness.expected_pages);
        let mut findings = Vec::new();
        for page_index in 0..readiness.expected_pages {
            let capture = loaded
                .browser
                .capture_print_page_with_output_viewport(page_index, viewport)
                .map_err(map_chromium_error)?;
            findings.extend(validate_browser_page(
                page_index + 1,
                &capture.observation,
                PngViewport::default(),
            ));
            let output_png = output_dir.join(format!("page-{:03}.png", page_index + 1));
            validate_png_bytes(&capture.png, &output_png, viewport)?;
            captures.push(capture.png);
        }
        ensure_no_browser_findings(findings)?;
        drop(loaded);
        let publication = raster_publication::publish_raster_page_set_under_namespace(
            &namespace,
            output_dir,
            RasterFormat::Png,
            captures,
        )
        .map_err(|source| map_raster_publication_error(output_dir, source))?;
        let mut warnings = readiness.warnings;
        warnings.extend(publication.warnings);

        Ok(PngExportReport {
            output_dir: output_dir.to_path_buf(),
            expected_pages: readiness.expected_pages,
            exported_pages: publication.files.len(),
            viewport,
            files: publication.files,
            raster_generation: publication.generation,
            warnings,
        })
    }

    pub fn export_jpeg_pages_with_viewport(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_dir: &Path,
        options: html::StaticExportOptions,
        viewport: PngViewport,
        quality: JpegQuality,
    ) -> Result<JpegExportReport, PdfError> {
        theme::validate_deck_for_theme_contract(deck, &theme.manifest)?;
        validate_raster_output_viewport(viewport)?;
        let namespace = OutputNamespaceGuard::acquire(output_dir)
            .map_err(|source| map_output_ownership_error(output_dir, source))?;
        raster_publication::preflight_raster_page_set_destination_under_namespace(
            &namespace,
            output_dir,
            RasterFormat::Jpeg,
        )
        .map_err(|source| map_raster_publication_error(output_dir, source))?;
        let readiness = check_pdf_readiness_for_theme_with_options(deck, theme, options)?;
        let capture_scratch = raster_capture_scratch_directory(output_dir);
        let mut loaded = self.load_print_document(
            deck,
            theme,
            options,
            PngViewport::default(),
            capture_scratch,
            readiness.expected_pages,
        )?;

        let mut captures = Vec::with_capacity(readiness.expected_pages);
        let mut findings = Vec::new();
        for page_index in 0..readiness.expected_pages {
            let capture = loaded
                .browser
                .capture_print_page_with_output_viewport(page_index, viewport)
                .map_err(map_chromium_error)?;
            findings.extend(validate_browser_page(
                page_index + 1,
                &capture.observation,
                PngViewport::default(),
            ));
            let output_jpeg = output_dir.join(format!("page-{:03}.jpg", page_index + 1));
            validate_png_bytes(
                &capture.png,
                &output_jpeg.with_extension("source.png"),
                viewport,
            )?;
            let jpeg = encode_png_bytes_as_jpeg(&capture.png, &output_jpeg, quality)?;
            validate_jpeg_bytes(&jpeg, &output_jpeg, viewport)?;
            captures.push(jpeg);
        }
        ensure_no_browser_findings(findings)?;
        drop(loaded);
        let publication = raster_publication::publish_raster_page_set_under_namespace(
            &namespace,
            output_dir,
            RasterFormat::Jpeg,
            captures,
        )
        .map_err(|source| map_raster_publication_error(output_dir, source))?;
        let mut warnings = readiness.warnings;
        warnings.extend(publication.warnings);

        Ok(JpegExportReport {
            output_dir: output_dir.to_path_buf(),
            expected_pages: readiness.expected_pages,
            exported_pages: publication.files.len(),
            viewport,
            quality,
            files: publication.files,
            warnings,
        })
    }

    pub fn export_png_contact_sheet_for_report(
        &self,
        report: &PngExportReport,
        output_png: &Path,
    ) -> Result<PngContactSheetReport, PdfError> {
        self.ensure_executable()?;
        let namespace = OutputNamespaceGuard::acquire(output_png)
            .map_err(|source| map_output_ownership_error(output_png, source))?;
        if path_is_within_directory(output_png, &report.output_dir) {
            return Err(PdfError::ContactSheetInsideRasterDirectory {
                path: output_png.to_path_buf(),
                directory: report.output_dir.clone(),
            });
        }
        namespace
            .ensure_peer_output_allowed(output_png, OutputTargetKind::File)
            .map_err(|source| map_output_ownership_error(output_png, source))?;
        let page_set = raster_publication::lock_raster_page_set_for_read_under_namespace(
            &namespace,
            &report.output_dir,
            RasterFormat::Png,
            &report.raster_generation,
            &report.files,
        )
        .map_err(|source| map_raster_publication_error(&report.output_dir, source))?;
        if path_is_within_directory(output_png, page_set.output_dir()) {
            return Err(PdfError::ContactSheetInsideRasterDirectory {
                path: output_png.to_path_buf(),
                directory: report.output_dir.clone(),
            });
        }
        self.export_png_contact_sheet_from_paths(&namespace, page_set.files(), output_png)
    }

    fn export_png_contact_sheet_from_paths(
        &self,
        namespace: &OutputNamespaceGuard,
        page_images: &[PathBuf],
        output_png: &Path,
    ) -> Result<PngContactSheetReport, PdfError> {
        namespace
            .ensure_peer_output_allowed(output_png, OutputTargetKind::File)
            .map_err(|source| map_output_ownership_error(output_png, source))?;
        ensure_parent_dir(output_png)?;
        self.ensure_executable()?;

        let layout = PngContactSheetLayout::for_page_count(page_images.len());
        let html = render_png_contact_sheet_html(page_images, layout);
        let html_dir = output_png.parent().unwrap_or_else(|| Path::new("."));
        let sheet_html = TemporaryFile::create(html_dir, "contact-sheet", "html", html.as_bytes())?;
        let viewport = PngViewport {
            width: layout.width,
            height: layout.height,
        };
        let mut browser = ChromiumSession::launch_with_executable(
            self.executable.clone(),
            ChromiumSessionOptions::default(),
        )
        .map_err(map_chromium_error)?;
        let png = browser
            .capture_document(&file_url(sheet_html.path()), viewport)
            .map_err(map_chromium_error)?;
        validate_png_bytes(&png, output_png, viewport)?;
        publish_output_set(namespace, vec![(output_png.to_path_buf(), png)])?;

        Ok(PngContactSheetReport {
            path: output_png.to_path_buf(),
            page_count: page_images.len(),
            width: layout.width,
            height: layout.height,
        })
    }

    pub fn preflight_static_export_with_options(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        options: html::StaticExportOptions,
    ) -> Result<StaticExportPreflightReport, PdfError> {
        theme::validate_deck_for_theme_contract(deck, &theme.manifest)?;
        let readiness = check_pdf_readiness_for_theme_with_options(deck, theme, options)?;
        let html_dir = deck.deck_root().unwrap_or_else(|| Path::new("."));
        let _namespace = OutputNamespaceGuard::acquire(html_dir)
            .map_err(|source| map_output_ownership_error(html_dir, source))?;
        _namespace
            .ensure_peer_output_allowed(html_dir, OutputTargetKind::Directory)
            .map_err(|source| map_output_ownership_error(html_dir, source))?;
        let loaded = self.load_print_document(
            deck,
            theme,
            options,
            PngViewport::default(),
            html_dir,
            readiness.expected_pages,
        )?;
        Ok(StaticExportPreflightReport {
            expected_pages: readiness.expected_pages,
            observed_pages: loaded.observed_pages,
            validated_pages: readiness.expected_pages,
            warnings: readiness.warnings,
        })
    }

    fn ensure_executable(&self) -> Result<(), PdfError> {
        if self.executable.exists() {
            Ok(())
        } else {
            Err(PdfError::MissingChromium {
                requested: Some(self.executable.clone()),
                searched: Vec::new(),
            })
        }
    }

    fn load_print_document(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        options: html::StaticExportOptions,
        viewport: PngViewport,
        html_dir: &Path,
        expected_pages: usize,
    ) -> Result<LoadedPrintDocument, PdfError> {
        theme::validate_deck_for_theme_contract(deck, &theme.manifest)?;
        self.ensure_executable()?;
        let print_html = html::render_debug_print_html_with_options(deck, theme, options);
        let presentation_plan = (theme.manifest.api() == ThemeApiVersion::V1)
            .then(|| PresentationPlan::for_theme_api_v1(deck));
        let planned_print_pages = presentation_plan
            .as_ref()
            .map(|plan| plan.print_pages(options.include_speaker_notes));
        let background_expectations = if let Some(plan) = presentation_plan.as_ref() {
            expected_print_backgrounds_for_plan(plan, options)
        } else {
            vec![None; expected_pages]
        };
        debug_assert_eq!(background_expectations.len(), expected_pages);
        let temporary_html =
            TemporaryFile::create(html_dir, "static-export", "html", print_html.as_bytes())?;
        let mut browser = ChromiumSession::launch_with_executable(
            self.executable.clone(),
            ChromiumSessionOptions::default(),
        )
        .map_err(map_chromium_error)?;
        browser
            .load_print_document(&file_url(temporary_html.path()), viewport)
            .map_err(map_chromium_error)?;
        let static_readiness = browser
            .await_static_readiness(STATIC_READINESS_TIMEOUT)
            .map_err(map_chromium_error)?;
        let pages = browser.print_pages().map_err(map_chromium_error)?;
        let observed_pages = pages.len();
        let mut findings = validate_browser_page_count(expected_pages, observed_pages);
        for page_index in 0..observed_pages {
            let observation = browser
                .observe_print_page(page_index)
                .map_err(map_chromium_error)?;
            if let Some(planned) = planned_print_pages
                .as_ref()
                .and_then(|pages| pages.get(page_index))
            {
                let (expected_step_state, expected_step) = planned.pdf_step_attributes();
                if observation.slide_id != planned.slide_id()
                    || observation.role != planned.role()
                    || observation.generated.as_deref() != planned.generated()
                    || observation.step_state.as_deref() != expected_step_state
                    || observation.pdf_step != expected_step
                {
                    findings.push(BrowserFinding::deck(
                        "print",
                        "print-page-plan-order-mismatch",
                        format!(
                            "print page {} is slide '{}' role '{}' generated {:?} Step {:?}/{:?}; the Theme API v1 Presentation plan requires slide '{}' role '{}' generated {:?} Step {:?}/{:?}",
                            page_index + 1,
                            observation.slide_id,
                            observation.role,
                            observation.generated,
                            observation.step_state,
                            observation.pdf_step,
                            planned.slide_id(),
                            planned.role(),
                            planned.generated(),
                            expected_step_state,
                            expected_step,
                        ),
                    ));
                }
            }
            findings.extend(validate_browser_page(
                page_index + 1,
                &observation,
                viewport,
            ));
            findings.extend(validate_authored_background(
                "print",
                page_index + 1,
                None,
                background_expectations
                    .get(page_index)
                    .and_then(Option::as_ref),
                presentation_plan.is_some(),
                &observation,
            ));
        }
        if !static_readiness.promise_present
            || !static_readiness.ready
            || !static_readiness.errors.is_empty()
        {
            findings.push(BrowserFinding::deck(
                "print",
                "static-readiness-failed",
                format!(
                    "static readiness was {:?} (promise_present={}, document_ready_state={}, errors={:?}); diagnostics: {}",
                    static_readiness.promise_status,
                    static_readiness.promise_present,
                    static_readiness.document_ready_state,
                    static_readiness.errors,
                    format_browser_diagnostics(&browser),
                ),
            ));
        }
        ensure_no_browser_findings(findings)?;
        Ok(LoadedPrintDocument {
            browser,
            _temporary_html: temporary_html,
            observed_pages,
        })
    }
}

impl PdfRenderer for ChromiumPdfRenderer {
    fn export(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_pdf: &Path,
    ) -> Result<PdfSmokeReport, PdfError> {
        self.export_with_options(
            deck,
            theme,
            output_pdf,
            html::StaticExportOptions::default(),
        )
    }
}

impl ChromiumPdfRenderer {
    pub fn export_with_options(
        &self,
        deck: &Deck,
        theme: &RenderedTheme,
        output_pdf: &Path,
        options: html::StaticExportOptions,
    ) -> Result<PdfSmokeReport, PdfError> {
        theme::validate_deck_for_theme_contract(deck, &theme.manifest)?;
        let readiness = check_pdf_readiness_for_theme_with_options(deck, theme, options)?;
        let namespace = OutputNamespaceGuard::acquire(output_pdf)
            .map_err(|source| map_output_ownership_error(output_pdf, source))?;
        namespace
            .ensure_peer_output_allowed(output_pdf, OutputTargetKind::File)
            .map_err(|source| map_output_ownership_error(output_pdf, source))?;
        ensure_parent_dir(output_pdf)?;
        let html_dir = output_pdf.parent().unwrap_or_else(|| Path::new("."));
        let mut loaded = self.load_print_document(
            deck,
            theme,
            options,
            PngViewport::default(),
            html_dir,
            readiness.expected_pages,
        )?;
        let pdf = loaded.browser.print_pdf().map_err(map_chromium_error)?;
        let mut report = smoke_check_pdf_bytes(&pdf, output_pdf, readiness.expected_pages)?;
        publish_output_set(&namespace, vec![(output_pdf.to_path_buf(), pdf)])?;
        report.warnings = readiness.warnings;
        Ok(report)
    }
}

struct LoadedPrintDocument {
    browser: ChromiumSession,
    _temporary_html: TemporaryFile,
    observed_pages: usize,
}

fn ensure_no_browser_findings(findings: Vec<BrowserFinding>) -> Result<(), PdfError> {
    if findings.is_empty() {
        return Ok(());
    }
    let message = findings
        .into_iter()
        .map(|finding| {
            let page = finding
                .page
                .map(|page| format!("page {page}"))
                .unwrap_or_else(|| "document".to_string());
            let slide = finding
                .slide_id
                .as_deref()
                .map(|slide_id| format!(" slide '{slide_id}'"))
                .unwrap_or_default();
            format!("{}: {page}{slide}: {}", finding.code, finding.message)
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(PdfError::BrowserValidation { message })
}

fn map_chromium_error(error: ChromiumError) -> PdfError {
    match error {
        ChromiumError::MissingChromium {
            requested,
            searched,
        } => PdfError::MissingChromium {
            requested,
            searched,
        },
        error => PdfError::Browser {
            message: error.to_string(),
        },
    }
}

fn map_raster_publication_error(
    path: &Path,
    source: raster_publication::RasterPublicationError,
) -> PdfError {
    PdfError::RasterPublication {
        path: path.to_path_buf(),
        source: Box::new(source),
    }
}

fn map_output_ownership_error(
    path: &Path,
    source: crate::output_ownership::OutputOwnershipError,
) -> PdfError {
    PdfError::OutputOwnership {
        path: path.to_path_buf(),
        source: Box::new(source),
    }
}

fn raster_capture_scratch_directory(output_dir: &Path) -> &Path {
    output_dir
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn path_is_within_directory(path: &Path, directory: &Path) -> bool {
    if let Ok(canonical) = fs::canonicalize(path) {
        return crate::path_is_equal_or_within(&canonical, directory);
    }
    let mut ancestor = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    loop {
        if let Ok(canonical) = fs::canonicalize(ancestor) {
            return crate::path_is_equal_or_within(&canonical, directory);
        }
        let Some(parent) = ancestor.parent() else {
            return false;
        };
        ancestor = parent;
    }
}

fn format_browser_diagnostics(browser: &ChromiumSession) -> String {
    let diagnostics = browser.diagnostics();
    if diagnostics.is_empty() {
        return "none captured".to_string();
    }
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            format!(
                "[{}:{}] {}",
                diagnostic.kind, diagnostic.level, diagnostic.text
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryFile {
    path: PathBuf,
}

impl TemporaryFile {
    fn create(
        directory: &Path,
        purpose: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<Self, PdfError> {
        fs::create_dir_all(directory).map_err(|source| PdfError::Write {
            path: directory.to_path_buf(),
            source,
        })?;
        for _ in 0..32 {
            let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(
                ".zpres-{purpose}-{}-{sequence}.{extension}",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(source) = file.write_all(bytes) {
                        let _ = fs::remove_file(&path);
                        return Err(PdfError::Write { path, source });
                    }
                    return Ok(Self { path });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(PdfError::Write { path, source }),
            }
        }
        Err(PdfError::Write {
            path: directory.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a unique staged artifact",
            ),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn publish(mut self, destination: &Path) -> Result<(), PdfError> {
        fs::rename(&self.path, destination).map_err(|source| PdfError::Write {
            path: destination.to_path_buf(),
            source,
        })?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn publish_output_set(
    namespace: &OutputNamespaceGuard,
    outputs: Vec<(PathBuf, Vec<u8>)>,
) -> Result<Vec<PathBuf>, PdfError> {
    for (destination, _) in &outputs {
        namespace
            .ensure_peer_output_allowed(destination, OutputTargetKind::File)
            .map_err(|source| map_output_ownership_error(destination, source))?;
    }
    let staged = stage_output_set(outputs)?;
    publish_staged_output_set(staged)
}

fn stage_output_set(
    outputs: Vec<(PathBuf, Vec<u8>)>,
) -> Result<Vec<(PathBuf, TemporaryFile)>, PdfError> {
    let mut staged = Vec::with_capacity(outputs.len());
    for (destination, bytes) in outputs {
        ensure_parent_dir(&destination)?;
        let directory = destination.parent().unwrap_or_else(|| Path::new("."));
        let file = TemporaryFile::create(directory, "artifact", "tmp", &bytes)?;
        staged.push((destination, file));
    }
    Ok(staged)
}

fn publish_staged_output_set(
    staged: Vec<(PathBuf, TemporaryFile)>,
) -> Result<Vec<PathBuf>, PdfError> {
    let mut published = Vec::with_capacity(staged.len());
    for (destination, file) in staged {
        file.publish(&destination)?;
        published.push(destination);
    }
    Ok(published)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfSmokeReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub expected_pages: usize,
    pub observed_pages: usize,
    pub observed_content_streams: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngExportReport {
    pub output_dir: PathBuf,
    pub expected_pages: usize,
    pub exported_pages: usize,
    pub viewport: PngViewport,
    pub files: Vec<PathBuf>,
    raster_generation: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegExportReport {
    pub output_dir: PathBuf,
    pub expected_pages: usize,
    pub exported_pages: usize,
    pub viewport: PngViewport,
    pub quality: JpegQuality,
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngViewport {
    pub width: u32,
    pub height: u32,
}

impl Default for PngViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

pub(crate) fn validate_raster_output_viewport(viewport: PngViewport) -> Result<(), PdfError> {
    let has_positive_dimensions = viewport.width > 0 && viewport.height > 0;
    let is_exact_16_by_9 = u64::from(viewport.width) * 9 == u64::from(viewport.height) * 16;
    if has_positive_dimensions && is_exact_16_by_9 {
        Ok(())
    } else {
        Err(PdfError::InvalidRasterOutputSize {
            width: viewport.width,
            height: viewport.height,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegQuality(pub u8);

impl Default for JpegQuality {
    fn default() -> Self {
        Self(90)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngContactSheetReport {
    pub path: PathBuf,
    pub page_count: usize,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticExportPreflightReport {
    pub expected_pages: usize,
    pub observed_pages: usize,
    pub validated_pages: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfReadinessReport {
    pub expected_pages: usize,
    pub checked_math_blocks: usize,
    pub checked_chart_blocks: usize,
    pub checked_image_blocks: usize,
    pub checked_media_blocks: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum PdfError {
    #[error(
        "Chrome/Chromium was not found. Install Google Chrome or Chromium, or set ZPRES_CHROMIUM to the executable path. Requested: {requested:?}. Searched: {searched:?}"
    )]
    MissingChromium {
        requested: Option<PathBuf>,
        searched: Vec<PathBuf>,
    },
    #[error(transparent)]
    ThemeContract(#[from] theme::ThemeDeckContractError),
    #[error("failed to write PDF artifact at {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot publish Output target at {path}: {source}")]
    OutputOwnership {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("failed to publish raster page set at {path}: {source}")]
    RasterPublication {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(
        "PNG contact sheet '{}' must be outside the exclusively owned page directory '{}'",
        path.display(),
        directory.display()
    )]
    ContactSheetInsideRasterDirectory { path: PathBuf, directory: PathBuf },
    #[error("browser export failed: {message}")]
    Browser { message: String },
    #[error("browser-rendered static export failed validation: {message}")]
    BrowserValidation { message: String },
    #[error(
        "static image output size {width}x{height} must use positive dimensions and an exact 16:9 aspect ratio (for example, 1920x1080)"
    )]
    InvalidRasterOutputSize { width: u32, height: u32 },
    #[error("PDF export did not create {path}")]
    MissingPdf { path: PathBuf },
    #[error("PDF export created an empty file at {path}")]
    EmptyPdf { path: PathBuf },
    #[error("PNG export did not create {path}")]
    MissingPng { path: PathBuf },
    #[error("PNG export created an empty file at {path}")]
    EmptyPng { path: PathBuf },
    #[error("PNG export created an invalid image at {path}: {source}")]
    InvalidPng {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error(
        "PNG dimensions mismatch for {path}: expected {expected_width}x{expected_height}, observed {observed_width}x{observed_height}"
    )]
    PngDimensionMismatch {
        path: PathBuf,
        expected_width: u32,
        expected_height: u32,
        observed_width: u32,
        observed_height: u32,
    },
    #[error("JPEG export did not create {path}")]
    MissingJpeg { path: PathBuf },
    #[error("JPEG export created an empty file at {path}")]
    EmptyJpeg { path: PathBuf },
    #[error("JPEG export created an invalid image at {path}: {source}")]
    InvalidJpeg {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error(
        "JPEG dimensions mismatch for {path}: expected {expected_width}x{expected_height}, observed {observed_width}x{observed_height}"
    )]
    JpegDimensionMismatch {
        path: PathBuf,
        expected_width: u32,
        expected_height: u32,
        observed_width: u32,
        observed_height: u32,
    },
    #[error("PDF export created an invalid document at {path}: {source}")]
    InvalidPdf { path: PathBuf, source: lopdf::Error },
    #[error("failed to encode JPEG {path}: {source}")]
    EncodeJpeg {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error("PDF page count mismatch for {path}: expected {expected}, observed {observed}")]
    PageCountMismatch {
        path: PathBuf,
        expected: usize,
        observed: usize,
    },
    #[error(
        "PDF smoke check found blank or missing page content in {path}: expected at least {expected_pages} content stream(s), observed {observed_content_streams}"
    )]
    BlankPages {
        path: PathBuf,
        expected_pages: usize,
        observed_content_streams: usize,
    },
    #[error("PDF smoke check found unresolved content marker '{marker}' in {path}")]
    UnresolvedPdfContent { path: PathBuf, marker: String },
    #[error("PDF print HTML is missing the zpres-ready signal")]
    MissingReadySignal,
    #[error(
        "PDF print HTML readiness page count mismatch: expected {expected}, observed {observed}"
    )]
    ReadyPageCountMismatch { expected: usize, observed: usize },
    #[error("PDF export cannot reliably render {block} on slide '{slide_id}': {reason}")]
    UnsupportedPdfContent {
        slide_id: String,
        block: &'static str,
        reason: String,
    },
}

pub fn smoke_check_pdf(path: &Path, expected_pages: usize) -> Result<PdfSmokeReport, PdfError> {
    let bytes = fs::read(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PdfError::MissingPdf {
                path: path.to_path_buf(),
            }
        } else {
            PdfError::Write {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if bytes.is_empty() {
        return Err(PdfError::EmptyPdf {
            path: path.to_path_buf(),
        });
    }

    smoke_check_pdf_bytes(&bytes, path, expected_pages)
}

fn smoke_check_pdf_bytes(
    bytes: &[u8],
    path: &Path,
    expected_pages: usize,
) -> Result<PdfSmokeReport, PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::EmptyPdf {
            path: path.to_path_buf(),
        });
    }

    let document = Document::load_mem(bytes).map_err(|source| PdfError::InvalidPdf {
        path: path.to_path_buf(),
        source,
    })?;
    let pages = document.get_pages();

    let observed_pages = pages.len();
    if observed_pages != expected_pages {
        return Err(PdfError::PageCountMismatch {
            path: path.to_path_buf(),
            expected: expected_pages,
            observed: observed_pages,
        });
    }

    let mut observed_content_streams = 0;
    let mut page_content = Vec::new();
    for page_id in pages.values() {
        let content =
            document
                .get_page_content(*page_id)
                .map_err(|source| PdfError::InvalidPdf {
                    path: path.to_path_buf(),
                    source,
                })?;
        if content.iter().any(|byte| !byte.is_ascii_whitespace()) {
            observed_content_streams += 1;
        }
        page_content.extend_from_slice(&content);
    }
    if observed_content_streams < expected_pages {
        return Err(PdfError::BlankPages {
            path: path.to_path_buf(),
            expected_pages,
            observed_content_streams,
        });
    }
    for marker in unresolved_pdf_markers() {
        if contains_bytes(bytes, marker.as_bytes())
            || contains_bytes(&page_content, marker.as_bytes())
        {
            return Err(PdfError::UnresolvedPdfContent {
                path: path.to_path_buf(),
                marker: marker.to_string(),
            });
        }
    }

    Ok(PdfSmokeReport {
        path: path.to_path_buf(),
        bytes: bytes.len() as u64,
        expected_pages,
        observed_pages,
        observed_content_streams,
        warnings: Vec::new(),
    })
}

pub fn smoke_check_png(path: &Path, expected: PngViewport) -> Result<(), PdfError> {
    let bytes = fs::read(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PdfError::MissingPng {
                path: path.to_path_buf(),
            }
        } else {
            PdfError::Write {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if bytes.is_empty() {
        return Err(PdfError::EmptyPng {
            path: path.to_path_buf(),
        });
    }

    validate_png_bytes(&bytes, path, expected)
}

fn validate_png_bytes(bytes: &[u8], path: &Path, expected: PngViewport) -> Result<(), PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::EmptyPng {
            path: path.to_path_buf(),
        });
    }
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png).map_err(|source| {
        PdfError::InvalidPng {
            path: path.to_path_buf(),
            source,
        }
    })?;
    if image.width() != expected.width || image.height() != expected.height {
        return Err(PdfError::PngDimensionMismatch {
            path: path.to_path_buf(),
            expected_width: expected.width,
            expected_height: expected.height,
            observed_width: image.width(),
            observed_height: image.height(),
        });
    }
    Ok(())
}

fn encode_png_bytes_as_jpeg(
    input_png: &[u8],
    output_jpeg: &Path,
    quality: JpegQuality,
) -> Result<Vec<u8>, PdfError> {
    let image =
        image::load_from_memory_with_format(input_png, ImageFormat::Png).map_err(|source| {
            PdfError::InvalidPng {
                path: output_jpeg.with_extension("source.png"),
                source,
            }
        })?;
    let rgb = image.to_rgb8();
    let mut output = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut output, quality.0);
    encoder
        .encode(
            &rgb,
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|source| PdfError::EncodeJpeg {
            path: output_jpeg.to_path_buf(),
            source,
        })?;
    Ok(output)
}

pub fn smoke_check_jpeg(path: &Path, expected: PngViewport) -> Result<(), PdfError> {
    let bytes = fs::read(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PdfError::MissingJpeg {
                path: path.to_path_buf(),
            }
        } else {
            PdfError::Write {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if bytes.is_empty() {
        return Err(PdfError::EmptyJpeg {
            path: path.to_path_buf(),
        });
    }

    validate_jpeg_bytes(&bytes, path, expected)
}

fn validate_jpeg_bytes(bytes: &[u8], path: &Path, expected: PngViewport) -> Result<(), PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::EmptyJpeg {
            path: path.to_path_buf(),
        });
    }
    let image =
        image::load_from_memory_with_format(bytes, ImageFormat::Jpeg).map_err(|source| {
            PdfError::InvalidJpeg {
                path: path.to_path_buf(),
                source,
            }
        })?;
    if image.width() != expected.width || image.height() != expected.height {
        return Err(PdfError::JpegDimensionMismatch {
            path: path.to_path_buf(),
            expected_width: expected.width,
            expected_height: expected.height,
            observed_width: image.width(),
            observed_height: image.height(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PngContactSheetLayout {
    columns: u32,
    rows: u32,
    tile_width: u32,
    tile_height: u32,
    gap: u32,
    margin: u32,
    label_height: u32,
    width: u32,
    height: u32,
}

impl PngContactSheetLayout {
    fn for_page_count(page_count: usize) -> Self {
        let columns: u32 = match page_count {
            0 | 1 => 1,
            2..=4 => 2,
            5..=12 => 4,
            _ => 5,
        };
        let rows = (page_count as u32).div_ceil(columns).max(1);
        let tile_width = 300;
        let tile_height = 169;
        let gap = 18;
        let margin = 24;
        let label_height = 28;
        let width = margin * 2 + columns * tile_width + columns.saturating_sub(1) * gap;
        let height =
            margin * 2 + rows * (tile_height + label_height) + rows.saturating_sub(1) * gap;
        Self {
            columns,
            rows,
            tile_width,
            tile_height,
            gap,
            margin,
            label_height,
            width,
            height,
        }
    }
}

fn render_png_contact_sheet_html(page_images: &[PathBuf], layout: PngContactSheetLayout) -> String {
    let mut html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>zpres PNG contact sheet</title>\n<style>\nhtml, body {{ margin: 0; width: {}px; min-height: {}px; background: #111827; }}\nbody {{ box-sizing: border-box; padding: {}px; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif; color: #e5e7eb; }}\n.zpres-contact-sheet {{ display: grid; grid-template-columns: repeat({}, {}px); gap: {}px; }}\n.zpres-contact-page {{ display: grid; gap: 8px; margin: 0; }}\n.zpres-contact-page img {{ display: block; width: {}px; height: {}px; object-fit: cover; background: #020617; box-shadow: 0 8px 24px rgb(0 0 0 / 0.28); }}\n.zpres-contact-page span {{ font-size: 13px; line-height: 20px; color: #cbd5e1; }}\n</style>\n</head>\n<body>\n<main class=\"zpres-contact-sheet\" aria-label=\"PNG page contact sheet\">\n",
        layout.width,
        layout.height,
        layout.margin,
        layout.columns,
        layout.tile_width,
        layout.gap,
        layout.tile_width,
        layout.tile_height
    );
    for (index, image) in page_images.iter().enumerate() {
        let page_number = index + 1;
        html.push_str(&format!(
            "<figure class=\"zpres-contact-page\"><img src=\"{}\" alt=\"Page {}\"><span>page {}</span></figure>\n",
            escape_html(&file_url(image)),
            page_number,
            page_number
        ));
    }
    html.push_str("</main>\n</body>\n</html>\n");
    html
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
pub(crate) fn check_pdf_readiness(deck: &Deck) -> Result<PdfReadinessReport, PdfError> {
    check_pdf_readiness_with_options(deck, html::StaticExportOptions::default())
}

#[cfg(test)]
pub(crate) fn check_pdf_readiness_with_options(
    deck: &Deck,
    options: html::StaticExportOptions,
) -> Result<PdfReadinessReport, PdfError> {
    check_pdf_readiness_impl(
        deck,
        options,
        PresentationPlan::for_theme_api_v1(deck)
            .print_pages(options.include_speaker_notes)
            .len(),
        Some(&PresentationPlan::for_theme_api_v1(deck)),
    )
}

pub fn check_pdf_readiness_for_theme_with_options(
    deck: &Deck,
    theme: &RenderedTheme,
    options: html::StaticExportOptions,
) -> Result<PdfReadinessReport, PdfError> {
    let plan = (theme.manifest.api() == ThemeApiVersion::V1)
        .then(|| PresentationPlan::for_theme_api_v1(deck));
    check_pdf_readiness_impl(
        deck,
        options,
        html::print_page_count_for_theme_with_options(deck, theme, options),
        plan.as_ref(),
    )
}

fn check_pdf_readiness_impl(
    deck: &Deck,
    _options: html::StaticExportOptions,
    expected_pages: usize,
    plan: Option<&PresentationPlan<'_>>,
) -> Result<PdfReadinessReport, PdfError> {
    let mut report = PdfReadinessReport {
        expected_pages,
        checked_math_blocks: 0,
        checked_chart_blocks: 0,
        checked_image_blocks: 0,
        checked_media_blocks: 0,
        warnings: Vec::new(),
    };
    let deck_root = deck.deck_root();

    if let Some(plan) = plan {
        for (owner, background_image) in plan.declared_backgrounds() {
            report.checked_image_blocks += 1;
            ensure_pdf_image_ready(owner, &background_image.src, None, deck_root)?;
        }
        for planned in plan.authored_slides() {
            for block in &planned.slide.blocks {
                check_block_pdf_readiness(block, &planned.slide.id, deck_root, &mut report)?;
            }
        }
    } else {
        for slide in deck.pdf_slide_order() {
            if let Some(background_image) = &slide.background_image {
                report.checked_image_blocks += 1;
                ensure_pdf_image_ready(&slide.id, &background_image.src, None, deck_root)?;
            }
            for block in &slide.blocks {
                check_block_pdf_readiness(block, &slide.id, deck_root, &mut report)?;
            }
        }
    }

    Ok(report)
}

fn check_block_pdf_readiness(
    block: &ContentBlock,
    slide_id: &str,
    deck_root: Option<&Path>,
    report: &mut PdfReadinessReport,
) -> Result<(), PdfError> {
    match block {
        ContentBlock::Paragraph { inline_math, .. }
        | ContentBlock::FitText { inline_math, .. }
        | ContentBlock::Quote { inline_math, .. } => {
            for latex in inline_math {
                report.checked_math_blocks += 1;
                ensure_katex_renders(slide_id, latex)?;
            }
        }
        ContentBlock::Heading { text, .. } => {
            for latex in inline_math_segments(text) {
                report.checked_math_blocks += 1;
                ensure_katex_renders(slide_id, &latex)?;
            }
        }
        ContentBlock::Callout {
            title, inline_math, ..
        } => {
            for latex in inline_math {
                report.checked_math_blocks += 1;
                ensure_katex_renders(slide_id, latex)?;
            }
            if let Some(title) = title {
                for latex in inline_math_segments(title) {
                    report.checked_math_blocks += 1;
                    ensure_katex_renders(slide_id, &latex)?;
                }
            }
        }
        ContentBlock::List { items, .. } => {
            for item in items {
                for latex in &item.inline_math {
                    report.checked_math_blocks += 1;
                    ensure_katex_renders(slide_id, latex)?;
                }
            }
        }
        ContentBlock::Math { latex, .. } => {
            report.checked_math_blocks += 1;
            ensure_katex_renders(slide_id, latex)?;
        }
        ContentBlock::Table { headers, rows, .. } => {
            for cell in headers.iter().chain(rows.iter().flatten()) {
                for latex in inline_math_segments(cell) {
                    report.checked_math_blocks += 1;
                    ensure_katex_renders(slide_id, &latex)?;
                }
            }
        }
        ContentBlock::Figure {
            src,
            static_src,
            caption,
            ..
        } => {
            report.checked_image_blocks += 1;
            ensure_pdf_image_ready(slide_id, src, static_src.as_deref(), deck_root)?;
            if let Some(caption) = caption {
                for latex in inline_math_segments(caption) {
                    report.checked_math_blocks += 1;
                    ensure_katex_renders(slide_id, &latex)?;
                }
            }
        }
        ContentBlock::Footnotes { notes } => {
            for note in notes {
                for latex in &note.inline_math {
                    report.checked_math_blocks += 1;
                    ensure_katex_renders(slide_id, latex)?;
                }
            }
        }
        ContentBlock::Gallery { items, .. } => {
            for item in items {
                report.checked_image_blocks += 1;
                ensure_pdf_image_ready(slide_id, &item.src, item.static_src.as_deref(), deck_root)?;
                if let Some(caption) = &item.caption {
                    for latex in inline_math_segments(caption) {
                        report.checked_math_blocks += 1;
                        ensure_katex_renders(slide_id, &latex)?;
                    }
                }
            }
        }
        ContentBlock::Media {
            kind,
            src,
            poster,
            caption,
            visual_hidden,
            ..
        } => {
            report.checked_media_blocks += 1;
            ensure_pdf_media_ready(
                slide_id,
                *kind,
                src,
                poster.as_deref(),
                *visual_hidden,
                deck_root,
                report,
            )?;
            if let Some(caption) = caption {
                for latex in inline_math_segments(caption) {
                    report.checked_math_blocks += 1;
                    ensure_katex_renders(slide_id, &latex)?;
                }
            }
        }
        ContentBlock::Diagram { language, source } => {
            ensure_pdf_diagram_ready(slide_id, *language, source)?;
        }
        ContentBlock::Chart { spec, data, .. } => {
            report.checked_chart_blocks += 1;
            ensure_pdf_chart_ready(slide_id, spec, data.as_ref(), deck_root)?;
        }
        ContentBlock::Layout { regions, .. } => {
            for region in regions {
                for block in &region.blocks {
                    check_block_pdf_readiness(block, slide_id, deck_root, report)?;
                }
            }
        }
        ContentBlock::HtmlOnly { .. } => {
            report.warnings.push(format!(
                "slide '{slide_id}' contains HTML-only content; static export captures its Chromium rendering"
            ));
        }
        ContentBlock::UnsupportedDirective { name, .. } => {
            return Err(PdfError::UnsupportedPdfContent {
                slide_id: slide_id.to_string(),
                block: "unsupported directive",
                reason: format!("directive '{name}' has no declared PDF/static representation"),
            });
        }
        ContentBlock::Code { .. }
        | ContentBlock::Steps { .. }
        | ContentBlock::SpeakerNotes { .. } => {}
    }
    Ok(())
}

fn ensure_katex_renders(slide_id: &str, latex: &str) -> Result<(), PdfError> {
    let opts = katex::Opts::builder()
        .output_type(katex::OutputType::Mathml)
        .throw_on_error(true)
        .build()
        .map_err(|error| PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block: "math",
            reason: format!("failed to prepare KaTeX options: {error}"),
        })?;
    katex::render_with_opts(latex, &opts).map_err(|error| PdfError::UnsupportedPdfContent {
        slide_id: slide_id.to_string(),
        block: "math",
        reason: format!("KaTeX could not render '{latex}' for PDF: {error}"),
    })?;
    Ok(())
}

fn ensure_pdf_image_ready(
    slide_id: &str,
    src: &str,
    static_src: Option<&str>,
    deck_root: Option<&Path>,
) -> Result<(), PdfError> {
    if let Some(static_src) = static_src {
        return ensure_local_pdf_image_ready(slide_id, static_src, deck_root, "static fallback");
    }
    if looks_remote(src) {
        return Err(PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block: "figure",
            reason: format!("remote image '{src}' is not a reliable PDF dependency"),
        });
    }
    if looks_like_gif(src) {
        return Err(PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block: "figure",
            reason: format!(
                "GIF image '{src}' needs a local pdf-src/static-src fallback for reliable static export"
            ),
        });
    }
    ensure_local_pdf_image_ready(slide_id, src, deck_root, "local image")
}

fn looks_like_gif(src: &str) -> bool {
    let path = src.split(['?', '#']).next().unwrap_or(src);
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("gif"))
}

fn ensure_local_pdf_image_ready(
    slide_id: &str,
    src: &str,
    deck_root: Option<&Path>,
    label: &'static str,
) -> Result<(), PdfError> {
    let Some(deck_root) = deck_root else {
        return Err(PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block: "figure",
            reason: format!("{label} cannot be resolved without a Deck root"),
        });
    };
    if !deck_root.join(src).is_file() {
        return Err(PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block: "figure",
            reason: format!("{label} '{src}' is missing before PDF capture"),
        });
    }
    Ok(())
}

fn ensure_pdf_media_ready(
    slide_id: &str,
    kind: MediaKind,
    src: &str,
    poster: Option<&str>,
    visual_hidden: bool,
    deck_root: Option<&Path>,
    report: &mut PdfReadinessReport,
) -> Result<(), PdfError> {
    if !looks_remote(src) {
        ensure_local_media_dependency(slide_id, "media", src, deck_root)?;
    }
    if visual_hidden {
        report.warnings.push(format!(
            "slide '{slide_id}' contains hidden {} media; static export omits its visual representation",
            media_kind_name(kind)
        ));
        return Ok(());
    }
    match kind {
        MediaKind::Video | MediaKind::Iframe => {
            let Some(poster) = poster else {
                return Err(PdfError::UnsupportedPdfContent {
                    slide_id: slide_id.to_string(),
                    block: "media",
                    reason: format!(
                        "{} media requires a local poster image for reliable PDF export",
                        media_kind_name(kind)
                    ),
                });
            };
            ensure_pdf_image_ready(slide_id, poster, None, deck_root)?;
        }
        MediaKind::Audio => {
            report.warnings.push(format!(
                "slide '{slide_id}' contains audio media; static export renders it as a static media card"
            ));
        }
    }
    Ok(())
}

fn ensure_pdf_diagram_ready(
    slide_id: &str,
    language: DiagramLanguage,
    source: &str,
) -> Result<(), PdfError> {
    match language {
        DiagramLanguage::Mermaid => {
            if let Some(reason) = mermaid_static_renderability_error(source) {
                return Err(PdfError::UnsupportedPdfContent {
                    slide_id: slide_id.to_string(),
                    block: "diagram",
                    reason,
                });
            }
        }
    }
    Ok(())
}

fn ensure_local_media_dependency(
    slide_id: &str,
    block: &'static str,
    src: &str,
    deck_root: Option<&Path>,
) -> Result<(), PdfError> {
    let Some(deck_root) = deck_root else {
        return Err(PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block,
            reason: "local media cannot be resolved without a Deck root".to_string(),
        });
    };
    if !deck_root.join(src).is_file() {
        return Err(PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block,
            reason: format!("local media '{src}' is missing before PDF capture"),
        });
    }
    Ok(())
}

fn media_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Iframe => "iframe",
    }
}

fn ensure_pdf_chart_ready(
    slide_id: &str,
    spec: &serde_json::Value,
    data: Option<&ChartData>,
    deck_root: Option<&Path>,
) -> Result<(), PdfError> {
    crate::chart::load(spec, data, deck_root)
        .map(|_| ())
        .map_err(|reason| PdfError::UnsupportedPdfContent {
            slide_id: slide_id.to_string(),
            block: "chart",
            reason,
        })
}

pub(crate) fn check_print_html_ready(
    print_html: &str,
    expected_pages: usize,
) -> Result<(), PdfError> {
    if !print_html.contains(r#"data-zpres-ready="pending""#)
        && !print_html.contains(r#"data-zpres-ready="true""#)
    {
        return Err(PdfError::MissingReadySignal);
    }
    let marker = r#"data-zpres-page-count=""#;
    let Some(start) = print_html.find(marker).map(|index| index + marker.len()) else {
        return Err(PdfError::ReadyPageCountMismatch {
            expected: expected_pages,
            observed: 0,
        });
    };
    let observed = print_html[start..]
        .split('"')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if observed != expected_pages {
        return Err(PdfError::ReadyPageCountMismatch {
            expected: expected_pages,
            observed,
        });
    }
    let body = print_html
        .find("<body")
        .map(|index| &print_html[index..])
        .unwrap_or(print_html);
    for marker in [
        "debug-unresolved-math",
        "debug-placeholder",
        "data-zpres-overflow=\"clipped\"",
    ] {
        if body.contains(marker) {
            return Err(PdfError::UnsupportedPdfContent {
                slide_id: "print-html".to_string(),
                block: "readiness marker",
                reason: format!("print HTML contains unresolved marker '{marker}'"),
            });
        }
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), PdfError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| PdfError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn unresolved_pdf_markers() -> &'static [&'static str] {
    &[
        "debug-unresolved-math",
        "debug-placeholder",
        "zpres-unresolved",
        "data-zpres-overflow=\"clipped\"",
    ]
}

fn contains_bytes(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn looks_remote(src: &str) -> bool {
    src.starts_with("http://") || src.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::deck::{parse_source_file, parse_source_text};
    use crate::theme;
    use base64::Engine;
    use image::{Rgb, RgbImage};
    use lopdf::{Object, Stream, dictionary};
    use tempfile::tempdir;

    fn test_pdf_bytes(page_contents: &[&[u8]]) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::new();

        for content in page_contents {
            let content_id = document.add_object(Stream::new(dictionary! {}, content.to_vec()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "MediaBox" => vec![0.into(), 0.into(), 1280.into(), 720.into()],
            });
            page_ids.push(page_id.into());
        }

        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids,
                "Count" => page_contents.len() as i64,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn write_test_image(path: &Path, format: ImageFormat, width: u32, height: u32) {
        RgbImage::from_pixel(width, height, Rgb([18, 52, 86]))
            .save_with_format(path, format)
            .unwrap();
    }

    fn rendered_theme(deck: &Deck) -> theme::RenderedTheme {
        theme::render_deck_theme(
            deck.metadata.theme.as_deref(),
            deck.deck_root().unwrap_or_else(|| Path::new(".")),
            &[theme::builtin_theme_search_path()],
            &deck.metadata.theme_params,
        )
        .unwrap()
    }

    fn v1_rendered_theme(path: &Path) -> theme::RenderedTheme {
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join("theme.toml"),
            r#"[theme]
name = "pdf-v1"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(path.join("theme.css.tmpl"), "").unwrap();
        fs::write(path.join("print.css.tmpl"), "").unwrap();
        let manifest = theme::load_theme_manifest(&path.join("theme.toml")).unwrap();
        theme::render_theme(&manifest, &BTreeMap::new()).unwrap()
    }

    fn v1_theme_with_external_font_and_texture(path: &Path) -> theme::RenderedTheme {
        const TEST_FONT_BASE64: &str = "AAEAAAAKAIAAAwAgT1MvMkUAQ7AAAAEoAAAAYGNtYXAAdABcAAABlAAAADxnbHlmpjZxAAAAAdgAAAA8aGVhZC4/TwIAAACsAAAANmhoZWEFJQHfAAAA5AAAACRobXR4BXgASwAAAYgAAAAMbG9jYQANACsAAAHQAAAACG1heHAABQAJAAABCAAAACBuYW1lsCwxeAAAAhQAAAHOcG9zdAAIACQAAAPkAAAAKAABAAAAAQAA+/4pQ18PPPUAAQPoAAAAAOZ4hbMAAAAA5niFswAyAAACCAK8AAAAAwACAAAAAAAAAAEAAAMg/zgAAAJYABkASwHqAAEAAAAAAAAAAAAAAAAAAAADAAEAAAADAAcAAQAAAAAAAgAAAAAAAAAAAAAAAAAAAAAAAwHTAZAABQAEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAPz8/PwAAACAAQQMg/zgAAAMgAMgAAAAAAAAAAAAAAAAAAAAgAAACWAAyASwAAAH0ABkAAAACAAAAAwAAABQAAwABAAAAFAAEACgAAAAGAAQAAQACACAAQf//AAAAIABB////4f/BAAEAAAAAAAAAAAANAA0AHgABAFAAAAIIArwAAwAAMxEhEVABuAK8/UQAAAEAMgAAAcICvAAGAAAzExMjJyMHMsjIbihkKAK8/USgoAAAAAwAlgABAAAAAAABAA8AAAABAAAAAAACAAcADwABAAAAAAADABsAFgABAAAAAAAEABcAMQABAAAAAAAFAAsASAABAAAAAAAGABUAUwADAAEECQABAB4AaAADAAEECQACAA4AhgADAAEECQADADYAlAADAAEECQAEAC4AygADAAEECQAFABYA+AADAAEECQAGACoBDlpwcmVzIFRlc3QgRmFjZVJlZ3VsYXJacHJlcyBUZXN0IEZhY2UgUmVndWxhciAxLjBacHJlcyBUZXN0IEZhY2UgUmVndWxhclZlcnNpb24gMS4wWnByZXNUZXN0RmFjZS1SZWd1bGFyAFoAcAByAGUAcwAgAFQAZQBzAHQAIABGAGEAYwBlAFIAZQBnAHUAbABhAHIAWgBwAHIAZQBzACAAVABlAHMAdAAgAEYAYQBjAGUAIABSAGUAZwB1AGwAYQByACAAMQAuADAAWgBwAHIAZQBzACAAVABlAHMAdAAgAEYAYQBjAGUAIABSAGUAZwB1AGwAYQByAFYAZQByAHMAaQBvAG4AIAAxAC4AMABaAHAAcgBlAHMAVABlAHMAdABGAGEAYwBlAC0AUgBlAGcAdQBsAGEAcgAAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAMAJA==";
        fs::create_dir_all(path.join("fonts")).unwrap();
        fs::create_dir_all(path.join("textures")).unwrap();
        fs::write(
            path.join("theme.toml"),
            r#"[theme]
name = "static-dependencies"
version = "0.1.0"
api_version = 1
fonts = ["fonts/zpres-test.ttf"]
assets = ["textures/grid.svg"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(
            path.join("theme.css.tmpl"),
            r#"@font-face {
  font-family: "Zpres Test Face";
  src: url("fonts/zpres-test.ttf") format("truetype");
  font-display: block;
}
.zpres-slide-title { font-family: "Zpres Test Face", sans-serif; }
.zpres-slide { background-image: url("textures/grid.svg"); }
"#,
        )
        .unwrap();
        fs::write(
            path.join("print.css.tmpl"),
            ".zpres-print-slide { background-image: url(\"textures/grid.svg\"); }\n",
        )
        .unwrap();
        fs::write(
            path.join("fonts/zpres-test.ttf"),
            base64::engine::general_purpose::STANDARD
                .decode(TEST_FONT_BASE64)
                .unwrap(),
        )
        .unwrap();
        fs::write(
            path.join("textures/grid.svg"),
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#f8fafc"/><path d="M0 0h32M0 0v32" stroke="#dbeafe"/></svg>"##,
        )
        .unwrap();
        let manifest = theme::load_theme_manifest(&path.join("theme.toml")).unwrap();
        theme::render_theme(&manifest, &BTreeMap::new()).unwrap()
    }

    #[test]
    fn publishing_an_arbitrary_page_named_png_does_not_claim_sibling_page_files() {
        let temp = tempdir().unwrap();
        let output_dir = temp.path().join("contact");
        fs::create_dir_all(&output_dir).unwrap();
        let requested_output = output_dir.join("page-001.png");
        let sibling_page = output_dir.join("page-002.png");
        fs::write(&requested_output, b"old contact sheet").unwrap();
        fs::write(&sibling_page, b"user page").unwrap();

        let namespace = OutputNamespaceGuard::acquire(&requested_output).unwrap();
        publish_output_set(
            &namespace,
            vec![(requested_output.clone(), b"new contact sheet".to_vec())],
        )
        .unwrap();

        assert_eq!(fs::read(&requested_output).unwrap(), b"new contact sheet");
        assert_eq!(fs::read(&sibling_page).unwrap(), b"user page");
    }

    #[test]
    fn generic_pdf_file_publisher_cannot_replace_the_namespace_lock() {
        let temp = tempdir().unwrap();
        let namespace = OutputNamespaceGuard::acquire(temp.path()).unwrap();
        let lock_path = namespace.lock_path().to_path_buf();
        let before = fs::metadata(&lock_path).unwrap();

        assert!(matches!(
            publish_output_set(
                &namespace,
                vec![(lock_path.clone(), b"replacement".to_vec())]
            ),
            Err(PdfError::OutputOwnership { .. })
        ));
        let after = fs::metadata(&lock_path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        }
    }

    #[test]
    fn contact_sheet_overlap_check_covers_the_page_root_and_missing_descendants() {
        let temp = tempdir().unwrap();
        let pages = temp.path().join("pages");
        let outside = temp.path().join("outside");
        fs::create_dir(&pages).unwrap();
        fs::create_dir(&outside).unwrap();
        let canonical_pages = fs::canonicalize(&pages).unwrap();

        assert!(path_is_within_directory(&pages, &canonical_pages));
        assert!(path_is_within_directory(
            &pages.join("contact.png"),
            &canonical_pages
        ));
        assert!(path_is_within_directory(
            &pages.join("missing/subdir/contact.png"),
            &canonical_pages
        ));
        assert!(!path_is_within_directory(
            &outside.join("contact.png"),
            &canonical_pages
        ));
    }

    #[test]
    fn missing_chromium_error_is_actionable() {
        let source = "# Test\n";
        let deck = parse_source_text(source, Some(PathBuf::from("test.zp.md"))).unwrap();
        let temp = tempdir().unwrap();
        let renderer = ChromiumPdfRenderer::from_executable(temp.path().join("missing-chromium"));

        let error = renderer
            .export(&deck, &rendered_theme(&deck), &temp.path().join("out.pdf"))
            .unwrap_err();

        assert!(matches!(error, PdfError::MissingChromium { .. }));
        assert!(error.to_string().contains("ZPRES_CHROMIUM"));
        assert!(error.to_string().contains("missing-chromium"));
    }

    #[test]
    fn missing_chromium_error_is_actionable_for_png_export() {
        let source = "# Test\n";
        let deck = parse_source_text(source, Some(PathBuf::from("test.zp.md"))).unwrap();
        let temp = tempdir().unwrap();
        let renderer = ChromiumPdfRenderer::from_executable(temp.path().join("missing-chromium"));

        let error = renderer
            .export_png_pages(&deck, &rendered_theme(&deck), &temp.path().join("png"))
            .unwrap_err();

        assert!(matches!(error, PdfError::MissingChromium { .. }));
        assert!(error.to_string().contains("ZPRES_CHROMIUM"));
        assert!(error.to_string().contains("missing-chromium"));
    }

    #[test]
    fn v1_pdf_export_rejects_unsupported_deck_before_creating_output() {
        let temp = tempdir().unwrap();
        let deck = parse_source_text(
            "---\naspect: \"4:3\"\n---\n\n# Unsupported aspect\n\nBody\n",
            Some(temp.path().join("four-three.zp.md")),
        )
        .unwrap();
        let theme = v1_rendered_theme(&temp.path().join("theme"));
        let renderer = ChromiumPdfRenderer::from_executable(temp.path().join("missing-chromium"));
        let output = temp.path().join("unpublished").join("out.pdf");

        let error = renderer.export(&deck, &theme, &output).unwrap_err();

        assert!(matches!(error, PdfError::ThemeContract(_)));
        assert!(error.to_string().contains("supports only 16:9"));
        assert!(!output.parent().unwrap().exists());
    }

    #[test]
    fn invalid_raster_aspect_fails_before_creating_output_directories() {
        let source = "# Test\n";
        let deck = parse_source_text(source, Some(PathBuf::from("test.zp.md"))).unwrap();
        let temp = tempdir().unwrap();
        let renderer = ChromiumPdfRenderer::from_executable(temp.path().join("missing-chromium"));
        let invalid_viewport = PngViewport {
            width: 1024,
            height: 768,
        };

        let png_dir = temp.path().join("png");
        let png_error = renderer
            .export_png_pages_with_viewport(
                &deck,
                &rendered_theme(&deck),
                &png_dir,
                html::StaticExportOptions::default(),
                invalid_viewport,
            )
            .unwrap_err();
        assert!(matches!(
            &png_error,
            PdfError::InvalidRasterOutputSize {
                width: 1024,
                height: 768
            }
        ));
        assert!(png_error.to_string().contains("exact 16:9"));
        assert!(!png_dir.exists());

        let jpeg_dir = temp.path().join("jpeg");
        let jpeg_error = renderer
            .export_jpeg_pages_with_viewport(
                &deck,
                &rendered_theme(&deck),
                &jpeg_dir,
                html::StaticExportOptions::default(),
                invalid_viewport,
                JpegQuality::default(),
            )
            .unwrap_err();
        assert!(matches!(
            &jpeg_error,
            PdfError::InvalidRasterOutputSize {
                width: 1024,
                height: 768
            }
        ));
        assert!(!jpeg_dir.exists());
    }

    #[test]
    fn real_chromium_exports_pdf_png_jpeg_and_contact_sheet() {
        let renderer = match ChromiumPdfRenderer::discover() {
            Ok(renderer) => renderer,
            Err(PdfError::MissingChromium { .. }) => {
                eprintln!(
                    "skipping real Chromium export test: install Chrome/Chromium or set ZPRES_CHROMIUM"
                );
                return;
            }
            Err(error) => panic!("unexpected Chromium discovery failure: {error}"),
        };
        let temp = tempdir().unwrap();
        let root = temp.path().join("source # % ü");
        fs::create_dir_all(&root).unwrap();
        let source = "---\naspect: \"16:9\"\n---\n\n# AAAA\n\nVisible first-page content.\n\n---\n\n# AAAA\n\nVisible second-page content.\n";
        let deck = parse_source_text(source, Some(root.join("talk.zp.md"))).unwrap();
        let theme = v1_theme_with_external_font_and_texture(&root.join("theme # % ü"));

        let preflight = renderer
            .preflight_static_export_with_options(
                &deck,
                &theme,
                html::StaticExportOptions::default(),
            )
            .unwrap();
        assert_eq!(preflight.expected_pages, 2);
        assert_eq!(preflight.observed_pages, 2);
        assert_eq!(preflight.validated_pages, 2);

        let pdf_path = root.join("talk.pdf");
        let pdf = renderer.export(&deck, &theme, &pdf_path).unwrap();
        assert_eq!(pdf.observed_pages, 2);
        assert!(pdf_path.is_file());

        let raster_viewport = PngViewport {
            width: 1920,
            height: 1080,
        };
        let png_dir = root.join("png");
        let png = renderer
            .export_png_pages_with_viewport(
                &deck,
                &theme,
                &png_dir,
                html::StaticExportOptions::default(),
                raster_viewport,
            )
            .unwrap();
        assert_eq!(png.exported_pages, 2);
        assert_eq!(png.viewport, raster_viewport);
        for path in &png.files {
            smoke_check_png(path, raster_viewport).unwrap();
        }

        let jpeg_dir = root.join("jpeg");
        let jpeg = renderer
            .export_jpeg_pages_with_viewport(
                &deck,
                &theme,
                &jpeg_dir,
                html::StaticExportOptions::default(),
                raster_viewport,
                JpegQuality(85),
            )
            .unwrap();
        assert_eq!(jpeg.exported_pages, 2);
        assert_eq!(jpeg.viewport, raster_viewport);
        for path in &jpeg.files {
            smoke_check_jpeg(path, raster_viewport).unwrap();
        }

        let contact_path = root.join("contact.png");
        let contact = renderer
            .export_png_contact_sheet_for_report(&png, &contact_path)
            .unwrap();
        smoke_check_png(
            &contact.path,
            PngViewport {
                width: contact.width,
                height: contact.height,
            },
        )
        .unwrap();

        let shorter_source = "# Replacement page\n\nOnly one page remains.\n";
        let shorter_deck =
            parse_source_text(shorter_source, Some(root.join("shorter.zp.md"))).unwrap();
        let shorter_theme = rendered_theme(&shorter_deck);
        let shorter_png = renderer
            .export_png_pages_with_viewport(
                &shorter_deck,
                &shorter_theme,
                &png_dir,
                html::StaticExportOptions::default(),
                raster_viewport,
            )
            .unwrap();
        assert_eq!(shorter_png.exported_pages, 1);
        assert!(!png_dir.join("page-002.png").exists());

        let shorter_jpeg = renderer
            .export_jpeg_pages_with_viewport(
                &shorter_deck,
                &shorter_theme,
                &jpeg_dir,
                html::StaticExportOptions::default(),
                raster_viewport,
                JpegQuality(85),
            )
            .unwrap();
        assert_eq!(shorter_jpeg.exported_pages, 1);
        assert!(!jpeg_dir.join("page-002.jpg").exists());
    }

    #[test]
    fn real_chromium_preflights_and_exports_speaker_notes_pages() {
        let renderer = match ChromiumPdfRenderer::discover() {
            Ok(renderer) => renderer,
            Err(PdfError::MissingChromium { .. }) => {
                eprintln!(
                    "skipping real Chromium speaker-notes test: install Chrome/Chromium or set ZPRES_CHROMIUM"
                );
                return;
            }
            Err(error) => panic!("unexpected Chromium discovery failure: {error}"),
        };
        let temp = tempdir().unwrap();
        let source = r#"# Result

The result is visible on the Main slide.

::: notes
Pause before explaining the consequence.
:::
"#;
        let deck = parse_source_text(source, Some(temp.path().join("notes.zp.md"))).unwrap();
        let theme = rendered_theme(&deck);
        let options = html::StaticExportOptions {
            include_speaker_notes: true,
        };

        let preflight = renderer
            .preflight_static_export_with_options(&deck, &theme, options)
            .unwrap();
        assert_eq!(preflight.expected_pages, 2);
        assert_eq!(preflight.observed_pages, 2);
        assert_eq!(preflight.validated_pages, 2);

        let pdf_path = temp.path().join("notes.pdf");
        let report = renderer
            .export_with_options(&deck, &theme, &pdf_path, options)
            .unwrap();
        assert_eq!(report.expected_pages, 2);
        assert_eq!(report.observed_pages, 2);
        assert!(pdf_path.is_file());
    }

    #[test]
    fn real_chromium_rejects_overflowing_speaker_notes_page() {
        let renderer = match ChromiumPdfRenderer::discover() {
            Ok(renderer) => renderer,
            Err(PdfError::MissingChromium { .. }) => {
                eprintln!(
                    "skipping real Chromium speaker-notes overflow test: install Chrome/Chromium or set ZPRES_CHROMIUM"
                );
                return;
            }
            Err(error) => panic!("unexpected Chromium discovery failure: {error}"),
        };
        let temp = tempdir().unwrap();
        let notes = (1..=80)
            .map(|line| format!("Line {line}: a presenter note that must remain visible."))
            .collect::<Vec<_>>()
            .join("\n");
        let source =
            format!("# Result\n\nThe Main slide remains short.\n\n::: notes\n{notes}\n:::\n");
        let deck = parse_source_text(&source, Some(temp.path().join("long-notes.zp.md"))).unwrap();
        let theme = rendered_theme(&deck);

        let error = renderer
            .preflight_static_export_with_options(
                &deck,
                &theme,
                html::StaticExportOptions {
                    include_speaker_notes: true,
                },
            )
            .unwrap_err();

        assert!(matches!(error, PdfError::BrowserValidation { .. }));
        assert!(
            error.to_string().contains("content-outside-canvas")
                || error.to_string().contains("scroll-overflow"),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn real_chromium_rejects_clipped_slide_before_replacing_pdf() {
        let renderer = match ChromiumPdfRenderer::discover() {
            Ok(renderer) => renderer,
            Err(PdfError::MissingChromium { .. }) => {
                eprintln!(
                    "skipping real Chromium clipping test: install Chrome/Chromium or set ZPRES_CHROMIUM"
                );
                return;
            }
            Err(error) => panic!("unexpected Chromium discovery failure: {error}"),
        };
        let temp = tempdir().unwrap();
        let mut source = String::from("---\nautoscale: true\n---\n\n# Deliberate overflow\n\n");
        for item in 1..=160 {
            source.push_str(&format!("- visible overflow item {item}\n"));
        }
        let deck = parse_source_text(&source, Some(temp.path().join("overflow.zp.md"))).unwrap();
        let theme = rendered_theme(&deck);
        let output_pdf = temp.path().join("last-good.pdf");
        fs::write(&output_pdf, b"last-good-artifact").unwrap();

        let error = renderer.export(&deck, &theme, &output_pdf).unwrap_err();

        assert!(matches!(error, PdfError::BrowserValidation { .. }));
        assert!(error.to_string().contains("section-1-main"));
        assert!(error.to_string().contains("clipped-content"));
        assert_eq!(fs::read(&output_pdf).unwrap(), b"last-good-artifact");
    }

    #[test]
    fn png_viewport_default_matches_existing_export_size() {
        assert_eq!(
            PngViewport::default(),
            PngViewport {
                width: 1280,
                height: 720,
            }
        );
    }

    #[test]
    fn contact_sheet_html_uses_page_grid_and_local_file_urls() {
        let temp = tempdir().unwrap();
        let pages = (1..=10)
            .map(|page| temp.path().join(format!("page-{page:03}.png")))
            .collect::<Vec<_>>();
        let layout = PngContactSheetLayout::for_page_count(pages.len());

        let html = render_png_contact_sheet_html(&pages, layout);

        assert_eq!(layout.columns, 4);
        assert_eq!(layout.rows, 3);
        assert_eq!(layout.width, 1302);
        assert_eq!(layout.height, 675);
        assert!(html.contains("grid-template-columns: repeat(4, 300px)"));
        assert!(html.contains("aria-label=\"PNG page contact sheet\""));
        assert_eq!(html.matches("class=\"zpres-contact-page\"").count(), 10);
        assert!(html.contains("page-001.png"));
        assert!(html.contains("alt=\"Page 10\""));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn contact_sheet_containment_recognizes_apfs_case_and_unicode_aliases() {
        let temp = tempdir().unwrap();
        for (index, (owned_name, alias_name)) in [
            ("Pages", "pages"),
            ("caf\u{e9}", "cafe\u{301}"),
            ("stra\u{df}e", "STRASSE"),
            ("\u{fb03}", "FFI"),
        ]
        .into_iter()
        .enumerate()
        {
            let root = temp.path().join(format!("case-{index}"));
            let owned = root.join(owned_name);
            fs::create_dir_all(&owned).unwrap();
            let canonical_owned = fs::canonicalize(&owned).unwrap();
            let aliased_contact = root.join(alias_name).join("contact.png");

            assert!(
                path_is_within_directory(&aliased_contact, &canonical_owned),
                "did not recognize APFS alias {alias_name:?} for {owned_name:?}"
            );
        }

        let firmlink_temp = tempfile::tempdir_in("/private/tmp").unwrap();
        let owned = firmlink_temp.path().join("Pages");
        fs::create_dir(&owned).unwrap();
        let canonical_owned = fs::canonicalize(&owned).unwrap();
        let data_alias = Path::new("/System/Volumes/Data")
            .join(firmlink_temp.path().strip_prefix(Path::new("/")).unwrap());
        let aliased_contact = data_alias.join("pages").join("contact.png");
        assert!(path_is_within_directory(&aliased_contact, &canonical_owned));
    }

    #[test]
    fn smoke_check_rejects_wrong_page_count() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("sample.pdf");
        fs::write(&path, test_pdf_bytes(&[b"q\nQ\n"])).unwrap();

        let error = smoke_check_pdf(&path, 2).unwrap_err();

        assert!(matches!(error, PdfError::PageCountMismatch { .. }));
    }

    #[test]
    fn smoke_check_parses_page_tree_and_content_streams() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("sample.pdf");
        fs::write(&path, test_pdf_bytes(&[b"q\nQ\n"])).unwrap();

        let report = smoke_check_pdf(&path, 1).unwrap();

        assert_eq!(report.observed_pages, 1);
        assert_eq!(report.observed_content_streams, 1);
    }

    #[test]
    fn smoke_check_rejects_blank_pages_without_content_streams() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("blank.pdf");
        fs::write(&path, test_pdf_bytes(&[b""])).unwrap();

        let error = smoke_check_pdf(&path, 1).unwrap_err();

        assert!(matches!(error, PdfError::BlankPages { .. }));
    }

    #[test]
    fn smoke_check_rejects_unresolved_pdf_markers() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("unresolved.pdf");
        fs::write(&path, test_pdf_bytes(&[b"debug-unresolved-math\n"])).unwrap();

        let error = smoke_check_pdf(&path, 1).unwrap_err();

        assert!(matches!(error, PdfError::UnresolvedPdfContent { .. }));
    }

    #[test]
    fn smoke_check_rejects_nonempty_malformed_pdf() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("malformed.pdf");
        fs::write(&path, b"%PDF-1.7\nnot actually a PDF\n").unwrap();

        let error = smoke_check_pdf(&path, 1).unwrap_err();

        assert!(matches!(error, PdfError::InvalidPdf { .. }));
    }

    #[test]
    fn smoke_check_png_decodes_and_verifies_dimensions() {
        let temp = tempdir().unwrap();
        let missing = temp.path().join("missing.png");
        let empty = temp.path().join("empty.png");
        fs::write(&empty, []).unwrap();
        let valid = temp.path().join("valid.png");
        let invalid = temp.path().join("invalid.png");
        let wrong_size = temp.path().join("wrong-size.png");
        let viewport = PngViewport {
            width: 4,
            height: 3,
        };
        write_test_image(&valid, ImageFormat::Png, viewport.width, viewport.height);
        fs::write(&invalid, [0x89, b'P', b'N', b'G']).unwrap();
        write_test_image(&wrong_size, ImageFormat::Png, 3, 4);

        assert!(matches!(
            smoke_check_png(&missing, viewport).unwrap_err(),
            PdfError::MissingPng { .. }
        ));
        assert!(matches!(
            smoke_check_png(&empty, viewport).unwrap_err(),
            PdfError::EmptyPng { .. }
        ));
        assert!(matches!(
            smoke_check_png(&invalid, viewport).unwrap_err(),
            PdfError::InvalidPng { .. }
        ));
        assert!(matches!(
            smoke_check_png(&wrong_size, viewport).unwrap_err(),
            PdfError::PngDimensionMismatch { .. }
        ));
        smoke_check_png(&valid, viewport).unwrap();
    }

    #[test]
    fn smoke_check_jpeg_decodes_and_verifies_dimensions() {
        let temp = tempdir().unwrap();
        let missing = temp.path().join("missing.jpg");
        let empty = temp.path().join("empty.jpg");
        let not_jpeg = temp.path().join("not-jpeg.jpg");
        let valid = temp.path().join("valid.jpg");
        let wrong_size = temp.path().join("wrong-size.jpg");
        let viewport = PngViewport {
            width: 4,
            height: 3,
        };
        fs::write(&empty, []).unwrap();
        fs::write(&not_jpeg, [0x89, b'P', b'N', b'G']).unwrap();
        write_test_image(&valid, ImageFormat::Jpeg, viewport.width, viewport.height);
        write_test_image(&wrong_size, ImageFormat::Jpeg, 3, 4);

        assert!(matches!(
            smoke_check_jpeg(&missing, viewport).unwrap_err(),
            PdfError::MissingJpeg { .. }
        ));
        assert!(matches!(
            smoke_check_jpeg(&empty, viewport).unwrap_err(),
            PdfError::EmptyJpeg { .. }
        ));
        assert!(matches!(
            smoke_check_jpeg(&not_jpeg, viewport).unwrap_err(),
            PdfError::InvalidJpeg { .. }
        ));
        assert!(matches!(
            smoke_check_jpeg(&wrong_size, viewport).unwrap_err(),
            PdfError::JpegDimensionMismatch { .. }
        ));
        smoke_check_jpeg(&valid, viewport).unwrap();
    }

    #[test]
    fn readiness_checks_math_charts_images_and_static_warnings() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.expected_pages, 10);
        assert!(report.checked_math_blocks > 0);
        assert!(report.checked_chart_blocks > 0);
        assert!(report.checked_image_blocks > 0);
        assert_eq!(report.checked_media_blocks, 0);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("HTML-only content"))
        );
    }

    #[test]
    fn readiness_accepts_remote_figure_with_static_fallback() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("fallback.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("remote-figure.zp.md");
        fs::write(
            &source_path,
            r#"# Remote figure

![pdf-src="assets/fallback.svg" alt="Remote plot"](https://example.com/plot.png "Remote plot")
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_image_blocks, 1);
    }

    #[test]
    fn readiness_accepts_gif_figure_with_static_fallback() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("animated.gif"), "gif").unwrap();
        fs::write(asset_dir.join("fallback.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("gif-figure.zp.md");
        fs::write(
            &source_path,
            r#"# GIF figure

![static-src="assets/fallback.svg" alt="Animated plot"](assets/animated.gif "Animated plot")
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_image_blocks, 1);
    }

    #[test]
    fn readiness_rejects_gif_figure_without_static_fallback() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("animated.gif"), "gif").unwrap();
        let source_path = temp.path().join("gif-figure.zp.md");
        fs::write(
            &source_path,
            r#"# GIF figure

![Animated plot](assets/animated.gif)
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let error = check_pdf_readiness(&deck).unwrap_err();

        assert!(error.to_string().contains("GIF image"));
        assert!(error.to_string().contains("static-src fallback"));
    }

    #[test]
    fn readiness_checks_inline_gallery_static_fallbacks() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("animated.gif"), "gif").unwrap();
        fs::write(asset_dir.join("fallback.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("gif-gallery.zp.md");
        fs::write(
            &source_path,
            r#"# GIF gallery

![inline static-src="assets/fallback.svg" alt="Animated plot"](assets/animated.gif)
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_image_blocks, 1);
    }

    #[test]
    fn readiness_rejects_remote_figure_without_static_fallback() {
        let deck = parse_source_text(
            r#"# Remote figure

![Remote plot](https://example.com/plot.png)
"#,
            Some(PathBuf::from("remote-figure.zp.md")),
        )
        .unwrap();

        let error = check_pdf_readiness(&deck).unwrap_err();

        assert!(matches!(
            error,
            PdfError::UnsupportedPdfContent {
                block: "figure",
                ..
            }
        ));
        assert!(error.to_string().contains("remote image"));
        assert!(error.to_string().contains("not a reliable PDF dependency"));
    }

    #[test]
    fn readiness_rejects_unsupported_chart_before_chromium() {
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

        let error = check_pdf_readiness(&deck).unwrap_err();

        assert!(matches!(
            error,
            PdfError::UnsupportedPdfContent { block: "chart", .. }
        ));
        assert!(error.to_string().contains("only Vega-Lite line charts"));
    }

    #[test]
    fn readiness_accepts_simple_mermaid_flowchart() {
        let deck = parse_source_text(
            r#"# Diagram

```mermaid
flowchart TD
  A[Start] --> B[Result]
```
"#,
            Some(PathBuf::from("talk.zp.md")),
        )
        .unwrap();

        check_pdf_readiness(&deck).unwrap();
    }

    #[test]
    fn readiness_accepts_video_with_poster_and_warns_for_audio_static_card() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("clip.mp4"), "video").unwrap();
        fs::write(asset_dir.join("poster.png"), "poster").unwrap();
        fs::write(asset_dir.join("voice.mp3"), "audio").unwrap();
        let source_path = temp.path().join("media.zp.md");
        fs::write(
            &source_path,
            r#"# Media

::: video src="assets/clip.mp4" poster="assets/poster.png"
Video fallback.
:::

::: audio src="assets/voice.mp3" title="Narration"
Audio fallback.
:::
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_media_blocks, 2);
        assert!(report.warnings.iter().any(|warning| {
            warning.contains("audio media") && warning.contains("static media card")
        }));
    }

    #[test]
    fn readiness_accepts_hidden_video_without_pdf_poster() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("clip.mp4"), "video").unwrap();
        let source_path = temp.path().join("hidden-media.zp.md");
        fs::write(
            &source_path,
            r#"# Hidden media

::: video src="assets/clip.mp4" hide=true
Hidden soundtrack.
:::
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_media_blocks, 1);
        assert!(report.warnings.iter().any(|warning| {
            warning.contains("hidden video media")
                && warning.contains("omits its visual representation")
        }));
    }

    #[test]
    fn readiness_checks_layout_region_images_and_display_math() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("inset.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("layout.zp.md");
        fs::write(
            &source_path,
            r#"# Layout

::: columns widths="1/1"
Left column:
![Inset](assets/inset.svg)

Right column:
\(x\)

$$
x^2
$$
:::
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_image_blocks, 1);
        assert_eq!(report.checked_math_blocks, 2);
    }

    #[test]
    fn readiness_checks_inline_image_gallery_items() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("left.svg"), "<svg></svg>").unwrap();
        fs::write(asset_dir.join("right.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("gallery.zp.md");
        fs::write(
            &source_path,
            r#"# Gallery

![inline fill columns=2](assets/left.svg "Left")
![inline fill](assets/right.svg "Right")
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_image_blocks, 2);
    }

    #[test]
    fn readiness_checks_fit_text_inline_math() {
        let deck = parse_source_text(
            r#"# Fit

[fit] Search becomes operational at \(x_t\).
"#,
            Some(PathBuf::from("talk.zp.md")),
        )
        .unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_math_blocks, 1);
    }

    #[test]
    fn readiness_checks_footnote_inline_math() {
        let deck = parse_source_text(
            r#"# Footnote

A sourced claim[^source].

[^source]: The note includes \(x_t\).
"#,
            Some(PathBuf::from("talk.zp.md")),
        )
        .unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_math_blocks, 1);
    }

    #[test]
    fn readiness_checks_slide_background_images() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("background.svg"), "<svg></svg>").unwrap();
        let source_path = temp.path().join("background.zp.md");
        fs::write(
            &source_path,
            r#"# Background

::: background src="assets/background.svg"
:::

Foreground text.
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let report = check_pdf_readiness(&deck).unwrap();

        assert_eq!(report.checked_image_blocks, 1);
    }

    #[test]
    fn v1_readiness_checks_a_dormant_deck_background_once() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        let background = asset_dir.join("background.svg");
        fs::write(&background, "<svg></svg>").unwrap();
        let source_path = temp.path().join("background.zp.md");
        fs::write(
            &source_path,
            r#"---
background_image:
  src: assets/background.svg
---

# Clean title
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();
        let plan = PresentationPlan::for_theme_api_v1(&deck);

        let report = check_pdf_readiness_impl(
            &deck,
            html::StaticExportOptions::default(),
            plan.print_pages(false).len(),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(report.checked_image_blocks, 1);

        fs::remove_file(background).unwrap();
        let error = check_pdf_readiness_impl(
            &deck,
            html::StaticExportOptions::default(),
            plan.print_pages(false).len(),
            Some(&plan),
        )
        .unwrap_err();
        assert!(error.to_string().contains("slide 'deck'"));
        assert!(error.to_string().contains("assets/background.svg"));
    }

    #[test]
    fn readiness_counts_opt_in_speaker_notes_export_pages() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();

        let default_report = check_pdf_readiness(&deck).unwrap();
        let notes_report = check_pdf_readiness_with_options(
            &deck,
            html::StaticExportOptions {
                include_speaker_notes: true,
            },
        )
        .unwrap();

        assert_eq!(default_report.expected_pages, 10);
        assert_eq!(notes_report.expected_pages, 11);
        assert_eq!(
            notes_report.checked_image_blocks,
            default_report.checked_image_blocks
        );
        assert_eq!(
            notes_report.checked_chart_blocks,
            default_report.checked_chart_blocks
        );
    }

    #[test]
    fn readiness_rejects_video_without_pdf_poster() {
        let temp = tempdir().unwrap();
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("clip.mp4"), "video").unwrap();
        let source_path = temp.path().join("media.zp.md");
        fs::write(
            &source_path,
            r#"# Media

::: video src="assets/clip.mp4"
Video without fallback.
:::
"#,
        )
        .unwrap();
        let deck = parse_source_file(&source_path).unwrap();

        let error = check_pdf_readiness(&deck).unwrap_err();

        assert!(matches!(
            error,
            PdfError::UnsupportedPdfContent { block: "media", .. }
        ));
        assert!(error.to_string().contains("requires a local poster image"));
    }

    #[test]
    fn print_html_must_declare_pdf_readiness() {
        let source = include_str!("../fixtures/canonical/canonical.zp.md");
        let deck = parse_source_text(
            source,
            Some(PathBuf::from("fixtures/canonical/canonical.zp.md")),
        )
        .unwrap();
        let print_html = html::render_debug_print_html(&deck, &rendered_theme(&deck));

        check_print_html_ready(&print_html, 10).unwrap();

        let error = check_print_html_ready(&print_html, 9).unwrap_err();
        assert!(matches!(error, PdfError::ReadyPageCountMismatch { .. }));
    }
}
