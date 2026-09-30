use crate::deck::{
    self, BackgroundTitleApplication, ContentBlock, Deck, DeckBackgroundImage, PdfStepState,
    Section, Slide, StepPdfPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentationBackgroundOrigin {
    Deck,
    Slide,
    GeneratedSplash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentationBackgroundPhase {
    Title,
    Content,
    Splash,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlannedBackground<'a> {
    pub(crate) image: Option<&'a DeckBackgroundImage>,
    pub(crate) origin: Option<PresentationBackgroundOrigin>,
    pub(crate) phase: PresentationBackgroundPhase,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlannedSlide<'a> {
    pub(crate) section_index: usize,
    pub(crate) detail_index: Option<usize>,
    pub(crate) section_title: Option<&'a str>,
    pub(crate) slide: &'a Slide,
    pub(crate) logical_slide_number: usize,
    pub(crate) background: PlannedBackground<'a>,
}

#[derive(Debug)]
pub(crate) struct PlannedSection<'a> {
    pub(crate) section: &'a Section,
    pub(crate) slides: Vec<PlannedSlide<'a>>,
}

#[derive(Debug)]
pub(crate) enum PresentationGroup<'a> {
    Section(PlannedSection<'a>),
    Splash {
        section_index: usize,
        background: PlannedBackground<'a>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PlannedPrintPage<'a> {
    Slide {
        planned: PlannedSlide<'a>,
        step_state: PdfStepState,
    },
    SpeakerNotes {
        planned: PlannedSlide<'a>,
    },
    Splash {
        background: PlannedBackground<'a>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedScreenState<'a> {
    pub(crate) section: usize,
    pub(crate) detail: usize,
    pub(crate) step: usize,
    pub(crate) step_count: usize,
    pub(crate) slide_id: &'a str,
    pub(crate) role: &'static str,
    pub(crate) generated: Option<&'static str>,
}

impl PlannedScreenState<'_> {
    pub(crate) fn route_hash(&self) -> String {
        let step = if self.step > 0 {
            format!("/{}", self.step)
        } else {
            String::new()
        };
        format!("#/{}/{}{}", self.section, self.detail, step)
    }
}

impl<'a> PlannedPrintPage<'a> {
    pub(crate) fn background(self) -> PlannedBackground<'a> {
        match self {
            Self::Slide { planned, .. } => planned.background,
            Self::SpeakerNotes { .. } => PlannedBackground {
                image: None,
                origin: None,
                phase: PresentationBackgroundPhase::Content,
            },
            Self::Splash { background } => background,
        }
    }

    pub(crate) fn slide_id(self) -> &'a str {
        match self {
            Self::Slide { planned, .. } | Self::SpeakerNotes { planned } => &planned.slide.id,
            Self::Splash { .. } => "background-image-splash",
        }
    }

    pub(crate) fn role(self) -> &'static str {
        match self {
            Self::Slide { planned, .. } | Self::SpeakerNotes { planned } => {
                slide_role_name(planned.slide)
            }
            Self::Splash { .. } => "generated",
        }
    }

    pub(crate) fn generated(self) -> Option<&'static str> {
        match self {
            Self::Slide { .. } => None,
            Self::SpeakerNotes { .. } => Some("speaker-notes"),
            Self::Splash { .. } => Some("background-image"),
        }
    }

    pub(crate) fn step_state(self) -> Option<PdfStepState> {
        match self {
            Self::Slide { step_state, .. } => Some(step_state),
            Self::SpeakerNotes { .. } | Self::Splash { .. } => None,
        }
    }

    pub(crate) fn pdf_step_attributes(self) -> (Option<&'static str>, Option<usize>) {
        match self.step_state() {
            Some(PdfStepState::Final) => (Some("final"), None),
            Some(PdfStepState::UpTo { step }) => (Some("up-to"), Some(step)),
            None => (None, None),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PresentationPlan<'a> {
    groups: Vec<PresentationGroup<'a>>,
    logical_slide_count: usize,
    deck_background: Option<&'a DeckBackgroundImage>,
}

impl<'a> PresentationPlan<'a> {
    pub(crate) fn for_theme_api_v1(deck: &'a Deck) -> Self {
        let mut groups = Vec::new();
        let mut logical_slide_number = 1usize;
        let deck_background = deck.metadata.background_image.as_ref();

        for (section_offset, section) in deck.sections.iter().enumerate() {
            let mut slides = Vec::with_capacity(1 + section.detail_slides.len());
            slides.push(planned_slide(
                section,
                &section.main_slide,
                None,
                section_offset == 0,
                deck_background,
                logical_slide_number,
            ));
            logical_slide_number += 1;
            for (detail_offset, slide) in section.detail_slides.iter().enumerate() {
                slides.push(planned_slide(
                    section,
                    slide,
                    Some(detail_offset + 1),
                    false,
                    deck_background,
                    logical_slide_number,
                ));
                logical_slide_number += 1;
            }
            groups.push(PresentationGroup::Section(PlannedSection {
                section,
                slides,
            }));

            if section_offset == 0
                && let Some(image) =
                    deck_background.filter(|image| image.splash_explicit && image.splash)
            {
                groups.push(PresentationGroup::Splash {
                    section_index: section.index + 1,
                    background: PlannedBackground {
                        image: Some(image),
                        origin: Some(PresentationBackgroundOrigin::GeneratedSplash),
                        phase: PresentationBackgroundPhase::Splash,
                    },
                });
            }
        }

        Self {
            groups,
            logical_slide_count: logical_slide_number.saturating_sub(1),
            deck_background,
        }
    }

    pub(crate) fn groups(&self) -> &[PresentationGroup<'a>] {
        &self.groups
    }

    pub(crate) fn logical_slide_count(&self) -> usize {
        self.logical_slide_count
    }

    pub(crate) fn screen_slide_count(&self) -> usize {
        self.groups
            .iter()
            .map(|group| match group {
                PresentationGroup::Section(section) => section.slides.len(),
                PresentationGroup::Splash { .. } => 1,
            })
            .sum()
    }

    pub(crate) fn screen_state_count(&self) -> usize {
        self.screen_states().len()
    }

    pub(crate) fn screen_states(&self) -> Vec<PlannedScreenState<'a>> {
        let mut states = Vec::new();
        for (screen_section, group) in self.groups.iter().enumerate() {
            match group {
                PresentationGroup::Section(section) => {
                    for planned in &section.slides {
                        let step_count = deck::slide_screen_step_count(planned.slide);
                        for step in 0..=step_count {
                            states.push(PlannedScreenState {
                                section: screen_section,
                                detail: planned.detail_index.unwrap_or(0),
                                step,
                                step_count,
                                slide_id: &planned.slide.id,
                                role: slide_role_name(planned.slide),
                                generated: None,
                            });
                        }
                    }
                }
                PresentationGroup::Splash { .. } => states.push(PlannedScreenState {
                    section: screen_section,
                    detail: 0,
                    step: 0,
                    step_count: 0,
                    slide_id: "background-image-splash",
                    role: "generated",
                    generated: Some("background-image"),
                }),
            }
        }
        states
    }

    pub(crate) fn print_pages(&self, include_speaker_notes: bool) -> Vec<PlannedPrintPage<'a>> {
        let mut pages = Vec::new();
        for group in &self.groups {
            match group {
                PresentationGroup::Section(section) => {
                    for planned in &section.slides {
                        match deck::slide_step_pdf_policy(planned.slide) {
                            Some(StepPdfPolicy::OnePagePerStep) => {
                                for step in 1..=deck::slide_step_count(planned.slide) {
                                    pages.push(PlannedPrintPage::Slide {
                                        planned: *planned,
                                        step_state: PdfStepState::UpTo { step },
                                    });
                                }
                            }
                            Some(StepPdfPolicy::FinalState) | None => {
                                pages.push(PlannedPrintPage::Slide {
                                    planned: *planned,
                                    step_state: PdfStepState::Final,
                                });
                            }
                        }
                        if include_speaker_notes && has_speaker_notes(planned.slide) {
                            pages.push(PlannedPrintPage::SpeakerNotes { planned: *planned });
                        }
                    }
                }
                PresentationGroup::Splash { background, .. } => {
                    pages.push(PlannedPrintPage::Splash {
                        background: *background,
                    })
                }
            }
        }
        pages
    }

    pub(crate) fn slide_for_id(&self, slide_id: &str) -> Option<PlannedSlide<'a>> {
        self.groups.iter().find_map(|group| {
            let PresentationGroup::Section(section) = group else {
                return None;
            };
            section
                .slides
                .iter()
                .copied()
                .find(|planned| planned.slide.id == slide_id)
        })
    }

    pub(crate) fn authored_slides(&self) -> Vec<PlannedSlide<'a>> {
        self.groups
            .iter()
            .filter_map(|group| match group {
                PresentationGroup::Section(section) => Some(section.slides.iter().copied()),
                PresentationGroup::Splash { .. } => None,
            })
            .flatten()
            .collect()
    }

    pub(crate) fn declared_backgrounds(&self) -> Vec<(&'a str, &'a DeckBackgroundImage)> {
        let mut backgrounds = Vec::new();
        if let Some(background) = self.deck_background {
            backgrounds.push(("deck", background));
        }
        for slide in self.authored_slides() {
            if let Some(background) = slide.slide.background_image.as_ref() {
                backgrounds.push((slide.slide.id.as_str(), background));
            }
        }
        backgrounds
    }

    pub(crate) fn splash_background(&self) -> Option<PlannedBackground<'a>> {
        self.groups.iter().find_map(|group| match group {
            PresentationGroup::Splash { background, .. } => Some(*background),
            PresentationGroup::Section(_) => None,
        })
    }

    pub(crate) fn deck_background_is_dormant(&self) -> bool {
        self.deck_background.is_some()
            && !self.groups.iter().any(|group| match group {
                PresentationGroup::Section(section) => section.slides.iter().any(|slide| {
                    slide.background.origin == Some(PresentationBackgroundOrigin::Deck)
                }),
                PresentationGroup::Splash { background, .. } => {
                    background.origin == Some(PresentationBackgroundOrigin::GeneratedSplash)
                }
            })
    }
}

