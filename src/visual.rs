use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use image::GenericImageView;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::background_validation::{
    AuthoredBackgroundExpectation, expected_print_backgrounds_for_plan,
    expected_screen_background_for_plan, validate_authored_background,
};
use crate::browser_validation::{
    BrowserFinding as VisualFinding, validate_browser_page, validate_browser_page_count,
    validate_browser_screen_route,
};
use crate::chromium::{
    BrowserPageObservation, BrowserScreenRoute, ChromiumError, ChromiumSession,
    ChromiumSessionOptions, ChromiumVersion, STATIC_READINESS_TIMEOUT,
};
use crate::deck::{Deck, PdfStepState};
use crate::file_url::file_url;
use crate::html::{self, StaticExportOptions};
use crate::output_ownership::{OutputNamespaceGuard, OutputOwnershipError, OutputTargetKind};
use crate::pdf::PngViewport;
use crate::presentation_plan::{
    PlannedScreenState, PresentationBackgroundOrigin, PresentationBackgroundPhase, PresentationPlan,
};
use crate::room_profile::{
    ResolvedRoomProfile, RoomProfileEnforcementStatus, RoomProfileStatus, resolve_builtin,
};
use crate::theme::{RenderedTheme, ThemeApiVersion, theme_dependency_paths};

const VISUAL_REPORT_SCHEMA_VERSION: u32 = 9;
const ROOM_PROFILE_CALIBRATION_SCHEMA_VERSION: u32 = 1;
const VISUAL_PROVENANCE_SCHEMA_VERSION: u32 = 6;
const SPARSE_OCCUPANCY_WARNING_THRESHOLD: f64 = 0.18;

#[derive(Debug, Clone, Copy)]
struct ContactSheetOutput<'a> {
    html_name: &'a str,
    png_name: &'a str,
    artifact: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualReviewOptions {
    pub viewport: Option<PngViewport>,
    pub chromium: ChromiumSessionOptions,
    pub room_profile: ResolvedRoomProfile,
    pub inspection: bool,
}

