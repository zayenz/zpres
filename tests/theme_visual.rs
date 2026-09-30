use std::fs;
use std::path::Path;

use assert_cmd::Command;
use image::GenericImageView;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::tempdir;

fn write_visual_test_theme(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("theme.toml"),
        r##"[theme]
name = "visual-test"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
feature_hooks = ["fit-text"]
"##,
    )
    .unwrap();
    fs::write(
        path.join("theme.css.tmpl"),
        r#".debug-topbar, .debug-nav, .debug-diagnostics, .zpres-slide-meta { display: none; }
.reveal { inset: 0; }
.zpres-slide { color: #172033; background: #f3f6fb; }
.zpres-slide-canvas { display: grid; width: 100%; height: 100%; padding: 72px 88px; overflow: hidden; }
.zpres-slide-content { display: grid; align-content: start; gap: 24px; width: 100%; }
.zpres-block { min-width: 0; font: 24px/1.35 system-ui, sans-serif; }
.zpres-block h1 { margin: 0; font-size: 52px; line-height: 1.05; }
"#,
    )
    .unwrap();
    fs::write(
        path.join("print.css.tmpl"),
        r#".zpres-print-slide { color: #172033; background: #f3f6fb; }
.zpres-slide { color: #172033; }
.zpres-slide-canvas { display: grid; width: 100%; height: 100%; padding: 0.7in 0.85in; overflow: hidden; }
.zpres-slide-content { display: grid; align-content: start; gap: 0.22in; width: 100%; }
.zpres-block { min-width: 0; font: 24px/1.35 system-ui, sans-serif; }
.zpres-block h1 { margin: 0; font-size: 52px; line-height: 1.05; }
"#,
    )
    .unwrap();
}

fn write_two_page_fixture(path: &Path) {
    fs::write(
        path,
        r#"---
title: "Visual gate integration fixture"
aspect: "16:9"
---

# First visual page

This page exercises the browser-rendered Theme review path.

* The output is measured after browser layout.
* The screenshot must decode at the Deck aspect.

---

# Second visual page

The second page makes the expected and captured page counts observable.

> A visual pass still requires a human to review composition.
"#,
    )
    .unwrap();
}

fn write_overflowing_visual_test_theme(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("theme.toml"),
        r##"[theme]
name = "visual-overflow-test"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
"##,
    )
    .unwrap();
    fs::write(
        path.join("theme.css.tmpl"),
        r#".debug-topbar, .debug-nav, .debug-diagnostics, .zpres-slide-meta { display: none; }
.reveal { inset: 0; }
.zpres-slide { color: #172033; background: #f3f6fb; }
.zpres-slide-canvas { display: grid; width: 100%; height: 100%; overflow: hidden; }
.zpres-slide-content { width: 100%; }
.zpres-block { min-width: 0; font: 24px/1.35 system-ui, sans-serif; }
.zpres-block-paragraph p { position: relative; left: 1400px; width: 320px; }
"#,
    )
    .unwrap();
    fs::write(
        path.join("print.css.tmpl"),
        r#".zpres-print-slide { color: #172033; background: #f3f6fb; }
.zpres-slide { color: #172033; }
.zpres-slide-canvas { display: grid; width: 100%; height: 100%; overflow: hidden; }
.zpres-slide-content { width: 100%; }
.zpres-block { min-width: 0; font: 24px/1.35 system-ui, sans-serif; }
.zpres-block-paragraph p { position: relative; left: 1400px; width: 320px; }
"#,
    )
    .unwrap();
}

fn write_overflowing_fixture(path: &Path) {
    fs::write(
        path,
        r#"---
title: "Visual gate overflow fixture"
aspect: "16:9"
---

# Deliberately outside the canvas

This content must make the objective browser-rendered visual gate fail.
"#,
    )
    .unwrap();
}

fn write_unrecoverable_autoscale_fixture(path: &Path) {
    let mut source = String::from(
        r#"---
title: "Unrecoverable autoscale fixture"
aspect: "16:9"
autoscale: true
---

# Deliberate minimum-scale overflow

"#,
    );
    for item in 1..=160 {
        source.push_str(&format!("- visible overflow item {item}\n"));
    }
    fs::write(path, source).unwrap();
}

fn write_v1_autoscale_test_theme(path: &Path, extra_css: &str) {
    fs::create_dir_all(path).unwrap();
    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes/reference");
    fs::copy(reference.join("theme.toml"), path.join("theme.toml")).unwrap();
    for stylesheet in ["theme.css.tmpl", "print.css.tmpl"] {
        let mut css = fs::read_to_string(reference.join(stylesheet)).unwrap();
        css.push('\n');
        css.push_str(extra_css);
        css.push('\n');
        fs::write(path.join(stylesheet), css).unwrap();
    }
}

fn max_body_text_edge(observation: &Value, edge: &str) -> f64 {
    observation["semantic_text_regions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|region| region["region"] == "body")
        .flat_map(|region| region["text_bounds"].as_array().unwrap())
        .filter_map(|bounds| bounds[edge].as_f64())
        .fold(f64::NEG_INFINITY, f64::max)
}

fn write_four_three_fixture(path: &Path) {
    fs::write(
        path,
        r#"---
title: "Visual gate aspect mismatch fixture"
aspect: "4:3"
---

# Four by three must not crop silently

Until the print canvas follows the Deck aspect, the visual gate must block this review.
"#,
    )
    .unwrap();
}

#[test]
fn visual_theme_check_requires_a_specimen_output_directory() {
    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["theme", "check", "themes/science", "--visual"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--visual requires --write-specimen <dir>",
        ));
}

#[test]
fn nonvisual_theme_check_reports_that_the_visual_gate_was_not_run() {
    let temp = tempdir().unwrap();
    let missing_chromium = temp.path().join("missing-chromium");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("ZPRES_CHROMIUM", missing_chromium)
        .args(["theme", "check", "themes/science", "--no-fixture"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("contract_status = valid")
                .and(predicate::str::contains("visual_status = not-run")),
        );
}

#[test]
fn nonvisual_theme_check_keeps_custom_fixture_feature_coverage_strict() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("visual-test");
    let fixture = temp.path().join("two-pages.zp.md");
    write_visual_test_theme(&theme_dir);
    write_two_page_fixture(&fixture);

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--fixture",
            fixture.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("fixture does not exercise declared feature_hooks")
                .and(predicate::str::contains("fit-text")),
        );
}

#[test]
fn visual_theme_check_writes_decodable_artifacts_and_release_provenance() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("visual-test");
    let fixture = temp.path().join("two-pages.zp.md");
    let output_dir = temp.path().join("review");
    write_visual_test_theme(&theme_dir);
    write_two_page_fixture(&fixture);

    let output = Command::cargo_bin("zpres")
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
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed visual check: {stderr}");
        return;
    }
    let report_debug = fs::read_to_string(output_dir.join("visual-report.json"))
        .unwrap_or_else(|_| "<visual report unavailable>".to_string());
    assert!(
        output.status.success(),
        "visual check failed\nstdout:\n{stdout}\nstderr:\n{stderr}\nreport:\n{report_debug}"
    );
    assert!(stdout.contains("contract_status = valid"));
    assert!(stdout.contains("fixture_feature_coverage = incomplete"));
    assert!(stdout.contains("fixture_missing_declared_features = fit-text"));
    assert!(stdout.contains("visual_status = passed"));
    assert!(stdout.contains("screen_status = passed"));
    assert!(stdout.contains("print_status = passed"));
    assert!(stdout.contains("human_review = required"));
    assert!(stdout.contains("release_status = pending-review"));
    assert!(stdout.contains("visual_pages = 2/2"));
    assert!(
        stderr.contains("this visual review covers only the features used by the selected fixture")
    );

    for page in 1..=2 {
        let path = output_dir.join(format!("pages/page-{page:03}.png"));
        let image = image::open(&path)
            .unwrap_or_else(|error| panic!("failed to decode {}: {error}", path.display()));
        assert_eq!(image.dimensions(), (1280, 720));
    }
    for state in 1..=4 {
        let path = output_dir.join(format!("screen-pages/state-{state:03}.png"));
        let image = image::open(&path)
            .unwrap_or_else(|error| panic!("failed to decode {}: {error}", path.display()));
        assert_eq!(image.dimensions(), (1280, 720));
    }
    let contact_sheet = image::open(output_dir.join("contact-sheet.png")).unwrap();
    assert!(contact_sheet.width() > 0);
    assert!(contact_sheet.height() > 0);
    let screen_contact_sheet = image::open(output_dir.join("screen-contact-sheet.png")).unwrap();
    assert!(screen_contact_sheet.width() > 0);
    assert!(screen_contact_sheet.height() > 0);
    assert!(output_dir.join("print.html").is_file());
    assert!(output_dir.join("visual-report.txt").is_file());

    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["schema_version"], 9);
    assert_eq!(
        report["room_profile"]["selection"],
        "projected-room-default"
    );
    assert_eq!(report["room_profile"]["enforcement_status"], "report_only");
    let calibration: Value = serde_json::from_str(
        &fs::read_to_string(output_dir.join("room-profile-calibration.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(calibration["schema_version"], 1);
    assert_eq!(calibration["profile_selection"], "projected-room-default");
    assert_eq!(calibration["surfaces"]["screen"]["observation_count"], 4);
    assert_eq!(calibration["surfaces"]["print"]["observation_count"], 2);
    assert_eq!(report["room_profile_calibration"], calibration);
    assert_eq!(report["contract_status"], "valid");
    assert_eq!(report["visual_status"], "passed");
    assert_eq!(report["objective_gate_status"], "passed");
    assert_eq!(report["screen_status"], "passed");
    assert_eq!(report["print_status"], "passed");
    assert_eq!(report["human_review"], "required");
    assert_eq!(report["composition_review_status"], "required");
    assert_eq!(report["release_status"], "pending-review");
    assert_eq!(report["final_source_release_approval"], "pending-review");
    assert_eq!(report["expected_pages"], 2);
    assert_eq!(report["captured_pages"], 2);
    assert_eq!(report["expected_screen_slides"], 2);
    assert_eq!(report["captured_screen_slides"], 2);
    assert_eq!(report["expected_screen_states"], 4);
    assert_eq!(report["captured_screen_states"], 4);
    assert!(report["screen_failures"].as_array().unwrap().is_empty());
    assert!(report["print_failures"].as_array().unwrap().is_empty());
    assert_eq!(report["failures"].as_array().unwrap().len(), 0);
    assert!(
        report["design_summary"]["measurement_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        report["design_summary"]["min_essential_type_px"]
            .as_f64()
            .is_some_and(|size| size > 0.0)
    );
    let first_measurement = &report["screen_states"][0]["design_measurements"][0];
    assert_eq!(first_measurement["surface"], "screen");
    assert_eq!(first_measurement["palette"], "default");
    assert_eq!(first_measurement["slide_id"], "section-1-main");
    assert!(first_measurement["element"].as_str().is_some());
    assert!(first_measurement["type_role"].as_str().is_some());
    let pages = report["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["slide_id"], "section-1-main");
    assert_eq!(pages[0]["title"], "First visual page");
    assert_eq!(pages[0]["screenshot"], "pages/page-001.png");
    assert_eq!(pages[1]["slide_id"], "section-2-main");
    assert_eq!(pages[1]["title"], "Second visual page");
    assert_eq!(pages[1]["screenshot"], "pages/page-002.png");
    let screen_states = report["screen_states"].as_array().unwrap();
    assert_eq!(screen_states.len(), 4);
    assert_eq!(screen_states[0]["route"]["hash"], "#/0/0");
    assert_eq!(screen_states[0]["slide_id"], "section-1-main");
    assert_eq!(screen_states[0]["screenshot"], "screen-pages/state-001.png");
    assert_eq!(screen_states[1]["route"]["hash"], "#/0/0/1");
    assert_eq!(screen_states[2]["route"]["hash"], "#/0/0/2");
    assert_eq!(screen_states[3]["route"]["hash"], "#/1/0");
    assert_eq!(screen_states[3]["slide_id"], "section-2-main");
    assert_eq!(screen_states[3]["screenshot"], "screen-pages/state-004.png");
    assert_eq!(report["artifacts"]["screen_html"], "index.html");
    assert_eq!(report["artifacts"]["screen_pages_dir"], "screen-pages");
    assert_eq!(
        report["artifacts"]["screen_contact_sheet"],
        "screen-contact-sheet.png"
    );
    assert_eq!(report["artifacts"]["print_html"], "print.html");
    assert_eq!(report["artifacts"]["pages_dir"], "pages");
    assert_eq!(report["artifacts"]["contact_sheet"], "contact-sheet.png");
    assert_eq!(
        report["artifacts"]["visual_report_json"],
        "visual-report.json"
    );
    assert_eq!(
        report["artifacts"]["room_profile_calibration"],
        "room-profile-calibration.json"
    );
    assert_eq!(report["artifacts"]["provenance"], "provenance.json");

    let provenance: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["schema_version"], 6);
    assert_eq!(provenance["capture_mode"], "normal");
    assert_eq!(
        provenance["room_profile"]["selection"],
        "projected-room-default"
    );
    assert!(provenance["generated_at_unix_seconds"].as_u64().unwrap() > 0);
    assert_eq!(
        provenance["source_path"],
        fixture.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(provenance["theme_name"], "visual-test");
    assert_eq!(provenance["theme_version"], "0.1.0");
    assert_eq!(provenance["theme_api_version"], 1);
    assert!(
        provenance["theme_manifest_path"]
            .as_str()
            .unwrap()
            .ends_with("visual-test/theme.toml")
    );
    assert!(provenance["theme_params"].as_object().unwrap().is_empty());
    assert_eq!(provenance["viewport"]["width"], 1280);
    assert_eq!(provenance["viewport"]["height"], 720);
    assert_eq!(provenance["viewport"]["device_scale_factor"], 1);
    assert!(!provenance["platform"]["os"].as_str().unwrap().is_empty());
    assert!(
        !provenance["platform"]["architecture"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert!(
        !provenance["chromium"]["product"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert!(
        !provenance["chromium_executable"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    let commit = provenance["git"]["commit"].as_str().unwrap();
    assert_eq!(commit.len(), 40);
    assert!(
        commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    assert!(provenance["git"]["dirty"].is_boolean());
    assert_eq!(provenance["source_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(
        provenance["theme_package_sha256"].as_str().unwrap().len(),
        64
    );
    assert_eq!(provenance["theme_files"].as_array().unwrap().len(), 3);

    let text_report = fs::read_to_string(output_dir.join("visual-report.txt")).unwrap();
    assert!(text_report.contains("contract_status = valid"));
    assert!(text_report.contains("theme_api_version = 1"));
    assert!(text_report.contains("visual_status = passed"));
    assert!(text_report.contains("screen_status = passed"));
    assert!(text_report.contains("print_status = passed"));
    assert!(text_report.contains("human_review = required"));
    assert!(text_report.contains("release_status = pending-review"));
    let theme_check = fs::read_to_string(output_dir.join("theme-check.txt")).unwrap();
    assert!(theme_check.contains("fixture_feature_coverage = incomplete"));
    assert!(theme_check.contains("fixture_missing_declared_features = fit-text"));
}

#[test]
fn visual_theme_check_fails_after_writing_the_overflowing_slide_report() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("visual-overflow-test");
    let fixture = temp.path().join("overflow.zp.md");
    let output_dir = temp.path().join("review");
    write_overflowing_visual_test_theme(&theme_dir);
    write_overflowing_fixture(&fixture);

    let output = Command::cargo_bin("zpres")
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
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed visual check: {stderr}");
        return;
    }

    assert!(
        !output.status.success(),
        "overflowing visual check unexpectedly passed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("contract_status = valid"));
    assert!(stdout.contains("visual_status = failed"));
    assert!(stdout.contains("screen_status = failed"));
    assert!(stdout.contains("print_status = failed"));
    assert!(stdout.contains("human_review = required"));
    assert!(stdout.contains("release_status = blocked"));
    assert!(stderr.contains("browser-rendered visual gate failed"));

    assert!(output_dir.join("pages/page-001.png").is_file());
    assert!(output_dir.join("screen-pages/state-001.png").is_file());
    assert!(output_dir.join("contact-sheet.png").is_file());
    assert!(output_dir.join("screen-contact-sheet.png").is_file());
    assert!(output_dir.join("visual-report.txt").is_file());
    assert!(output_dir.join("provenance.json").is_file());
    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["contract_status"], "valid");
    assert_eq!(report["visual_status"], "failed");
    assert_eq!(report["screen_status"], "failed");
    assert_eq!(report["print_status"], "failed");
    assert_eq!(report["release_status"], "blocked");
    let failures = report["failures"].as_array().unwrap();
    assert!(failures.iter().any(|finding| {
        matches!(
            finding["code"].as_str(),
            Some("content-outside-canvas" | "clipped-content")
        ) && finding["slide_id"] == "section-1-main"
    }));
    assert!(
        report["screen_failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| {
                matches!(
                    finding["code"].as_str(),
                    Some("content-outside-canvas" | "clipped-content")
                ) && finding["slide_id"] == "section-1-main"
                    && finding["route"] == "#/0/0"
            })
    );
}

#[test]
fn visual_theme_check_accepts_content_that_fits_after_autoscale() {
    let temp = tempdir().unwrap();
    let output_dir = temp.path().join("review");

    let output = Command::cargo_bin("zpres")
        .unwrap()
        .args([
            "theme",
            "check",
            "themes/debug",
            "--visual",
            "--fixture",
            "fixtures/calibration/room-profile-calibration.zp.md",
            "--write-specimen",
            output_dir.to_str().unwrap(),
            "--room-profile",
            "room-profiles/projected-room-default.toml",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed visual check: {stderr}");
        return;
    }
    assert!(
        output.status.success(),
        "autoscale calibration unexpectedly failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["print_status"], "passed");
    assert_eq!(report["screen_status"], "passed");

    for (surface, key) in [("screen", "screen_states"), ("print", "pages")] {
        let observations = report[key].as_array().unwrap();
        for (slide_id, expected) in [("section-9-main", 0.92), ("section-10-main", 0.80)] {
            let observation = observations
                .iter()
                .find(|observation| observation["slide_id"] == slide_id)
                .unwrap();
            let factor = observation["autoscale_factor"].as_f64().unwrap();
            assert!(
                (factor - expected).abs() <= 0.08,
                "{surface} {slide_id} factor {factor} did not bracket {expected}"
            );
            assert_eq!(observation["clip_marker"], false);
        }
    }
}

#[test]
fn v1_percentage_columns_and_gap_fit_the_safe_content_track() {
    let temp = tempdir().unwrap();
    let theme = temp.path().join("percentage-columns-theme");
    let fixture = temp.path().join("percentage-columns.zp.md");
    let output_dir = temp.path().join("review");
    write_v1_autoscale_test_theme(&theme, ".zpres-slide-primary { max-width: none; }");
    fs::write(
        &fixture,
        r#"---
title: "Percentage Columns autoscale fixture"
aspect: "16:9"
autoscale: true
---

# Percentage Columns keep their gap

:::: {.columns}
::: {.column width="35%" name="Evidence"}
The first authored region remains inside its track.
:::
::: {.column width="65%" name="Explanation"}
The second authored region and the shared gap fit the safe content lane.
:::
::::
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("zpres")
        .unwrap()
        .args([
            "theme",
            "check",
            theme.to_str().unwrap(),
            "--visual",
            "--fixture",
            fixture.to_str().unwrap(),
            "--write-specimen",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed percentage Columns regression: {stderr}");
        return;
    }
    let report_debug = fs::read_to_string(output_dir.join("visual-report.json"))
        .unwrap_or_else(|_| "<visual report unavailable>".to_string());
    assert!(
        output.status.success(),
        "percentage Columns unexpectedly failed\nstdout:\n{stdout}\nstderr:\n{stderr}\nreport:\n{report_debug}"
    );

    let report: Value = serde_json::from_str(&report_debug).unwrap();
    for (surface, key) in [("screen", "screen_states"), ("print", "pages")] {
        let observation = &report[key].as_array().unwrap()[0];
        assert_eq!(
            observation["clip_marker"], false,
            "{surface} retained a false percentage Columns clip marker: {observation}"
        );
        assert!(
            observation["autoscale_factor"].is_null(),
            "{surface} percentage Columns should fit without autoscaling: {observation}"
        );
        assert!(
            observation["visible_content_bounds"]["right"]
                .as_f64()
                .unwrap()
                <= observation["content_bounds"]["right"].as_f64().unwrap() + 2.0,
            "{surface} percentage Columns plus gap exceeded the safe content track: {observation}"
        );
    }
}

#[test]
fn v1_autoscale_allows_theme_surface_side_bleed_only_with_safe_authored_evidence() {
    let temp = tempdir().unwrap();
    let theme = temp.path().join("side-bleed-theme");
    let fixture = temp.path().join("side-bleed.zp.md");
    let output_dir = temp.path().join("review");
    write_v1_autoscale_test_theme(
        &theme,
        r#"
.zpres-slide-primary { max-width: none; }
.zpres-slide[data-slide-variant="derivation"] .zpres-layout-region[data-derivation-role] {
  width: 100%;
  margin-inline-start: 40px;
}
"#,
    );
    fs::write(
        &fixture,
        r#"---
title: "Theme treatment side-bleed fixture"
aspect: "16:9"
autoscale: true
---

# The semantic evidence remains in the safe lane

::::: derivation
:::: context label="Invariant"
The stable authored statement stays clear of the outer treatment.
::::
:::: stage label="Result"
The changing authored evidence also stays inside the safe lane.

```mermaid
flowchart LR
  safe[Safe evidence] --> result[Safe result]
```
::::
:::::
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("zpres")
        .unwrap()
        .args([
            "theme",
            "check",
            theme.to_str().unwrap(),
            "--visual",
            "--fixture",
            fixture.to_str().unwrap(),
            "--write-specimen",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed Theme treatment regression: {stderr}");
        return;
    }
    let report_debug = fs::read_to_string(output_dir.join("visual-report.json"))
        .unwrap_or_else(|_| "<visual report unavailable>".to_string());
    let report: Value = serde_json::from_str(&report_debug).unwrap();
    let unexpected_failures = ["screen_failures", "print_failures"]
        .into_iter()
        .flat_map(|key| report[key].as_array().unwrap())
        .filter(|failure| failure["code"] != "insufficient-non-text-contrast")
        .collect::<Vec<_>>();
    assert!(
        output.status.success() || unexpected_failures.is_empty(),
        "bounded Theme treatment unexpectedly failed\nstdout:\n{stdout}\nstderr:\n{stderr}\nunexpected failures:\n{unexpected_failures:?}"
    );

    for (surface, key) in [("screen", "screen_states"), ("print", "pages")] {
        for observation in report[key].as_array().unwrap() {
            let content_right = observation["content_bounds"]["right"].as_f64().unwrap();
            let treatment_right = observation["visible_content_bounds"]["right"]
                .as_f64()
                .unwrap();
            let canvas_right = observation["canvas_bounds"]["right"].as_f64().unwrap();
            assert!(
                treatment_right > content_right + 20.0,
                "{surface} did not exercise intentional container side-bleed: {observation}"
            );
            assert!(
                treatment_right <= canvas_right + 2.0,
                "{surface} Theme treatment crossed the canvas: {observation}"
            );
            assert!(
                max_body_text_edge(observation, "right") <= content_right + 2.0,
                "{surface} authored glyphs crossed the safe content lane: {observation}"
            );
            assert_eq!(
                observation["clip_marker"], false,
                "{surface} retained a false Theme-treatment clip marker: {observation}"
            );
        }
    }
}

#[test]
fn v1_autoscale_rejects_authored_glyphs_three_pixels_beyond_the_safe_lane() {
    let temp = tempdir().unwrap();
    let theme = temp.path().join("glyph-overflow-theme");
    let fixture = temp.path().join("glyph-overflow.zp.md");
    let output_dir = temp.path().join("review");
    write_v1_autoscale_test_theme(
        &theme,
        r#"
.zpres-slide-content { position: relative; }
.zpres-slide-primary { max-width: none; }
.zpres-block-paragraph {
  position: absolute;
  inset-block-start: 12rem;
  inset-inline-end: -5px;
  width: max-content;
}
"#,
    );
    fs::write(
        &fixture,
        r#"---
title: "Semantic glyph overflow fixture"
aspect: "16:9"
autoscale: true
---

# A real glyph overflow remains a failure

This authored text edge must stay outside.
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("zpres")
        .unwrap()
        .args([
            "theme",
            "check",
            theme.to_str().unwrap(),
            "--visual",
            "--fixture",
            fixture.to_str().unwrap(),
            "--write-specimen",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed semantic glyph overflow regression: {stderr}");
        return;
    }
    assert!(!output.status.success());

    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();
    for (surface, key) in [("screen", "screen_states"), ("print", "pages")] {
        let observation = &report[key].as_array().unwrap()[0];
        let content_right = observation["content_bounds"]["right"].as_f64().unwrap();
        let glyph_right = max_body_text_edge(observation, "right");
        assert_eq!(
            observation["autoscale_factor"], 0.62,
            "{surface} must reach the hard floor for a scale-resistant glyph overflow"
        );
        assert_eq!(
            observation["clip_marker"], true,
            "{surface} must retain a true semantic-glyph clip marker"
        );
        assert!(
            glyph_right > content_right + 3.0 && glyph_right < content_right + 3.3,
            "{surface} control must place the authored Range edge about 3.1px beyond the safe lane: {observation}"
        );
    }
}

#[test]
fn visual_theme_check_keeps_unrecoverable_autoscale_clipped_at_the_minimum() {
    let temp = tempdir().unwrap();
    let fixture = temp.path().join("unrecoverable.zp.md");
    let output_dir = temp.path().join("review");
    write_unrecoverable_autoscale_fixture(&fixture);

    let output = Command::cargo_bin("zpres")
        .unwrap()
        .args([
            "theme",
            "check",
            "themes/debug",
            "--visual",
            "--fixture",
            fixture.to_str().unwrap(),
            "--write-specimen",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Chrome/Chromium was not found")
        && stderr.contains("ZPRES_CHROMIUM")
    {
        eprintln!("skipping browser-backed visual check: {stderr}");
        return;
    }
    assert!(!output.status.success());

    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();
    for key in ["screen_states", "pages"] {
        let observation = &report[key].as_array().unwrap()[0];
        assert_eq!(observation["autoscale_factor"], 0.62);
        assert_eq!(observation["clip_marker"], true);
        assert!(
            max_body_text_edge(observation, "bottom")
                > observation["content_bounds"]["bottom"].as_f64().unwrap() + 2.0,
            "the unrecoverable control must retain a true semantic-glyph overflow: {observation}"
        );
    }
}

#[test]
fn visual_theme_check_rejects_unsupported_aspect_before_rendering() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("visual-test");
    let fixture = temp.path().join("four-three.zp.md");
    let output_dir = temp.path().join("review");
    write_visual_test_theme(&theme_dir);
    write_four_three_fixture(&fixture);

    let output = Command::cargo_bin("zpres")
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
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("supports only 16:9"), "{stderr}");
    assert!(!output_dir.join("visual-report.json").exists());
}
