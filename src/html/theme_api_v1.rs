use std::fs;
use std::path::Path;

use crate::deck::{ContentBlock, Deck, PdfStepState, Slide, SlideRole, slide_screen_step_count};
use crate::presentation_plan::{
    PlannedBackground, PlannedPrintPage, PlannedSlide, PresentationBackgroundPhase,
    PresentationGroup, PresentationPlan,
};
use crate::theme::RenderedTheme;

use super::{
    AUTOSCALE_JS, AssetRenderMode, BackgroundPhase, HtmlError, RenderContext, STATIC_READINESS_JS,
    SlideNumberContext, StaticExportOptions, background_phase_attr, background_split_attr,
    copy_local_assets, copy_theme_dependencies, escape_attr, escape_html, pdf_step_state_attr,
    render_block, render_slide_footer, render_speaker_notes_body, render_speaker_notes_panel,
    render_v1_background_layer, slide_autoscale_attr, slide_classes_attr,
    slide_footer_control_attrs, slide_preset_attr, slide_section_classes, slide_theme_attrs,
    slide_theme_style_attr, slide_theme_style_attr_with_extra, slide_transition_attr,
    slide_variant_attr, write_file,
};

pub(super) const FOUNDATION_ASSET_PATH: &str = "assets/zpres-theme-api-v1.css";
pub(super) const FOUNDATION_CSS: &str = include_str!("../generated/theme-api-v1.css");

