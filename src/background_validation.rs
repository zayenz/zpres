use serde::{Deserialize, Serialize};

use crate::browser_validation::BrowserFinding;
use crate::chromium::{BrowserPageObservation, BrowserScreenRoute};
#[cfg(test)]
use crate::deck::Deck;
use crate::deck::{
    BackgroundImageFit, BackgroundImageIntent, BackgroundImageSplitSide, DeckBackgroundImage,
};
use crate::html::StaticExportOptions;
use crate::presentation_plan::{
    PlannedBackground, PresentationBackgroundOrigin, PresentationBackgroundPhase, PresentationPlan,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthoredBackgroundOrigin {
    Deck,
    Slide,
    GeneratedSplash,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthoredBackgroundPhase {
    Title,
    Content,
    Splash,
}

impl AuthoredBackgroundPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Content => "content",
            Self::Splash => "splash",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AuthoredBackgroundExpectation {
    pub source: String,
    pub origin: AuthoredBackgroundOrigin,
    pub phase: AuthoredBackgroundPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_size: Option<String>,
    pub intent: String,
    pub short_alternative_present: bool,
    pub long_description_present: bool,
    pub focal_crop_review: String,
    pub image_backed_contrast_review_required: bool,
}

impl AuthoredBackgroundExpectation {
    fn painted(
        image: &DeckBackgroundImage,
        origin: AuthoredBackgroundOrigin,
        phase: AuthoredBackgroundPhase,
    ) -> Self {
        let split = image.split.as_ref().map(|split| match split.side {
            BackgroundImageSplitSide::Left => "left".to_string(),
            BackgroundImageSplitSide::Right => "right".to_string(),
        });
        let split_size = image.split.as_ref().map(|split| split.size.clone());
        Self {
            source: image.src.clone(),
            origin,
            phase,
            split,
            split_size,
            intent: background_intent_name(image.intent).to_string(),
            short_alternative_present: !image.alt.trim().is_empty(),
            long_description_present: image
                .description
                .as_deref()
                .is_some_and(|description| !description.trim().is_empty()),
            focal_crop_review: focal_crop_review(image).to_string(),
            image_backed_contrast_review_required: phase != AuthoredBackgroundPhase::Splash,
        }
    }

    fn splash(image: &DeckBackgroundImage) -> Self {
        Self {
            source: image.src.clone(),
            origin: AuthoredBackgroundOrigin::GeneratedSplash,
            phase: AuthoredBackgroundPhase::Splash,
            split: None,
            split_size: None,
            intent: background_intent_name(image.intent).to_string(),
            short_alternative_present: !image.alt.trim().is_empty(),
            long_description_present: image
                .description
                .as_deref()
                .is_some_and(|description| !description.trim().is_empty()),
            focal_crop_review: focal_crop_review(image).to_string(),
            image_backed_contrast_review_required: false,
        }
    }
}

fn background_intent_name(intent: BackgroundImageIntent) -> &'static str {
    match intent {
        BackgroundImageIntent::Decorative => "decorative",
        BackgroundImageIntent::Contextual => "contextual",
        BackgroundImageIntent::Evidence => "evidence",
    }
}

fn focal_crop_review(image: &DeckBackgroundImage) -> &'static str {
    if image.intent == BackgroundImageIntent::Decorative || image.fit == BackgroundImageFit::Contain
    {
        "not_applicable"
    } else {
        "manual_review_required"
    }
}

#[cfg(test)]
pub(crate) fn expected_screen_background(
    deck: &Deck,
    route: &BrowserScreenRoute,
) -> Option<AuthoredBackgroundExpectation> {
    let plan = PresentationPlan::for_theme_api_v1(deck);
    expected_screen_background_for_plan(&plan, route)
}

pub(crate) fn expected_screen_background_for_plan(
    plan: &PresentationPlan<'_>,
    route: &BrowserScreenRoute,
) -> Option<AuthoredBackgroundExpectation> {
    if route.generated.as_deref() == Some("background-image") {
        return plan
            .splash_background()
            .and_then(expectation_for_background);
    }
    plan.slide_for_id(&route.slide_id)
        .and_then(|slide| expectation_for_background(slide.background))
}