impl Default for VisualReviewOptions {
    fn default() -> Self {
        Self {
            viewport: None,
            chromium: ChromiumSessionOptions::default(),
            room_profile: resolve_builtin("projected-room-default")
                .expect("the bundled room profile is validated by tests"),
            inspection: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContractStatus {
    Valid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VisualStatus {
    Passed,
    Failed,
}

impl VisualStatus {
    pub(crate) fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HumanReviewStatus {
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReleaseStatus {
    PendingReview,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct VisualReport {
    pub schema_version: u32,
    pub contract_status: ContractStatus,
    pub visual_status: VisualStatus,
    pub objective_gate_status: VisualStatus,
    pub screen_status: VisualStatus,
    pub print_status: VisualStatus,
    pub human_review: HumanReviewStatus,
    pub composition_review_status: HumanReviewStatus,
    pub release_status: ReleaseStatus,
    pub final_source_release_approval: ReleaseStatus,
    pub room_profile: ResolvedRoomProfile,
    pub expected_screen_slides: usize,
    pub captured_screen_slides: usize,
    pub expected_screen_states: usize,
    pub captured_screen_states: usize,
    pub screen_states: Vec<VisualScreenStateReport>,
    pub expected_pages: usize,
    pub captured_pages: usize,
    pub pages: Vec<VisualPageReport>,
    pub screen_warnings: Vec<VisualFinding>,
    pub screen_failures: Vec<VisualFinding>,
    pub print_warnings: Vec<VisualFinding>,
    pub print_failures: Vec<VisualFinding>,
    pub warnings: Vec<VisualFinding>,
    pub failures: Vec<VisualFinding>,
    pub design_summary: VisualDesignSummary,
    pub room_profile_calibration: RoomProfileCalibrationSummary,
    pub design_review_notes: Vec<VisualDesignReviewNote>,
    pub artifacts: VisualArtifacts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct VisualDesignSummary {
    pub measurement_count: usize,
    pub min_essential_type_px: Option<f64>,
    pub min_technical_type_px: Option<f64>,
    pub micro_measurement_count: usize,
    pub image_backed_text_count: usize,
    pub missing_figure_alternative_count: usize,
    pub undersampled_image_count: usize,
    pub actual_fonts: Vec<String>,
    pub autoscale_distribution: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct VisualDesignReviewNote {
    pub code: String,
    pub surface: String,
    pub palette: String,
    pub slide_id: String,
    pub step: usize,
    pub element: String,
    pub role: String,
    pub measured_basis: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct RoomProfileCalibrationSummary {
    pub schema_version: u32,
    pub profile_selection: String,
    pub profile_sha256: String,
    pub decision_status: RoomProfileStatus,
    pub sample_semantics: String,
    pub surfaces: BTreeMap<String, RoomProfileSurfaceCalibration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct RoomProfileSurfaceCalibration {
    pub observation_count: usize,
    pub type_roles: BTreeMap<String, RoleCalibration>,
    pub contrast: BTreeMap<String, ThresholdCalibration>,
    pub evidence_occupancy: ThresholdCalibration,
    pub autoscale: AutoscaleCalibration,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct RoleCalibration {
    pub floor_px: f64,
    pub sample_count: usize,
    pub essential_sample_count: usize,
    pub essential_below_floor_count: usize,
    pub distribution_px: NumericDistribution,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct ThresholdCalibration {
    pub threshold: f64,
    pub sample_count: usize,
    pub below_threshold_count: usize,
    pub distribution: NumericDistribution,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct AutoscaleCalibration {
    pub warning_below: f64,
    pub hard_floor: f64,
    pub sample_count: usize,
    pub below_warning_count: usize,
    pub below_hard_floor_count: usize,
    pub distribution: NumericDistribution,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct NumericDistribution {
    pub min: Option<f64>,
    pub p10: Option<f64>,
    pub median: Option<f64>,
    pub p90: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct VisualScreenStateReport {
    pub route: BrowserScreenRoute,
    pub screenshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_background_expectation: Option<AuthoredBackgroundExpectation>,
    #[serde(flatten)]
    pub observation: BrowserPageObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct VisualPageReport {
    pub screenshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_background_expectation: Option<AuthoredBackgroundExpectation>,
    #[serde(flatten)]
    pub observation: BrowserPageObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VisualArtifacts {
    pub screen_html: String,
    pub screen_pages_dir: String,
    pub screen_contact_sheet: String,
    pub print_html: String,
    pub pages_dir: String,
    pub contact_sheet: String,
    pub visual_report_json: String,
    pub visual_report_text: String,
    pub room_profile_calibration: String,
    pub provenance: String,
}

impl Default for VisualArtifacts {
    fn default() -> Self {
        Self {
            screen_html: "index.html".to_string(),
            screen_pages_dir: "screen-pages".to_string(),
            screen_contact_sheet: "screen-contact-sheet.png".to_string(),
            print_html: "print.html".to_string(),
            pages_dir: "pages".to_string(),
            contact_sheet: "contact-sheet.png".to_string(),
            visual_report_json: "visual-report.json".to_string(),
            visual_report_text: "visual-report.txt".to_string(),
            room_profile_calibration: "room-profile-calibration.json".to_string(),
            provenance: "provenance.json".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct VisualProvenance {
    pub schema_version: u32,
    pub generated_at_unix_seconds: u64,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub theme_name: String,
    pub theme_version: String,
    pub theme_api_version: u32,
    pub capture_mode: String,
    pub theme_manifest_path: PathBuf,
    pub theme_params: BTreeMap<String, String>,
    pub theme_package_sha256: String,
    pub theme_files: Vec<VisualProvenanceFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deck_background_files: Vec<VisualProvenanceFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deck_background_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_plan_sha256: Option<String>,
    pub room_profile: ResolvedRoomProfile,
    pub git: GitProvenance,
    pub viewport: ProvenanceViewport,
    pub platform: PlatformProvenance,
    pub chromium_executable: PathBuf,
    pub chromium: ChromiumVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VisualProvenanceFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThemeProvenanceInput {
    declared_path: PathBuf,
    resolved_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GitProvenance {
    pub commit: Option<String>,
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProvenanceViewport {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PlatformProvenance {
    pub os: String,
    pub architecture: String,
}

#[derive(Debug, Error)]
pub(crate) enum VisualError {
    #[error(transparent)]
    OutputOwnership(#[from] OutputOwnershipError),
    #[error("{0}")]
    Chromium(#[from] ChromiumError),
    #[error("failed to write visual review artifact at {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("failed to read visual review input at {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to serialize {artifact}: {source}")]
    Serialize {
        artifact: &'static str,
        source: serde_json::Error,
    },
    #[error("Chromium returned an invalid PNG for {artifact}: {source}")]
    InvalidPng {
        artifact: String,
        source: image::ImageError,
    },
    #[error(
        "Chromium returned {observed_width}x{observed_height} for {artifact}; expected {expected_width}x{expected_height}"
    )]
    WrongPngDimensions {
        artifact: String,
        expected_width: u32,
        expected_height: u32,
        observed_width: u32,
        observed_height: u32,
    },
}

pub(crate) fn write_theme_visual_review(
    deck: &Deck,
    theme: &RenderedTheme,
    source_path: &Path,
    output_dir: &Path,
    options: VisualReviewOptions,
) -> Result<VisualReport, VisualError> {
    let room_profile = options.room_profile.clone();
    let namespace = prepare_visual_output_namespace(output_dir)?;
    create_dir(output_dir)?;
    clear_previous_visual_artifacts(output_dir)?;
    let pages_dir = output_dir.join("pages");
    let screen_pages_dir = output_dir.join("screen-pages");
    remove_owned_dir(&pages_dir)?;
    remove_owned_dir(&screen_pages_dir)?;
    create_dir(&pages_dir)?;
    create_dir(&screen_pages_dir)?;

    let viewport = options.viewport.unwrap_or_else(|| viewport_for_deck(deck));
    let expected_pages =
        html::print_page_count_for_theme_with_options(deck, theme, StaticExportOptions::default());
    let enforce_authored_backgrounds = theme.manifest.api() == ThemeApiVersion::V1;
    let presentation_plan =
        enforce_authored_backgrounds.then(|| PresentationPlan::for_theme_api_v1(deck));
    let planned_print_pages = presentation_plan
        .as_ref()
        .map(|plan| plan.print_pages(false));
    let print_background_expectations = if let Some(plan) = presentation_plan.as_ref() {
        expected_print_backgrounds_for_plan(plan, StaticExportOptions::default())
    } else {
        vec![None; expected_pages]
    };
    debug_assert_eq!(print_background_expectations.len(), expected_pages);
    let expected_screen_slides = presentation_plan.as_ref().map_or_else(
        || expected_screen_slide_count(deck),
        PresentationPlan::screen_slide_count,
    );
    let mut browser = ChromiumSession::launch(options.chromium)?;
    let provenance = build_provenance(
        source_path,
        deck,
        theme,
        viewport,
        browser.executable(),
        browser.version(),
        &room_profile,
        options.inspection,
    )?;

    let mut screen_reports = Vec::new();
    let mut screen_warnings = Vec::new();
    let mut screen_failures = Vec::new();
    let mut screen_images = Vec::new();
    let screen_url = format!(
        "{}?zpres-visual-review=1{}",
        file_url(&output_dir.join("index.html")),
        if options.inspection {
            "&zpres-debug=1"
        } else {
            ""
        }
    );
    browser.load_screen_document(&screen_url, viewport)?;
    let screen_readiness = browser.await_static_readiness(STATIC_READINESS_TIMEOUT)?;
    if screen_readiness.target.as_deref() != Some("screen") {
        screen_failures.push(VisualFinding::deck(
            "screen",
            "screen-readiness-target-mismatch",
            format!(
                "the live readiness contract reported target {:?}; expected screen",
                screen_readiness.target
            ),
        ));
    }
    if !screen_readiness.ready {
        screen_failures.push(VisualFinding::deck(
            "screen",
            "screen-readiness-failed",
            if screen_readiness.errors.is_empty() {
                format!(
                    "the browser-owned live readiness promise finished with status {:?}",
                    screen_readiness.promise_status
                )
            } else {
                screen_readiness.errors.join("; ")
            },
        ));
    }
    let screen_routes = browser.screen_routes()?;
    let expected_screen_states = presentation_plan
        .as_ref()
        .map_or(screen_routes.len(), PresentationPlan::screen_state_count);
    if let Some(plan) = presentation_plan.as_ref() {
        validate_screen_route_order(&plan.screen_states(), &screen_routes, &mut screen_failures);
    }
    let discovered_screen_slides = screen_routes
        .iter()
        .map(|route| (route.section, route.detail))
        .collect::<BTreeSet<_>>()
        .len();
    if discovered_screen_slides != expected_screen_slides {
        screen_failures.push(VisualFinding::deck(
            "screen",
            "wrong-screen-slide-count",
            format!(
                "expected {expected_screen_slides} addressable live Slides, but the production runtime exposed {discovered_screen_slides}"
            ),
        ));
    }
    for (state_offset, route) in screen_routes.iter().enumerate() {
        let state_index = state_offset + 1;
        let mut capture = browser.capture_screen_route(route)?;
        stamp_measurement_palette(&mut capture.observation, theme);
        let background_expectation = presentation_plan
            .as_ref()
            .and_then(|plan| expected_screen_background_for_plan(plan, route));
        let screenshot_name = format!("state-{state_index:03}.png");
        let screenshot_relative = format!("screen-pages/{screenshot_name}");
        let screenshot_path = screen_pages_dir.join(&screenshot_name);
        verify_png_dimensions(
            &capture.png,
            viewport,
            &format!("screen state {} ({})", state_index, route.hash),
        )?;
        annotate_semantic_text_ink(
            &mut capture.observation,
            &capture.png,
            &capture.semantic_text_suppressed_png,
            viewport,
            &format!("screen state {} ({})", state_index, route.hash),
        )?;
        write_file(&namespace, &screenshot_path, &capture.png)?;
        let warning_start = screen_warnings.len();
        let failure_start = screen_failures.len();
        classify_screen_state(
            state_index,
            route,
            &capture.observation,
            viewport,
            &mut screen_warnings,
            &mut screen_failures,
        );
        screen_failures.extend(validate_authored_background(
            "screen",
            state_index,
            Some(route),
            background_expectation.as_ref(),
            enforce_authored_backgrounds,
            &capture.observation,
        ));
        if enforce_authored_backgrounds {
            screen_failures.extend(validate_v1_text_contrast(
                "screen",
                state_index,
                Some(route),
                &capture.observation,
            ));
            screen_failures.extend(validate_v1_accessibility_preferences(
                "screen",
                state_index,
                Some(route),
                &capture.observation,
            ));
        }
        screen_failures.extend(validate_room_profile(
            "screen",
            state_index,
            Some(route),
            &capture.observation,
            &room_profile,
            enforce_authored_backgrounds,
        ));
        classify_semantic_text_ink(
            "screen",
            state_index,
            Some(route),
            &capture.observation,
            &screenshot_relative,
            &mut screen_failures,
        );
        attach_evidence(&mut screen_warnings[warning_start..], &screenshot_relative);
        attach_evidence(&mut screen_failures[failure_start..], &screenshot_relative);
        screen_images.push(screenshot_path);
        screen_reports.push(VisualScreenStateReport {
            route: route.clone(),
            screenshot: screenshot_relative,
            authored_background_expectation: background_expectation,
            observation: capture.observation,
        });
    }
    let captured_screen_slides = screen_reports
        .iter()
        .map(|report| (report.route.section, report.route.detail))
        .collect::<BTreeSet<_>>()
        .len();

    let mut page_reports = Vec::with_capacity(expected_pages);
    let mut print_warnings = Vec::new();
    let mut print_failures = Vec::new();
    let mut page_images = Vec::with_capacity(expected_pages);
    let print_url = format!(
        "{}{}",
        file_url(&output_dir.join("print.html")),
        if options.inspection {
            "?zpres-debug=1"
        } else {
            ""
        }
    );
    browser.load_print_document(&print_url, viewport)?;
    let readiness = browser.await_static_readiness(STATIC_READINESS_TIMEOUT)?;
    let print_pages = browser.print_pages()?;
    classify_page_count(expected_pages, print_pages.len(), &mut print_failures);
    if readiness.declared_page_count != Some(expected_pages) {
        print_failures.push(VisualFinding::deck(
            "print",
            "declared-page-count-mismatch",
            format!(
                "expected the executed document to declare {expected_pages} static pages, but it declared {:?}",
                readiness.declared_page_count
            ),
        ));
    }
    if !readiness.ready {
        print_failures.push(VisualFinding::deck(
            "print",
            "static-readiness-failed",
            if readiness.errors.is_empty() {
                format!(
                    "the browser-owned readiness promise finished with status {:?}",
                    readiness.promise_status
                )
            } else {
                readiness.errors.join("; ")
            },
        ));
    }

    for page_index in 0..print_pages.len() {
        let page_number = page_index + 1;
        let mut capture = browser.capture_print_page(page_index)?;
        stamp_measurement_palette(&mut capture.observation, theme);
        if let Some(planned) = planned_print_pages
            .as_ref()
            .and_then(|pages| pages.get(page_index))
        {
            let (expected_step_state, expected_step) = planned.pdf_step_attributes();
            if capture.observation.slide_id != planned.slide_id()
                || capture.observation.role != planned.role()
                || capture.observation.generated.as_deref() != planned.generated()
                || capture.observation.step_state.as_deref() != expected_step_state
                || capture.observation.pdf_step != expected_step
            {
                print_failures.push(VisualFinding::deck(
                    "print",
                    "print-page-plan-order-mismatch",
                    format!(
                        "print page {page_number} is slide '{}' role '{}' generated {:?} Step {:?}/{:?}; the Theme API v1 Presentation plan requires slide '{}' role '{}' generated {:?} Step {:?}/{:?}",
                        capture.observation.slide_id,
                        capture.observation.role,
                        capture.observation.generated,
                        capture.observation.step_state,
                        capture.observation.pdf_step,
                        planned.slide_id(),
                        planned.role(),
                        planned.generated(),
                        expected_step_state,
                        expected_step,
                    ),
                ));
            }
        }
        let background_expectation = print_background_expectations
            .get(page_index)
            .and_then(Option::as_ref);

        let screenshot_name = format!("page-{page_number:03}.png");
        let screenshot_relative = format!("pages/{screenshot_name}");
        let screenshot_path = pages_dir.join(&screenshot_name);
        verify_png_dimensions(&capture.png, viewport, &format!("page {page_number}"))?;
        annotate_semantic_text_ink(
            &mut capture.observation,
            &capture.png,
            &capture.semantic_text_suppressed_png,
            viewport,
            &format!("page {page_number}"),
        )?;
        write_file(&namespace, &screenshot_path, &capture.png)?;
        let warning_start = print_warnings.len();
        let failure_start = print_failures.len();
        classify_page(
            page_number,
            &capture.observation,
            viewport,
            &mut print_warnings,
            &mut print_failures,
        );
        print_failures.extend(validate_authored_background(
            "print",
            page_number,
            None,
            background_expectation,
            enforce_authored_backgrounds,
            &capture.observation,
        ));
        if enforce_authored_backgrounds {
            print_failures.extend(validate_v1_text_contrast(
                "print",
                page_number,
                None,
                &capture.observation,
            ));
            print_failures.extend(validate_v1_accessibility_preferences(
                "print",
                page_number,
                None,
                &capture.observation,
            ));
        }
        print_failures.extend(validate_room_profile(
            "print",
            page_number,
            None,
            &capture.observation,
            &room_profile,
            enforce_authored_backgrounds,
        ));
        classify_semantic_text_ink(
            "print",
            page_number,
            None,
            &capture.observation,
            &screenshot_relative,
            &mut print_failures,
        );
        attach_evidence(&mut print_warnings[warning_start..], &screenshot_relative);
        attach_evidence(&mut print_failures[failure_start..], &screenshot_relative);
        page_images.push(screenshot_path);
        page_reports.push(VisualPageReport {
            screenshot: screenshot_relative,
            authored_background_expectation: background_expectation.cloned(),
            observation: capture.observation,
        });
    }

    write_contact_sheet(
        &namespace,
        &mut browser,
        &screen_images,
        viewport,
        output_dir,
        ContactSheetOutput {
            html_name: ".zpres-screen-contact-sheet.html",
            png_name: "screen-contact-sheet.png",
            artifact: "screen contact sheet",
        },
    )?;
    write_contact_sheet(
        &namespace,
        &mut browser,
        &page_images,
        viewport,
        output_dir,
        ContactSheetOutput {
            html_name: ".zpres-contact-sheet.html",
            png_name: "contact-sheet.png",
            artifact: "print contact sheet",
        },
    )?;

    let screen_status = visual_status_for_failures(&screen_failures);
    let print_status = visual_status_for_failures(&print_failures);
    let warnings = screen_warnings
        .iter()
        .chain(&print_warnings)
        .cloned()
        .collect::<Vec<_>>();
    let failures = screen_failures
        .iter()
        .chain(&print_failures)
        .cloned()
        .collect::<Vec<_>>();
    let design_summary = summarize_design_measurements(&screen_reports, &page_reports);
    let room_profile_calibration =
        summarize_room_profile_calibration(&screen_reports, &page_reports, &room_profile);
    let mut design_review_notes = collect_design_review_notes(&screen_reports, &page_reports);
    design_review_notes.extend(collect_room_profile_notes(
        &screen_reports,
        &page_reports,
        &room_profile,
    ));
    let (visual_status, human_review, release_status) = statuses_for_failures(&failures);
    let report = VisualReport {
        schema_version: VISUAL_REPORT_SCHEMA_VERSION,
        contract_status: ContractStatus::Valid,
        objective_gate_status: visual_status.clone(),
        visual_status,
        screen_status,
        print_status,
        composition_review_status: human_review.clone(),
        human_review,
        final_source_release_approval: release_status.clone(),
        release_status,
        room_profile,
        expected_screen_slides,
        captured_screen_slides,
        expected_screen_states,
        captured_screen_states: screen_reports.len(),
        screen_states: screen_reports,
        expected_pages,
        captured_pages: page_reports.len(),
        pages: page_reports,
        screen_warnings,
        screen_failures,
        print_warnings,
        print_failures,
        warnings,
        failures,
        design_summary,
        room_profile_calibration: room_profile_calibration.clone(),
        design_review_notes,
        artifacts: VisualArtifacts::default(),
    };
    write_json(
        &namespace,
        &output_dir.join("provenance.json"),
        "visual provenance",
        &provenance,
    )?;
    write_json(
        &namespace,
        &output_dir.join("visual-report.json"),
        "visual report",
        &report,
    )?;
    write_json(
        &namespace,
        &output_dir.join("room-profile-calibration.json"),
        "room profile calibration",
        &room_profile_calibration,
    )?;
    write_file(
        &namespace,
        &output_dir.join("visual-report.txt"),
        render_text_report(&report, &provenance).as_bytes(),
    )?;
    Ok(report)
}

fn validate_v1_text_contrast(
    surface: &str,
    page: usize,
    route: Option<&BrowserScreenRoute>,
    observation: &BrowserPageObservation,
) -> Vec<VisualFinding> {
    observation
        .design_measurements
        .iter()
        .filter_map(|measurement| {
            let contrast = measurement.contrast_ratio?;
            let required = if measurement.wcag_large_text { 3.0 } else { 4.5 };
            (contrast + f64::EPSILON < required).then(|| VisualFinding {
                surface: surface.to_string(),
                code: "insufficient-text-contrast".to_string(),
                page: Some(page),
                route: route.map(|route| route.hash.clone()),
                step: Some(measurement.step),
                slide_id: Some(measurement.slide_id.clone()),
                title: (!observation.title.is_empty()).then(|| observation.title.clone()),
                element: Some(measurement.element.clone()),
                boundary: None,
                element_bounds: None,
                boundary_bounds: None,
                intersection_bounds: None,
                deltas: None,
                evidence: None,
                message: format!(
                    "solid-color text contrast is {contrast:.2}:1; {} text at {:.2}px weight {} requires at least {required:.1}:1 under WCAG 2.x",
                    if measurement.wcag_large_text { "large" } else { "ordinary" },
                    measurement.font_size_px.unwrap_or_default(),
                    measurement.font_weight.unwrap_or(400),
                ),
            })
        })
        .collect()
}

fn validate_v1_accessibility_preferences(
    surface: &str,
    page: usize,
    route: Option<&BrowserScreenRoute>,
    observation: &BrowserPageObservation,
) -> Vec<VisualFinding> {
    let evidence = &observation.accessibility_preferences;
    [
        (
            "missing-focus-visible",
            evidence.focus_visible,
            "a keyboard-focusable control did not retain a visible focus indicator",
        ),
        (
            "reduced-motion-state-loss",
            evidence.reduced_motion_preserves_state,
            "reduced motion did not remove renderer-owned motion while preserving the current state",
        ),
        (
            "increased-contrast-state-loss",
            evidence.increased_contrast_preserves_state,
            "increased contrast did not preserve a non-color current-Step cue",
        ),
        (
            "forced-colors-state-loss",
            evidence.forced_colors_preserves_state,
            "forced colors did not preserve a non-color current-Step cue",
        ),
        (
            "color-only-current-step",
            evidence.current_step_has_non_color_cue,
            "the current Step lacks both aria-current and a visible outline cue",
        ),
        (
            "color-only-semantic-state",
            evidence.non_color_state_cues_present,
            "a visible callout, comparison category, or chart legend lacks its required text, shape, or border cue",
        ),
    ]
    .into_iter()
    .filter(|(_, passed, _)| !passed)
    .map(|(code, _, message)| VisualFinding {
        surface: surface.to_string(),
        code: code.to_string(),
        page: Some(page),
        route: route.map(|route| route.hash.clone()),
        step: observation.screen_step.or(observation.pdf_step),
        slide_id: Some(observation.slide_id.clone()),
        title: (!observation.title.is_empty()).then(|| observation.title.clone()),
        element: Some(".zpres-slide".to_string()),
        boundary: None,
        element_bounds: None,
        boundary_bounds: None,
        intersection_bounds: None,
        deltas: None,
        evidence: None,
        message: message.to_string(),
    })
    .chain(
        evidence
            .min_non_text_contrast
            .filter(|contrast| *contrast < 3.0)
            .map(|contrast| VisualFinding {
                surface: surface.to_string(),
                code: "insufficient-non-text-contrast".to_string(),
                page: Some(page),
                route: route.map(|route| route.hash.clone()),
                step: observation.screen_step.or(observation.pdf_step),
                slide_id: Some(observation.slide_id.clone()),
                title: (!observation.title.is_empty()).then(|| observation.title.clone()),
                element: Some(":focus-visible, .zpres-step[data-step-state=\"active\"]".to_string()),
                boundary: None,
                element_bounds: None,
                boundary_bounds: None,
                intersection_bounds: None,
                deltas: None,
                evidence: None,
                message: format!(
                    "a reliably computable focus or current-Step indicator has {contrast:.2}:1 solid-color contrast; WCAG non-text contrast requires at least 3.0:1"
                ),
            }),
    )
    .collect()
}

fn room_profile_type_floor(profile: &ResolvedRoomProfile, role: &str) -> Option<f64> {
    let floors = &profile.profile.type_floors;
    match role {
        "display" => Some(floors.display),
        "title" => Some(floors.title),
        "heading" => Some(floors.heading),
        "body" => Some(floors.body),
        "technical" => Some(floors.technical),
        "micro" => Some(floors.micro),
        _ => None,
    }
}

fn validate_room_profile(
    surface: &str,
    page: usize,
    route: Option<&BrowserScreenRoute>,
    observation: &BrowserPageObservation,
    profile: &ResolvedRoomProfile,
    v1: bool,
) -> Vec<VisualFinding> {
    if !v1 || profile.enforcement_status != RoomProfileEnforcementStatus::BlockingV1 {
        return Vec::new();
    }
    let finding = |code: &str, element: Option<String>, message: String| VisualFinding {
        surface: surface.to_string(),
        code: code.to_string(),
        page: Some(page),
        route: route.map(|route| route.hash.clone()),
        step: observation.screen_step.or(observation.pdf_step),
        slide_id: Some(observation.slide_id.clone()),
        title: (!observation.title.is_empty()).then(|| observation.title.clone()),
        element,
        boundary: None,
        element_bounds: None,
        boundary_bounds: None,
        intersection_bounds: None,
        deltas: None,
        evidence: None,
        message,
    };
    let mut failures = observation
        .design_measurements
        .iter()
        .filter(|measurement| measurement.essential_content)
        .filter_map(|measurement| {
            let size = measurement.font_size_px?;
            let floor = room_profile_type_floor(profile, &measurement.type_role)?;
            (size < floor).then(|| {
                finding(
                    "room-profile-type-floor",
                    Some(measurement.element.clone()),
                    format!(
                        "room profile '{}' requires {} essential content at {:.1}px or larger; Chromium measured {:.2}px",
                        profile.selection, measurement.type_role, floor, size,
                    ),
                )
            })
        })
        .collect::<Vec<_>>();
    if observation
        .autoscale_factor
        .is_some_and(|factor| factor < profile.profile.autoscale.hard_floor)
    {
        failures.push(finding(
            "room-profile-autoscale-hard-floor",
            Some(".zpres-slide-content".to_string()),
            format!(
                "room profile '{}' forbids autoscale below {:.2}; Chromium measured {:.3}",
                profile.selection,
                profile.profile.autoscale.hard_floor,
                observation.autoscale_factor.unwrap_or_default(),
            ),
        ));
    }
    failures
}

fn collect_room_profile_notes(
    screen: &[VisualScreenStateReport],
    pages: &[VisualPageReport],
    profile: &ResolvedRoomProfile,
) -> Vec<VisualDesignReviewNote> {
    let mut notes = Vec::new();
    for observation in screen
        .iter()
        .map(|report| &report.observation)
        .chain(pages.iter().map(|report| &report.observation))
    {
        for measurement in &observation.design_measurements {
            let Some(size) = measurement.font_size_px else {
                continue;
            };
            let palette = measurement.palette.clone();
            let note = |code: &str, basis: String, message: String| VisualDesignReviewNote {
                code: code.to_string(),
                surface: measurement.surface.clone(),
                palette: palette.clone(),
                slide_id: measurement.slide_id.clone(),
                step: measurement.step,
                element: measurement.element.clone(),
                role: measurement.type_role.clone(),
                measured_basis: basis,
                message,
            };
            if measurement.essential_content
                && let Some(floor) = room_profile_type_floor(profile, &measurement.type_role)
                && size < floor
            {
                notes.push(note(
                    "room-profile-type-floor-review",
                    format!(
                        "profile={} status={:?} role={} measured={size:.2}px floor={floor:.2}px",
                        profile.selection, profile.decision_status, measurement.type_role,
                    ),
                    format!(
                        "Essential content is below the '{}' {} floor; this is {}.",
                        profile.selection,
                        measurement.type_role,
                        if profile.decision_status == RoomProfileStatus::Approved {
                            "also a blocking v1 failure"
                        } else {
                            "report-only until physical room approval"
                        },
                    ),
                ));
            }
            if let Some(contrast) = measurement.contrast_ratio {
                let target = if measurement.wcag_large_text {
                    profile.profile.contrast_targets.large
                } else {
                    profile.profile.contrast_targets.ordinary
                };
                if contrast < target {
                    notes.push(note(
                        "room-profile-enhanced-contrast-review",
                        format!(
                            "profile={} status={:?} contrast={contrast:.2}:1 target={target:.2}:1",
                            profile.selection, profile.decision_status,
                        ),
                        "The solid-color pair meets a separate projection-profile review; WCAG enforcement remains independent.".to_string(),
                    ));
                }
            }
        }
        if let Some(factor) = observation.autoscale_factor
            && factor < profile.profile.autoscale.warning_below
        {
            let first = observation.design_measurements.first();
            notes.push(VisualDesignReviewNote {
                code: "room-profile-autoscale-review".to_string(),
                surface: first
                    .map_or("unknown", |value| value.surface.as_str())
                    .to_string(),
                palette: first
                    .map_or("default", |value| value.palette.as_str())
                    .to_string(),
                slide_id: observation.slide_id.clone(),
                step: observation
                    .screen_step
                    .or(observation.pdf_step)
                    .unwrap_or(0),
                element: ".zpres-slide-content".to_string(),
                role: "autoscale".to_string(),
                measured_basis: format!(
                    "profile={} status={:?} factor={factor:.3} warning={:.2} hard_floor={:.2}",
                    profile.selection,
                    profile.decision_status,
                    profile.profile.autoscale.warning_below,
                    profile.profile.autoscale.hard_floor,
                ),
                message: "Autoscale is below the selected room profile's review threshold."
                    .to_string(),
            });
        }
    }
    notes
}

fn prepare_visual_output_namespace(output_dir: &Path) -> Result<OutputNamespaceGuard, VisualError> {
    let namespace = OutputNamespaceGuard::acquire(output_dir)?;
    namespace.ensure_peer_output_allowed(output_dir, OutputTargetKind::Directory)?;
    namespace.ensure_tree_removal_allowed(&output_dir.join("pages"))?;
    namespace.ensure_tree_removal_allowed(&output_dir.join("screen-pages"))?;
    Ok(namespace)
}

fn remove_owned_dir(path: &Path) -> Result<(), VisualError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| VisualError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn expected_screen_slide_count(deck: &Deck) -> usize {
    let authored = deck
        .sections
        .iter()
        .map(|section| 1 + section.detail_slides.len())
        .sum::<usize>();
    authored
        + usize::from(
            deck.metadata
                .background_image
                .as_ref()
                .is_some_and(|image| image.splash),
        )
}

fn validate_screen_route_order(
    expected: &[PlannedScreenState<'_>],
    actual: &[BrowserScreenRoute],
    failures: &mut Vec<VisualFinding>,
) {
    if expected.len() != actual.len() {
        failures.push(VisualFinding::deck(
            "screen",
            "screen-route-plan-length-mismatch",
            format!(
                "the Theme API v1 Presentation plan declares {} screen states, but the production runtime exposes {}",
                expected.len(),
                actual.len()
            ),
        ));
    }
    for (offset, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        let matches = actual.section == expected.section
            && actual.detail == expected.detail
            && actual.step == expected.step
            && actual.step_count == expected.step_count
            && actual.slide_id == expected.slide_id
            && actual.role == expected.role
            && actual.generated.as_deref() == expected.generated
            && actual.hash == expected.route_hash();
        if !matches {
            failures.push(VisualFinding::deck(
                "screen",
                "screen-route-plan-order-mismatch",
                format!(
                    "screen state {} is route '{}' ({}/{}/{}) slide '{}' role '{}' generated {:?}; the Theme API v1 Presentation plan requires route '{}' ({}/{}/{}) slide '{}' role '{}' generated {:?}",
                    offset + 1,
                    actual.hash,
                    actual.section,
                    actual.detail,
                    actual.step,
                    actual.slide_id,
                    actual.role,
                    actual.generated,
                    expected.route_hash(),
                    expected.section,
                    expected.detail,
                    expected.step,
                    expected.slide_id,
                    expected.role,
                    expected.generated,
                ),
            ));
        }
    }
}

fn write_contact_sheet(
    namespace: &OutputNamespaceGuard,
    browser: &mut ChromiumSession,
    images: &[PathBuf],
    viewport: PngViewport,
    output_dir: &Path,
    output: ContactSheetOutput<'_>,
) -> Result<(), VisualError> {
    let layout = ContactSheetLayout::for_page_count(images.len(), viewport);
    let html_path = output_dir.join(output.html_name);
    write_reserved_file(
        namespace,
        &html_path,
        render_contact_sheet_html(images, layout).as_bytes(),
    )?;
    let contact_viewport = PngViewport {
        width: layout.width,
        height: layout.height,
    };
    let capture = browser.capture_document(&file_url(&html_path), contact_viewport);
    let _ = fs::remove_file(&html_path);
    let png = capture?;
    verify_png_dimensions(&png, contact_viewport, output.artifact)?;
    write_file(namespace, &output_dir.join(output.png_name), &png)
}

fn classify_page_count(
    expected_pages: usize,
    captured_pages: usize,
    failures: &mut Vec<VisualFinding>,
) {
    failures.extend(validate_browser_page_count(expected_pages, captured_pages));
}

fn statuses_for_failures(
    failures: &[VisualFinding],
) -> (VisualStatus, HumanReviewStatus, ReleaseStatus) {
    if failures.is_empty() {
        (
            VisualStatus::Passed,
            HumanReviewStatus::Required,
            ReleaseStatus::PendingReview,
        )
    } else {
        (
            VisualStatus::Failed,
            HumanReviewStatus::Required,
            ReleaseStatus::Blocked,
        )
    }
}

fn visual_status_for_failures(failures: &[VisualFinding]) -> VisualStatus {
    if failures.is_empty() {
        VisualStatus::Passed
    } else {
        VisualStatus::Failed
    }
}

const MIN_SEMANTIC_TEXT_INK_PIXELS: usize = 12;
const MIN_SEMANTIC_TEXT_INK_RATIO: f64 = 0.0015;

fn annotate_semantic_text_ink(
    observation: &mut BrowserPageObservation,
    png: &[u8],
    semantic_text_suppressed_png: &[u8],
    viewport: PngViewport,
    artifact: &str,
) -> Result<(), VisualError> {
    if observation.semantic_text_regions.is_empty() {
        return Ok(());
    }
    let image = image::load_from_memory(png)
        .map_err(|source| VisualError::InvalidPng {
            artifact: artifact.to_string(),
            source,
        })?
        .to_rgba8();
    let suppressed = image::load_from_memory(semantic_text_suppressed_png)
        .map_err(|source| VisualError::InvalidPng {
            artifact: format!("{artifact} semantic-text-suppressed comparison"),
            source,
        })?
        .to_rgba8();
    if image.dimensions() != suppressed.dimensions() {
        return Err(VisualError::WrongPngDimensions {
            artifact: format!("{artifact} semantic-text-suppressed comparison"),
            expected_width: image.width(),
            expected_height: image.height(),
            observed_width: suppressed.width(),
            observed_height: suppressed.height(),
        });
    }
    let slide = observation
        .slide_bounds
        .unwrap_or(crate::chromium::BrowserRect {
            width: viewport.width as f64,
            height: viewport.height as f64,
            right: viewport.width as f64,
            bottom: viewport.height as f64,
            ..Default::default()
        });
    let scale_x = image.width() as f64 / slide.width.max(1.0);
    let scale_y = image.height() as f64 / slide.height.max(1.0);

    for region in &mut observation.semantic_text_regions {
        let mut sampled_pixels = 0usize;
        let mut ink_pixels = 0usize;
        for bounds in &region.text_bounds {
            let left = ((bounds.x - slide.x) * scale_x)
                .floor()
                .clamp(0.0, image.width() as f64) as u32;
            let top = ((bounds.y - slide.y) * scale_y)
                .floor()
                .clamp(0.0, image.height() as f64) as u32;
            let right = ((bounds.right - slide.x) * scale_x)
                .ceil()
                .clamp(0.0, image.width() as f64) as u32;
            let bottom = ((bounds.bottom - slide.y) * scale_y)
                .ceil()
                .clamp(0.0, image.height() as f64) as u32;
            if right <= left || bottom <= top {
                continue;
            }

            for y in top..bottom {
                for x in left..right {
                    let actual = image.get_pixel(x, y).0;
                    let without_text = suppressed.get_pixel(x, y).0;
                    if actual
                        .iter()
                        .zip(without_text)
                        .any(|(actual, suppressed)| actual.abs_diff(suppressed) >= 8)
                    {
                        ink_pixels += 1;
                    }
                }
            }
            let count = (right - left) as usize * (bottom - top) as usize;
            sampled_pixels += count;
        }
        region.raster_ink = Some(crate::chromium::BrowserRasterInkObservation {
            sampled_pixels,
            ink_pixels,
            ink_ratio: if sampled_pixels == 0 {
                0.0
            } else {
                ink_pixels as f64 / sampled_pixels as f64
            },
        });
    }
    Ok(())
}

fn classify_semantic_text_ink(
    surface: &str,
    index: usize,
    route: Option<&BrowserScreenRoute>,
    observation: &BrowserPageObservation,
    screenshot: &str,
    failures: &mut Vec<VisualFinding>,
) {
    for region in &observation.semantic_text_regions {
        let Some(ink) = region.raster_ink else {
            continue;
        };
        if ink.ink_pixels >= MIN_SEMANTIC_TEXT_INK_PIXELS
            && ink.ink_ratio >= MIN_SEMANTIC_TEXT_INK_RATIO
        {
            continue;
        }
        failures.push(VisualFinding {
            surface: surface.to_string(),
            code: "missing-semantic-text-ink".to_string(),
            page: Some(index),
            route: route.map(|value| value.hash.clone()),
            step: route.map(|value| value.step),
            slide_id: (!observation.slide_id.is_empty()).then(|| observation.slide_id.clone()),
            title: (!observation.title.is_empty()).then(|| observation.title.clone()),
            element: Some(region.element.clone()),
            boundary: None,
            element_bounds: None,
            boundary_bounds: None,
            intersection_bounds: None,
            deltas: None,
            evidence: Some(screenshot.to_string()),
            message: format!(
                "authored {} text produced effectively no visible raster ink ({} differing pixels across {} sampled pixels, {:.3}%); inspect original-size evidence {}",
                region.region,
                ink.ink_pixels,
                ink.sampled_pixels,
                ink.ink_ratio * 100.0,
                screenshot,
            ),
        });
    }
}

fn attach_evidence(findings: &mut [VisualFinding], screenshot: &str) {
    for finding in findings {
        if finding.evidence.is_none() {
            finding.evidence = Some(screenshot.to_string());
        }
    }
}

fn classify_screen_state(
    state_index: usize,
    route: &BrowserScreenRoute,
    observation: &BrowserPageObservation,
    viewport: PngViewport,
    warnings: &mut Vec<VisualFinding>,
    failures: &mut Vec<VisualFinding>,
) {
    failures.extend(validate_browser_screen_route(
        state_index,
        route,
        observation,
        viewport,
    ));
    if !observation.slide_present {
        return;
    }

    let finding = |code: &str, message: String| VisualFinding {
        surface: "screen".to_string(),
        code: code.to_string(),
        page: Some(state_index),
        route: Some(route.hash.clone()),
        step: Some(route.step),
        slide_id: (!observation.slide_id.is_empty()).then(|| observation.slide_id.clone()),
        title: (!observation.title.is_empty()).then(|| observation.title.clone()),
        element: None,
        boundary: None,
        element_bounds: None,
        boundary_bounds: None,
        intersection_bounds: None,
        deltas: None,
        evidence: None,
        message,
    };
    classify_composition(observation, finding, warnings);
}

fn classify_page(
    page: usize,
    observation: &BrowserPageObservation,
    viewport: PngViewport,
    warnings: &mut Vec<VisualFinding>,
    failures: &mut Vec<VisualFinding>,
) {
    let finding = |code: &str, message: String| VisualFinding {
        surface: "print".to_string(),
        code: code.to_string(),
        page: Some(page),
        route: None,
        step: None,
        slide_id: (!observation.slide_id.is_empty()).then(|| observation.slide_id.clone()),
        title: (!observation.title.is_empty()).then(|| observation.title.clone()),
        element: None,
        boundary: None,
        element_bounds: None,
        boundary_bounds: None,
        intersection_bounds: None,
        deltas: None,
        evidence: None,
        message,
    };

    failures.extend(validate_browser_page(page, observation, viewport));
    if !observation.slide_present {
        return;
    }
    classify_composition(observation, finding, warnings);
}

fn classify_composition(
    observation: &BrowserPageObservation,
    finding: impl Fn(&str, String) -> VisualFinding,
    warnings: &mut Vec<VisualFinding>,
) {
    if observation
        .autoscale_factor
        .is_some_and(|factor| factor < 1.0)
    {
        warnings.push(finding(
            "autoscaled-content",
            format!(
                "content was reduced to {:.1}% of its authored size; confirm that the resulting text and composition remain presentation-ready",
                observation.autoscale_factor.unwrap_or(1.0) * 100.0,
            ),
        ));
    }
    if observation
        .occupancy
        .is_some_and(|occupancy| occupancy < SPARSE_OCCUPANCY_WARNING_THRESHOLD)
    {
        warnings.push(finding(
            "sparse-composition",
            format!(
                "visible content occupies {:.1}% of the canvas; review the composition by eye{}",
                observation.occupancy.unwrap_or_default() * 100.0,
                if observation.variant.as_deref() == Some("section-title") {
                    " (Section-title slides may intentionally use negative space)"
                } else {
                    ""
                }
            ),
        ));
    }
    if observation.variant.as_deref() != Some("section-title")
        && observation
            .whitespace
            .is_some_and(|whitespace| whitespace.bottom > 0.45 && whitespace.top < 0.16)
    {
        warnings.push(finding(
            "upper-stacked-composition",
            "most visible content sits in the upper part of the canvas; review vertical composition"
                .to_string(),
        ));
    }
}

fn viewport_for_deck(deck: &Deck) -> PngViewport {
    let Some(aspect) = deck.metadata.aspect.as_deref() else {
        return PngViewport::default();
    };
    let Some((width, height)) = aspect.split_once([':', 'x', 'X']) else {
        return PngViewport::default();
    };
    let Some(width) = width.trim().parse::<f64>().ok() else {
        return PngViewport::default();
    };
    let Some(height) = height.trim().parse::<f64>().ok() else {
        return PngViewport::default();
    };
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return PngViewport::default();
    }
    PngViewport {
        width: (720.0 * width / height).round().clamp(320.0, 7680.0) as u32,
        height: 720,
    }
}

fn build_provenance(
    source_path: &Path,
    deck: &Deck,
    theme: &RenderedTheme,
    viewport: PngViewport,
    chromium_executable: &Path,
    chromium: &ChromiumVersion,
    room_profile: &ResolvedRoomProfile,
    inspection: bool,
) -> Result<VisualProvenance, VisualError> {
    let source_path = canonical_or_original(source_path);
    let source_sha256 = digest_file(&source_path)?;
    let manifest_path = canonical_or_original(&theme.manifest.path);
    let theme_root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let manifest_relative = manifest_path
        .strip_prefix(theme_root)
        .unwrap_or(&manifest_path)
        .to_path_buf();
    let mut paths = BTreeMap::from([(manifest_relative, manifest_path.clone())]);
    for relative in [
        theme.manifest.stylesheet.as_str(),
        theme.manifest.print_stylesheet.as_str(),
    ]
    .into_iter()
    .chain(theme_dependency_paths(&theme.manifest))
    .chain(theme.manifest.style.inspiration.iter().map(String::as_str))
    {
        paths.insert(
            PathBuf::from(relative),
            canonical_or_original(&theme_root.join(relative)),
        );
    }
    let paths = paths
        .into_iter()
        .map(|(declared_path, resolved_path)| ThemeProvenanceInput {
            declared_path,
            resolved_path,
        })
        .collect::<Vec<_>>();
    let theme_files = paths
        .iter()
        .map(|input| {
            Ok(VisualProvenanceFile {
                path: input.declared_path.clone(),
                sha256: digest_file(&input.resolved_path)?,
            })
        })
        .collect::<Result<Vec<_>, VisualError>>()?;
    let theme_package_sha256 = digest_package(&paths)?;
    let presentation_plan = (theme.manifest.api() == ThemeApiVersion::V1)
        .then(|| PresentationPlan::for_theme_api_v1(deck));
    let deck_root = deck
        .deck_root()
        .or_else(|| source_path.parent())
        .unwrap_or_else(|| Path::new("."));
    let deck_background_paths = presentation_plan
        .as_ref()
        .map(|plan| {
            plan.declared_backgrounds()
                .into_iter()
                .map(|(_, background)| {
                    (
                        PathBuf::from(&background.src),
                        canonical_or_original(&deck_root.join(&background.src)),
                    )
                })
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .map(|(declared_path, resolved_path)| ThemeProvenanceInput {
                    declared_path,
                    resolved_path,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let deck_background_files = deck_background_paths
        .iter()
        .map(|input| {
            Ok(VisualProvenanceFile {
                path: input.declared_path.clone(),
                sha256: digest_file(&input.resolved_path)?,
            })
        })
        .collect::<Result<Vec<_>, VisualError>>()?;
    let deck_background_sha256 = (!deck_background_paths.is_empty())
        .then(|| digest_inputs(b"zpres-deck-backgrounds-v1\0", &deck_background_paths))
        .transpose()?;
    let presentation_plan_sha256 = presentation_plan.as_ref().map(digest_presentation_plan);

    Ok(VisualProvenance {
        schema_version: VISUAL_PROVENANCE_SCHEMA_VERSION,
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        source_path,
        source_sha256,
        theme_name: theme.name().to_string(),
        theme_version: theme.manifest.version.clone(),
        theme_api_version: theme.manifest.api_version,
        capture_mode: if inspection { "inspection" } else { "normal" }.to_string(),
        theme_manifest_path: manifest_path,
        theme_params: theme.params.clone(),
        theme_package_sha256,
        theme_files,
        deck_background_files,
        deck_background_sha256,
        presentation_plan_sha256,
        room_profile: room_profile.clone(),
        git: git_provenance(),
        viewport: ProvenanceViewport {
            width: viewport.width,
            height: viewport.height,
            device_scale_factor: 1,
        },
        platform: PlatformProvenance {
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
        },
        chromium_executable: chromium_executable.to_path_buf(),
        chromium: chromium.clone(),
    })
}

fn digest_package(paths: &[ThemeProvenanceInput]) -> Result<String, VisualError> {
    digest_inputs(b"zpres-theme-package-v1\0", paths)
}

fn digest_inputs(domain: &[u8], paths: &[ThemeProvenanceInput]) -> Result<String, VisualError> {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for input in paths {
        let name = input.declared_path.to_string_lossy();
        let bytes = read_file(&input.resolved_path)?;
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn digest_presentation_plan(plan: &PresentationPlan<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"zpres-presentation-plan-v1\0");
    for state in plan.screen_states() {
        hasher.update((state.section as u64).to_be_bytes());
        hasher.update((state.detail as u64).to_be_bytes());
        hasher.update((state.step as u64).to_be_bytes());
        hasher.update((state.step_count as u64).to_be_bytes());
        hash_text(&mut hasher, state.slide_id);
        hash_text(&mut hasher, state.role);
        hash_text(&mut hasher, state.generated.unwrap_or(""));
    }
    hasher.update(b"\0print\0");
    for page in plan.print_pages(true) {
        hash_text(&mut hasher, page.slide_id());
        hash_text(&mut hasher, page.role());
        hash_text(&mut hasher, page.generated().unwrap_or(""));
        match page.step_state() {
            Some(PdfStepState::Final) => hasher.update(b"final"),
            Some(PdfStepState::UpTo { step }) => {
                hasher.update(b"up-to");
                hasher.update((step as u64).to_be_bytes());
            }
            None => hasher.update(b"generated"),
        }
        let background = page.background();
        hash_text(
            &mut hasher,
            background.image.map_or("", |image| image.src.as_str()),
        );
        hasher.update(match background.origin {
            Some(PresentationBackgroundOrigin::Deck) => b"deck".as_slice(),
            Some(PresentationBackgroundOrigin::Slide) => b"slide".as_slice(),
            Some(PresentationBackgroundOrigin::GeneratedSplash) => b"splash".as_slice(),
            None => b"none".as_slice(),
        });
        hasher.update(match background.phase {
            PresentationBackgroundPhase::Title => b"title".as_slice(),
            PresentationBackgroundPhase::Content => b"content".as_slice(),
            PresentationBackgroundPhase::Splash => b"splash".as_slice(),
        });
    }
    format!("{:x}", hasher.finalize())
}

fn hash_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn digest_file(path: &Path) -> Result<String, VisualError> {
    let bytes = read_file(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn git_provenance() -> GitProvenance {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commit = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let dirty = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty());
    GitProvenance { commit, dirty }
}

fn render_text_report(report: &VisualReport, provenance: &VisualProvenance) -> String {
    let mut text = String::new();
    text.push_str("# zpres browser-rendered visual check\n\n");
    text.push_str(&format!(
        "contract_status = {}\nobjective_gate_status = {}\nvisual_status = {}\nscreen_status = {}\nprint_status = {}\ncomposition_review_status = {}\nhuman_review = {}\nfinal_source_release_approval = {}\nrelease_status = {}\nroom_profile = {}\nroom_profile_status = {}\nroom_profile_enforcement = {}\nroom_profile_sha256 = {}\n",
        enum_json(&report.contract_status),
        enum_json(&report.objective_gate_status),
        enum_json(&report.visual_status),
        enum_json(&report.screen_status),
        enum_json(&report.print_status),
        enum_json(&report.composition_review_status),
        enum_json(&report.human_review),
        enum_json(&report.final_source_release_approval),
        enum_json(&report.release_status),
        report.room_profile.selection,
        enum_json(&report.room_profile.decision_status),
        enum_json(&report.room_profile.enforcement_status),
        report.room_profile.sha256,
    ));
    text.push_str(&format!(
        "screen_slides = {}/{}\nscreen_states = {}/{}\nprint_pages = {}/{}\nsource = {}\ntheme = {} {}\ntheme_api_version = {}\ntheme_package_sha256 = {}\ngit_commit = {}\ngit_dirty = {}\nviewport = {}x{}@{}\nchromium = {}\n\n",
        report.captured_screen_slides,
        report.expected_screen_slides,
        report.captured_screen_states,
        report.expected_screen_states,
        report.captured_pages,
        report.expected_pages,
        provenance.source_path.display(),
        provenance.theme_name,
        provenance.theme_version,
        provenance.theme_api_version,
        provenance.theme_package_sha256,
        provenance.git.commit.as_deref().unwrap_or("unknown"),
        provenance.git.dirty.map(|value| value.to_string()).unwrap_or_else(|| "unknown".to_string()),
        provenance.viewport.width,
        provenance.viewport.height,
        provenance.viewport.device_scale_factor,
        provenance.chromium.product,
    ));
    text.push_str("## Design measurement summary (report-only)\n\n");
    text.push_str(&format!(
        "measurements = {}\nmin_essential_type_px = {}\nmin_technical_type_px = {}\nmicro_measurements = {}\nimage_backed_text = {}\nmissing_figure_alternatives = {}\nundersampled_images = {}\nactual_fonts = {}\nautoscale_distribution = {:?}\n\n",
        report.design_summary.measurement_count,
        report.design_summary.min_essential_type_px.map(|value| format!("{value:.1}")).unwrap_or_else(|| "n/a".to_string()),
        report.design_summary.min_technical_type_px.map(|value| format!("{value:.1}")).unwrap_or_else(|| "n/a".to_string()),
        report.design_summary.micro_measurement_count,
        report.design_summary.image_backed_text_count,
        report.design_summary.missing_figure_alternative_count,
        report.design_summary.undersampled_image_count,
        if report.design_summary.actual_fonts.is_empty() { "unobserved".to_string() } else { report.design_summary.actual_fonts.join(", ") },
        report.design_summary.autoscale_distribution,
    ));
    text.push_str("## Design review notes (report-only)\n\n");
    if report.design_review_notes.is_empty() {
        text.push_str("none\n\n");
    } else {
        for note in &report.design_review_notes {
            text.push_str(&format!(
                "- [{}] surface={} palette={} slide={} step={} element={} role={}: {} ({})\n",
                note.code,
                note.surface,
                note.palette,
                note.slide_id,
                note.step,
                note.element,
                note.role,
                note.message,
                note.measured_basis,
            ));
        }
        text.push('\n');
    }
    text.push_str("## Screen states\n\n");
    for state in &report.screen_states {
        let observation = &state.observation;
        text.push_str(&format!(
            "route {}: slide={} role={} step={}/{} occupancy={} min_body_text_px={} autoscale={} screenshot={}\n",
            state.route.hash,
            observation.slide_id,
            observation.role,
            state.route.step,
            state.route.step_count,
            observation.occupancy.map(|value| format!("{:.1}%", value * 100.0)).unwrap_or_else(|| "n/a".to_string()),
            observation.min_body_text_px.map(|value| format!("{value:.1}")).unwrap_or_else(|| "n/a".to_string()),
            observation.autoscale_factor.map(|value| format!("{value:.3}")).unwrap_or_else(|| "1.000".to_string()),
            state.screenshot,
        ));
    }
    text.push_str("\n## Print pages\n\n");
    for page in &report.pages {
        let observation = &page.observation;
        text.push_str(&format!(
            "page {}: slide={} title={:?} occupancy={} min_body_text_px={} autoscale={} screenshot={}\n",
            observation.page.unwrap_or_default(),
            observation.slide_id,
            observation.title,
            observation.occupancy.map(|value| format!("{:.1}%", value * 100.0)).unwrap_or_else(|| "n/a".to_string()),
            observation.min_body_text_px.map(|value| format!("{value:.1}")).unwrap_or_else(|| "n/a".to_string()),
            observation.autoscale_factor.map(|value| format!("{value:.3}")).unwrap_or_else(|| "1.000".to_string()),
            page.screenshot,
        ));
    }
    text.push_str("\n## Screen warnings\n\n");
    append_findings(&mut text, &report.screen_warnings);
    text.push_str("\n## Screen failures\n\n");
    append_findings(&mut text, &report.screen_failures);
    text.push_str("\n## Print warnings\n\n");
    append_findings(&mut text, &report.print_warnings);
    text.push_str("\n## Print failures\n\n");
    append_findings(&mut text, &report.print_failures);
    text.push_str("\nA passed objective check on both surfaces still requires human review of both contact sheets before release.\n");
    text
}

fn stamp_measurement_palette(observation: &mut BrowserPageObservation, theme: &RenderedTheme) {
    let palette = theme
        .params
        .get(&theme.manifest.palette_parameter)
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    for measurement in &mut observation.design_measurements {
        measurement.palette.clone_from(&palette);
    }
}

fn summarize_design_measurements(
    screen: &[VisualScreenStateReport],
    pages: &[VisualPageReport],
) -> VisualDesignSummary {
    let observations = screen
        .iter()
        .map(|report| &report.observation)
        .chain(pages.iter().map(|report| &report.observation))
        .collect::<Vec<_>>();
    let measurements = observations
        .iter()
        .flat_map(|observation| &observation.design_measurements)
        .collect::<Vec<_>>();
    let minimum = |role: Option<&str>| {
        measurements
            .iter()
            .filter(|measurement| role.is_none_or(|role| measurement.type_role == role))
            .filter(|measurement| measurement.type_role != "micro")
            .filter_map(|measurement| measurement.font_size_px)
            .min_by(f64::total_cmp)
    };
    let mut actual_fonts = measurements
        .iter()
        .flat_map(|measurement| measurement.actual_font_families.iter().cloned())
        .collect::<Vec<_>>();
    actual_fonts.sort();
    actual_fonts.dedup();
    let mut autoscale_distribution = BTreeMap::new();
    for observation in &observations {
        let factor = observation.autoscale_factor.unwrap_or(1.0);
        let bucket = if factor >= 0.999 {
            "1.000".to_string()
        } else if factor >= 0.9 {
            "0.900-0.998".to_string()
        } else {
            "below-0.900".to_string()
        };
        *autoscale_distribution.entry(bucket).or_insert(0) += 1;
    }
    VisualDesignSummary {
        measurement_count: measurements.len(),
        min_essential_type_px: minimum(None),
        min_technical_type_px: minimum(Some("technical")),
        micro_measurement_count: measurements
            .iter()
            .filter(|measurement| measurement.type_role == "micro")
            .count(),
        image_backed_text_count: measurements
            .iter()
            .filter(|measurement| measurement.image_backed_text)
            .count(),
        missing_figure_alternative_count: measurements
            .iter()
            .filter(|measurement| {
                measurement.figure_alternative_status.as_deref() == Some("missing")
            })
            .count(),
        undersampled_image_count: measurements
            .iter()
            .filter(|measurement| {
                measurement
                    .resolution_scale
                    .is_some_and(|scale| scale < 1.0)
            })
            .count(),
        actual_fonts,
        autoscale_distribution,
    }
}

fn summarize_room_profile_calibration(
    screen: &[VisualScreenStateReport],
    pages: &[VisualPageReport],
    profile: &ResolvedRoomProfile,
) -> RoomProfileCalibrationSummary {
    let surfaces = BTreeMap::from([
        (
            "screen".to_string(),
            summarize_room_profile_surface(
                screen.iter().map(|report| &report.observation),
                profile,
            ),
        ),
        (
            "print".to_string(),
            summarize_room_profile_surface(pages.iter().map(|report| &report.observation), profile),
        ),
    ]);
    RoomProfileCalibrationSummary {
        schema_version: ROOM_PROFILE_CALIBRATION_SCHEMA_VERSION,
        profile_selection: profile.selection.clone(),
        profile_sha256: profile.sha256.clone(),
        decision_status: profile.decision_status,
        sample_semantics: "Each rendered screen state or print page is one observation; repeated Step states repeat their visible measurements. Percentiles select the nearest observed rank after total ordering.".to_string(),
        surfaces,
    }
}

fn summarize_room_profile_surface<'a>(
    observations: impl IntoIterator<Item = &'a BrowserPageObservation>,
    profile: &ResolvedRoomProfile,
) -> RoomProfileSurfaceCalibration {
    let observations = observations.into_iter().collect::<Vec<_>>();
    let measurements = observations
        .iter()
        .flat_map(|observation| &observation.design_measurements)
        .collect::<Vec<_>>();
    let mut type_roles = BTreeMap::new();
    for role in ["display", "title", "heading", "body", "technical", "micro"] {
        let floor_px = room_profile_type_floor(profile, role).unwrap_or_default();
        let role_measurements = measurements
            .iter()
            .filter(|measurement| measurement.type_role == role)
            .collect::<Vec<_>>();
        let sizes = role_measurements
            .iter()
            .filter_map(|measurement| measurement.font_size_px)
            .collect::<Vec<_>>();
        let essential = role_measurements
            .iter()
            .filter(|measurement| measurement.essential_content)
            .collect::<Vec<_>>();
        type_roles.insert(
            role.to_string(),
            RoleCalibration {
                floor_px,
                sample_count: sizes.len(),
                essential_sample_count: essential.len(),
                essential_below_floor_count: essential
                    .iter()
                    .filter(|measurement| {
                        measurement.font_size_px.is_some_and(|size| size < floor_px)
                    })
                    .count(),
                distribution_px: numeric_distribution(sizes),
            },
        );
    }

    let contrast = BTreeMap::from([
        (
            "ordinary".to_string(),
            threshold_calibration(
                measurements
                    .iter()
                    .filter(|measurement| !measurement.wcag_large_text)
                    .filter_map(|measurement| measurement.contrast_ratio),
                profile.profile.contrast_targets.ordinary,
            ),
        ),
        (
            "large".to_string(),
            threshold_calibration(
                measurements
                    .iter()
                    .filter(|measurement| measurement.wcag_large_text)
                    .filter_map(|measurement| measurement.contrast_ratio),
                profile.profile.contrast_targets.large,
            ),
        ),
    ]);
    let evidence_occupancy = threshold_calibration(
        measurements
            .iter()
            .filter(|measurement| measurement.content_role == "evidence")
            .filter_map(|measurement| measurement.occupancy),
        profile.profile.occupancy.evidence_review_below,
    );
    let autoscale_values = observations
        .iter()
        .map(|observation| observation.autoscale_factor.unwrap_or(1.0))
        .collect::<Vec<_>>();
    let autoscale = AutoscaleCalibration {
        warning_below: profile.profile.autoscale.warning_below,
        hard_floor: profile.profile.autoscale.hard_floor,
        sample_count: autoscale_values.len(),
        below_warning_count: autoscale_values
            .iter()
            .filter(|factor| **factor < profile.profile.autoscale.warning_below)
            .count(),
        below_hard_floor_count: autoscale_values
            .iter()
            .filter(|factor| **factor < profile.profile.autoscale.hard_floor)
            .count(),
        distribution: numeric_distribution(autoscale_values),
    };

    RoomProfileSurfaceCalibration {
        observation_count: observations.len(),
        type_roles,
        contrast,
        evidence_occupancy,
        autoscale,
    }
}

fn threshold_calibration(
    values: impl IntoIterator<Item = f64>,
    threshold: f64,
) -> ThresholdCalibration {
    let values = values.into_iter().collect::<Vec<_>>();
    ThresholdCalibration {
        threshold,
        sample_count: values.len(),
        below_threshold_count: values.iter().filter(|value| **value < threshold).count(),
        distribution: numeric_distribution(values),
    }
}

fn numeric_distribution(values: impl IntoIterator<Item = f64>) -> NumericDistribution {
    let mut values = values
        .into_iter()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let observed = |quantile: f64| {
        (!values.is_empty()).then(|| {
            let index = (quantile * (values.len() - 1) as f64).round() as usize;
            values[index]
        })
    };
    NumericDistribution {
        min: values.first().copied(),
        p10: observed(0.10),
        median: observed(0.50),
        p90: observed(0.90),
        max: values.last().copied(),
    }
}

fn collect_design_review_notes(
    screen: &[VisualScreenStateReport],
    pages: &[VisualPageReport],
) -> Vec<VisualDesignReviewNote> {
    let observations = screen
        .iter()
        .map(|report| &report.observation)
        .chain(pages.iter().map(|report| &report.observation));
    let mut notes = Vec::new();
    for observation in observations {
        let default_palette = observation
            .design_measurements
            .first()
            .map_or("default", |measurement| measurement.palette.as_str());
        if observation.variant.as_deref() != Some("section-title")
            && observation
                .whitespace
                .is_some_and(|whitespace| whitespace.bottom > 0.45 && whitespace.top < 0.16)
        {
            notes.push(VisualDesignReviewNote {
                code: "upper-stacked-composition".to_string(),
                surface: observation
                    .design_measurements
                    .first()
                    .map_or("unknown", |measurement| measurement.surface.as_str())
                    .to_string(),
                palette: default_palette.to_string(),
                slide_id: observation.slide_id.clone(),
                step: observation
                    .screen_step
                    .or(observation.pdf_step)
                    .unwrap_or(0),
                element: ".zpres-slide-content".to_string(),
                role: "composition".to_string(),
                measured_basis: format!("whitespace={:?}", observation.whitespace),
                message: "Most visible content is concentrated in the upper part of the canvas."
                    .to_string(),
            });
        }
        for measurement in &observation.design_measurements {
            let note = |code: &str, basis: String, message: &str| VisualDesignReviewNote {
                code: code.to_string(),
                surface: measurement.surface.clone(),
                palette: measurement.palette.clone(),
                slide_id: measurement.slide_id.clone(),
                step: measurement.step,
                element: measurement.element.clone(),
                role: if measurement.type_role.is_empty() {
                    measurement.content_role.clone()
                } else {
                    measurement.type_role.clone()
                },
                measured_basis: basis,
                message: message.to_string(),
            };
            if measurement.type_role == "display"
                && measurement.line_count.is_some_and(|lines| lines > 1)
            {
                notes.push(note(
                    "wrapped-title",
                    format!("line_count={}", measurement.line_count.unwrap_or_default()),
                    "The title wraps; review the break and resulting hierarchy.",
                ));
            }
            if measurement.content_role == "evidence"
                && measurement.element.contains("zpres-block")
                && measurement
                    .occupancy
                    .is_some_and(|occupancy| occupancy < 0.18)
            {
                notes.push(note(
                    "weak-evidence-occupancy",
                    format!(
                        "occupancy={:.1}%",
                        measurement.occupancy.unwrap_or_default() * 100.0
                    ),
                    "The evidence block uses a small share of the canvas; confirm that fine labels remain useful at room distance.",
                ));
            }
            if measurement.content_role == "text"
                && measurement.type_role == "body"
                && (measurement
                    .prose_characters
                    .is_some_and(|characters| characters > 220)
                    || measurement.line_count.is_some_and(|lines| lines > 5))
            {
                notes.push(note(
                    "long-prose",
                    format!(
                        "characters={} line_count={}",
                        measurement.prose_characters.unwrap_or_default(),
                        measurement.line_count.unwrap_or_default()
                    ),
                    "The prose block is long for a projected slide; review reading burden and speaker duplication.",
                ));
            }
            if measurement.image_backed_text {
                notes.push(note(
                    "image-backed-text-manual-contrast",
                    "background=image".to_string(),
                    "Text crosses an image-backed region, so local contrast needs manual review.",
                ));
            } else if measurement
                .contrast_ratio
                .is_some_and(|contrast| contrast < 4.5)
            {
                notes.push(note(
                    "weak-solid-contrast",
                    format!(
                        "contrast_ratio={:.2}:1",
                        measurement.contrast_ratio.unwrap_or_default()
                    ),
                    "Computed solid-color text contrast is below the report-only candidate floor of 4.5:1.",
                ));
            }
            if measurement.figure_alternative_status.as_deref() == Some("present") {
                notes.push(note(
                    "figure-alternative-quality-manual-review",
                    "short_alternative=present caption=optional".to_string(),
                    "The Figure has a short alternative; a human must confirm that the wording adequately conveys its purpose and evidence and that complex relations remain available in nearby prose or speaker description.",
                ));
            }
            if measurement
                .resolution_scale
                .is_some_and(|scale| scale < 1.0)
            {
                notes.push(note(
                    "undersampled-image",
                    format!(
                        "resolution_scale={:.2}",
                        measurement.resolution_scale.unwrap_or_default()
                    ),
                    "The image is rendered larger than its intrinsic pixel dimensions.",
                ));
            }
        }
    }
    for (surface, expectation, observation) in screen
        .iter()
        .map(|report| {
            (
                "screen",
                report.authored_background_expectation.as_ref(),
                &report.observation,
            )
        })
        .chain(pages.iter().map(|report| {
            (
                "print",
                report.authored_background_expectation.as_ref(),
                &report.observation,
            )
        }))
    {
        let Some(expectation) = expectation else {
            continue;
        };
        let palette = observation
            .design_measurements
            .first()
            .map_or("default", |measurement| measurement.palette.as_str());
        let note = |code: &str, basis: String, message: &str| VisualDesignReviewNote {
            code: code.to_string(),
            surface: surface.to_string(),
            palette: palette.to_string(),
            slide_id: observation.slide_id.clone(),
            step: observation
                .screen_step
                .or(observation.pdf_step)
                .unwrap_or(0),
            element: ".zpres-background-semantic".to_string(),
            role: format!("background-{}", expectation.intent),
            measured_basis: basis,
            message: message.to_string(),
        };
        if expectation.intent != "decorative" {
            notes.push(note(
                "background-alternative-quality-manual-review",
                format!(
                    "intent={} short_alternative_present={} long_description_present={}",
                    expectation.intent,
                    expectation.short_alternative_present,
                    expectation.long_description_present,
                ),
                "The background has a structural alternative route; a human must confirm that the wording adequately conveys its purpose.",
            ));
        }
        if expectation.focal_crop_review == "manual_review_required" {
            notes.push(note(
                "background-focal-crop-manual-review",
                format!(
                    "intent={} split={:?} focal_crop_review={}",
                    expectation.intent, expectation.split, expectation.focal_crop_review,
                ),
                "A meaningful cover or split background needs full-size focal-content and crop review.",
            ));
        }
        if expectation.image_backed_contrast_review_required {
            notes.push(note(
                "background-local-contrast-manual-review",
                format!(
                    "intent={} phase={:?}",
                    expectation.intent, expectation.phase,
                ),
                "Text may cross this authored image; arbitrary local photographic contrast remains a human-review item.",
            ));
        }
    }
    notes
}

fn append_findings(text: &mut String, findings: &[VisualFinding]) {
    if findings.is_empty() {
        text.push_str("none\n");
        return;
    }
    for finding in findings {
        text.push_str(&format!(
            "- [{}] surface={} item={} route={} step={} slide={} element={} boundary={} evidence={}: {}\n",
            finding.code,
            finding.surface,
            finding
                .page
                .map(|page| page.to_string())
                .unwrap_or_else(|| "deck".to_string()),
            finding.route.as_deref().unwrap_or("n/a"),
            finding
                .step
                .map(|step| step.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            finding.slide_id.as_deref().unwrap_or("unknown"),
            finding.element.as_deref().unwrap_or("n/a"),
            finding.boundary.as_deref().unwrap_or("n/a"),
            finding.evidence.as_deref().unwrap_or("n/a"),
            finding.message
        ));
    }
}

fn enum_json(value: &impl Serialize) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"unknown\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn write_json(
    namespace: &OutputNamespaceGuard,
    path: &Path,
    artifact: &'static str,
    value: &impl Serialize,
) -> Result<(), VisualError> {
    let mut json = serde_json::to_vec_pretty(value)
        .map_err(|source| VisualError::Serialize { artifact, source })?;
    json.push(b'\n');
    write_file(namespace, path, &json)
}

fn verify_png_dimensions(
    bytes: &[u8],
    expected: PngViewport,
    artifact: &str,
) -> Result<(), VisualError> {
    let image = image::load_from_memory(bytes).map_err(|source| VisualError::InvalidPng {
        artifact: artifact.to_string(),
        source,
    })?;
    let (observed_width, observed_height) = image.dimensions();
    if observed_width != expected.width || observed_height != expected.height {
        return Err(VisualError::WrongPngDimensions {
            artifact: artifact.to_string(),
            expected_width: expected.width,
            expected_height: expected.height,
            observed_width,
            observed_height,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ContactSheetLayout {
    columns: u32,
    tile_width: u32,
    tile_height: u32,
    gap: u32,
    margin: u32,
    label_height: u32,
    width: u32,
    height: u32,
}

impl ContactSheetLayout {
    fn for_page_count(page_count: usize, viewport: PngViewport) -> Self {
        let columns: u32 = match page_count {
            0 | 1 => 1,
            2..=4 => 2,
            5..=12 => 4,
            _ => 5,
        };
        let rows = (page_count as u32).div_ceil(columns).max(1);
        let tile_width = 300;
        let tile_height = ((tile_width as f64 * viewport.height.max(1) as f64
            / viewport.width.max(1) as f64)
            .round() as u32)
            .max(1);
        let gap = 18;
        let margin = 24;
        let label_height = 28;
        let width = margin * 2 + columns * tile_width + columns.saturating_sub(1) * gap;
        let height =
            margin * 2 + rows * (tile_height + label_height) + rows.saturating_sub(1) * gap;
        Self {
            columns,
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

fn render_contact_sheet_html(page_images: &[PathBuf], layout: ContactSheetLayout) -> String {
    let mut html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;width:{}px;height:{}px;overflow:hidden;background:#111827}}body{{box-sizing:border-box;padding:{}px;font-family:system-ui,sans-serif;color:#e5e7eb}}main{{display:grid;grid-template-columns:repeat({},{}px);gap:{}px}}figure{{display:grid;gap:8px;margin:0}}img{{display:block;width:{}px;height:{}px;object-fit:contain;background:#020617}}span{{font-size:13px;line-height:{}px}}</style></head><body><main>",
        layout.width,
        layout.height,
        layout.margin,
        layout.columns,
        layout.tile_width,
        layout.gap,
        layout.tile_width,
        layout.tile_height,
        layout.label_height,
    );
    for (index, path) in page_images.iter().enumerate() {
        html.push_str(&format!(
            "<figure><img src=\"{}\" alt=\"Page {}\"><span>page {}</span></figure>",
            escape_html_attr(&file_url(path)),
            index + 1,
            index + 1,
        ));
    }
    html.push_str("</main></body></html>");
    html
}

fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn clear_previous_visual_artifacts(output_dir: &Path) -> Result<(), VisualError> {
    for name in [
        "contact-sheet.png",
        "screen-contact-sheet.png",
        "visual-report.json",
        "visual-report.txt",
        "room-profile-calibration.json",
        "provenance.json",
        ".zpres-contact-sheet.html",
        ".zpres-screen-contact-sheet.html",
    ] {
        let path = output_dir.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(VisualError::Write { path, source }),
        }
    }
    let entries = fs::read_dir(output_dir).map_err(|source| VisualError::Read {
        path: output_dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| VisualError::Read {
            path: output_dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".zpres-visual-page-") && name.ends_with(".html") {
            let path = entry.path();
            fs::remove_file(&path).map_err(|source| VisualError::Write { path, source })?;
        }
    }
    Ok(())
}

fn create_dir(path: &Path) -> Result<(), VisualError> {
    fs::create_dir_all(path).map_err(|source| VisualError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn write_file(
    namespace: &OutputNamespaceGuard,
    path: &Path,
    bytes: &[u8],
) -> Result<(), VisualError> {
    let publication = namespace.publish_file(path, bytes)?;
    for warning in publication.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn write_reserved_file(
    namespace: &OutputNamespaceGuard,
    path: &Path,
    bytes: &[u8],
) -> Result<(), VisualError> {
    let publication = namespace.publish_reserved_file(path, bytes)?;
    for warning in publication.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>, VisualError> {
    fs::read(path).map_err(|source| VisualError::Read {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chromium::{
        BrowserDesignMeasurement, BrowserImageObservation, BrowserOverflowDeltas,
        BrowserSemanticTextRegion, BrowserUnresolved, BrowserWhitespace,
    };
    use crate::theme::{load_theme_manifest, render_theme};
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;
    use tempfile::tempdir;

    fn ordinary_observation() -> BrowserPageObservation {
        BrowserPageObservation {
            document_ready_state: "complete".to_string(),
            autoscale_ready: true,
            slide_present: true,
            page: Some(1),
            slide_id: "section-1-main".to_string(),
            title: "A result".to_string(),
            role: "main".to_string(),
            slide_bounds: Some(crate::chromium::BrowserRect {
                width: 1280.0,
                height: 720.0,
                right: 1280.0,
                bottom: 720.0,
                ..Default::default()
            }),
            canvas_bounds: Some(Default::default()),
            content_bounds: Some(Default::default()),
            content_union: Some(Default::default()),
            occupancy: Some(0.55),
            whitespace: Some(BrowserWhitespace {
                left: 0.1,
                top: 0.1,
                right: 0.1,
                bottom: 0.1,
            }),
            ..Default::default()
        }
    }

    fn semantic_region_observation() -> BrowserPageObservation {
        let mut observation = ordinary_observation();
        observation.semantic_text_regions = vec![BrowserSemanticTextRegion {
            region: "header".to_string(),
            element: "header.zpres-slide-header".to_string(),
            authored_text: "Canonical wide-sweep zpres fixture".to_string(),
            text_bounds: vec![crate::chromium::BrowserRect {
                x: 10.0,
                y: 10.0,
                width: 40.0,
                height: 20.0,
                right: 50.0,
                bottom: 30.0,
            }],
            raster_ink: None,
        }];
        observation.slide_bounds = Some(crate::chromium::BrowserRect {
            width: 100.0,
            height: 60.0,
            right: 100.0,
            bottom: 60.0,
            ..Default::default()
        });
        observation
    }

    fn png_bytes(image: RgbaImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode test PNG");
        bytes.into_inner()
    }

    #[test]
    fn v1_reference_shaped_blank_semantic_text_blocks_screen_and_print_with_evidence() {
        let bytes = png_bytes(RgbaImage::from_pixel(100, 60, Rgba([250, 249, 245, 255])));
        for (surface, screenshot) in [
            ("screen", "screen-pages/state-001.png"),
            ("print", "pages/page-001.png"),
        ] {
            let mut observation = semantic_region_observation();
            annotate_semantic_text_ink(
                &mut observation,
                &bytes,
                &bytes,
                PngViewport {
                    width: 100,
                    height: 60,
                },
                screenshot,
            )
            .unwrap();
            let mut failures = Vec::new();
            classify_semantic_text_ink(surface, 1, None, &observation, screenshot, &mut failures);
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].code, "missing-semantic-text-ink");
            assert_eq!(failures[0].surface, surface);
            assert_eq!(failures[0].slide_id.as_deref(), Some("section-1-main"));
            assert_eq!(failures[0].evidence.as_deref(), Some(screenshot));
            assert!(failures[0].message.contains("original-size evidence"));
        }
    }

    #[test]
    fn sparse_text_and_image_dominant_semantic_ranges_remain_green() {
        let viewport = PngViewport {
            width: 100,
            height: 60,
        };
        let sparse_suppressed = RgbaImage::from_pixel(100, 60, Rgba([250, 249, 245, 255]));
        let mut sparse = sparse_suppressed.clone();
        for x in 14..34 {
            sparse.put_pixel(x, 18, Rgba([20, 30, 28, 255]));
        }
        let mut image_led_suppressed = RgbaImage::from_pixel(100, 60, Rgba([250, 249, 245, 255]));
        for y in 10..30 {
            for x in 10..50 {
                image_led_suppressed.put_pixel(
                    x,
                    y,
                    Rgba([(x * 5) as u8, (y * 7) as u8, 120, 255]),
                );
            }
        }
        let mut image_led = image_led_suppressed.clone();
        for x in 14..34 {
            image_led.put_pixel(x, 18, Rgba([20, 30, 28, 255]));
        }

        for (bytes, suppressed) in [
            (png_bytes(sparse), png_bytes(sparse_suppressed)),
            (png_bytes(image_led), png_bytes(image_led_suppressed)),
        ] {
            let mut observation = semantic_region_observation();
            observation.occupancy = Some(0.04);
            annotate_semantic_text_ink(
                &mut observation,
                &bytes,
                &suppressed,
                viewport,
                "control.png",
            )
            .unwrap();
            let mut failures = Vec::new();
            classify_semantic_text_ink(
                "print",
                1,
                None,
                &observation,
                "control.png",
                &mut failures,
            );
            assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        }
    }

    #[test]
    fn visual_preflight_preserves_a_raster_root_nested_below_replaced_pages() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("specimen");
        let nested = output.join("pages").join("archive");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join(crate::output_ownership::RASTER_OWNERSHIP_MARKER_FILE),
            b"owned",
        )
        .unwrap();
        fs::write(nested.join("page-001.png"), b"preserved").unwrap();

        let error = match prepare_visual_output_namespace(&output) {
            Ok(_) => panic!("visual preflight accepted a nested raster page set"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            VisualError::OutputOwnership(OutputOwnershipError::OwnedRasterAncestor { .. })
        ));
        assert_eq!(fs::read(nested.join("page-001.png")).unwrap(), b"preserved");
    }

    #[test]
    fn clipped_slide_is_an_objective_failure_with_identity() {
        let mut observation = ordinary_observation();
        observation.clip_marker = true;
        let mut warnings = Vec::new();
        let mut failures = Vec::new();
        classify_page(
            1,
            &observation,
            PngViewport::default(),
            &mut warnings,
            &mut failures,
        );
        assert!(failures.iter().any(|finding| {
            finding.code == "clipped-content"
                && finding.slide_id.as_deref() == Some("section-1-main")
                && finding.title.as_deref() == Some("A result")
        }));
    }

    #[test]
    fn sparse_section_title_warns_without_failing() {
        let mut observation = ordinary_observation();
        observation.variant = Some("section-title".to_string());
        observation.occupancy = Some(0.08);
        let mut warnings = Vec::new();
        let mut failures = Vec::new();
        classify_page(
            1,
            &observation,
            PngViewport::default(),
            &mut warnings,
            &mut failures,
        );
        assert!(
            warnings
                .iter()
                .any(|finding| finding.code == "sparse-composition")
        );
        assert!(failures.is_empty());
    }

    #[test]
    fn autoscaling_is_retained_as_a_calibration_warning() {
        let mut observation = ordinary_observation();
        observation.autoscale_factor = Some(0.84);
        let mut warnings = Vec::new();
        let mut failures = Vec::new();

        classify_page(
            1,
            &observation,
            PngViewport::default(),
            &mut warnings,
            &mut failures,
        );

        assert!(warnings.iter().any(|finding| {
            finding.code == "autoscaled-content"
                && finding.message.contains("84.0%")
                && finding.surface == "print"
        }));
        assert!(failures.is_empty());
    }

    #[test]
    fn design_measurements_create_notes_without_changing_release_status() {
        let mut observation = ordinary_observation();
        observation.design_measurements = vec![BrowserDesignMeasurement {
            element: "p.lead".to_string(),
            type_role: "body".to_string(),
            content_role: "text".to_string(),
            surface: "screen".to_string(),
            palette: "dark".to_string(),
            slide_id: observation.slide_id.clone(),
            prose_characters: Some(260),
            line_count: Some(7),
            font_size_px: Some(32.0),
            contrast_ratio: Some(3.8),
            actual_font_families: vec!["Talk Sans".to_string()],
            ..Default::default()
        }];
        let screen = vec![VisualScreenStateReport {
            route: BrowserScreenRoute {
                hash: "#/0/0".to_string(),
                section: 0,
                detail: 0,
                step: 0,
                step_count: 0,
                slide_id: observation.slide_id.clone(),
                role: "main".to_string(),
                generated: None,
            },
            screenshot: "screen-pages/state-001.png".to_string(),
            authored_background_expectation: None,
            observation,
        }];

        let summary = summarize_design_measurements(&screen, &[]);
        let notes = collect_design_review_notes(&screen, &[]);
        let (status, _, release) = statuses_for_failures(&[]);

        assert_eq!(summary.min_essential_type_px, Some(32.0));
        assert_eq!(summary.actual_fonts, vec!["Talk Sans"]);
        assert!(notes.iter().any(|note| note.code == "long-prose"));
        assert!(notes.iter().any(|note| note.code == "weak-solid-contrast"));
        assert_eq!(status, VisualStatus::Passed);
        assert_eq!(release, ReleaseStatus::PendingReview);
    }

    #[test]
    fn provisional_room_profile_reports_but_only_approved_v1_profiles_block() {
        let mut observation = ordinary_observation();
        observation.autoscale_factor = Some(0.70);
        observation.design_measurements = vec![BrowserDesignMeasurement {
            element: "p".to_string(),
            type_role: "body".to_string(),
            surface: "screen".to_string(),
            slide_id: observation.slide_id.clone(),
            font_size_px: Some(28.0),
            essential_content: true,
            ..Default::default()
        }];
        let provisional = resolve_builtin("projected-room-default").unwrap();

        assert!(
            validate_room_profile("screen", 1, None, &observation, &provisional, true,).is_empty()
        );
        let notes = collect_room_profile_notes(
            &[VisualScreenStateReport {
                route: BrowserScreenRoute {
                    hash: "#/0/0".to_string(),
                    section: 0,
                    detail: 0,
                    step: 0,
                    step_count: 0,
                    slide_id: observation.slide_id.clone(),
                    role: "main".to_string(),
                    generated: None,
                },
                screenshot: "screen-pages/state-001.png".to_string(),
                authored_background_expectation: None,
                observation: observation.clone(),
            }],
            &[],
            &provisional,
        );
        assert!(notes.iter().any(|note| {
            note.code == "room-profile-type-floor-review" && note.message.contains("report-only")
        }));

        let mut approved = provisional;
        approved.decision_status = RoomProfileStatus::Approved;
        approved.enforcement_status = RoomProfileEnforcementStatus::BlockingV1;
        let failures = validate_room_profile("screen", 1, None, &observation, &approved, true);
        assert!(
            failures
                .iter()
                .any(|finding| finding.code == "room-profile-type-floor")
        );
        assert!(
            failures
                .iter()
                .any(|finding| finding.code == "room-profile-autoscale-hard-floor")
        );
        assert!(
            validate_room_profile("screen", 1, None, &observation, &approved, false).is_empty()
        );
    }

    #[test]
    fn room_profile_calibration_preserves_surface_threshold_distributions() {
        let mut observation = ordinary_observation();
        observation.autoscale_factor = Some(0.70);
        observation.design_measurements = vec![
            BrowserDesignMeasurement {
                element: "p".to_string(),
                type_role: "body".to_string(),
                content_role: "text".to_string(),
                surface: "screen".to_string(),
                slide_id: observation.slide_id.clone(),
                font_size_px: Some(28.0),
                contrast_ratio: Some(6.5),
                essential_content: true,
                ..Default::default()
            },
            BrowserDesignMeasurement {
                element: "figure".to_string(),
                type_role: "body".to_string(),
                content_role: "evidence".to_string(),
                surface: "screen".to_string(),
                slide_id: observation.slide_id.clone(),
                font_size_px: Some(32.0),
                occupancy: Some(0.12),
                ..Default::default()
            },
        ];
        let screen = vec![VisualScreenStateReport {
            route: BrowserScreenRoute {
                hash: "#/0/0".to_string(),
                section: 0,
                detail: 0,
                step: 0,
                step_count: 0,
                slide_id: observation.slide_id.clone(),
                role: "main".to_string(),
                generated: None,
            },
            screenshot: "screen-pages/state-001.png".to_string(),
            authored_background_expectation: None,
            observation,
        }];
        let profile = resolve_builtin("projected-room-default").unwrap();

        let summary = summarize_room_profile_calibration(&screen, &[], &profile);
        let screen = &summary.surfaces["screen"];

        assert_eq!(summary.schema_version, 1);
        assert_eq!(screen.observation_count, 1);
        assert_eq!(screen.type_roles["body"].sample_count, 2);
        assert_eq!(screen.type_roles["body"].essential_sample_count, 1);
        assert_eq!(screen.type_roles["body"].essential_below_floor_count, 1);
        assert_eq!(screen.type_roles["body"].distribution_px.min, Some(28.0));
        assert_eq!(screen.contrast["ordinary"].below_threshold_count, 1);
        assert_eq!(screen.evidence_occupancy.below_threshold_count, 1);
        assert_eq!(screen.autoscale.below_warning_count, 1);
        assert_eq!(screen.autoscale.below_hard_floor_count, 1);
        assert_eq!(summary.surfaces["print"].observation_count, 0);
    }

    #[test]
    fn v1_text_contrast_uses_wcag_large_classification_as_a_blocking_floor() {
        let mut observation = ordinary_observation();
        observation.design_measurements = vec![
            BrowserDesignMeasurement {
                element: "p".to_string(),
                slide_id: observation.slide_id.clone(),
                font_size_px: Some(20.0),
                font_weight: Some(400),
                wcag_large_text: false,
                contrast_ratio: Some(4.49),
                ..Default::default()
            },
            BrowserDesignMeasurement {
                element: "h1".to_string(),
                slide_id: observation.slide_id.clone(),
                font_size_px: Some(32.0),
                font_weight: Some(700),
                wcag_large_text: true,
                contrast_ratio: Some(3.01),
                ..Default::default()
            },
        ];

        let failures = validate_v1_text_contrast("screen", 1, None, &observation);

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "insufficient-text-contrast");
        assert_eq!(failures[0].element.as_deref(), Some("p"));
        assert!(failures[0].message.contains("requires at least 4.5:1"));
    }

    #[test]
    fn unresolved_failed_images_and_outside_content_are_failures() {
        let mut observation = ordinary_observation();
        observation.outside_canvas = Some(BrowserOverflowDeltas {
            right: 9.0,
            ..Default::default()
        });
        observation.unresolved.push(BrowserUnresolved {
            marker: "chart".to_string(),
            excerpt: "chart could not be rendered".to_string(),
        });
        observation.images.push(BrowserImageObservation {
            source: "file:///missing.png".to_string(),
            visible: true,
            decoded: false,
            error: Some("load failed".to_string()),
            ..Default::default()
        });
        let mut warnings = Vec::new();
        let mut failures = Vec::new();
        classify_page(
            1,
            &observation,
            PngViewport::default(),
            &mut warnings,
            &mut failures,
        );
        let codes = failures
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("content-outside-canvas"));
        assert!(codes.contains("unresolved-content"));
        assert!(codes.contains("failed-image"));
    }

    #[test]
    fn slide_canvas_that_does_not_match_the_deck_viewport_fails() {
        let observation = ordinary_observation();
        let mut warnings = Vec::new();
        let mut failures = Vec::new();

        classify_page(
            1,
            &observation,
            PngViewport {
                width: 960,
                height: 720,
            },
            &mut warnings,
            &mut failures,
        );

        assert!(failures.iter().any(|finding| {
            finding.code == "slide-viewport-mismatch"
                && finding.slide_id.as_deref() == Some("section-1-main")
        }));
    }

    #[test]
    fn report_blocks_release_on_wrong_page_count() {
        let mut failures = Vec::new();
        classify_page_count(3, 2, &mut failures);
        let (visual_status, human_review, release_status) = statuses_for_failures(&failures);
        assert!(
            failures
                .iter()
                .any(|finding| finding.code == "wrong-page-count")
        );
        assert!(visual_status.is_failed());
        assert_eq!(human_review, HumanReviewStatus::Required);
        assert_eq!(release_status, ReleaseStatus::Blocked);

        failures.clear();
        classify_page_count(3, 3, &mut failures);
        let (visual_status, human_review, release_status) = statuses_for_failures(&failures);
        assert_eq!(visual_status, VisualStatus::Passed);
        assert_eq!(human_review, HumanReviewStatus::Required);
        assert_eq!(release_status, ReleaseStatus::PendingReview);
    }

    #[test]
    fn package_digest_is_stable_and_path_sensitive() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first.css");
        let second = temp.path().join("second.css");
        fs::write(&first, "a{}").unwrap();
        fs::write(&second, "b{}").unwrap();
        let paths = vec![
            ThemeProvenanceInput {
                declared_path: PathBuf::from("first.css"),
                resolved_path: first,
            },
            ThemeProvenanceInput {
                declared_path: PathBuf::from("second.css"),
                resolved_path: second,
            },
        ];
        let digest = digest_package(&paths).unwrap();
        assert_eq!(digest, digest_package(&paths).unwrap());
        assert_ne!(
            digest,
            digest_package(&[paths[1].clone(), paths[0].clone()]).unwrap()
        );
    }

    #[test]
    fn visual_provenance_hashes_declared_fonts_and_assets() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("dependency-theme");
        fs::create_dir_all(theme_dir.join("fonts")).unwrap();
        fs::create_dir_all(theme_dir.join("textures")).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "dependency-theme"
version = "0.1.0"
api_version = 1
fonts = ["fonts/body.woff2"]
assets = ["textures/paper.png"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(theme_dir.join("theme.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("print.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("fonts/body.woff2"), b"font one").unwrap();
        fs::write(theme_dir.join("textures/paper.png"), b"paper").unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "# Talk\n").unwrap();
        let deck = crate::deck::parse_source_file(&source).unwrap();
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = render_theme(&manifest, &BTreeMap::new()).unwrap();
        let chromium = ChromiumVersion {
            product: "test".to_string(),
            revision: "test".to_string(),
            user_agent: "test".to_string(),
            js_version: "test".to_string(),
            protocol_version: "test".to_string(),
        };

        let first = build_provenance(
            &source,
            &deck,
            &rendered,
            PngViewport::default(),
            Path::new("chromium"),
            &chromium,
            &resolve_builtin("projected-room-default").unwrap(),
            false,
        )
        .unwrap();
        let files = first
            .theme_files
            .iter()
            .map(|file| file.path.as_path())
            .collect::<BTreeSet<_>>();
        assert!(files.contains(Path::new("fonts/body.woff2")));
        assert!(files.contains(Path::new("textures/paper.png")));
        let first_font_hash = first
            .theme_files
            .iter()
            .find(|file| file.path == Path::new("fonts/body.woff2"))
            .unwrap()
            .sha256
            .clone();

        fs::write(theme_dir.join("fonts/body.woff2"), b"font two").unwrap();
        let second = build_provenance(
            &source,
            &deck,
            &rendered,
            PngViewport::default(),
            Path::new("chromium"),
            &chromium,
            &resolve_builtin("projected-room-default").unwrap(),
            false,
        )
        .unwrap();
        let second_font_hash = second
            .theme_files
            .iter()
            .find(|file| file.path == Path::new("fonts/body.woff2"))
            .unwrap()
            .sha256
            .as_str();

        assert_ne!(first_font_hash, second_font_hash);
        assert_ne!(first.theme_package_sha256, second.theme_package_sha256);
    }

    #[test]
    fn v1_visual_provenance_hashes_even_a_dormant_deck_background() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("v1-theme");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "v1-theme"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(theme_dir.join("theme.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("print.css.tmpl"), "").unwrap();
        let assets = temp.path().join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("field.svg"), b"first field").unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            "---\nbackground_image:\n  src: assets/field.svg\n---\n\n# Clean title\n",
        )
        .unwrap();
        let deck = crate::deck::parse_source_file(&source).unwrap();
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = render_theme(&manifest, &BTreeMap::new()).unwrap();
        let chromium = ChromiumVersion {
            product: "test".to_string(),
            revision: "test".to_string(),
            user_agent: "test".to_string(),
            js_version: "test".to_string(),
            protocol_version: "test".to_string(),
        };

        let first = build_provenance(
            &source,
            &deck,
            &rendered,
            PngViewport::default(),
            Path::new("chromium"),
            &chromium,
            &resolve_builtin("projected-room-default").unwrap(),
            false,
        )
        .unwrap();
        fs::write(assets.join("field.svg"), b"second field").unwrap();
        let second = build_provenance(
            &source,
            &deck,
            &rendered,
            PngViewport::default(),
            Path::new("chromium"),
            &chromium,
            &resolve_builtin("projected-room-default").unwrap(),
            false,
        )
        .unwrap();

        assert_eq!(first.schema_version, VISUAL_PROVENANCE_SCHEMA_VERSION);
        assert_eq!(first.source_sha256, second.source_sha256);
        assert_eq!(first.theme_package_sha256, second.theme_package_sha256);
        assert_eq!(
            first.presentation_plan_sha256,
            second.presentation_plan_sha256
        );
        assert_ne!(first.deck_background_sha256, second.deck_background_sha256);
        assert_ne!(
            first.deck_background_files[0].sha256,
            second.deck_background_files[0].sha256
        );
    }

    #[cfg(unix)]
    #[test]
    fn visual_provenance_preserves_declared_dependency_aliases() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("alias-theme");
        fs::create_dir_all(theme_dir.join("fonts")).unwrap();
        fs::create_dir_all(theme_dir.join("shared")).unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "alias-theme"
version = "0.1.0"
api_version = 1
fonts = ["fonts/body.woff2", "fonts/body-alias.woff2"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(theme_dir.join("theme.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("print.css.tmpl"), "").unwrap();
        fs::write(theme_dir.join("shared/body.woff2"), b"shared font").unwrap();
        symlink("../shared/body.woff2", theme_dir.join("fonts/body.woff2")).unwrap();
        symlink(
            "../shared/body.woff2",
            theme_dir.join("fonts/body-alias.woff2"),
        )
        .unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "# Talk\n").unwrap();
        let deck = crate::deck::parse_source_file(&source).unwrap();
        let manifest = load_theme_manifest(&theme_dir.join("theme.toml")).unwrap();
        let rendered = render_theme(&manifest, &BTreeMap::new()).unwrap();
        let chromium = ChromiumVersion {
            product: "test".to_string(),
            revision: "test".to_string(),
            user_agent: "test".to_string(),
            js_version: "test".to_string(),
            protocol_version: "test".to_string(),
        };

        let provenance = build_provenance(
            &source,
            &deck,
            &rendered,
            PngViewport::default(),
            Path::new("chromium"),
            &chromium,
            &resolve_builtin("projected-room-default").unwrap(),
            false,
        )
        .unwrap();
        let aliases = provenance
            .theme_files
            .iter()
            .filter(|file| file.path.starts_with("fonts"))
            .collect::<Vec<_>>();

        assert_eq!(aliases.len(), 2);
        assert!(
            aliases
                .iter()
                .any(|file| file.path == Path::new("fonts/body.woff2"))
        );
        assert!(
            aliases
                .iter()
                .any(|file| file.path == Path::new("fonts/body-alias.woff2"))
        );
        assert_eq!(aliases[0].sha256, aliases[1].sha256);
        assert!(
            !provenance
                .theme_files
                .iter()
                .any(|file| file.path == Path::new("shared/body.woff2"))
        );
    }

    #[test]
    fn deck_aspect_controls_review_viewport() {
        let mut deck = Deck {
            source_path: None,
            metadata: Default::default(),
            sections: Vec::new(),
            diagnostics: Vec::new(),
        };
        deck.metadata.aspect = Some("4:3".to_string());
        assert_eq!(
            viewport_for_deck(&deck),
            PngViewport {
                width: 960,
                height: 720
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_url_percent_encodes_spaces() {
        let value = file_url(Path::new("/tmp/a visual/page.html"));
        assert_eq!(value, "file:///tmp/a%20visual/page.html");
    }

    #[test]
    fn contact_sheet_tiles_follow_the_review_viewport_aspect() {
        let layout = ContactSheetLayout::for_page_count(
            2,
            PngViewport {
                width: 960,
                height: 720,
            },
        );
        assert_eq!(layout.tile_width, 300);
        assert_eq!(layout.tile_height, 225);
    }

    #[test]
    fn old_visual_results_are_removed_before_a_new_run() {
        let temp = tempdir().unwrap();
        for name in [
            "contact-sheet.png",
            "screen-contact-sheet.png",
            "visual-report.json",
            "visual-report.txt",
            "room-profile-calibration.json",
            "provenance.json",
            ".zpres-contact-sheet.html",
            ".zpres-screen-contact-sheet.html",
            ".zpres-visual-page-999.html",
        ] {
            fs::write(temp.path().join(name), "old").unwrap();
        }

        clear_previous_visual_artifacts(temp.path()).unwrap();

        for name in [
            "contact-sheet.png",
            "screen-contact-sheet.png",
            "visual-report.json",
            "visual-report.txt",
            "room-profile-calibration.json",
            "provenance.json",
            ".zpres-contact-sheet.html",
            ".zpres-screen-contact-sheet.html",
            ".zpres-visual-page-999.html",
        ] {
            assert!(!temp.path().join(name).exists(), "stale {name} survived");
        }
    }
}