fn slide_role_name(slide: &Slide) -> &'static str {
    match slide.role {
        crate::deck::SlideRole::Main => "main",
        crate::deck::SlideRole::Detail => "detail",
    }
}

fn planned_slide<'a>(
    section: &'a Section,
    slide: &'a Slide,
    detail_index: Option<usize>,
    is_title: bool,
    deck_background: Option<&'a DeckBackgroundImage>,
    logical_slide_number: usize,
) -> PlannedSlide<'a> {
    let background = if let Some(image) = slide.background_image.as_ref() {
        PlannedBackground {
            image: Some(image),
            origin: Some(PresentationBackgroundOrigin::Slide),
            phase: if is_title {
                PresentationBackgroundPhase::Title
            } else {
                PresentationBackgroundPhase::Content
            },
        }
    } else if is_title {
        match deck_background {
            Some(image) if image.title_application == BackgroundTitleApplication::Paint => {
                PlannedBackground {
                    image: Some(image),
                    origin: Some(PresentationBackgroundOrigin::Deck),
                    phase: PresentationBackgroundPhase::Title,
                }
            }
            _ => PlannedBackground {
                image: None,
                origin: None,
                phase: PresentationBackgroundPhase::Title,
            },
        }
    } else {
        PlannedBackground {
            image: deck_background,
            origin: deck_background.map(|_| PresentationBackgroundOrigin::Deck),
            phase: PresentationBackgroundPhase::Content,
        }
    };
    PlannedSlide {
        section_index: section.index,
        detail_index,
        section_title: section.main_slide.title.as_deref(),
        slide,
        logical_slide_number,
        background,
    }
}

