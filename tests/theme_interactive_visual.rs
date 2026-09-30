use std::fs;
use std::path::Path;
use std::process::Output;

use assert_cmd::Command;
use image::GenericImageView;
use serde_json::Value;
use tempfile::tempdir;

const SCREEN_BASE: &str = r#"
html,
body {
  width: 100%;
  height: 100%;
  margin: 0;
}

.debug-topbar,
.debug-nav,
.debug-diagnostics,
.zpres-slide-meta {
  display: none;
}

.reveal {
  inset: 0;
}

.slides,
.zpres-section-stack,
.zpres-slide,
.zpres-slide-canvas {
  width: 100%;
  height: 100%;
}

.zpres-slide {
  padding: 0;
  color: #172033;
  background: #e8edf5;
}

.zpres-slide-canvas {
  display: grid;
  padding: 56px 72px 68px;
  overflow: hidden;
  background: #f8fafc;
}

.zpres-slide-content {
  display: grid;
  align-content: start;
  gap: 24px;
  min-width: 0;
}

.zpres-block {
  min-width: 0;
  font: 24px/1.35 system-ui, sans-serif;
}

.zpres-block h1,
.zpres-block h2 {
  margin: 0;
  font-size: 48px;
  line-height: 1.05;
}
"#;

const PRINT_BASE: &str = r#"
.zpres-print-slide {
  color: #172033;
  background: #e8edf5;
}

.zpres-slide-canvas {
  display: grid;
  width: 100%;
  height: 100%;
  padding: 56px 72px 68px;
  overflow: hidden;
  background: #f8fafc;
}

.zpres-slide-content {
  display: grid;
  align-content: start;
  gap: 24px;
  min-width: 0;
}

.zpres-block {
  min-width: 0;
  font: 24px/1.35 system-ui, sans-serif;
}

.zpres-block h1,
.zpres-block h2 {
  margin: 0;
  font-size: 48px;
  line-height: 1.05;
}
"#;

fn write_test_theme(path: &Path, screen_extra: &str, print_extra: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("theme.toml"),
        r#"[theme]
name = "interactive-visual-test"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
"#,
    )
    .unwrap();
    fs::write(
        path.join("theme.css.tmpl"),
        format!("{SCREEN_BASE}\n{screen_extra}\n"),
    )
    .unwrap();
    fs::write(
        path.join("print.css.tmpl"),
        format!("{PRINT_BASE}\n{print_extra}\n"),
    )
    .unwrap();
}