const DEBUG_OVERLAY_JS: &str = r#"(() => {
  const params = new URLSearchParams(window.location.search);
  if (params.get("zpres-debug") !== "1") return;
  const regions = [
    [".zpres-slide-frame", "frame"], [".zpres-slide-header", "header"],
    [".zpres-slide-body", "body"], [".zpres-slide-primary", "primary"],
    [".zpres-slide-supporting, [data-comparison-role='supporting']", "supporting"], [".zpres-slide-sources", "sources"],
    [".zpres-slide-footer", "footer"], [".zpres-slide-ornament", "ornament"],
  ];
  const titleCase = (value) => value ? value[0].toUpperCase() + value.slice(1) : "Unknown";
  const diagnosticStatus = (slide) => {
    const content = slide.querySelector(".zpres-slide-content");
    const frame = slide.querySelector(".zpres-slide-frame")?.getBoundingClientRect();
    const measuredOverflow = content && [...content.querySelectorAll("*")].some((element) => {
      if (!(element instanceof HTMLElement) || !element.getClientRects().length) return false;
      const style = getComputedStyle(element);
      const clipsX = ["auto", "scroll", "hidden", "clip"].includes(style.overflowX);
      const clipsY = ["auto", "scroll", "hidden", "clip"].includes(style.overflowY);
      return (clipsX && element.scrollWidth - element.clientWidth > 2)
        || (clipsY && element.scrollHeight - element.clientHeight > 2);
    });
    const textOverflow = content && frame && (() => {
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (!node.textContent?.trim() || node.parentElement.closest(".zpres-debug-boundary-label")
          || getComputedStyle(node.parentElement).visibility === "hidden") continue;
        const range = document.createRange();
        range.selectNodeContents(node);
        for (const rect of range.getClientRects()) {
          if (rect.left < frame.left - 2 || rect.top < frame.top - 2
            || rect.right > frame.right + 2 || rect.bottom > frame.bottom + 2) return true;
        }
      }
      return false;
    })();
    if (slide.dataset.zpresOverflow === "clipped" || measuredOverflow || textOverflow) return ["FAILED", "overflow remains after autoscale"];
    const factor = Number(slide.dataset.autoscaleFactor || "1");
    if (Number.isFinite(factor) && factor < 1) return ["WARNING", `autoscale ${Math.round(factor * 100)}%`];
    return ["PASS", "no detected overflow"];
  };
  const stepStateLabel = (state) => ({
    future: "QUEUED",
    active: "CURRENT",
    complete: "COMPLETED",
  })[state] || "STATIC";
  const labelPlacements = [
    "outside-top-start", "outside-top-end", "outside-bottom-start", "outside-bottom-end",
    "inside-top-start", "inside-top-end", "inside-bottom-start", "inside-bottom-end",
  ];
  const intersectionArea = (first, second) => {
    const width = Math.max(0, Math.min(first.right, second.right) - Math.max(first.left, second.left));
    const height = Math.max(0, Math.min(first.bottom, second.bottom) - Math.max(first.top, second.top));
    return width > 1 && height > 1 ? width * height : 0;
  };
  const outsideArea = (bounds, boundary) => bounds.width * bounds.height
    - Math.max(0, Math.min(bounds.right, boundary.right) - Math.max(bounds.left, boundary.left))
      * Math.max(0, Math.min(bounds.bottom, boundary.bottom) - Math.max(bounds.top, boundary.top));
  const authoredTextRects = (slide) => {
    const rects = [];
    const walker = document.createTreeWalker(slide, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!node.textContent?.trim() || !parent
        || parent.closest(".zpres-debug-boundary-label, .zpres-slide-meta, .zpres-block-label, script, style")) continue;
      const style = getComputedStyle(parent);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity || 1) === 0) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      for (const rect of range.getClientRects()) {
        if (rect.width > 0 && rect.height > 0) rects.push(rect);
      }
    }
    return rects;
  };
  const candidateBounds = (placement, region, label) => {
    const end = placement.endsWith("end");
    const bottom = placement.includes("bottom");
    const outside = placement.startsWith("outside");
    const left = end ? region.right - label.width : region.left;
    const top = bottom
      ? region.bottom + (outside ? 0 : -label.height)
      : region.top - (outside ? label.height : 0);
    return { left, top, right: left + label.width, bottom: top + label.height, width: label.width, height: label.height };
  };
  const placeBoundaryLabel = (slide, region, role, bounds, slideBounds, textRects, placedLabels) => {
    let label = slide.querySelector(`:scope > .zpres-debug-boundary-label[data-zpres-debug-label-role="${role}"]`);
    if (!label) {
      label = document.createElement("span");
      label.className = "zpres-debug-boundary-label";
      label.setAttribute("aria-hidden", "true");
      label.dataset.zpresDebugLabelRole = role;
      slide.append(label);
    }
    label.textContent = `${role} · ${bounds}`;
    label.style.left = "0";
    label.style.top = "0";
    const measuredLabel = label.getBoundingClientRect();
    const scaleX = slideBounds.width / slide.offsetWidth;
    const scaleY = slideBounds.height / slide.offsetHeight;
    const regionBounds = region.getBoundingClientRect();
    let best = null;
    for (const [preference, placement] of labelPlacements.entries()) {
      const labelBounds = candidateBounds(placement, regionBounds, measuredLabel);
      const textOverlap = textRects.reduce((sum, textBounds) => sum + intersectionArea(labelBounds, textBounds), 0);
      const labelOverlap = placedLabels.reduce((sum, placed) => sum + intersectionArea(labelBounds, placed), 0);
      const score = [textOverlap > 0 ? 1 : 0, textOverlap, outsideArea(labelBounds, slideBounds), labelOverlap, preference];
      if (!best || score.some((value, index) => value < best.score[index]
        && score.slice(0, index).every((prior, priorIndex) => prior === best.score[priorIndex]))) {
        best = { placement, score };
      }
    }
    label.dataset.zpresDebugLabelPlacement = best.placement;
    const finalBounds = candidateBounds(best.placement, regionBounds, measuredLabel);
    label.style.left = `${(finalBounds.left - slideBounds.left) / scaleX}px`;
    label.style.top = `${(finalBounds.top - slideBounds.top) / scaleY}px`;
    placedLabels.push(label.getBoundingClientRect());
  };
  const measure = () => {
    if (document.body.dataset.zpresAutoscaleReady !== "true") return;
    document.querySelectorAll(".zpres-slide").forEach((slide) => {
    const canvas = slide.querySelector(":scope > .zpres-slide-frame");
    if (!canvas) return;
    const origin = canvas.getBoundingClientRect();
    const slideBounds = slide.getBoundingClientRect();
    const textRects = authoredTextRects(slide);
    const placedLabels = [];
    for (const [selector, role] of regions) {
      const candidates = [...slide.querySelectorAll(selector)];
      for (const candidate of candidates) {
        delete candidate.dataset.zpresDebugRegion;
        delete candidate.dataset.zpresDebugBounds;
      }
      const region = candidates.find((candidate) => {
        const rect = candidate.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      });
      if (!region) continue;
      const rect = region.getBoundingClientRect();
      region.dataset.zpresDebugRegion = role;
      region.dataset.zpresDebugBounds = `${Math.round(rect.left - origin.left)},${Math.round(rect.top - origin.top)} ${Math.round(rect.width)}×${Math.round(rect.height)}`;
      placeBoundaryLabel(slide, region, role, region.dataset.zpresDebugBounds, slideBounds, textRects, placedLabels);
    }
    const target = document.body.dataset.zpresOutputTarget === "print" ? "PDF export" : "HTML presentation";
    const section = Number(slide.dataset.sectionIndex || 1);
    const role = titleCase(slide.dataset.slideRole);
    const progress = slide.querySelector("[data-zpres-step-progress]")?.textContent?.trim() || "Step static";
    const steps = [...slide.querySelectorAll("[data-step-index]")];
    const states = { future: 0, active: 0, complete: 0 };
    for (const step of steps) {
      const state = step.dataset.stepState || step.dataset.zpresPrintStepState
        || (step.classList.contains("is-visible") ? "complete" : "future");
      if (state in states) states[state] += 1;
      step.dataset.zpresDebugStateLabel = stepStateLabel(state);
    }
    const [status, diagnostic] = diagnosticStatus(slide);
    const stepBlock = slide.querySelector("[data-step-pdf-policy]");
    const policy = stepBlock?.dataset.stepPdfPolicy || "final-state";
    const pageState = slide.dataset.pdfStepState === "up-to"
      ? `through Step ${Number(slide.dataset.pdfStep || 0)}`
      : slide.dataset.pdfStepState === "final" ? "final coherent state" : progress;
    const route = target === "PDF export"
      ? `Page ${slide.dataset.page || "?"}`
      : (window.location.hash || `#/section-${section}`);
    slide.dataset.zpresDebugIdentity = `Slide ${slide.dataset.slideId || "unknown"}`;
    slide.dataset.zpresDebugRole = `Section ${section} · ${role}`;
    slide.dataset.zpresDebugRoute = `${route} · ${progress}`;
    slide.dataset.zpresDebugTarget = target;
    slide.dataset.zpresDebugStepSummary = steps.length
      ? `COMPLETED ${states.complete}  CURRENT ${states.active}  QUEUED ${states.future}`
      : "STEP STATIC";
    if (stepBlock) stepBlock.dataset.zpresDebugStepSummary = slide.dataset.zpresDebugStepSummary;
    slide.dataset.zpresDebugStatus = status;
    slide.dataset.zpresDebugDiagnostics = `${status} · ${diagnostic}`;
    slide.dataset.zpresDebugPrintRail = `PDF PAGE ${slide.dataset.page || "?"} · ${policy.replaceAll("-", " ")} · ${pageState} · ${status} · ${diagnostic}`;
    });
  };
  document.body.dataset.zpresDebug = "enabled";
  measure();
  window.zpresRefreshDebugInspection = measure;
  window.addEventListener("resize", measure, { passive: true });
  window.addEventListener("hashchange", measure, { passive: true });
  new MutationObserver(measure).observe(document.body, {
    attributes: true,
    subtree: true,
    attributeFilter: ["class", "data-step-state", "data-autoscale-factor", "data-zpres-overflow", "data-zpres-autoscale-ready"],
  });
  document.fonts?.ready.then(measure);
})();"#;

