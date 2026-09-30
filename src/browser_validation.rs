use serde::{Deserialize, Serialize};

use crate::chromium::{
    BrowserFontEvidenceStatus, BrowserOverflowDeltas, BrowserPageObservation, BrowserRect,
    BrowserScreenRoute,
};
use crate::pdf::PngViewport;

pub(crate) const CONTENT_BOUNDS_TOLERANCE_PX: f64 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserFinding {
    pub surface: String,
    pub code: String,
    pub page: Option<usize>,
    pub route: Option<String>,
    pub step: Option<usize>,
    pub slide_id: Option<String>,
    pub title: Option<String>,
    pub element: Option<String>,
    pub boundary: Option<String>,
    pub element_bounds: Option<BrowserRect>,
    pub boundary_bounds: Option<BrowserRect>,
    pub intersection_bounds: Option<BrowserRect>,
    pub deltas: Option<BrowserOverflowDeltas>,
    pub evidence: Option<String>,
    pub message: String,
}

impl BrowserFinding {
    pub(crate) fn deck(surface: &str, code: &str, message: String) -> Self {
        Self {
            surface: surface.to_string(),
            code: code.to_string(),
            page: None,
            route: None,
            step: None,
            slide_id: None,
            title: None,
            element: None,
            boundary: None,
            element_bounds: None,
            boundary_bounds: None,
            intersection_bounds: None,
            deltas: None,
            evidence: None,
            message,
        }
    }
}