pub(crate) fn has_speaker_notes(slide: &Slide) -> bool {
    slide.blocks.iter().any(|block| {
        matches!(block, ContentBlock::SpeakerNotes { markdown } if !markdown.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::parse_source_text;

    #[test]
    fn v1_plan_keeps_slide_states_together_and_places_splash_after_the_first_section() {
        let deck = parse_source_text(
            r#"---
background_image:
  src: assets/deck.svg
  title: clean
  splash: true
---

# Clean title

::: steps pdf="pages"
1. First title state.
2. Second title state.
:::

::: notes
Title notes.
:::

--

::: background src="assets/detail.svg"
:::

## Detail

::: steps pdf="pages"
1. First detail state.
2. Second detail state.
:::

::: notes
Detail notes.
:::

---

# Later Main
"#,
            None,
        )
        .unwrap();
        let plan = PresentationPlan::for_theme_api_v1(&deck);

        assert_eq!(plan.logical_slide_count(), 3);
        assert_eq!(plan.screen_slide_count(), 4);
        assert_eq!(plan.screen_state_count(), 8);
        assert_eq!(
            plan.screen_states()
                .iter()
                .map(PlannedScreenState::route_hash)
                .collect::<Vec<_>>(),
            [
                "#/0/0", "#/0/0/1", "#/0/0/2", "#/0/1", "#/0/1/1", "#/0/1/2", "#/1/0", "#/2/0",
            ]
        );

        let pages = plan.print_pages(true);
        assert_eq!(pages.len(), 8);
        let identity = pages
            .iter()
            .map(|page| match page {
                PlannedPrintPage::Slide {
                    planned,
                    step_state,
                } => format!("{}:{step_state:?}", planned.slide.id),
                PlannedPrintPage::SpeakerNotes { planned } => {
                    format!("{}:notes", planned.slide.id)
                }
                PlannedPrintPage::Splash { .. } => "splash".to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            identity,
            [
                "section-1-main:UpTo { step: 1 }",
                "section-1-main:UpTo { step: 2 }",
                "section-1-main:notes",
                "section-1-detail-1:UpTo { step: 1 }",
                "section-1-detail-1:UpTo { step: 2 }",
                "section-1-detail-1:notes",
                "splash",
                "section-2-main:Final",
            ]
        );
        assert_eq!(plan.print_pages(false).len(), 6);

        assert_eq!(
            plan.slide_for_id("section-1-main")
                .unwrap()
                .background
                .origin,
            None
        );
        assert_eq!(
            plan.slide_for_id("section-1-detail-1")
                .unwrap()
                .background
                .origin,
            Some(PresentationBackgroundOrigin::Slide)
        );
        assert_eq!(
            plan.splash_background().unwrap().origin,
            Some(PresentationBackgroundOrigin::GeneratedSplash)
        );
        assert_eq!(
            plan.slide_for_id("section-2-main")
                .unwrap()
                .background
                .origin,
            Some(PresentationBackgroundOrigin::Deck)
        );
    }

    #[test]
    fn v1_title_policy_and_slide_local_precedence_are_explicit() {
        for (title, local, expected) in [
            ("clean", false, None),
            ("paint", false, Some(PresentationBackgroundOrigin::Deck)),
            ("clean", true, Some(PresentationBackgroundOrigin::Slide)),
            ("paint", true, Some(PresentationBackgroundOrigin::Slide)),
        ] {
            let local = if local {
                "::: background src=\"assets/local.svg\"\n:::\n\n"
            } else {
                ""
            };
            let source = format!(
                "---\nbackground_image:\n  src: assets/deck.svg\n  title: {title}\n---\n\n{local}# Title\n"
            );
            let deck = parse_source_text(&source, None).unwrap();
            let plan = PresentationPlan::for_theme_api_v1(&deck);
            let title_slide = plan.slide_for_id("section-1-main").unwrap();
            assert_eq!(
                title_slide.background.origin,
                expected,
                "title={title}, local={}",
                !local.is_empty()
            );
            assert_eq!(
                title_slide.background.phase,
                PresentationBackgroundPhase::Title
            );
        }
    }

    #[test]
    fn v1_splash_is_explicit_and_a_clean_title_only_background_is_dormant() {
        for (splash_line, has_splash) in [
            ("", false),
            ("  splash: false\n", false),
            ("  splash: true\n", true),
        ] {
            let source = format!(
                "---\nbackground_image:\n  src: assets/deck.svg\n{splash_line}---\n\n# Title\n"
            );
            let deck = parse_source_text(&source, None).unwrap();
            let plan = PresentationPlan::for_theme_api_v1(&deck);
            assert_eq!(plan.splash_background().is_some(), has_splash);
            assert_eq!(plan.deck_background_is_dormant(), !has_splash);
        }
    }

    #[test]
    fn v1_screen_states_include_fragmented_lists_without_expanding_static_pages() {
        let deck = parse_source_text(
            r#"---
build_lists: true
---

# Fragmented list

- First reveal.
- Second reveal.
"#,
            None,
        )
        .unwrap();
        let plan = PresentationPlan::for_theme_api_v1(&deck);

        assert_eq!(plan.screen_state_count(), 3);
        assert_eq!(plan.print_pages(false).len(), 1);
    }
}