const STAGE_FIT_JS: &str = r#"(function () {
  const stage = document.querySelector(".slides");
  const viewport = document.querySelector(".reveal");
  if (!stage || !viewport) return;

  let pendingFrame = null;
  function fitStage() {
    pendingFrame = null;
    const logicalWidth = stage.offsetWidth;
    const logicalHeight = stage.offsetHeight;
    if (!logicalWidth || !logicalHeight) return;
    const scale = Math.min(
      viewport.clientWidth / logicalWidth,
      viewport.clientHeight / logicalHeight,
    );
    const boundedScale = Math.max(scale, 0.01);
    const offsetX = (viewport.clientWidth - logicalWidth * boundedScale) / 2;
    const offsetY = (viewport.clientHeight - logicalHeight * boundedScale) / 2;
    stage.style.setProperty("--zpres-stage-scale", String(boundedScale));
    stage.style.setProperty("--zpres-stage-x", `${offsetX}px`);
    stage.style.setProperty("--zpres-stage-y", `${offsetY}px`);
  }

  function scheduleStageFit() {
    if (pendingFrame !== null) cancelAnimationFrame(pendingFrame);
    pendingFrame = requestAnimationFrame(fitStage);
  }

  fitStage();
  window.addEventListener("resize", scheduleStageFit);
  window.visualViewport?.addEventListener("resize", scheduleStageFit);
  document.fonts?.ready.then(scheduleStageFit);
  window.zpresFitStage = fitStage;
})();
"#;