#[cfg(test)]
pub(crate) fn expected_print_backgrounds(
    deck: &Deck,
    options: StaticExportOptions,
) -> Vec<Option<AuthoredBackgroundExpectation>> {
    let plan = PresentationPlan::for_theme_api_v1(deck);
    expected_print_backgrounds_for_plan(&plan, options)
}

pub(crate) fn expected_print_backgrounds_for_plan(
    plan: &PresentationPlan<'_>,
    options: StaticExportOptions,
) -> Vec<Option<AuthoredBackgroundExpectation>> {
    plan.print_pages(options.include_speaker_notes)
        .into_iter()
        .map(|page| expectation_for_background(page.background()))
        .collect()
}

fn expectation_for_background(
    background: PlannedBackground<'_>,
) -> Option<AuthoredBackgroundExpectation> {
    let image = background.image?;
    match (background.origin?, background.phase) {
        (origin, PresentationBackgroundPhase::Title)
            if matches!(
                origin,
                PresentationBackgroundOrigin::Deck | PresentationBackgroundOrigin::Slide
            ) =>
        {
            Some(AuthoredBackgroundExpectation::painted(
                image,
                match origin {
                    PresentationBackgroundOrigin::Deck => AuthoredBackgroundOrigin::Deck,
                    PresentationBackgroundOrigin::Slide => AuthoredBackgroundOrigin::Slide,
                    PresentationBackgroundOrigin::GeneratedSplash => unreachable!(),
                },
                AuthoredBackgroundPhase::Title,
            ))
        }
        (PresentationBackgroundOrigin::Deck, PresentationBackgroundPhase::Content) => {
            Some(AuthoredBackgroundExpectation::painted(
                image,
                AuthoredBackgroundOrigin::Deck,
                AuthoredBackgroundPhase::Content,
            ))
        }
        (PresentationBackgroundOrigin::Slide, PresentationBackgroundPhase::Content) => {
            Some(AuthoredBackgroundExpectation::painted(
                image,
                AuthoredBackgroundOrigin::Slide,
                AuthoredBackgroundPhase::Content,
            ))
        }
        (PresentationBackgroundOrigin::GeneratedSplash, PresentationBackgroundPhase::Splash) => {
            Some(AuthoredBackgroundExpectation::splash(image))
        }
        _ => None,
    }
}