pub(crate) fn validate_browser_page(
    page: usize,
    observation: &BrowserPageObservation,
    viewport: PngViewport,
) -> Vec<BrowserFinding> {
    let finding = |code: &str, message: String| BrowserFinding {
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
    let mut failures = Vec::new();

    if !observation.slide_present {
        failures.push(finding(
            "missing-slide",
            "the executed page contains no static Slide element".to_string(),
        ));
        return failures;
    }
    if observation.page != Some(page) {
        failures.push(finding(
            "page-index-mismatch",
            format!(
                "expected page index {page}, but the executed Slide reports {:?}",
                observation.page
            ),
        ));
    }
    match observation.slide_bounds {
        Some(bounds) => {
            let full_document_y = page.saturating_sub(1) as f64 * viewport.height as f64;
            let y_offset_error = (bounds.y - full_document_y).abs();
            if bounds.x.abs() > CONTENT_BOUNDS_TOLERANCE_PX
                || y_offset_error > CONTENT_BOUNDS_TOLERANCE_PX
                || (bounds.width - viewport.width as f64).abs() > CONTENT_BOUNDS_TOLERANCE_PX
                || (bounds.height - viewport.height as f64).abs() > CONTENT_BOUNDS_TOLERANCE_PX
            {
                failures.push(finding(
                    "slide-viewport-mismatch",
                    format!(
                        "the rendered Slide is {:.1}x{:.1} at ({:.1}, {:.1}), but the Deck review viewport is {}x{}; non-matching canvases can crop captured content",
                        bounds.width,
                        bounds.height,
                        bounds.x,
                        bounds.y,
                        viewport.width,
                        viewport.height,
                    ),
                ));
            }
        }
        None => failures.push(finding(
            "missing-slide-bounds",
            "the executed Slide has no measurable browser bounds".to_string(),
        )),
    }
    if observation.document_ready_state != "complete" {
        failures.push(finding(
            "document-not-ready",
            format!(
                "document.readyState is '{}' after the readiness wait",
                observation.document_ready_state
            ),
        ));
    }
    if !observation.autoscale_ready {
        failures.push(finding(
            "autoscale-not-ready",
            "autoscale did not complete before capture".to_string(),
        ));
    }
    for error in &observation.readiness_errors {
        failures.push(finding("readiness-error", error.clone()));
    }
    if observation.clip_marker {
        failures.push(finding(
            "clipped-content",
            "the Slide retained data-zpres-overflow=clipped after final layout".to_string(),
        ));
    }
    if let Some(outside) = observation.outside_canvas
        && outside.maximum() > CONTENT_BOUNDS_TOLERANCE_PX
    {
        failures.push(finding(
            "content-outside-canvas",
            format!(
                "visible content extends {:.1}px outside the canvas (left {:.1}, top {:.1}, right {:.1}, bottom {:.1})",
                outside.maximum(),
                outside.left,
                outside.top,
                outside.right,
                outside.bottom
            ),
        ));
    }
    for unresolved in &observation.unresolved {
        failures.push(finding(
            "unresolved-content",
            format!("{}: {}", unresolved.marker, unresolved.excerpt),
        ));
    }
    for image in observation
        .images
        .iter()
        .chain(observation.background_images.iter())
        .filter(|image| image.visible && (!image.decoded || image.error.is_some()))
    {
        failures.push(finding(
            "failed-image",
            format!(
                "visible image '{}' did not decode{}",
                image.source,
                image
                    .error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ),
        ));
    }
    for font in observation
        .font_faces
        .iter()
        .filter(|font| font.status == "error")
    {
        failures.push(finding(
            "failed-font",
            format!("font face '{}' failed to load", font.family),
        ));
    }
    if observation.platform_font_candidate_count > 0
        && observation.platform_font_evidence.actual_font_use != BrowserFontEvidenceStatus::Observed
    {
        failures.push(finding(
            "missing-platform-font-evidence",
            format!(
                "Chrome found {} visible text candidate(s), but reported no actual platform-font use for the active Slide",
                observation.platform_font_candidate_count
            ),
        ));
    }
    // Representative-probe truncation is retained in the report but is not a
    // failure: actual faces and glyph counts come from one complete Slide probe.
    // A loaded face can also be intentionally unused (for example because of
    // unicode-range), so requested-vs-actual family differences are evidence,
    // not an automatic failure.
    for overflow in &observation.overflow_elements {
        failures.push(finding(
            "scroll-overflow",
            format!(
                "{} has {:.1}px horizontal and {:.1}px vertical scroll overflow",
                overflow.element, overflow.horizontal_px, overflow.vertical_px
            ),
        ));
    }
    for violation in &observation.geometry_violations {
        let message = if let Some(intersection) = violation.intersection_bounds {
            format!(
                "{} and {} intersect at ({:.1}, {:.1}) with exact bounds {:.1}x{:.1} (right {:.1}, bottom {:.1})",
                violation.element,
                violation.boundary,
                intersection.x,
                intersection.y,
                intersection.width,
                intersection.height,
                intersection.right,
                intersection.bottom,
            )
        } else {
            format!(
                "{} crosses {} by {:.1}px (left {:.1}, top {:.1}, right {:.1}, bottom {:.1})",
                violation.element,
                violation.boundary,
                violation.deltas.maximum(),
                violation.deltas.left,
                violation.deltas.top,
                violation.deltas.right,
                violation.deltas.bottom,
            )
        };
        let mut failure = finding(&violation.kind, message);
        failure.element = Some(violation.element.clone());
        failure.boundary = Some(violation.boundary.clone());
        failure.element_bounds = Some(violation.element_bounds);
        failure.boundary_bounds = Some(violation.boundary_bounds);
        failure.intersection_bounds = violation.intersection_bounds;
        failure.deltas = Some(violation.deltas);
        failures.push(failure);
    }
    for violation in &observation.step_visibility_violations {
        let expectation = if violation.expected_visible {
            "visible"
        } else {
            "hidden"
        };
        let actual = if violation.visible {
            "visible"
        } else {
            "hidden"
        };
        let step = violation
            .step_index
            .map(|step| format!(" at Step {step}"))
            .unwrap_or_default();
        let mut failure = finding(
            &violation.kind,
            format!(
                "{} should be {expectation}{step} but is rendered {actual}",
                violation.element,
            ),
        );
        failure.element = Some(violation.element.clone());
        failures.push(failure);
    }
    if observation.generated.as_deref() != Some("background-image")
        && (observation.canvas_bounds.is_none() || observation.content_bounds.is_none())
    {
        failures.push(finding(
            "missing-canvas",
            "the executed Slide is missing its canvas or content wrapper".to_string(),
        ));
    }
    if observation.generated.as_deref() != Some("background-image")
        && observation.content_union.is_none()
    {
        failures.push(finding(
            "missing-visible-content",
            "the Slide has no measurable visible content; background-only pages require an explicit generated background-image role"
                .to_string(),
        ));
    }

    failures
}

pub(crate) fn validate_browser_screen_route(
    state_index: usize,
    route: &BrowserScreenRoute,
    observation: &BrowserPageObservation,
    viewport: PngViewport,
) -> Vec<BrowserFinding> {
    let finding = |code: &str, message: String| BrowserFinding {
        surface: "screen".to_string(),
        code: code.to_string(),
        page: Some(state_index),
        route: Some(route.hash.clone()),
        step: Some(route.step),
        slide_id: (!observation.slide_id.is_empty())
            .then(|| observation.slide_id.clone())
            .or_else(|| Some(route.slide_id.clone())),
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

    let mut failures = validate_browser_page(1, observation, viewport)
        .into_iter()
        .filter(|failure| failure.code != "page-index-mismatch")
        .map(|mut failure| {
            failure.surface = "screen".to_string();
            failure.page = Some(state_index);
            failure.route = Some(route.hash.clone());
            failure.step = Some(route.step);
            failure
        })
        .collect::<Vec<_>>();
    if !observation.slide_present {
        return failures;
    }

    if observation.screen_route.as_deref() != Some(route.hash.as_str())
        || observation.screen_section != Some(route.section)
        || observation.screen_detail != Some(route.detail)
        || observation.screen_step != Some(route.step)
        || observation.screen_step_count != Some(route.step_count)
    {
        failures.push(finding(
            "screen-route-mismatch",
            format!(
                "requested {}, but the settled runtime reported route {:?} at section {:?}, detail {:?}, step {:?} of {:?}",
                route.hash,
                observation.screen_route,
                observation.screen_section,
                observation.screen_detail,
                observation.screen_step,
                observation.screen_step_count,
            ),
        ));
    }
    if observation.slide_id != route.slide_id || observation.role != route.role {
        failures.push(finding(
            "screen-slide-identity-mismatch",
            format!(
                "route {} should activate {} ({}) but activated {} ({})",
                route.hash, route.slide_id, route.role, observation.slide_id, observation.role,
            ),
        ));
    }
    if observation.active_stack_count != Some(1) || observation.active_slide_count != Some(1) {
        failures.push(finding(
            "screen-active-state-mismatch",
            format!(
                "route {} settled with {:?} active Section stacks and {:?} active Slides; expected exactly one of each",
                route.hash, observation.active_stack_count, observation.active_slide_count,
            ),
        ));
    }
    if observation.visible_stack_count != Some(1)
        || observation.visible_slide_count != Some(1)
        || observation.routed_stack_visible != Some(true)
        || observation.routed_slide_visible != Some(true)
    {
        failures.push(finding(
            "screen-visible-state-mismatch",
            format!(
                "route {} rendered {:?} Section stacks and {:?} Slides; routed stack visible={:?}, routed Slide visible={:?}; expected only the routed stack and Slide to be visible",
                route.hash,
                observation.visible_stack_count,
                observation.visible_slide_count,
                observation.routed_stack_visible,
                observation.routed_slide_visible,
            ),
        ));
    }

    for (code, label, bounds) in [
        (
            "screen-viewport-mismatch",
            "browser viewport",
            observation.screen_bounds,
        ),
        (
            "screen-root-mismatch",
            "presentation root",
            observation.root_bounds,
        ),
        (
            "screen-stage-mismatch",
            "presentation stage",
            observation.stage_bounds,
        ),
        (
            "screen-stack-mismatch",
            "active Section stack",
            observation.active_stack_bounds,
        ),
    ] {
        match bounds {
            Some(bounds) if rect_matches_viewport(bounds, viewport) => {}
            Some(bounds) => failures.push(finding(
                code,
                format!(
                    "the {label} is {:.1}x{:.1} at ({:.1}, {:.1}); expected {}x{} at the viewport origin",
                    bounds.width, bounds.height, bounds.x, bounds.y, viewport.width, viewport.height,
                ),
            )),
            None => failures.push(finding(code, format!("the {label} has no measurable bounds"))),
        }
    }

    if let Some(scroll) = observation.document_scroll
        && scroll.maximum() > CONTENT_BOUNDS_TOLERANCE_PX
    {
        let mut failure = finding(
            "screen-document-scroll",
            format!(
                "the live document extends {:.1}px beyond the configured viewport (left {:.1}, top {:.1}, right {:.1}, bottom {:.1})",
                scroll.maximum(),
                scroll.left,
                scroll.top,
                scroll.right,
                scroll.bottom,
            ),
        );
        failure.deltas = Some(scroll);
        failures.push(failure);
    }

    failures
}

fn rect_matches_viewport(bounds: BrowserRect, viewport: PngViewport) -> bool {
    bounds.x.abs() <= CONTENT_BOUNDS_TOLERANCE_PX
        && bounds.y.abs() <= CONTENT_BOUNDS_TOLERANCE_PX
        && (bounds.width - viewport.width as f64).abs() <= CONTENT_BOUNDS_TOLERANCE_PX
        && (bounds.height - viewport.height as f64).abs() <= CONTENT_BOUNDS_TOLERANCE_PX
}

pub(crate) fn validate_browser_page_count(
    expected_pages: usize,
    observed_pages: usize,
) -> Vec<BrowserFinding> {
    if expected_pages == observed_pages {
        Vec::new()
    } else {
        vec![BrowserFinding {
            surface: "print".to_string(),
            code: "wrong-page-count".to_string(),
            page: None,
            route: None,
            step: None,
            slide_id: None,
            title: None,
            element: None,
            boundary: None,
            element_bounds: None,
            boundary_bounds: None,
            intersection_bounds: None,
            deltas: None,
            evidence: None,
            message: format!(
                "expected {expected_pages} static pages but the executed document contains {observed_pages}"
            ),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chromium::{
        BrowserFontFace, BrowserGeometryViolation, BrowserPlatformFont,
        BrowserPlatformFontEvidence, BrowserPlatformFontProbe, BrowserRect,
        BrowserStepVisibilityViolation, BrowserTextStyle,
    };

    fn empty_page() -> BrowserPageObservation {
        BrowserPageObservation {
            document_ready_state: "complete".to_string(),
            autoscale_ready: true,
            slide_present: true,
            page: Some(1),
            slide_id: "section-1-main".to_string(),
            slide_bounds: Some(BrowserRect {
                width: 1280.0,
                height: 720.0,
                right: 1280.0,
                bottom: 720.0,
                ..Default::default()
            }),
            canvas_bounds: Some(Default::default()),
            content_bounds: Some(Default::default()),
            ..Default::default()
        }
    }

    fn screen_route() -> BrowserScreenRoute {
        BrowserScreenRoute {
            hash: "#/0/0/2".to_string(),
            section: 0,
            detail: 0,
            step: 2,
            step_count: 2,
            slide_id: "section-1-main".to_string(),
            role: "main".to_string(),
            generated: None,
        }
    }

    fn screen_observation() -> BrowserPageObservation {
        let mut observation = empty_page();
        let viewport = BrowserRect {
            width: 1280.0,
            height: 720.0,
            right: 1280.0,
            bottom: 720.0,
            ..Default::default()
        };
        observation.page = None;
        observation.role = "main".to_string();
        observation.content_union = Some(Default::default());
        observation.screen_bounds = Some(viewport);
        observation.root_bounds = Some(viewport);
        observation.stage_bounds = Some(viewport);
        observation.active_stack_bounds = Some(viewport);
        observation.active_stack_count = Some(1);
        observation.active_slide_count = Some(1);
        observation.visible_stack_count = Some(1);
        observation.visible_slide_count = Some(1);
        observation.routed_stack_visible = Some(true);
        observation.routed_slide_visible = Some(true);
        observation.screen_route = Some("#/0/0/2".to_string());
        observation.screen_section = Some(0);
        observation.screen_detail = Some(0);
        observation.screen_step = Some(2);
        observation.screen_step_count = Some(2);
        observation
    }

    #[test]
    fn matching_screen_route_passes_screen_geometry_validation() {
        let failures = validate_browser_screen_route(
            1,
            &screen_route(),
            &screen_observation(),
            PngViewport::default(),
        );

        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    }

    #[test]
    fn screen_document_scroll_is_an_objective_failure() {
        let mut observation = screen_observation();
        observation.document_scroll = Some(BrowserOverflowDeltas {
            bottom: 14.0,
            ..Default::default()
        });

        let failures =
            validate_browser_screen_route(1, &screen_route(), &observation, PngViewport::default());

        assert!(failures.iter().any(|failure| {
            failure.code == "screen-document-scroll"
                && failure.surface == "screen"
                && failure.route.as_deref() == Some("#/0/0/2")
                && failure.step == Some(2)
        }));
    }

    #[test]
    fn screen_geometry_violation_retains_the_offender_and_boundary() {
        let mut observation = screen_observation();
        let element_bounds = BrowserRect {
            y: 610.0,
            width: 900.0,
            height: 100.0,
            right: 900.0,
            bottom: 710.0,
            ..Default::default()
        };
        let boundary_bounds = BrowserRect {
            y: 650.0,
            width: 900.0,
            height: 40.0,
            right: 900.0,
            bottom: 690.0,
            ..Default::default()
        };
        observation
            .geometry_violations
            .push(BrowserGeometryViolation {
                kind: "caption-overlap".to_string(),
                element: "img.zpres-figure-image".to_string(),
                boundary: "figcaption".to_string(),
                element_bounds,
                boundary_bounds,
                intersection_bounds: None,
                deltas: BrowserOverflowDeltas {
                    bottom: 60.0,
                    ..Default::default()
                },
            });

        let failures =
            validate_browser_screen_route(1, &screen_route(), &observation, PngViewport::default());

        let failure = failures
            .iter()
            .find(|failure| failure.code == "caption-overlap")
            .expect("caption overlap finding");
        assert_eq!(
            failures
                .iter()
                .filter(|failure| failure.code == "caption-overlap")
                .count(),
            1,
            "screen geometry violations must not be duplicated"
        );
        assert_eq!(failure.element.as_deref(), Some("img.zpres-figure-image"));
        assert_eq!(failure.boundary.as_deref(), Some("figcaption"));
        assert_eq!(failure.element_bounds, Some(element_bounds));
        assert_eq!(failure.boundary_bounds, Some(boundary_bounds));
    }

    #[test]
    fn science_shaped_header_body_overlap_retains_exact_intersection_on_both_surfaces() {
        let header = BrowserRect {
            x: 120.0,
            y: 96.0,
            width: 760.0,
            height: 210.0,
            right: 880.0,
            bottom: 306.0,
        };
        let body = BrowserRect {
            x: 120.0,
            y: 264.0,
            width: 920.0,
            height: 300.0,
            right: 1040.0,
            bottom: 564.0,
        };
        let intersection = BrowserRect {
            x: 120.0,
            y: 264.0,
            width: 760.0,
            height: 42.0,
            right: 880.0,
            bottom: 306.0,
        };
        let violation = BrowserGeometryViolation {
            kind: "header-body-overlap".to_string(),
            element: "header.zpres-slide-header".to_string(),
            boundary: "div.zpres-slide-body".to_string(),
            element_bounds: header,
            boundary_bounds: body,
            intersection_bounds: Some(intersection),
            deltas: BrowserOverflowDeltas {
                right: 760.0,
                bottom: 42.0,
                ..Default::default()
            },
        };

        let mut print = empty_page();
        print.content_union = Some(Default::default());
        print.geometry_violations.push(violation.clone());
        let print_failure = validate_browser_page(1, &print, PngViewport::default())
            .into_iter()
            .find(|finding| finding.code == "header-body-overlap")
            .unwrap();

        let mut screen = screen_observation();
        screen.geometry_violations.push(violation);
        let screen_failure =
            validate_browser_screen_route(1, &screen_route(), &screen, PngViewport::default())
                .into_iter()
                .find(|finding| finding.code == "header-body-overlap")
                .unwrap();

        for failure in [print_failure, screen_failure] {
            assert_eq!(failure.element_bounds, Some(header));
            assert_eq!(failure.boundary_bounds, Some(body));
            assert_eq!(failure.intersection_bounds, Some(intersection));
            assert!(failure.message.contains("header.zpres-slide-header"));
            assert!(failure.message.contains("div.zpres-slide-body"));
            assert!(failure.message.contains("760.0x42.0"));
        }
    }

    #[test]
    fn screen_step_visibility_violation_retains_route_and_element() {
        let mut observation = screen_observation();
        observation
            .step_visibility_violations
            .push(BrowserStepVisibilityViolation {
                kind: "unexpected-visible-step".to_string(),
                element: "li.zpres-step[data-step-index=\"3\"]".to_string(),
                step_index: Some(3),
                expected_visible: false,
                visible: true,
            });

        let failures =
            validate_browser_screen_route(1, &screen_route(), &observation, PngViewport::default());

        let failure = failures
            .iter()
            .find(|failure| failure.code == "unexpected-visible-step")
            .expect("unexpected visible Step finding");
        assert_eq!(failure.surface, "screen");
        assert_eq!(failure.route.as_deref(), Some("#/0/0/2"));
        assert_eq!(
            failure.element.as_deref(),
            Some("li.zpres-step[data-step-index=\"3\"]")
        );
        assert!(failure.message.contains("should be hidden at Step 3"));
    }

    #[test]
    fn ordinary_background_only_page_is_an_objective_failure() {
        let failures = validate_browser_page(1, &empty_page(), PngViewport::default());
        assert!(
            failures
                .iter()
                .any(|finding| finding.code == "missing-visible-content")
        );
    }

    #[test]
    fn generated_background_splash_may_be_background_only() {
        let mut observation = empty_page();
        observation.generated = Some("background-image".to_string());
        let failures = validate_browser_page(1, &observation, PngViewport::default());
        assert!(
            !failures
                .iter()
                .any(|finding| finding.code == "missing-visible-content")
        );
    }

    #[test]
    fn generated_speaker_notes_page_must_have_visible_content() {
        let mut observation = empty_page();
        observation.generated = Some("speaker-notes".to_string());
        let failures = validate_browser_page(1, &observation, PngViewport::default());
        assert!(
            failures
                .iter()
                .any(|finding| finding.code == "missing-visible-content")
        );
    }

    #[test]
    fn platform_font_evidence_does_not_guess_that_a_loaded_face_was_required() {
        let mut observation = empty_page();
        observation.content_union = Some(Default::default());
        observation.font_faces.push(BrowserFontFace {
            family: "Talk Sans".to_string(),
            status: "loaded".to_string(),
            ..Default::default()
        });
        observation.text_styles.push(BrowserTextStyle {
            family: "Talk Sans, sans-serif".to_string(),
            count: 1,
            ..Default::default()
        });
        observation.platform_fonts.push(BrowserPlatformFont {
            family: "Arial".to_string(),
            postscript_name: "ArialMT".to_string(),
            custom: false,
            glyph_count: 11,
        });
        observation.platform_fonts.push(BrowserPlatformFont {
            family: "Symbol".to_string(),
            postscript_name: "Symbol".to_string(),
            custom: false,
            glyph_count: 1,
        });
        observation.platform_font_probe_count = 1;
        observation.platform_font_candidate_count = 1;
        observation
            .platform_font_probes
            .push(BrowserPlatformFontProbe {
                index: 0,
                element: "p".to_string(),
                text_excerpt: "Visible text".to_string(),
                requested_family: "Talk Sans, sans-serif".to_string(),
                requested_style: "normal".to_string(),
                requested_weight: "400".to_string(),
                requested_font_synthesis: "auto".to_string(),
                fonts: observation.platform_fonts.clone(),
                multiple_faces_observed: true,
                secondary_face_glyph_count: 1,
            });
        observation.platform_font_evidence = BrowserPlatformFontEvidence {
            actual_font_use: BrowserFontEvidenceStatus::Observed,
            multiple_faces_observed: BrowserFontEvidenceStatus::Observed,
            fallback_activation: BrowserFontEvidenceStatus::Unavailable,
            missing_glyph_cause: BrowserFontEvidenceStatus::Unavailable,
            synthesized_face_activation: BrowserFontEvidenceStatus::Unavailable,
            limitations: vec!["CDP cannot prove why fallback occurred".to_string()],
        };

        let failures = validate_browser_page(1, &observation, PngViewport::default());

        assert!(
            !failures.iter().any(|finding| finding.code == "failed-font"),
            "a loaded FontFace can be unused because of unicode-range, weight, style, or intentional fallback; current evidence does not prove an error"
        );
    }

    #[test]
    fn visible_text_without_actual_platform_font_evidence_fails() {
        let mut observation = empty_page();
        observation.content_union = Some(Default::default());
        observation.platform_font_candidate_count = 3;
        observation.platform_font_probe_count = 3;
        observation.platform_font_evidence.actual_font_use = BrowserFontEvidenceStatus::NotObserved;

        let failures = validate_browser_page(1, &observation, PngViewport::default());

        assert!(failures.iter().any(|finding| {
            finding.code == "missing-platform-font-evidence"
                && finding.message.contains("3 visible text candidate")
        }));
    }

    #[test]
    fn representative_font_probe_truncation_is_not_a_failure() {
        let mut observation = empty_page();
        observation.content_union = Some(Default::default());
        observation.platform_font_candidate_count = 80;
        observation.platform_font_probe_count = 24;
        observation.platform_font_probe_truncated = true;
        observation.platform_fonts.push(BrowserPlatformFont {
            family: "Arial".to_string(),
            postscript_name: "ArialMT".to_string(),
            custom: false,
            glyph_count: 80,
        });
        observation.platform_font_evidence.actual_font_use = BrowserFontEvidenceStatus::Observed;

        let failures = validate_browser_page(1, &observation, PngViewport::default());

        assert!(
            !failures
                .iter()
                .any(|finding| finding.code.contains("platform-font")),
            "sample truncation should remain report evidence: {failures:?}"
        );
    }

    #[test]
    fn font_face_error_remains_an_objective_failure() {
        let mut observation = empty_page();
        observation.content_union = Some(Default::default());
        observation.font_faces.push(BrowserFontFace {
            family: "Broken Sans".to_string(),
            status: "error".to_string(),
            ..Default::default()
        });

        let failures = validate_browser_page(1, &observation, PngViewport::default());

        assert!(failures.iter().any(|finding| {
            finding.code == "failed-font" && finding.message.contains("Broken Sans")
        }));
    }

    #[test]
    fn full_document_page_offset_matches_the_logical_viewport() {
        let mut observation = empty_page();
        observation.page = Some(2);
        observation.slide_id = "section-2-main".to_string();
        observation.content_union = Some(Default::default());
        observation.slide_bounds = Some(BrowserRect {
            y: 720.0,
            width: 1280.0,
            height: 720.0,
            right: 1280.0,
            bottom: 1440.0,
            ..Default::default()
        });

        let failures = validate_browser_page(2, &observation, PngViewport::default());

        assert!(
            !failures
                .iter()
                .any(|finding| finding.code == "slide-viewport-mismatch")
        );
    }

    #[test]
    fn later_page_at_document_origin_is_a_viewport_mismatch() {
        let mut observation = empty_page();
        observation.page = Some(2);
        observation.slide_id = "section-2-main".to_string();
        observation.content_union = Some(Default::default());

        let failures = validate_browser_page(2, &observation, PngViewport::default());

        assert!(
            failures
                .iter()
                .any(|finding| finding.code == "slide-viewport-mismatch")
        );
    }
}