pub(super) fn populate_html_bundle(
    deck: &Deck,
    theme: &RenderedTheme,
    output_dir: &Path,
    options: super::LiveHtmlOptions,
) -> Result<(), HtmlError> {
    let asset_dir = output_dir.join("assets");
    fs::create_dir_all(&asset_dir).map_err(|source| HtmlError::Write {
        path: asset_dir.clone(),
        source,
    })?;

    write_file(
        &output_dir.join("index.html"),
        render_html(deck, theme, options),
    )?;
    write_file(&output_dir.join(FOUNDATION_ASSET_PATH), FOUNDATION_CSS)?;
    write_file(&asset_dir.join("theme.css"), &theme.screen_css)?;
    write_file(
        &asset_dir.join("reveal.js"),
        super::theme_runtime_js(theme.manifest.api()),
    )?;
    copy_theme_dependencies(theme, output_dir)?;
    copy_local_assets(deck, output_dir)?;
    Ok(())
}

pub(super) fn render_html(
    deck: &Deck,
    theme: &RenderedTheme,
    options: super::LiveHtmlOptions,
) -> String {
    let title = deck
        .metadata
        .title
        .as_deref()
        .unwrap_or("zpres presentation");
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str(&format!("  <title>{}</title>\n", escape_html(title)));
    html.push_str(&format!(
        "  <link rel=\"stylesheet\" href=\"{}\">\n",
        FOUNDATION_ASSET_PATH
    ));
    html.push_str("  <link rel=\"stylesheet\" href=\"assets/theme.css\">\n");
    html.push_str(&format!(
        "</head>\n<body class=\"{}\" data-zpres-theme-api=\"1\" data-zpres-output-target=\"screen\" data-zpres-palette=\"{}\">\n",
        body_classes(theme),
        escape_html(&theme_palette(theme))
    ));
    html.push_str("  <main class=\"reveal zpres-presentation\">\n");
    html.push_str("    <div class=\"slides\">\n");

    let plan = PresentationPlan::for_theme_api_v1(deck);
    for group in plan.groups() {
        match group {
            PresentationGroup::Section(section) => {
                html.push_str(&format!(
                    "      <section class=\"zpres-section-stack\" data-section-index=\"{}\">\n",
                    section.section.index
                ));
                for planned in &section.slides {
                    html.push_str(&render_screen_slide(
                        *planned,
                        theme,
                        deck,
                        plan.logical_slide_count(),
                        options,
                    ));
                }
                html.push_str("      </section>\n");
            }
            PresentationGroup::Splash {
                section_index,
                background,
            } => html.push_str(&render_background_splash_section(
                background
                    .image
                    .expect("a planned Splash always has its Deck background"),
                deck.deck_root(),
                *section_index,
            )),
        }
    }

    html.push_str("    </div>\n");
    html.push_str("  </main>\n");
    if options.include_speaker_notes {
        html.push_str(&render_speaker_notes_panel());
    }
    html.push_str(&super::render_diagnostics(deck));
    html.push_str("  <script>\n");
    html.push_str(STAGE_FIT_JS);
    html.push_str(AUTOSCALE_JS);
    html.push_str(DEBUG_OVERLAY_JS);
    html.push_str("  </script>\n");
    html.push_str("  <script src=\"assets/reveal.js\"></script>\n");
    html.push_str("  <script>\n");
    html.push_str(
        "  const zpresVisualReviewParams = new URLSearchParams(window.location.search);\n  if (zpresVisualReviewParams.get(\"zpres-visual-review\") === \"1\") {\n    document.body?.setAttribute(\"data-zpres-ready\", \"pending\");\n    document.body?.setAttribute(\"data-zpres-ready-target\", \"screen\");\n",
    );
    html.push_str(STATIC_READINESS_JS);
    html.push_str("  }\n");
    html.push_str("  </script>\n");
    html.push_str("</body>\n</html>\n");
    html
}

