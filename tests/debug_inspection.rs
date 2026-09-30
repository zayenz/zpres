use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use assert_cmd::Command;
use image::GenericImageView;
use serde_json::Value;
use tempfile::tempdir;

fn run_debug_capture(output: &Path, inspection: bool) {
    let mut command = Command::cargo_bin("zpres").unwrap();
    command.args([
        "theme",
        "check",
        "themes/debug",
        "--visual",
        "--write-specimen",
        output.to_str().unwrap(),
    ]);
    if inspection {
        command.arg("--inspection");
    }
    command.assert().success();
}

fn report(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path.join("visual-report.json")).unwrap()).unwrap()
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

#[test]
fn debug_inspection_is_complete_reproducible_and_geometry_invariant() {
    let temp = tempdir().unwrap();
    let normal_dir = temp.path().join("normal");
    let inspection_dir = temp.path().join("inspection");
    run_debug_capture(&normal_dir, false);
    run_debug_capture(&inspection_dir, true);

    let normal = report(&normal_dir);
    let inspection = report(&inspection_dir);
    assert_eq!(normal["objective_gate_status"], "passed");
    assert_eq!(inspection["objective_gate_status"], "passed");
    assert_eq!(normal["screen_states"].as_array().unwrap().len(), 27);
    assert_eq!(inspection["screen_states"].as_array().unwrap().len(), 27);
    assert_eq!(normal["pages"].as_array().unwrap().len(), 17);
    assert_eq!(inspection["pages"].as_array().unwrap().len(), 17);

    for surface in ["screen_states", "pages"] {
        let normal_items = normal[surface].as_array().unwrap();
        let inspection_items = inspection[surface].as_array().unwrap();
        for (normal_item, inspection_item) in normal_items.iter().zip(inspection_items) {
            assert_eq!(normal_item["slide_id"], inspection_item["slide_id"]);
            assert_eq!(normal_item["role"], inspection_item["role"]);
            for field in [
                "slide_bounds",
                "canvas_bounds",
                "content_bounds",
                "footer_bounds",
                "block_bounds",
                "autoscale_factor",
            ] {
                assert_eq!(
                    normal_item[field], inspection_item[field],
                    "inspection changed {surface} geometry field {field} for {}",
                    normal_item["slide_id"]
                );
            }
            assert!(normal_item.get("debug_inspection").is_none());
            let debug = &inspection_item["debug_inspection"];
            assert_eq!(
                debug["identity"],
                format!("Slide {}", inspection_item["slide_id"].as_str().unwrap())
            );
            assert!(debug["role"].as_str().unwrap().contains("Section "));
            assert!(debug["role"].as_str().unwrap().ends_with(&format!(
                "· {}",
                title_case(inspection_item["role"].as_str().unwrap())
            )));
            if surface == "screen_states" {
                assert_eq!(debug["target"], "HTML presentation");
                assert!(
                    debug["route"]
                        .as_str()
                        .unwrap()
                        .contains(inspection_item["route"]["hash"].as_str().unwrap())
                );
            } else {
                assert_eq!(debug["target"], "PDF export");
                assert!(debug["route"].as_str().unwrap().contains(&format!(
                    "Page {}",
                    inspection_item["page"].as_u64().unwrap()
                )));
            }
            assert!(debug["route"].as_str().unwrap().contains("Step "));
            assert!(matches!(
                debug["status"].as_str().unwrap(),
                "PASS" | "WARNING"
            ));
            assert!(
                debug["diagnostics"]
                    .as_str()
                    .unwrap()
                    .starts_with(debug["status"].as_str().unwrap())
            );
            if surface == "pages" {
                assert!(
                    debug["print_rail"]
                        .as_str()
                        .unwrap()
                        .starts_with("PDF PAGE ")
                );
            }
        }
    }

    let step_summaries = inspection["screen_states"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["slide_id"] == "section-5-main")
        .map(|item| item["debug_inspection"]["step_summary"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        step_summaries
            .iter()
            .any(|summary| summary.contains("QUEUED 3"))
    );
    assert!(
        step_summaries
            .iter()
            .any(|summary| summary.contains("CURRENT 1"))
    );
    assert!(
        step_summaries
            .iter()
            .any(|summary| summary.contains("COMPLETED 2"))
    );

    let process_rails = inspection["pages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["slide_id"] == "section-10-main")
        .map(|item| item["debug_inspection"]["print_rail"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(process_rails.len(), 3);
    assert!(
        process_rails
            .iter()
            .all(|rail| rail.contains("one page per step") && rail.contains("through Step"))
    );

    let warning_page = inspection["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["slide_id"] == "section-13-main")
        .unwrap();
    let autoscale_factor = warning_page["autoscale_factor"].as_f64().unwrap();
    assert!(
        (0.8..1.0).contains(&autoscale_factor),
        "expected a bounded sub-100% autoscale factor, got {autoscale_factor}"
    );
    assert_eq!(warning_page["debug_inspection"]["status"], "WARNING");
    assert!(
        warning_page["debug_inspection"]["print_rail"]
            .as_str()
            .unwrap()
            .contains("WARNING · autoscale")
    );
    assert!(
        warning_page["debug_inspection"]["painted_rail"]
            .as_str()
            .unwrap()
            .contains("WARNING · autoscale")
    );
    let warning_raster = image::open(inspection_dir.join("pages/page-017.png")).unwrap();
    let warning_pixels = warning_raster
        .pixels()
        .filter(|(_, y, pixel)| {
            *y < 80 && pixel[0].abs_diff(180) < 8 && pixel[1].abs_diff(83) < 8 && pixel[2] < 20
        })
        .count();
    assert!(
        warning_pixels > 100,
        "the page-17 raster did not paint the amber WARNING rail"
    );

    let observed_regions = inspection["screen_states"]
        .as_array()
        .unwrap()
        .iter()
        .chain(inspection["pages"].as_array().unwrap())
        .flat_map(|item| item["debug_inspection"]["regions"].as_array().unwrap())
        .map(|region| {
            assert!(!region["bounds"].as_str().unwrap().is_empty());
            assert!(!region["placement"].as_str().unwrap().is_empty());
            assert!(region["label_bounds"].is_object());
            assert!(
                region["authored_text_intersections"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "inspection boundary label intersects authored visible text: {region:?}"
            );
            region["role"].as_str().unwrap().to_string()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        observed_regions,
        [
            "body",
            "footer",
            "frame",
            "header",
            "ornament",
            "primary",
            "sources",
            "supporting",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );

    let normal_provenance: Value =
        serde_json::from_str(&fs::read_to_string(normal_dir.join("provenance.json")).unwrap())
            .unwrap();
    let inspection_provenance: Value =
        serde_json::from_str(&fs::read_to_string(inspection_dir.join("provenance.json")).unwrap())
            .unwrap();
    assert_eq!(normal_provenance["capture_mode"], "normal");
    assert_eq!(inspection_provenance["capture_mode"], "inspection");

    for (surface, prefix, count) in [("screen-pages", "state", 27), ("pages", "page", 17)] {
        for index in 1..=count {
            let image = inspection_dir
                .join(surface)
                .join(format!("{prefix}-{index:03}.png"));
            assert_eq!(image::open(image).unwrap().dimensions(), (1280, 720));
        }
    }
}

#[test]
fn debug_report_only_specimen_retains_real_failure_without_faking_a_step_state() {
    let temp = tempdir().unwrap();
    let output = temp.path().join("report-only");
    let mut command = Command::cargo_bin("zpres").unwrap();
    command.args([
        "theme",
        "check",
        "themes/debug",
        "--fixture",
        "themes/debug/diagnostic-specimen.zp.md",
        "--visual",
        "--inspection",
        "--write-specimen",
        output.to_str().unwrap(),
    ]);
    command.assert().failure();

    let report = report(&output);
    assert_eq!(report["objective_gate_status"], "failed");
    assert!(
        report["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| {
                finding["code"].as_str().is_some_and(|code| {
                    code.contains("overflow") || code.contains("outside-canvas")
                })
            })
    );
    assert!(
        report["screen_states"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["debug_inspection"]["status"].as_str())
            .any(|status| status == "FAILED")
    );

    let print = fs::read_to_string(output.join("print.html")).unwrap();
    assert!(print.contains("data-zpres-print-step-state=\"future\""));
    assert!(print.contains("data-zpres-print-step-state=\"active\""));
    assert!(!print.contains("data-zpres-print-step-state=\"failed\""));
}