fn run_visual_check(theme_dir: &Path, fixture: &Path, output_dir: &Path) -> Output {
    Command::cargo_bin("zpres")
        .unwrap()
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--visual",
            "--fixture",
            fixture.to_str().unwrap(),
            "--write-specimen",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

fn chromium_is_genuinely_absent(output: &Output) -> bool {
    if output.status.success() {
        return false;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Chrome/Chromium was not found") && stderr.contains("ZPRES_CHROMIUM") {
        eprintln!("skipping browser-backed interactive visual check: {stderr}");
        true
    } else {
        false
    }
}

fn output_context(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn read_report(output_dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
        .unwrap()
}

fn assert_surface_statuses(report: &Value, screen: &str, print: &str) {
    assert_eq!(report["screen_status"], screen);
    assert_eq!(report["print_status"], print);
}

fn rect_number(rect: &Value, field: &str) -> f64 {
    rect[field]
        .as_f64()
        .unwrap_or_else(|| panic!("missing numeric rectangle field {field}: {rect}"))
}

fn assert_rect_is_within(inner: &Value, outer: &Value) {
    let tolerance = 2.0;
    assert!(
        rect_number(inner, "x") >= rect_number(outer, "x") - tolerance,
        "inner rectangle starts left of outer rectangle: inner={inner}, outer={outer}"
    );
    assert!(
        rect_number(inner, "y") >= rect_number(outer, "y") - tolerance,
        "inner rectangle starts above outer rectangle: inner={inner}, outer={outer}"
    );
    assert!(
        rect_number(inner, "right") <= rect_number(outer, "right") + tolerance,
        "inner rectangle ends right of outer rectangle: inner={inner}, outer={outer}"
    );
    assert!(
        rect_number(inner, "bottom") <= rect_number(outer, "bottom") + tolerance,
        "inner rectangle ends below outer rectangle: inner={inner}, outer={outer}"
    );
}

#[test]
fn visual_theme_check_visits_main_detail_and_every_two_step_state() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("routes.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(&theme_dir, "", "");
    fs::write(
        &fixture,
        r#"---
title: "Interactive route fixture"
aspect: "16:9"
---

# Main route

The Main slide is the opening live route.

--

## Detail route

::: steps
1. First detail Step.
2. Final detail Step.
:::
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        output.status.success(),
        "interactive route review failed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "passed", "passed");
    assert_eq!(report["expected_screen_slides"], 2);
    assert_eq!(report["captured_screen_slides"], 2);
    assert_eq!(report["expected_screen_states"], 4);
    assert_eq!(report["captured_screen_states"], 4);
    assert!(report["screen_failures"].as_array().unwrap().is_empty());
    assert!(report["print_failures"].as_array().unwrap().is_empty());

    let states = report["screen_states"].as_array().unwrap();
    let observed_routes = states
        .iter()
        .map(|state| {
            let route = &state["route"];
            (
                route["hash"].as_str().unwrap(),
                route["section"].as_u64().unwrap(),
                route["detail"].as_u64().unwrap(),
                route["step"].as_u64().unwrap(),
                route["step_count"].as_u64().unwrap(),
                route["slide_id"].as_str().unwrap(),
                route["role"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        observed_routes,
        vec![
            ("#/0/0", 0, 0, 0, 0, "section-1-main", "main"),
            ("#/0/1", 0, 1, 0, 2, "section-1-detail-1", "detail"),
            ("#/0/1/1", 0, 1, 1, 2, "section-1-detail-1", "detail"),
            ("#/0/1/2", 0, 1, 2, 2, "section-1-detail-1", "detail"),
        ]
    );

    let screen_pages_dir = report["artifacts"]["screen_pages_dir"].as_str().unwrap();
    assert!(output_dir.join(screen_pages_dir).is_dir());
    for state in states {
        let screenshot = state["screenshot"].as_str().unwrap();
        let image = image::open(output_dir.join(screenshot)).unwrap();
        assert_eq!(image.dimensions(), (1280, 720));
    }
    let contact_sheet = report["artifacts"]["screen_contact_sheet"]
        .as_str()
        .unwrap();
    let contact_sheet = image::open(output_dir.join(contact_sheet)).unwrap();
    assert!(contact_sheet.width() > 0);
    assert!(contact_sheet.height() > 0);
}

#[test]
fn visual_theme_check_visits_the_generated_background_splash_route() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("background-splash.zp.md");
    let background = temp.path().join("background.svg");
    let output_dir = temp.path().join("review");
    write_test_theme(&theme_dir, "", "");
    fs::write(
        &background,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720" viewBox="0 0 1280 720"><rect width="1280" height="720" fill="#214f78"/><circle cx="930" cy="220" r="180" fill="#8fc8df"/></svg>"##,
    )
    .unwrap();
    fs::write(
        &fixture,
        r#"---
title: "Generated splash route fixture"
aspect: "16:9"
background_image:
  src: "background.svg"
  alt: "Synthetic blue background"
  splash: true
  intent: contextual
---

# Authored Main slide

The generated splash must also be navigable and captured.
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        output.status.success(),
        "generated splash review failed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "passed", "passed");
    assert_eq!(report["expected_screen_slides"], 2);
    assert_eq!(report["captured_screen_slides"], 2);
    assert_eq!(report["expected_screen_states"], 2);
    assert_eq!(report["captured_screen_states"], 2);
    let states = report["screen_states"].as_array().unwrap();
    assert_eq!(states[0]["route"]["hash"], "#/0/0");
    assert_eq!(states[0]["route"]["generated"], Value::Null);
    assert_eq!(states[1]["route"]["hash"], "#/1/0");
    assert_eq!(states[1]["route"]["slide_id"], "background-image-splash");
    assert_eq!(states[1]["route"]["role"], "generated");
    assert_eq!(states[1]["route"]["generated"], "background-image");
}

#[test]
fn visual_theme_check_reports_live_overflow_visible_only_on_the_final_step() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("final-step-overflow.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        r#"
.reveal .zpres-step[data-step-index="2"] {
  position: relative;
  left: 0;
  width: 260px;
  transition: left 450ms linear;
}

.reveal .zpres-step[data-step-index="2"].is-visible {
  left: 1200px;
}

@media (prefers-reduced-motion: reduce) {
  .reveal .zpres-step[data-step-index="2"] { transition: none; }
}
"#,
        "",
    );
    fs::write(
        &fixture,
        r#"---
title: "Final Step overflow fixture"
aspect: "16:9"
---

# The final Step must be measured

::: steps
1. This Step fits.
2. This final Step crosses the live canvas only when revealed.
:::
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "final-Step overflow review unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "failed", "passed");
    assert_eq!(report["expected_screen_slides"], 1);
    assert_eq!(report["captured_screen_slides"], 1);
    assert_eq!(report["expected_screen_states"], 3);
    assert_eq!(report["captured_screen_states"], 3);
    assert!(report["print_failures"].as_array().unwrap().is_empty());

    let states = report["screen_states"].as_array().unwrap();
    assert_eq!(
        states
            .iter()
            .map(|state| state["route"]["hash"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["#/0/0", "#/0/0/1", "#/0/0/2"]
    );
    let failures = report["screen_failures"].as_array().unwrap();
    assert!(
        failures.iter().all(|finding| finding["route"] == "#/0/0/2"),
        "an earlier Step failed even though only the final Step overflows: {failures:?}"
    );
    let finding = failures
        .iter()
        .find(|finding| {
            finding["route"] == "#/0/0/2"
                && finding["element"]
                    .as_str()
                    .is_some_and(|locator| locator.contains("data-step-index"))
        })
        .unwrap_or_else(|| panic!("no element-level final-Step finding: {failures:?}"));
    assert_eq!(finding["slide_id"], "section-1-main");
    assert!(
        finding["element"]
            .as_str()
            .is_some_and(|locator| locator.contains("data-step-index")),
        "the finding does not identify the overflowing Step: {finding}"
    );
    assert!(finding["element_bounds"].is_object());
    assert!(finding["boundary_bounds"].is_object());
    assert!(
        rect_number(&finding["element_bounds"], "x") > 1_000.0,
        "the final Step was captured before its transition settled: {finding}"
    );
}

#[test]
fn visual_theme_check_reports_an_image_crossing_a_caption_lane_when_its_parent_fits() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("caption-collision.zp.md");
    let figure = temp.path().join("figure.svg");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        r#"
.zpres-block-figure {
  display: grid;
  grid-template-rows: minmax(0, 1fr) 44px;
  width: 720px;
  height: 360px;
  overflow: hidden;
}

.zpres-block-figure img {
  grid-row: 1;
  width: 100%;
  height: 360px;
  min-height: 360px;
  object-fit: cover;
}

.zpres-block-figure figcaption {
  grid-row: 2;
  min-height: 44px;
  padding-top: 8px;
  font-size: 18px;
  line-height: 1.2;
}
"#,
        r#"
.zpres-block-figure {
  display: grid;
  grid-template-rows: 260px auto;
  width: 720px;
}

.zpres-block-figure img {
  width: 100%;
  height: 260px;
  min-height: 0;
  object-fit: cover;
}

.zpres-block-figure figcaption {
  padding-top: 8px;
  font-size: 18px;
  line-height: 1.2;
}
"#,
    );
    fs::write(
        &figure,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="720" height="360" viewBox="0 0 720 360"><rect width="720" height="360" fill="#7ba7c7"/><path d="M0 300 L720 80" stroke="#17324d" stroke-width="18"/></svg>"##,
    )
    .unwrap();
    fs::write(
        &fixture,
        r#"---
title: "Caption collision fixture"
aspect: "16:9"
---

# The Figure parent still fits

::: figure src="figure.svg" alt="Synthetic diagonal figure" caption="The image must stay out of this caption lane."
:::
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "caption-collision review unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "failed", "passed");
    assert!(report["print_failures"].as_array().unwrap().is_empty());
    let state = &report["screen_states"].as_array().unwrap()[0];
    let canvas_bounds = &state["canvas_bounds"];
    let figure_bounds = state["block_bounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["block_type"] == "figure")
        .map(|block| &block["bounds"])
        .expect("the screen observation did not retain the Figure parent bounds");
    assert_rect_is_within(figure_bounds, canvas_bounds);

    let failures = report["screen_failures"].as_array().unwrap();
    let finding = failures
        .iter()
        .find(|finding| finding["code"] == "caption-collision")
        .unwrap_or_else(|| panic!("no caption-collision finding: {failures:?}"));
    assert_eq!(finding["route"], "#/0/0");
    assert!(
        finding["element"]
            .as_str()
            .is_some_and(|locator| locator.contains("img"))
    );
    assert!(
        finding["boundary"]
            .as_str()
            .is_some_and(|locator| locator.contains("figcaption"))
    );
    let image_bounds = &finding["element_bounds"];
    let caption_bounds = &finding["boundary_bounds"];
    assert!(image_bounds.is_object());
    assert!(caption_bounds.is_object());
    assert!(
        rect_number(image_bounds, "bottom") > rect_number(caption_bounds, "y"),
        "the reported bounds do not demonstrate the collision: {finding}"
    );
}

#[test]
fn visual_theme_check_reports_canvas_and_footer_clipping_without_document_scroll() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("clipped-surfaces.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        r#"
.reveal .zpres-slide-canvas {
  position: absolute;
  top: 0;
  left: 24px;
  width: 100%;
  height: 100%;
}

.reveal .zpres-slide-footer {
  position: absolute;
  bottom: -36px;
  height: 24px;
}
"#,
        "",
    );
    fs::write(
        &fixture,
        r#"---
title: "Clipped presentation surfaces"
aspect: "16:9"
footer: "This footer is deliberately outside the live Slide"
---

# Content still fits its shifted canvas

The document itself remains locked to the viewport.
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "clipped live surfaces unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "failed", "passed");
    assert!(report["print_failures"].as_array().unwrap().is_empty());
    let failures = report["screen_failures"].as_array().unwrap();
    assert!(
        failures
            .iter()
            .any(|finding| finding["code"] == "canvas-outside-slide"),
        "the shifted canvas was not reported: {failures:?}"
    );
    assert!(
        failures
            .iter()
            .any(|finding| finding["code"] == "footer-outside-slide"),
        "the clipped footer was not reported: {failures:?}"
    );
    let scroll = &report["screen_states"][0]["document_scroll"];
    assert_eq!(scroll["left"], 0.0);
    assert_eq!(scroll["top"], 0.0);
    assert_eq!(scroll["right"], 0.0);
    assert_eq!(scroll["bottom"], 0.0);
}

#[test]
fn visual_theme_check_reports_unpositioned_text_outside_its_fitting_block() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("text-block-overflow.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        r#"
.reveal .zpres-block-paragraph {
  width: 420px;
  height: 64px;
  overflow: visible;
}

.reveal .zpres-block-paragraph p {
  width: 420px;
  margin: 0;
}
"#,
        "",
    );
    fs::write(
        &fixture,
        r#"---
title: "Text block overflow fixture"
aspect: "16:9"
---

# The parent block still fits

This deliberately long paragraph wraps across enough lines that its ordinary flow box extends well below the fixed-height Content block, while remaining inside the Slide canvas.
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "unpositioned text overflow unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "failed", "passed");
    let failures = report["screen_failures"].as_array().unwrap();
    let finding = failures
        .iter()
        .find(|finding| {
            finding["code"] == "outside-content-block"
                && finding["element"]
                    .as_str()
                    .is_some_and(|element| element.starts_with('p'))
        })
        .unwrap_or_else(|| panic!("no unpositioned text containment finding: {failures:?}"));
    assert_eq!(finding["route"], "#/0/0");
    assert!(
        rect_number(&finding["element_bounds"], "bottom")
            > rect_number(&finding["boundary_bounds"], "bottom") + 40.0
    );
}

#[test]
fn visual_theme_check_does_not_trust_aria_hidden_for_print_steps() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("visible-hidden-step.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        "",
        r#"
body[data-zpres-output-target="print"] .zpres-print-slide .zpres-step.zpres-print-step-hidden {
  visibility: visible !important;
  opacity: 1 !important;
}
"#,
    );
    fs::write(
        &fixture,
        r#"---
title: "Visible hidden Step fixture"
aspect: "16:9"
---

# Each print page has an exact Step state

::: steps pdf="pages"
1. First export page.
2. Second export page.
:::
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "CSS-visible aria-hidden Step unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "passed", "failed");
    let failures = report["print_failures"].as_array().unwrap();
    let finding = failures
        .iter()
        .find(|finding| finding["code"] == "unexpected-visible-step")
        .unwrap_or_else(|| panic!("no Step visibility finding: {failures:?}"));
    assert_eq!(finding["page"], 1);
    assert!(
        finding["element"]
            .as_str()
            .is_some_and(|element| element.contains("data-step-index=\"2\""))
    );
}

#[test]
fn visual_theme_check_rejects_rendered_inactive_slides() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("visible-inactive-slides.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        r#"
.reveal .zpres-section-stack,
.reveal .zpres-slide {
  position: absolute;
  inset: 0;
  display: grid !important;
}
"#,
        "",
    );
    fs::write(
        &fixture,
        r#"---
title: "Visible inactive Slides fixture"
aspect: "16:9"
---

# First Main slide

Only this route should be painted.

---

# Second Main slide

The inactive route must remain visually hidden.
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "rendered inactive Slides unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "failed", "passed");
    let failures = report["screen_failures"].as_array().unwrap();
    assert!(
        failures
            .iter()
            .any(|finding| finding["code"] == "screen-visible-state-mismatch"),
        "no rendered inactive-Slide finding: {failures:?}"
    );
    assert!(
        report["screen_states"]
            .as_array()
            .unwrap()
            .iter()
            .all(|state| state["visible_slide_count"].as_u64().unwrap() > 1)
    );
}

#[test]
fn visual_theme_check_detects_visible_descendants_inside_a_hidden_future_step() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("visible-step-descendant.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        r#"
.reveal .zpres-block-list li.fragment:not(.is-visible) {
  visibility: hidden !important;
  opacity: 1 !important;
}

.reveal .zpres-block-list li.fragment:not(.is-visible) math {
  visibility: visible !important;
}
"#,
        "",
    );
    fs::write(
        &fixture,
        r#"---
title: "Visible Step descendant fixture"
aspect: "16:9"
---

# Hidden descendants must not paint

* This future child incorrectly leaves \(x\) visible.
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "visible descendant inside a hidden Step unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "failed", "passed");
    let failures = report["screen_failures"].as_array().unwrap();
    let finding = failures
        .iter()
        .find(|finding| finding["code"] == "unexpected-visible-step" && finding["route"] == "#/0/0")
        .unwrap_or_else(|| panic!("no leaked future-Step finding: {failures:?}"));
    assert!(
        finding["element"]
            .as_str()
            .is_some_and(|element| element.contains("data-step-index=\"1\""))
    );
}

#[test]
fn visual_theme_check_keeps_a_print_failure_separate_from_a_passing_screen() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("theme");
    let fixture = temp.path().join("print-only-overflow.zp.md");
    let output_dir = temp.path().join("review");
    write_test_theme(
        &theme_dir,
        "",
        r#"
.zpres-block-paragraph p {
  position: relative;
  left: 1400px;
  width: 280px;
}
"#,
    );
    fs::write(
        &fixture,
        r#"---
title: "Print-only overflow fixture"
aspect: "16:9"
---

# Screen and print are independent surfaces

This paragraph fits the interactive HTML presentation but not the print canvas.
"#,
    )
    .unwrap();

    let output = run_visual_check(&theme_dir, &fixture, &output_dir);
    if chromium_is_genuinely_absent(&output) {
        return;
    }
    assert!(
        !output.status.success(),
        "print-only overflow review unexpectedly passed\n{}",
        output_context(&output)
    );

    let report = read_report(&output_dir);
    assert_surface_statuses(&report, "passed", "failed");
    assert!(report["screen_failures"].as_array().unwrap().is_empty());
    assert!(!report["print_failures"].as_array().unwrap().is_empty());
    assert_eq!(report["expected_screen_slides"], 1);
    assert_eq!(report["captured_screen_slides"], 1);
    assert_eq!(report["expected_screen_states"], 1);
    assert_eq!(report["captured_screen_states"], 1);
    assert_eq!(report["screen_states"][0]["route"]["hash"], "#/0/0");
}