pub(super) fn render_print_html_filtered(
    deck: &Deck,
    theme: &RenderedTheme,
    options: StaticExportOptions,
    selected_page_index: Option<usize>,
) -> String {
    let title = deck
        .metadata
        .title
        .as_deref()
        .unwrap_or("zpres presentation");
    let plan = PresentationPlan::for_theme_api_v1(deck);
    let print_pages = plan.print_pages(options.include_speaker_notes);
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str(&format!("  <title>{}</title>\n", escape_html(title)));
    html.push_str("  <style>\n");
    html.push_str(FOUNDATION_CSS);
    html.push('\n');
    html.push_str(&theme.static_screen_css);
    html.push('\n');
    html.push_str(&theme.static_print_css);
    html.push('\n');
    html.push_str("  </style>\n");
    let page_count = if selected_page_index.is_some() {
        1
    } else {
        print_pages.len()
    };
    html.push_str(&format!(
        "</head>\n<body class=\"{} zpres-print-body\" data-zpres-theme-api=\"1\" data-zpres-output-target=\"print\" data-zpres-palette=\"{}\" data-zpres-ready=\"pending\" data-zpres-ready-target=\"pdf\" data-zpres-page-count=\"{}\">\n",
        body_classes(theme),
        escape_html(&theme_palette(theme)),
        page_count
    ));

    for (page_offset, page) in print_pages.iter().enumerate() {
        let page_index = page_offset + 1;
        if selected_page_index.is_some_and(|selected| selected != page_index) {
            continue;
        }
        match page {
            PlannedPrintPage::Slide {
                planned,
                step_state,
            } => html.push_str(&render_print_slide(
                *planned,
                deck,
                theme,
                page_index,
                *step_state,
                plan.logical_slide_count(),
            )),
            PlannedPrintPage::SpeakerNotes { planned } => {
                let notes = speaker_notes(planned.slide)
                    .expect("a planned speaker-notes page always has non-empty notes");
                html.push_str(&render_speaker_notes_print_slide(
                    planned.slide,
                    notes,
                    page_index,
                    theme,
                ));
            }
            PlannedPrintPage::Splash { background, .. } => {
                html.push_str(&render_background_splash_print_slide(
                    background
                        .image
                        .expect("a planned Splash always has its Deck background"),
                    deck.deck_root(),
                    page_index,
                ));
            }
        }
    }

    html.push_str("  <script>\n");
    html.push_str(AUTOSCALE_JS);
    html.push_str(DEBUG_OVERLAY_JS);
    html.push_str(STATIC_READINESS_JS);
    html.push_str("  </script>\n");
    html.push_str("</body>\n</html>\n");
    html
}

fn theme_palette(theme: &RenderedTheme) -> String {
    theme
        .params
        .get(&theme.manifest.palette_parameter)
        .cloned()
        .unwrap_or_else(|| "default".to_string())
}

fn body_classes(theme: &RenderedTheme) -> String {
    let mut classes = vec![
        "zpres-api-v1".to_string(),
        "zpres-theme-api-v1".to_string(),
        format!("zpres-theme-{}", theme.name()),
    ];
    classes.extend(
        theme
            .manifest
            .modules
            .iter()
            .map(|module| format!("zpres-module-{module}")),
    );
    classes.join(" ")
}