pub(crate) fn validate_authored_background(
    surface: &str,
    page: usize,
    route: Option<&BrowserScreenRoute>,
    expectation: Option<&AuthoredBackgroundExpectation>,
    enforce_absence: bool,
    observation: &BrowserPageObservation,
) -> Vec<BrowserFinding> {
    let finding = |code: &str, message: String| BrowserFinding {
        surface: surface.to_string(),
        code: code.to_string(),
        page: Some(page),
        route: route.map(|route| route.hash.clone()),
        step: route.map(|route| route.step),
        slide_id: (!observation.slide_id.is_empty()).then(|| observation.slide_id.clone()),
        title: (!observation.title.is_empty()).then(|| observation.title.clone()),
        element: Some(".zpres-slide-background".to_string()),
        boundary: None,
        element_bounds: None,
        boundary_bounds: None,
        intersection_bounds: None,
        deltas: None,
        evidence: None,
        message,
    };

    let Some(expectation) = expectation else {
        if !enforce_absence {
            return Vec::new();
        }
        return observation
            .authored_background
            .as_ref()
            .filter(|actual| actual.layer_count > 0)
            .map(|actual| {
                finding(
                    "unexpected-authored-background",
                    format!(
                        "the Theme API v1 Presentation plan requires no authored background on this page, but Chromium found {} direct semantic background layer(s) with source {:?} and phase {:?}",
                        actual.layer_count, actual.source, actual.phase,
                    ),
                )
            })
            .into_iter()
            .collect();
    };

    let Some(actual) = observation.authored_background.as_ref() else {
        return vec![finding(
            "missing-authored-background",
            format!(
                "the v1 Slide expects the {} background '{}', but Chromium returned no semantic background observation",
                expectation.phase.as_str(),
                expectation.source,
            ),
        )];
    };
    if actual.layer_count != 1 {
        return vec![finding(
            "missing-authored-background",
            format!(
                "the v1 Slide expects exactly one direct semantic layer for the {} background '{}', but found {}",
                expectation.phase.as_str(),
                expectation.source,
                actual.layer_count,
            ),
        )];
    }

    if actual.phase.as_deref() != Some(expectation.phase.as_str()) {
        return vec![finding(
            "discarded-authored-background",
            format!(
                "the authored background '{}' expected phase '{}', but the rendered Slide reported {:?}",
                expectation.source,
                expectation.phase.as_str(),
                actual.phase,
            ),
        )];
    }
    if actual.source.as_deref() != Some(expectation.source.as_str()) {
        return vec![finding(
            "discarded-authored-background",
            format!(
                "the semantic background layer should preserve authored source '{}', but reported {:?}",
                expectation.source, actual.source,
            ),
        )];
    }
    if actual.split != expectation.split {
        return vec![finding(
            "discarded-authored-background",
            format!(
                "the authored background '{}' expected split {:?}, but the rendered Slide reported {:?}",
                expectation.source, expectation.split, actual.split,
            ),
        )];
    }
    if !actual.visible || !actual.intersects_slide || !actual.source_preserved {
        let mut failure = finding(
            "discarded-authored-background",
            format!(
                "the authored background '{}' did not survive as a visible semantic layer (visible={}, intersects_slide={}, source_preserved={}, display={:?}, visibility={:?}, content_visibility={:?}, opacity={:?}, computed_image={:?})",
                expectation.source,
                actual.visible,
                actual.intersects_slide,
                actual.source_preserved,
                actual.display,
                actual.visibility,
                actual.content_visibility,
                actual.opacity,
                actual.computed_image,
            ),
        );
        failure.boundary = Some(".zpres-slide".to_string());
        failure.element_bounds = actual.bounds;
        failure.boundary_bounds = observation.slide_bounds;
        return vec![failure];
    }
    if actual.intent.as_deref() != Some(expectation.intent.as_str())
        || actual.short_alternative_present != expectation.short_alternative_present
        || actual.long_description_present != expectation.long_description_present
        || (expectation.intent == "decorative" && !actual.decorative_hidden)
        || (expectation.intent != "decorative" && actual.semantic_layer_count != 1)
    {
        return vec![finding(
            "discarded-background-semantics",
            format!(
                "background '{}' declared intent '{}' with short alternative={} and long description={}, but Chromium observed intent={:?}, semantic layers={}, short alternative={}, long description={}, decorative hidden={}",
                expectation.source,
                expectation.intent,
                expectation.short_alternative_present,
                expectation.long_description_present,
                actual.intent,
                actual.semantic_layer_count,
                actual.short_alternative_present,
                actual.long_description_present,
                actual.decorative_hidden,
            ),
        )];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chromium::BrowserAuthoredBackgroundObservation;
    use crate::deck::parse_source_text;

    fn expectation() -> AuthoredBackgroundExpectation {
        AuthoredBackgroundExpectation {
            source: "assets/evidence.svg".to_string(),
            origin: AuthoredBackgroundOrigin::Slide,
            phase: AuthoredBackgroundPhase::Content,
            split: Some("right".to_string()),
            split_size: Some("35%".to_string()),
            intent: "evidence".to_string(),
            short_alternative_present: true,
            long_description_present: true,
            focal_crop_review: "manual_review_required".to_string(),
            image_backed_contrast_review_required: true,
        }
    }

    fn observation() -> BrowserPageObservation {
        BrowserPageObservation {
            slide_id: "section-2-main".to_string(),
            title: "Evidence".to_string(),
            authored_background: Some(BrowserAuthoredBackgroundObservation {
                phase: Some("content".to_string()),
                split: Some("right".to_string()),
                layer_count: 1,
                visible: true,
                intersects_slide: true,
                source: Some("assets/evidence.svg".to_string()),
                declared_image: Some("url(\"assets/evidence.svg\")".to_string()),
                computed_image: Some("url(\"file:///tmp/assets/evidence.svg\")".to_string()),
                source_preserved: true,
                intent: Some("evidence".to_string()),
                semantic_layer_count: 1,
                short_alternative_present: true,
                long_description_present: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn missing_semantic_layer_fails_with_slide_identity() {
        let mut observation = observation();
        observation
            .authored_background
            .as_mut()
            .unwrap()
            .layer_count = 0;

        let findings = validate_authored_background(
            "print",
            2,
            None,
            Some(&expectation()),
            true,
            &observation,
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "missing-authored-background");
        assert_eq!(findings[0].slide_id.as_deref(), Some("section-2-main"));
    }

    #[test]
    fn hidden_or_css_cleared_layer_fails_as_discarded() {
        for mutate in ["hidden", "cleared", "outside", "wrong-source"] {
            let mut observation = observation();
            let actual = observation.authored_background.as_mut().unwrap();
            match mutate {
                "hidden" => {
                    actual.visible = false;
                    actual.display = Some("none".to_string());
                }
                "cleared" => {
                    actual.source_preserved = false;
                    actual.computed_image = Some("none".to_string());
                }
                "outside" => actual.intersects_slide = false,
                "wrong-source" => actual.source = Some("assets/other.svg".to_string()),
                _ => unreachable!(),
            }

            let findings = validate_authored_background(
                "screen",
                3,
                None,
                Some(&expectation()),
                true,
                &observation,
            );

            assert_eq!(findings.len(), 1, "case {mutate}");
            assert_eq!(
                findings[0].code, "discarded-authored-background",
                "case {mutate}"
            );
        }
    }

    #[test]
    fn rejects_an_unexpected_layer_when_enforced() {
        assert!(
            validate_authored_background(
                "screen",
                1,
                None,
                Some(&expectation()),
                true,
                &observation(),
            )
            .is_empty()
        );
        assert!(
            validate_authored_background("screen", 1, None, None, false, &observation()).is_empty()
        );
        let findings = validate_authored_background("screen", 1, None, None, true, &observation());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "unexpected-authored-background");
    }

    #[test]
    fn model_expectations_cover_clean_title_splash_detail_slide_override_and_notes() {
        let deck = parse_source_text(
            r#"---
background_image:
  src: assets/deck.svg
  splash: true
---

# Clean title

--

## Detail

::: notes
Private notes.
:::

---

::: background src="assets/slide.svg"
:::

# Local background
"#,
            None,
        )
        .unwrap();
        let route = |slide_id: &str, role: &str, generated: Option<&str>| BrowserScreenRoute {
            hash: "#/test".to_string(),
            section: 0,
            detail: 0,
            step: 0,
            step_count: 0,
            slide_id: slide_id.to_string(),
            role: role.to_string(),
            generated: generated.map(str::to_string),
        };

        assert!(
            expected_screen_background(&deck, &route("section-1-main", "main", None),).is_none()
        );
        assert_eq!(
            expected_screen_background(&deck, &route("section-1-detail-1", "detail", None),)
                .unwrap()
                .origin,
            AuthoredBackgroundOrigin::Deck
        );
        assert_eq!(
            expected_screen_background(
                &deck,
                &route(
                    "background-image-splash",
                    "generated",
                    Some("background-image")
                ),
            )
            .unwrap()
            .phase,
            AuthoredBackgroundPhase::Splash
        );
        assert_eq!(
            expected_screen_background(&deck, &route("section-2-main", "main", None),)
                .unwrap()
                .origin,
            AuthoredBackgroundOrigin::Slide
        );

        let expectations = expected_print_backgrounds(
            &deck,
            StaticExportOptions {
                include_speaker_notes: true,
            },
        );
        assert_eq!(expectations.len(), 5);
        assert!(expectations[0].is_none());
        assert_eq!(
            expectations[1].as_ref().unwrap().origin,
            AuthoredBackgroundOrigin::Deck
        );
        assert!(expectations[2].is_none());
        assert_eq!(
            expectations[3].as_ref().unwrap().origin,
            AuthoredBackgroundOrigin::GeneratedSplash
        );
        assert_eq!(
            expectations[4].as_ref().unwrap().origin,
            AuthoredBackgroundOrigin::Slide
        );
    }
}