fn render_screen_slide(
    planned: PlannedSlide<'_>,
    theme: &RenderedTheme,
    deck: &Deck,
    logical_slide_count: usize,
    options: super::LiveHtmlOptions,
) -> String {
    let slide = planned.slide;
    let role = slide_role_name(slide.role);
    let background_image = planned.background.image;
    let background_phase = rendered_background_phase(planned.background);
    let mut html = String::new();
    html.push_str(&format!(
        "        <section class=\"{}\" data-section-index=\"{}\" data-slide-id=\"{}\" data-slide-role=\"{}\"{}{}{}{}{}{}{}{}{}{}>\n",
        slide_section_classes(slide, "zpres-slide"),
        planned.section_index,
        escape_attr(&slide.id),
        role,
        slide_variant_attr(slide),
        slide_preset_attr(slide),
        slide_classes_attr(slide),
        slide_theme_attrs(slide),
        slide_footer_control_attrs(slide),
        slide_autoscale_attr(slide, deck),
        slide_transition_attr(slide, deck),
        background_phase_attr(background_phase),
        background_split_attr(background_image),
        slide_style_attr(slide, theme, background_image)
    ));
    if let (
        Some(background_image),
        Some(phase @ (BackgroundPhase::Title | BackgroundPhase::Content)),
    ) = (background_image, background_phase)
    {
        html.push_str(&render_v1_background_layer(
            background_image,
            AssetRenderMode::Bundle {
                deck_root: deck.deck_root(),
            },
            phase,
        ));
    }
    html.push_str(&render_slide_frame(
        slide,
        deck,
        theme,
        AssetRenderMode::Bundle {
            deck_root: deck.deck_root(),
        },
        planned.section_title,
        SlideNumberContext {
            current: planned.logical_slide_number,
            total: logical_slide_count,
        },
        options,
    ));
    html.push_str("        </section>\n");
    html
}

fn render_print_slide(
    planned: PlannedSlide<'_>,
    deck: &Deck,
    theme: &RenderedTheme,
    page_index: usize,
    step_state: PdfStepState,
    logical_slide_count: usize,
) -> String {
    let slide = planned.slide;
    let role = slide_role_name(slide.role);
    let background_image = planned.background.image;
    let background_phase = rendered_background_phase(planned.background);
    let mut html = String::new();
    html.push_str(&format!(
        "  <section class=\"{}\" data-page=\"{}\" data-slide-id=\"{}\" data-section-index=\"{}\" data-slide-role=\"{}\"{}{}{}{}{}{}{}{}{}{}{}>\n",
        slide_section_classes(slide, "zpres-print-slide zpres-slide"),
        page_index,
        escape_attr(&slide.id),
        planned.section_index,
        role,
        slide_variant_attr(slide),
        slide_preset_attr(slide),
        slide_classes_attr(slide),
        slide_theme_attrs(slide),
        slide_footer_control_attrs(slide),
        slide_autoscale_attr(slide, deck),
        slide_transition_attr(slide, deck),
        background_phase_attr(background_phase),
        background_split_attr(background_image),
        pdf_step_state_attr(step_state),
        slide_style_attr(slide, theme, background_image)
    ));
    if let (
        Some(background_image),
        Some(phase @ (BackgroundPhase::Title | BackgroundPhase::Content)),
    ) = (background_image, background_phase)
    {
        html.push_str(&render_v1_background_layer(
            background_image,
            AssetRenderMode::Print {
                deck_root: deck.deck_root(),
                step_state,
            },
            phase,
        ));
    }
    html.push_str(&render_slide_frame(
        slide,
        deck,
        theme,
        AssetRenderMode::Print {
            deck_root: deck.deck_root(),
            step_state,
        },
        planned.section_title,
        SlideNumberContext {
            current: planned.logical_slide_number,
            total: logical_slide_count,
        },
        super::LiveHtmlOptions::default(),
    ));
    html.push_str("  </section>\n");
    html
}

fn render_slide_frame(
    slide: &Slide,
    deck: &Deck,
    theme: &RenderedTheme,
    asset_mode: AssetRenderMode<'_>,
    section_title: Option<&str>,
    slide_number: SlideNumberContext,
    options: super::LiveHtmlOptions,
) -> String {
    let context = RenderContext::for_page(slide, theme.manifest.api(), asset_mode.step_state());
    let title_index = title_block_index(slide);
    let mut html = String::new();
    html.push_str("          <div class=\"zpres-slide-frame zpres-slide-canvas\">\n");
    html.push_str("            <div class=\"zpres-slide-content\">\n");
    if let Some(title) = slide.title.as_deref() {
        let title_role = if slide.variant == Some(crate::deck::SlideVariant::SectionTitle) {
            "display"
        } else {
            "title"
        };
        html.push_str("              <header class=\"zpres-slide-header\">\n");
        html.push_str(&format!(
            "                <h1 class=\"zpres-slide-title\" data-zpres-type-role=\"{title_role}\">{}</h1>\n",
            super::render_paragraph(title, &crate::deck::inline_math_segments(title), &context)
        ));
        html.push_str("              </header>\n");
    }
    html.push_str("              <div class=\"zpres-slide-body\">\n");
    html.push_str(
        "                <div class=\"zpres-slide-primary\" data-zpres-type-role=\"body\">\n",
    );
    let mut primary_count = 0usize;
    for (index, block) in slide.blocks.iter().enumerate() {
        if Some(index) == title_index
            || matches!(
                block,
                ContentBlock::Footnotes { .. } | ContentBlock::SpeakerNotes { .. }
            )
        {
            continue;
        }
        html.push_str(&render_block(block, asset_mode, &context));
        primary_count += 1;
    }
    if slide.title.is_none() && primary_count == 0 && !has_sources(slide) {
        html.push_str("                  <div class=\"zpres-empty-slide\">empty slide</div>\n");
    }
    html.push_str("                </div>\n");
    html.push_str("                <aside class=\"zpres-slide-supporting\" data-zpres-type-role=\"supporting\" aria-hidden=\"true\"></aside>\n");
    html.push_str("              </div>\n");
    let step_count = slide_screen_step_count(slide);
    if step_count > 0 {
        let (current, live) = match asset_mode {
            AssetRenderMode::Bundle { .. } => (0, " aria-live=\"polite\""),
            AssetRenderMode::Print {
                step_state: PdfStepState::Final,
                ..
            } => (step_count, ""),
            AssetRenderMode::Print {
                step_state: PdfStepState::UpTo { step },
                ..
            } => (step.min(step_count), ""),
        };
        html.push_str(&format!(
            "              <div class=\"zpres-step-progress\" data-zpres-step-progress role=\"status\" aria-atomic=\"true\"{live}>Step {current} of {step_count}</div>\n"
        ));
    }
    if has_sources(slide) {
        html.push_str(
            "              <aside class=\"zpres-slide-sources\" aria-label=\"Sources\">\n",
        );
        for block in &slide.blocks {
            if matches!(block, ContentBlock::Footnotes { .. }) {
                html.push_str(&render_block(block, asset_mode, &context));
            }
        }
        html.push_str("              </aside>\n");
    }
    html.push_str("            </div>\n");
    html.push_str(&render_slide_footer(
        slide,
        deck,
        theme,
        section_title,
        slide_number,
    ));
    html.push_str("            <div class=\"zpres-slide-ornament\" aria-hidden=\"true\"></div>\n");
    html.push_str("          </div>\n");
    for block in &slide.blocks {
        if options.include_speaker_notes && matches!(block, ContentBlock::SpeakerNotes { .. }) {
            html.push_str(&render_block(block, asset_mode, &context));
        }
    }
    html
}

fn title_block_index(slide: &Slide) -> Option<usize> {
    slide.title.as_ref().and_then(|_| {
        slide
            .blocks
            .iter()
            .position(|block| matches!(block, ContentBlock::Heading { .. }))
    })
}

fn has_sources(slide: &Slide) -> bool {
    slide
        .blocks
        .iter()
        .any(|block| matches!(block, ContentBlock::Footnotes { notes } if !notes.is_empty()))
}

fn rendered_background_phase(background: PlannedBackground<'_>) -> Option<BackgroundPhase> {
    match background.phase {
        PresentationBackgroundPhase::Title => Some(BackgroundPhase::Title),
        PresentationBackgroundPhase::Content if background.image.is_some() => {
            Some(BackgroundPhase::Content)
        }
        PresentationBackgroundPhase::Content => None,
        PresentationBackgroundPhase::Splash => Some(BackgroundPhase::Splash),
    }
}

fn slide_style_attr(
    slide: &Slide,
    theme: &RenderedTheme,
    background_image: Option<&crate::deck::DeckBackgroundImage>,
) -> String {
    let split_size = background_image
        .and_then(|image| image.split.as_ref())
        .map(|split| split.size.as_str());
    split_size.map_or_else(
        || slide_theme_style_attr(slide, theme),
        |size| {
            slide_theme_style_attr_with_extra(
                slide,
                theme,
                &[("--zpres-background-split-size", size)],
            )
        },
    )
}

fn slide_role_name(role: SlideRole) -> &'static str {
    match role {
        SlideRole::Main => "main",
        SlideRole::Detail => "detail",
    }
}

fn speaker_notes(slide: &Slide) -> Option<String> {
    let notes = slide
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::SpeakerNotes { markdown } if !markdown.trim().is_empty() => {
                Some(markdown.trim())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!notes.is_empty()).then_some(notes)
}

fn render_background_splash_section(
    background_image: &crate::deck::DeckBackgroundImage,
    deck_root: Option<&Path>,
    section_index: usize,
) -> String {
    let mut html = format!(
        "      <section class=\"zpres-section-stack zpres-background-splash-stack\" data-section-index=\"{}\" data-generated-section=\"background-image\">\n",
        section_index
    );
    html.push_str(&format!(
        "        <section class=\"zpres-slide zpres-background-splash-slide\" data-section-index=\"{}\" data-slide-id=\"background-image-splash\" data-slide-role=\"generated\" data-generated-slide=\"background-image\"{}>\n",
        section_index,
        background_phase_attr(Some(BackgroundPhase::Splash))
    ));
    html.push_str(&render_v1_background_layer(
        background_image,
        AssetRenderMode::Bundle { deck_root },
        BackgroundPhase::Splash,
    ));
    html.push_str("        </section>\n");
    html.push_str("      </section>\n");
    html
}

fn render_background_splash_print_slide(
    background_image: &crate::deck::DeckBackgroundImage,
    deck_root: Option<&Path>,
    page_index: usize,
) -> String {
    let mut html = format!(
        "  <section class=\"zpres-print-slide zpres-slide zpres-background-splash-slide\" data-page=\"{}\" data-slide-id=\"background-image-splash\" data-slide-role=\"generated\" data-generated-slide=\"background-image\"{}>\n",
        page_index,
        background_phase_attr(Some(BackgroundPhase::Splash))
    );
    html.push_str(&render_v1_background_layer(
        background_image,
        AssetRenderMode::Print {
            deck_root,
            step_state: PdfStepState::Final,
        },
        BackgroundPhase::Splash,
    ));
    html.push_str("  </section>\n");
    html
}

fn render_speaker_notes_print_slide(
    slide: &Slide,
    notes: String,
    page_index: usize,
    theme: &RenderedTheme,
) -> String {
    let mut html = format!(
        "  <section class=\"{}\" data-page=\"{}\" data-slide-id=\"{}\" data-slide-role=\"{}\"{}{}{}{}{} data-generated-slide=\"speaker-notes\">\n",
        slide_section_classes(
            slide,
            "zpres-print-slide zpres-slide zpres-speaker-notes-print-slide"
        ),
        page_index,
        escape_attr(&slide.id),
        slide_role_name(slide.role),
        slide_variant_attr(slide),
        slide_preset_attr(slide),
        slide_classes_attr(slide),
        slide_theme_attrs(slide),
        slide_theme_style_attr(slide, theme)
    );
    html.push_str("    <div class=\"zpres-slide-frame zpres-slide-canvas zpres-speaker-notes-print-canvas\">\n");
    html.push_str("      <div class=\"zpres-slide-content zpres-speaker-notes-print-content\">\n");
    html.push_str("        <header class=\"zpres-slide-header\"><h1 class=\"zpres-slide-title\">Speaker notes</h1></header>\n");
    html.push_str("        <div class=\"zpres-slide-body\"><div class=\"zpres-slide-primary\">\n");
    html.push_str("          <div class=\"zpres-block zpres-speaker-notes-print-block\" data-block-type=\"speaker-notes-print\">\n");
    html.push_str(&format!(
        "            <div class=\"zpres-speaker-notes-print-body\">{}</div>\n",
        render_speaker_notes_body(&notes)
    ));
    html.push_str("          </div>\n");
    html.push_str("        </div></div>\n");
    html.push_str("      </div>\n");
    html.push_str("    </div>\n");
    html.push_str("  </section>\n");
    html
}
