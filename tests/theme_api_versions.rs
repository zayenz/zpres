use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use image::GenericImageView;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::tempdir;

fn repository_path(relative: impl AsRef<Path>) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn zpres(temp: &Path) -> Command {
    let mut command = Command::cargo_bin("zpres").unwrap();
    command.env("XDG_CONFIG_HOME", temp.join("xdg"));
    command
}

fn current_html_generation(output_root: &Path) -> zpres::html::CurrentHtmlBundlePublication {
    zpres::html::current_html_bundle_publication(output_root)
        .unwrap()
        .expect("the build should publish an HTML generation")
}

fn normalized_rgb_rmse(left: &Path, right: &Path) -> f64 {
    let left = image::open(left).unwrap().to_rgb8();
    let right = image::open(right).unwrap().to_rgb8();
    assert_eq!(left.dimensions(), right.dimensions());
    let squared_error = left
        .as_raw()
        .iter()
        .zip(right.as_raw())
        .map(|(left, right)| {
            let delta = f64::from(*left) - f64::from(*right);
            delta * delta
        })
        .sum::<f64>();
    (squared_error / left.as_raw().len() as f64).sqrt() / 255.0
}

fn dark_rgb_pixels(path: &Path, x: u32, y: u32, width: u32, height: u32) -> usize {
    image::open(path)
        .unwrap()
        .to_rgb8()
        .view(x, y, width, height)
        .pixels()
        .filter(|pixel| {
            let [red, green, blue] = pixel.2.0;
            (u16::from(red) + u16::from(green) + u16::from(blue)) / 3 < 200
        })
        .count()
}

fn text_bound_contrast_pixels(path: &Path, bound: &Value) -> usize {
    let image = image::open(path).unwrap().to_rgb8();
    let x = bound["x"].as_f64().unwrap().floor().max(0.0) as u32;
    let y = bound["y"].as_f64().unwrap().floor().max(0.0) as u32;
    let right = bound["right"]
        .as_f64()
        .unwrap()
        .ceil()
        .min(f64::from(image.width())) as u32;
    let bottom = bound["bottom"]
        .as_f64()
        .unwrap()
        .ceil()
        .min(f64::from(image.height())) as u32;
    let mut colors = BTreeMap::<[u8; 3], usize>::new();
    for pixel in image.view(x, y, right - x, bottom - y).pixels() {
        *colors.entry(pixel.2.0).or_default() += 1;
    }
    let background = colors
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(color, _)| color)
        .unwrap();
    image
        .view(x, y, right - x, bottom - y)
        .pixels()
        .filter(|pixel| {
            pixel
                .2
                .0
                .iter()
                .zip(background)
                .map(|(channel, background)| channel.abs_diff(background))
                .max()
                .unwrap_or_default()
                >= 32
        })
        .count()
}

#[test]
fn built_in_debug_uses_the_v1_contract() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/debug/theme.toml")).unwrap();
    assert_eq!(manifest.api_version, 1);
    assert_eq!(manifest.name, "debug");
    assert_eq!(manifest.modules, vec!["scientific-data"]);
}

#[test]
fn minimal_v1_theme_consumes_scientific_data_module_without_science_css() {
    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/scientific-data-consumer.zp.md");
    let theme_dir = repository_path("fixtures/theme-api-v1/themes/scientific-data-consumer");
    let output = temp.path().join("consumer-html");
    let print_html = temp.path().join("consumer-print.html");

    zpres(temp.path())
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--fixture",
            source.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(&publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    let foundation = fs::read_to_string(
        publication
            .generation_path
            .join("assets/zpres-theme-api-v1.css"),
    )
    .unwrap();
    let theme_css =
        fs::read_to_string(publication.generation_path.join("assets/theme.css")).unwrap();

    for document in [&screen, &print] {
        assert!(document.contains("zpres-module-scientific-data"));
        assert!(document.contains("data-slide-variant=\"claim\""));
        assert!(document.contains("data-slide-variant=\"dense\""));
        assert!(document.contains("data-slide-variant=\"comparison\""));
        assert!(document.contains("data-slide-variant=\"derivation\""));
    }
    assert!(foundation.contains("@layer zpres-module"));
    assert!(foundation.contains("--zpres-data-claim-measure"));
    assert!(!theme_css.contains("zpres-theme-science"));
    assert!(!theme_css.contains("zpres-data-claim-measure"));
    assert!(
        theme_css.len() < 2_000,
        "consumer Theme copied too much CSS"
    );
}

#[test]
fn science_v1_palettes_and_role_parameters_are_effective() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/science/theme.toml")).unwrap();
    assert_eq!(manifest.api_version, 1);

    let light = zpres::theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
    let dark = zpres::theme::render_theme(
        &manifest,
        &BTreeMap::from([("mode".to_string(), "dark".to_string())]),
    )
    .unwrap();

    for marker in [
        "--science-bg: #e7ecea",
        "--science-text: #14171a",
        "Charter, \"Bitstream Charter\"",
        "Aptos, \"Avenir Next\"",
        "Menlo, Consolas",
        "font-size: var(--text-slide-title)",
        "data-slide-variant=\"claim\"",
        "data-slide-variant=\"figure\"",
        "data-slide-variant=\"comparison\"",
        "data-slide-variant=\"derivation\"",
        "data-slide-variant=\"dense\"",
        "data-slide-variant=\"section-title\"",
        "data-slide-role=\"detail\"",
        "grid-template-areas:",
        "data-step-state=\"active\"",
    ] {
        assert!(light.screen_css.contains(marker), "missing {marker}");
    }
    assert!(dark.screen_css.contains("--science-bg: #101820"));
    assert!(dark.screen_css.contains("--science-text: #eef4f1"));
    assert_ne!(light.screen_css, dark.screen_css);
    assert!(light.print_css.contains("#fbfcfd"));
    assert!(dark.print_css.contains("#172126"));
}

#[test]
fn paper_chalk_v1_evidence_compositions_and_night_palette_are_effective() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/paper-chalk/theme.toml"))
            .unwrap();
    assert_eq!(manifest.api_version, 1);
    assert_eq!(manifest.modules, vec!["scientific-data"]);
    assert_eq!(manifest.style.family.as_deref(), Some("field-notebook"));

    let light = zpres::theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
    let dark = zpres::theme::render_theme(
        &manifest,
        &BTreeMap::from([("mode".to_string(), "dark".to_string())]),
    )
    .unwrap();

    for marker in [
        "--paper-background: #f4f0e6",
        "--paper-blue: #1e4f9a",
        "--paper-rose: #a4475a",
        "--paper-evidence-surface",
        "--paper-evidence-mat",
        "zpres-slide-class-paper-evidence-canvas",
        "zpres-slide-class-paper-comparison-baseline",
        "zpres-slide-class-annotation-delta-bracket",
        "zpres-slide-class-paper-derivation-path",
        "zpres-slide-class-paper-detail-ledger",
        "zpres-slide-class-paper-detail-trace",
        "data-step-state=\"active\"",
    ] {
        assert!(light.screen_css.contains(marker), "missing {marker}");
    }
    let future_checkpoint = light
        .screen_css
        .split_once("[data-step-state=\"future\"]::before")
        .expect("future Derivation checkpoint selector")
        .1
        .split_once('}')
        .expect("future Derivation checkpoint rule")
        .0;
    assert!(future_checkpoint.contains("width: 2.5rem"));
    assert!(future_checkpoint.contains("height: 2.5rem"));
    assert!(future_checkpoint.contains("font-size: var(--text-slide-technical)"));
    let vector_evidence = light
        .screen_css
        .split_once("img[src$=\".svg\" i]")
        .expect("filter-free SVG evidence selector")
        .1
        .split_once('}')
        .expect("filter-free SVG evidence rule")
        .0;
    assert!(vector_evidence.contains("filter: none"));
    assert!(!vector_evidence.contains("brightness("));
    let dense_detail_figure = light
        .screen_css
        .split_once(
            ".zpres-slide[data-slide-role=\"detail\"][data-slide-variant=\"dense\"] .zpres-slide-primary:has(> .zpres-block-figure + .zpres-block-quote) .zpres-block-figure img",
        )
        .expect("unscaled Dense Detail Figure selector")
        .1
        .split_once('}')
        .expect("unscaled Dense Detail Figure rule")
        .0;
    assert!(dense_detail_figure.contains("max-height: 390px"));
    for marker in [
        "--paper-background: #101a22",
        "--paper-surface: #16232b",
        "--paper-ink: #f4f0df",
        "--paper-blue: #82afe8",
        "--paper-rose: #d88799",
        "--paper-rule: #415766",
    ] {
        assert!(dark.screen_css.contains(marker), "missing {marker}");
    }
    for marker in [
        ".zpres-theme-paper-chalk[data-zpres-palette=\"dark\"] .zpres-slide",
        ".zpres-chart-svg [data-chart-series-index=\"1\"]",
        "--paper-chart-series: var(--paper-blue)",
        "--paper-chart-series: var(--paper-rose)",
        ":is(.zpres-chart-line, .zpres-chart-mark)",
        ":is(.zpres-chart-point:not(.zpres-chart-direct-annotation-point), .zpres-chart-mark)",
        "background: var(--paper-evidence-mat)",
        ".zpres-block-figure img:not([src$=\".svg\" i])",
        ".zpres-gallery-item img:not([src$=\".svg\" i])",
        ".zpres-block-media :where(img:not([src$=\".svg\" i]), video)",
        "img[src$=\"paper-chalk-search-tree.svg\"]",
        "background: var(--paper-evidence-surface)",
        "filter: none",
    ] {
        assert!(dark.screen_css.contains(marker), "missing {marker}");
    }
    assert_ne!(light.screen_css, dark.screen_css);
    assert!(dark.print_css.contains("#101a22"));
    assert!(dark.print_css.contains("#82afe8"));
    assert!(dark.print_css.contains("#d88799"));
}

#[test]
fn paper_chalk_evidence_specimen_keeps_explicit_roles_on_screen_and_print() {
    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/paper-chalk-evidence.zp.md");
    let output = temp.path().join("paper-chalk-evidence-html");
    let print_html = temp.path().join("paper-chalk-evidence-print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    for document in [&screen, &print] {
        for marker in [
            "zpres-slide-class-paper-evidence-canvas",
            "zpres-slide-class-paper-comparison-baseline",
            "zpres-slide-class-annotation-delta-bracket",
            "data-comparison-role=\"primary\"",
            "data-comparison-role=\"supporting\"",
            "zpres-slide-class-paper-derivation-path",
            "data-derivation-role=\"context\"",
            "data-derivation-role=\"stage\"",
            "zpres-slide-class-paper-detail-ledger",
            "zpres-slide-class-paper-detail-trace",
            "data-slide-role=\"detail\"",
            "zpres-slide-class-annotation-result-bracket",
            "zpres-chart-line",
            "zpres-chart-point",
            "zpres-chart-mark",
            "data-chart-series-index=\"0\"",
        ] {
            assert!(document.contains(marker), "missing {marker}");
        }
    }
    assert!(screen.contains("data-step-state=\"future\""));
    assert!(print.contains("data-step-state=\"complete\""));
    assert!(screen.contains("zpres-block-chart"));
    assert!(screen.contains("zpres-block-figure"));
    assert!(screen.contains("Consequence · The shared baseline"));

    let dark_source = temp.path().join("paper-chalk-dark.zp.md");
    let dark_output = temp.path().join("paper-chalk-dark-html");
    let dark_print_html = temp.path().join("paper-chalk-dark-print.html");
    fs::write(
        &dark_source,
        format!(
            "---\ntitle: Dark palette selector probe\ntheme: paper-chalk\ntheme_dirs:\n  - \"{}\"\ntheme_params:\n  mode: dark\naspect: \"16:9\"\n---\n\n# Dark palette selector probe\n\nThe body palette hook and Theme selector must agree.\n",
            repository_path("themes").display()
        ),
    )
    .unwrap();
    zpres(temp.path())
        .args([
            "build",
            dark_source.to_str().unwrap(),
            "--out",
            dark_output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            dark_source.to_str().unwrap(),
            "--print-html",
            dark_print_html.to_str().unwrap(),
        ])
        .assert()
        .success();
    let dark_publication = current_html_generation(&dark_output);
    let dark_screen = fs::read_to_string(dark_publication.presentation_index).unwrap();
    let dark_print = fs::read_to_string(dark_print_html).unwrap();
    for document in [&dark_screen, &dark_print] {
        assert!(document.contains("data-zpres-palette=\"dark\""));
    }
}

#[test]
fn paper_chalk_reference_reveal_keeps_initial_context_and_complete_print() {
    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/paper-chalk-reference.zp.md");
    let output = temp.path().join("paper-chalk-reference-html");
    let print_html = temp.path().join("paper-chalk-reference-print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    let screen_slide = screen
        .split_once("data-slide-id=\"section-7-main\"")
        .expect("reference Derivation in the HTML presentation")
        .1
        .split_once("</section>")
        .expect("reference Derivation closing element")
        .0;
    assert!(screen_slide.contains("data-code-reveal-steps=\"3\""));
    for line in [1, 2] {
        assert!(
            screen_slide.contains(&format!(
                "zpres-code-line is-visible\" data-line=\"{line}\""
            )),
            "line {line} should remain visible as initial context"
        );
        assert!(
            !screen_slide.contains(&format!("zpres-code-line fragment\" data-line=\"{line}\"")),
            "line {line} should not be Step-gated"
        );
    }
    for (line, step) in [(3, 1), (4, 2), (5, 3), (6, 3)] {
        assert!(screen_slide.contains(&format!(
            "zpres-code-line fragment\" data-line=\"{line}\" data-step-index=\"{step}\""
        )));
    }

    let print_slide = print
        .split_once("data-slide-id=\"section-7-main\"")
        .expect("reference Derivation in print HTML")
        .1
        .split_once("</section>")
        .expect("reference print Derivation closing element")
        .0;
    assert!(print_slide.contains("data-code-reveal-steps=\"3\""));
    for text in [
        "choose a base workload B",
        "select a scaling factor k",
        "lift B onto the k-input",
        "apply the sampling policy C",
        "validate the instance",
        "record generator and seed",
    ] {
        assert!(
            print_slide.contains(text),
            "missing final print line: {text}"
        );
    }
}

#[test]
fn sv_v1_palettes_fonts_and_workbench_treatments_are_effective() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/sv/theme.toml")).unwrap();
    assert_eq!(manifest.api_version, 1);
    assert_eq!(manifest.modules, vec!["scientific-data"]);
    assert_eq!(
        manifest.style.family.as_deref(),
        Some("propagation-workbench")
    );

    let light = zpres::theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
    let dark = zpres::theme::render_theme(
        &manifest,
        &BTreeMap::from([
            ("mode".to_string(), "dark".to_string()),
            ("accent".to_string(), "#d0a7e8".to_string()),
            ("accent_alt".to_string(), "#f2c45b".to_string()),
            ("background".to_string(), "#111419".to_string()),
            ("surface".to_string(), "#1a1e24".to_string()),
            ("text".to_string(), "#f0f2ec".to_string()),
            ("muted".to_string(), "#aeb6ae".to_string()),
            ("rule".to_string(), "#3b4248".to_string()),
            (
                "font_heading".to_string(),
                "Impact, ui-sans-serif, sans-serif".to_string(),
            ),
            (
                "font_body".to_string(),
                "Aptos, ui-sans-serif, sans-serif".to_string(),
            ),
            (
                "font_mono".to_string(),
                "Menlo, ui-monospace, monospace".to_string(),
            ),
            ("footer".to_string(), "none".to_string()),
        ]),
    )
    .unwrap();

    for marker in [
        "--sv-background: #eef0ec",
        "--sv-accent: #713f8f",
        "Avenir Next Condensed",
        "Avenir Next",
        "SFMono-Regular",
        "--zpres-data-dense-measure: none",
        "data-slide-variant=\"claim\"",
        "data-slide-variant=\"comparison\"",
        "data-slide-variant=\"figure\"",
        "data-slide-variant=\"dense\"",
    ] {
        assert!(light.screen_css.contains(marker), "missing {marker}");
    }
    for marker in [
        "--sv-background: #111419",
        "--sv-surface: #1a1e24",
        "--sv-text: #f0f2ec",
        "--sv-muted: #aeb6ae",
        "--sv-accent: #d0a7e8",
        "--sv-accent-alt: #f2c45b",
        "--sv-rule: #3b4248",
        "font-family: Impact, ui-sans-serif, sans-serif",
        "font-family: Aptos, ui-sans-serif, sans-serif",
        "font-family: Menlo, ui-monospace, monospace",
        "--zpres-param-footer: none",
    ] {
        assert!(dark.screen_css.contains(marker), "missing {marker}");
    }
    assert_ne!(light.screen_css, dark.screen_css);
    assert!(dark.print_css.contains("#111419"));
    assert!(dark.print_css.contains("#d0a7e8"));
}

#[test]
fn sv_reference_reveal_keeps_initial_context_and_complete_print() {
    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/sv-reference.zp.md");
    let output = temp.path().join("sv-reference-html");
    let print_html = temp.path().join("sv-reference-print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    let screen_slide = screen
        .split_once("data-slide-id=\"section-4-main\"")
        .expect("reference generic Step in the HTML presentation")
        .1
        .split_once("</section>")
        .expect("reference generic Step closing element")
        .0;
    assert!(screen_slide.contains("data-code-reveal-steps=\"3\""));
    assert!(!screen_slide.contains("zpres-slide-class-sv-state-"));
    for line in [1, 2] {
        assert!(
            screen_slide.contains(&format!(
                "zpres-code-line is-visible\" data-line=\"{line}\""
            )),
            "line {line} should remain visible as initial context"
        );
        assert!(
            !screen_slide.contains(&format!("zpres-code-line fragment\" data-line=\"{line}\"")),
            "line {line} should not be Step-gated"
        );
    }
    for (line, step) in [(3, 1), (4, 2), (5, 3), (6, 3)] {
        assert!(screen_slide.contains(&format!(
            "zpres-code-line fragment\" data-line=\"{line}\" data-step-index=\"{step}\""
        )));
    }

    let print_slide = print
        .split_once("data-slide-id=\"section-4-main\"")
        .expect("reference generic Step in print HTML")
        .1
        .split_once("</section>")
        .expect("reference print generic Step closing element")
        .0;
    assert!(print_slide.contains("data-code-reveal-steps=\"3\""));
    for text in [
        "A ← active sites",
        "T ← remaining thresholds",
        "build conflict graph Gₜ[A]",
        "compute a maximal matching M",
        "refute t when |A| - |M| &lt; p",
        "commit the largest surviving level",
    ] {
        assert!(
            print_slide.contains(text),
            "missing final print line: {text}"
        );
    }
}

#[test]
fn sv_unannotated_furniture_rebinds_inherited_accent_on_screen_and_print() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/sv/theme.toml")).unwrap();

    for parameters in [
        BTreeMap::new(),
        BTreeMap::from([("mode".to_string(), "dark".to_string())]),
    ] {
        let rendered = zpres::theme::render_theme(&manifest, &parameters).unwrap();
        let neutral_boundary = rendered
            .screen_css
            .split_once(".zpres-theme-sv .zpres-slide {")
            .expect("SV Slide-local neutral accent boundary")
            .1
            .split_once('}')
            .expect("SV Slide-local neutral accent rule")
            .0;
        assert!(neutral_boundary.contains("--zpres-color-accent: var(--sv-muted);"));
        assert!(neutral_boundary.contains("--zpres-color-accent-alt: var(--sv-rule);"));
        assert!(!neutral_boundary.contains("var(--sv-current)"));
        assert!(!neutral_boundary.contains("var(--sv-outcome)"));
    }

    let temp = tempdir().unwrap();
    let source = temp.path().join("sv-neutral-furniture.zp.md");
    let output = temp.path().join("sv-neutral-furniture-html");
    let print_html = temp.path().join("sv-neutral-furniture-print.html");
    fs::write(
        &source,
        format!(
            "---\ntheme: sv\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Scheduling is not propagation state\n\n> Advisors change scheduling and maintenance—not the filtering rule.\n\n> [!NOTE] Generic caveat\n> This callout is explanatory furniture, not current propagation.\n\n[Read the invariant](https://example.com/invariant).\n\n---\n\n# An overlay annotation stays neutral\n\n::::: overlay overlap=\"edge-only\"\n:::: base name=\"Evidence\"\n| Kernel | State |\n| --- | --- |\n| **Advisor queue** | stable |\n::::\n:::: annotation name=\"Caveat\" anchor=\"top-end\" width=\"compact\"\nNo causal relation is selected here.\n::::\n:::::\n\n---\n\n# Generic Derivation pacing stays neutral\n\n::::: derivation pdf=\"pages\"\n:::: context label=\"Context\"\nThe invariant holds before either explanation Step.\n::::\n:::: stage label=\"Explanation one\"\nDescribe the scheduling policy.\n::::\n:::: stage label=\"Explanation two\"\nDescribe the maintenance cost.\n::::\n:::::\n",
            repository_path("themes").display()
        ),
    )
    .unwrap();

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    let theme_css =
        fs::read_to_string(publication.generation_path.join("assets/theme.css")).unwrap();
    for document in [&screen, &print] {
        for marker in [
            "zpres-block-quote",
            "zpres-block-callout",
            "data-region-role=\"annotation\"",
            "<strong>Advisor queue</strong>",
            "https://example.com/invariant",
        ] {
            assert!(
                document.contains(marker),
                "missing neutral furniture {marker}"
            );
        }
        assert!(!document.contains("data-slide-classes=\"sv-state-"));
        assert!(!document.contains("data-slide-classes=\"sv-causal-trace"));
    }
    assert!(screen.contains("data-step-state=\"future\""));
    assert!(print.contains("data-step-state=\"active\""));
    for css in [&theme_css, &print] {
        assert!(css.contains("--zpres-color-accent: var(--sv-muted);"));
        assert!(css.contains("--zpres-color-accent-alt: var(--sv-rule);"));

        let generic_derivation_stack = css
            .split_once(
                ":not(.zpres-slide-class-sv-state-trace) .zpres-block-layout[data-derivation=\"true\"] {",
            )
            .expect("SV generic Derivation stack clearance rule")
            .1
            .split_once('}')
            .expect("SV generic Derivation stack rule body")
            .0;
        assert!(generic_derivation_stack.contains("--spacing-slide-3: var(--spacing-slide-4);"));

        let generic_derivation_region = css
            .split_once(
                ":not(.zpres-slide-class-sv-state-trace) .zpres-layout-region[data-derivation-role] {",
            )
            .expect("SV generic Derivation region clearance rule")
            .1
            .split_once('}')
            .expect("SV generic Derivation region rule body")
            .0;
        assert!(
            generic_derivation_region
                .contains("grid-template-columns: minmax(17rem, 0.35fr) minmax(0, 1fr);")
        );
        assert!(generic_derivation_region.contains("gap: var(--spacing-slide-6);"));
        assert!(
            generic_derivation_region
                .contains("padding: var(--spacing-slide-4) var(--spacing-slide-6);")
        );
        assert!(!generic_derivation_region.contains("--spacing-slide-5"));

        let generic_derivation_title = css
            .split_once(
                ":not(.zpres-slide-class-sv-state-trace) .zpres-layout-region[data-derivation-role=\"stage\"] .zpres-layout-region-title {",
            )
            .expect("SV generic Derivation title grid rule")
            .1
            .split_once('}')
            .expect("SV generic Derivation title rule body")
            .0;
        assert!(generic_derivation_title.contains("display: grid;"));
        assert!(generic_derivation_title.contains("grid-template-columns: 2rem minmax(0, 1fr);"));
        assert!(generic_derivation_title.contains("gap: var(--spacing-slide-2);"));

        let generic_derivation_marker = css
            .split_once(
                ":not(.zpres-slide-class-sv-state-trace) .zpres-layout-region[data-derivation-role=\"stage\"] .zpres-layout-region-title::before {",
            )
            .expect("SV generic Derivation marker clearance rule")
            .1
            .split_once('}')
            .expect("SV generic Derivation marker rule body")
            .0;
        assert!(generic_derivation_marker.contains("width: 2rem;"));
        assert!(generic_derivation_marker.contains("height: 2rem;"));
        assert!(generic_derivation_marker.contains("margin: 0;"));

        let authored_trace_region = css
            .split_once(
                ".zpres-slide.zpres-slide-class-sv-state-trace[data-slide-variant=\"derivation\"] .zpres-layout-region[data-derivation-role] {",
            )
            .expect("SV authored trace Derivation layout")
            .1
            .split_once('}')
            .expect("SV authored trace Derivation rule body")
            .0;
        assert!(
            authored_trace_region
                .contains("grid-template-columns: minmax(17rem, 0.4fr) minmax(0, 1fr);")
        );
        assert!(
            authored_trace_region
                .contains("padding: var(--spacing-slide-2) var(--spacing-slide-4);")
        );
    }
}

#[test]
fn sv_propagation_state_fixture_preserves_authored_state_and_snapshot_boundaries() {
    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/sv-propagation-states.zp.md");
    let output = temp.path().join("sv-state-html");
    let print_html = temp.path().join("sv-state-print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(&publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    let theme_css =
        fs::read_to_string(publication.generation_path.join("assets/theme.css")).unwrap();

    for marker in [
        "zpres-slide-class-sv-state-queued",
        "zpres-slide-class-sv-state-active",
        "zpres-slide-class-sv-state-trace",
        "zpres-slide-class-sv-state-fixed",
        "zpres-slide-class-sv-state-failed",
        "zpres-slide-class-sv-causal-trace",
        "QUEUED · domain event",
        "ACTIVE · causal selection",
        "FIXED · bound update",
        "SEPARATE SNAPSHOT · NOT A CONTINUATION",
    ] {
        assert!(screen.contains(marker), "missing screen marker {marker}");
        assert!(print.contains(marker), "missing print marker {marker}");
    }

    assert!(screen.contains("data-step-count=\"2\""));
    assert!(screen.contains("Step 0 of 2"));
    assert_eq!(
        print
            .matches("data-slide-classes=\"sv-state-trace sv-causal-trace\"")
            .count(),
        2,
        "the authored bound update must retain ACTIVE and FIXED print pages"
    );
    assert!(print.contains("data-pdf-step=\"1\""));
    assert!(print.contains("data-pdf-step=\"2\""));
    assert!(print.contains("data-slide-role=\"detail\""));

    for marker in [
        "--sv-current: var(--sv-accent)",
        "--sv-outcome: var(--sv-accent-alt)",
        "◇  - -  QUEUED\\A▶  ━━  ACTIVE\\A■  ==  FIXED\\A×  · ·  FAILED",
        "zpres-slide-class-sv-state-trace:has",
        "border-inline-start: 6px dashed",
        "text-decoration-style: double",
        "3px dotted var(--sv-outcome)",
    ] {
        assert!(
            theme_css.contains(marker),
            "missing Theme CSS marker {marker}"
        );
    }
    assert_eq!(
        theme_css.matches("var(--sv-accent)").count(),
        1,
        "violet must be consumed only through the current-state role"
    );
    assert_eq!(
        theme_css.matches("var(--sv-accent-alt)").count(),
        1,
        "gold must be consumed only through the outcome-state role"
    );
    assert!(!theme_css.contains("var(--sv-accent) 0 18%"));
    assert!(!theme_css.contains("inset 0 5px 0 var(--sv-accent-alt)"));

    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/sv/theme.toml")).unwrap();
    let rendered = zpres::theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
    assert!(
        rendered
            .print_css
            .contains("zpres-slide-class-sv-state-trace")
    );
    assert!(rendered.print_css.contains("text-shadow: none"));
}

#[test]
fn sv_detail_state_fixture_keeps_one_shared_rail_on_screen_and_print() {
    fn assert_every_state_slide_is_detail(document: &str, class: &str) {
        let marker = format!("zpres-slide-class-{class}");
        let slide_tags = document
            .match_indices(&marker)
            .filter_map(|(offset, _)| {
                let section_start = document[..offset].rfind("<section")?;
                let section_end = offset + document[offset..].find('>')? + 1;
                Some(&document[section_start..section_end])
            })
            .collect::<Vec<_>>();
        assert!(!slide_tags.is_empty(), "missing {class}");
        for slide_tag in slide_tags {
            assert!(
                slide_tag.contains("data-slide-role=\"detail\""),
                "{class} escaped the Detail-slide boundary"
            );
        }
    }

    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/sv-detail-state-boundaries.zp.md");
    let output = temp.path().join("sv-detail-state-html");
    let print_html = temp.path().join("sv-detail-state-print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    let theme_css =
        fs::read_to_string(publication.generation_path.join("assets/theme.css")).unwrap();
    for document in [&screen, &print] {
        for class in [
            "sv-state-queued",
            "sv-state-active",
            "sv-state-fixed",
            "sv-state-failed",
            "sv-state-trace",
        ] {
            assert_every_state_slide_is_detail(document, class);
        }
    }
    for css in [&theme_css, &print] {
        assert!(css.contains("◇  - -  QUEUED\\A▶  ━━  ACTIVE\\A■  ==  FIXED\\A×  · ·  FAILED"));
    }
    assert_eq!(
        print
            .matches("data-slide-classes=\"sv-state-trace sv-causal-trace\"")
            .count(),
        2,
        "the trace Detail must retain ACTIVE and FIXED print pages"
    );
    assert!(print.contains("data-pdf-step=\"1\""));
    assert!(print.contains("data-pdf-step=\"2\""));

    assert_eq!(
        theme_css
            .matches("content: \"◇  - -  QUEUED\\A▶  ━━  ACTIVE\\A■  ==  FIXED\\A×  · ·  FAILED\";")
            .count(),
        1,
        "all Main and Detail state Slides must share one rail definition"
    );
    let detail_selector = theme_css
        .split_once(".zpres-theme-sv .zpres-slide[data-slide-role=\"detail\"]:not(:is(")
        .expect("state-excluding generic Detail selector")
        .1
        .split_once(")) .zpres-slide-frame::before")
        .expect("complete state-excluding generic Detail selector")
        .0;
    for class in [
        "sv-state-queued",
        "sv-state-active",
        "sv-state-fixed",
        "sv-state-failed",
        "sv-state-trace",
    ] {
        assert!(
            detail_selector.contains(class),
            "Detail selector missed {class}"
        );
    }
    assert!(
        !theme_css
            .contains(".zpres-slide.zpres-slide-class-sv-state-failed[data-slide-role=\"detail\"]")
    );
}

#[test]
fn sv_propagation_evidence_css_keeps_authored_color_and_print_boundaries() {
    fn rule_body<'a>(css: &'a str, selector: &str) -> &'a str {
        css.split_once(&format!("{selector} {{"))
            .unwrap_or_else(|| panic!("missing focused CSS selector {selector}"))
            .1
            .split_once('}')
            .unwrap_or_else(|| panic!("unterminated focused CSS selector {selector}"))
            .0
    }

    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/sv/theme.toml")).unwrap();

    for parameters in [
        BTreeMap::new(),
        BTreeMap::from([("mode".to_string(), "dark".to_string())]),
    ] {
        let rendered = zpres::theme::render_theme(&manifest, &parameters).unwrap();
        let css = &rendered.screen_css;

        for hook in [
            "sv-evidence-domain-update",
            "sv-evidence-causal-schedule",
            "sv-evidence-resolved-outcome",
            "sv-evidence-causal-trace",
            "sv-evidence-claim",
            "sv-evidence-search-consequence",
            "sv-evidence-native",
            "sv-evidence-imported",
            "sv-evidence-inspectable-trace",
        ] {
            assert!(css.contains(hook), "missing authored evidence hook {hook}");
        }

        let evidence_current_rules = css
            .split('}')
            .filter(|rule| rule.contains("sv-evidence") && rule.contains("var(--sv-current)"))
            .collect::<Vec<_>>();
        assert!(
            !evidence_current_rules.is_empty(),
            "focused evidence needs at least one authored current-state rule"
        );
        for rule in evidence_current_rules {
            let selector = rule
                .split_once('{')
                .expect("complete SV evidence current-state rule")
                .0;
            assert!(
                selector.contains("sv-causal-trace"),
                "violet evidence escaped the authored causal hook: {selector}"
            );
        }

        let evidence_outcome_rules = css
            .split('}')
            .filter(|rule| rule.contains("sv-evidence") && rule.contains("var(--sv-outcome)"))
            .collect::<Vec<_>>();
        assert!(
            !evidence_outcome_rules.is_empty(),
            "focused evidence needs an authored result rule"
        );
        for rule in evidence_outcome_rules {
            let selector = rule
                .split_once('{')
                .expect("complete SV evidence outcome rule")
                .0;
            assert!(
                [
                    "sv-evidence-domain-update",
                    "sv-evidence-causal-schedule",
                    "sv-evidence-search-consequence",
                ]
                .iter()
                .any(|hook| selector.contains(hook)),
                "gold evidence escaped an authored eliminated/resolved hook: {selector}"
            );
            if selector.contains("sv-evidence-causal-schedule") {
                assert!(
                    selector.contains("sv-evidence-resolved-outcome"),
                    "schedule position alone spent gold without an authored outcome: {selector}"
                );
            }
        }

        assert!(css.contains("background: var(--sv-evidence-paper);"));
        assert!(!css.contains("IMPORTED EVIDENCE  /  PAPER FIELD"));
        assert!(!css.contains("RESOLVED SEARCH CONSEQUENCE"));
        assert_eq!(css.matches("content: \"→\";").count(), 1);
        assert!(!css.contains("content: \"REVISE  →\";"));
        assert!(!css.contains("content: \"EVENT  →\";"));
        assert!(css.contains("fill: transparent;"));

        let domain_fit = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-domain-update[data-slide-variant=\"comparison\"] .zpres-block-fit-text p {",
            )
            .expect("domain-update fit-text selector")
            .1
            .split_once('}')
            .expect("domain-update fit-text rule")
            .0;
        assert!(domain_fit.contains("white-space: pre;"));

        let native_primary = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-native[data-slide-variant=\"figure\"] .zpres-slide-primary {",
            )
            .expect("native Figure primary selector")
            .1
            .split_once('}')
            .expect("native Figure primary rule")
            .0;
        assert!(native_primary.contains("grid-template-rows: minmax(0, 1fr) max-content;"));

        let native_surface = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-native[data-slide-variant=\"figure\"] .zpres-diagram-surface {",
            )
            .expect("native Figure neutral Diagram surface selector")
            .1
            .split_once('}')
            .expect("native Figure neutral Diagram surface rule")
            .0;
        assert!(native_surface.contains("fill: transparent;"));
        assert!(native_surface.contains("stroke: var(--sv-muted);"));
        assert!(!native_surface.contains("var(--sv-current)"));

        let native_caption = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-native[data-slide-variant=\"figure\"] .zpres-slide-primary > .zpres-block-paragraph {",
            )
            .expect("native Figure authored path band selector")
            .1
            .split_once('}')
            .expect("native Figure authored path band rule")
            .0;
        assert!(native_caption.contains("border-inline-start: 5px double var(--sv-muted);"));
        assert!(native_caption.contains("font-size: var(--text-slide-technical);"));

        let inspectable_primary = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-inspectable-trace[data-slide-role=\"detail\"] .zpres-slide-primary {",
            )
            .expect("named inspectable-trace Detail primary selector")
            .1
            .split_once('}')
            .expect("named inspectable-trace Detail primary rule")
            .0;
        assert!(inspectable_primary.contains("height: 100%;"));
        assert!(inspectable_primary.contains("grid-template-rows: minmax(0, 1fr) max-content;"));
        assert!(inspectable_primary.contains("gap: var(--spacing-slide-6);"));

        let inspectable_code = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-inspectable-trace[data-slide-role=\"detail\"] .zpres-block-code {",
            )
            .expect("named inspectable-trace code-field selector")
            .1
            .split_once('}')
            .expect("named inspectable-trace code-field rule")
            .0;
        assert!(inspectable_code.contains("height: 100%;"));
        assert!(
            inspectable_code.contains("padding: var(--spacing-slide-6) var(--spacing-slide-8);")
        );
        assert!(inspectable_code.contains("font-size: var(--text-slide-body);"));

        let inspectable_band = css
            .split_once(
                ".zpres-theme-sv .zpres-slide.zpres-slide-class-sv-evidence-inspectable-trace[data-slide-role=\"detail\"] .zpres-slide-primary > .zpres-block-paragraph {",
            )
            .expect("named inspectable-trace explanation-band selector")
            .1
            .split_once('}')
            .expect("named inspectable-trace explanation-band rule")
            .0;
        assert!(
            inspectable_band.contains("padding: var(--spacing-slide-4) var(--spacing-slide-6);")
        );
        assert!(inspectable_band.contains("border-inline-start: 6px double var(--sv-muted);"));
        assert!(inspectable_band.contains("background: var(--sv-panel);"));
        assert_eq!(
            css.matches("var(--sv-accent)").count(),
            1,
            "violet must still be consumed only through --sv-current"
        );
        assert_eq!(
            css.matches("var(--sv-accent-alt)").count(),
            1,
            "gold must still be consumed only through --sv-outcome"
        );

        let print_expectations: [(&str, &[&str]); 6] = [
            (
                ".zpres-theme-sv .zpres-print-slide.zpres-slide-class-sv-evidence-domain-update[data-slide-variant=\"comparison\"] .zpres-layout-regions::after",
                &["text-shadow: none;"],
            ),
            (
                ".zpres-theme-sv .zpres-print-slide.zpres-slide-class-sv-evidence-causal-schedule[data-slide-variant=\"comparison\"] .zpres-layout-regions::after",
                &["text-shadow: none;"],
            ),
            (
                ".zpres-theme-sv .zpres-print-slide.zpres-slide-class-sv-evidence-search-consequence.zpres-slide-class-sv-evidence-claim[data-slide-variant=\"claim\"] .zpres-block-fit-text",
                &["background: transparent;", "box-shadow: none;"],
            ),
            (
                ".zpres-theme-sv .zpres-print-slide.zpres-slide-class-sv-evidence-search-consequence.zpres-slide-class-sv-evidence-claim[data-slide-variant=\"claim\"] .zpres-block-fit-text p",
                &["text-shadow: none;"],
            ),
            (
                ".zpres-theme-sv .zpres-print-slide.zpres-slide-class-sv-evidence-native[data-slide-variant=\"figure\"] :where(.zpres-diagram-svg, .zpres-chart-svg)",
                &["box-shadow: none;"],
            ),
            (
                ".zpres-theme-sv .zpres-print-slide.zpres-slide-class-sv-evidence-imported[data-slide-variant=\"figure\"] .zpres-block-figure img",
                &["box-shadow: none;"],
            ),
        ];
        for (selector, declarations) in print_expectations {
            let body = rule_body(&rendered.print_css, selector);
            for declaration in declarations {
                assert!(
                    body.contains(declaration),
                    "focused print selector {selector} missed {declaration}: {body}"
                );
            }
        }
    }
}

#[test]
fn sv_imported_evidence_does_not_claim_unless_authored_current_state() {
    let svg = fs::read_to_string(repository_path(
        "fixtures/theme-api-v1/assets/sv-imported-search-profile.svg",
    ))
    .unwrap();

    for reserved_violet in ["#6f4b8b", "#713f8f", "#d0a7e8"] {
        assert!(
            !svg.contains(reserved_violet),
            "static imported outcome spent reserved violet {reserved_violet}"
        );
    }
    assert_eq!(
        svg.matches("#8a5a00").count(),
        3,
        "the resolved line, square markers, and direct label must share dark ochre"
    );
    assert!(svg.contains("stroke-linecap=\"square\""));
    assert_eq!(svg.matches("<rect x=").count(), 4);
    assert_eq!(svg.matches("<circle cx=").count(), 4);
    assert!(svg.contains("resolved after-propagation series uses dark ochre square markers"));
    assert!(svg.contains("viewBox=\"0 0 960 480\""));
    assert!(svg.contains("<g text-anchor=\"middle\">"));
    assert!(svg.contains("<text x=\"20\" y=\"358\">n=20</text>"));
    assert!(svg.contains("<text x=\"740\" y=\"358\">n=80</text>"));
    assert!(
        svg.contains("<text x=\"380\" y=\"424\" text-anchor=\"middle\">candidate variables</text>")
    );
    assert!(!svg.contains("y=\"350\">n="));
    assert!(!svg.contains("y=\"372\">candidate variables"));
    assert!(svg.contains("<text x=\"390\" y=\"28\" fill=\"#465965\">before propagation</text>"));
    assert!(svg.contains("<text x=\"430\" y=\"236\" fill=\"#8a5a00\">after propagation</text>"));
    assert!(!svg.contains("<text x=\"500\" y=\"34\""));
    assert!(!svg.contains("<text x=\"500\" y=\"250\""));
}

#[test]
fn sv_propagation_evidence_fixture_renders_typed_jobs_and_truthful_snapshots() {
    fn print_pages_for_slide<'a>(document: &'a str, slide_id: &str) -> Vec<&'a str> {
        let marker = format!("data-slide-id=\"{slide_id}\"");
        document
            .split("<section class=\"zpres-print-slide")
            .skip(1)
            .filter(|page| page.contains(&marker))
            .collect()
    }

    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/sv-propagation-evidence.zp.md");
    let output = temp.path().join("sv-propagation-evidence-html");
    let print_html = temp.path().join("sv-propagation-evidence-print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(&publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();

    for hook in [
        "sv-evidence-domain-update",
        "sv-evidence-causal-schedule",
        "sv-evidence-resolved-outcome",
        "sv-evidence-causal-trace",
        "sv-evidence-claim",
        "sv-evidence-search-consequence",
        "sv-evidence-native",
        "sv-evidence-imported",
        "sv-evidence-inspectable-trace",
    ] {
        assert!(
            screen.contains(hook),
            "missing HTML presentation hook {hook}"
        );
        assert!(print.contains(hook), "missing print HTML hook {hook}");
    }
    assert!(!screen.contains("data-block-type=\"html\""));
    assert!(!print.contains("data-block-type=\"html\""));

    let domain = print_pages_for_slide(&print, "section-2-main");
    assert_eq!(domain.len(), 1);
    assert!(domain[0].contains("data-comparison-role=\"primary\""));
    assert!(domain[0].contains("data-comparison-role=\"supporting\""));
    assert_eq!(domain[0].matches("data-block-type=\"fit-text\"").count(), 2);
    assert!(domain[0].contains("Cause · fixing x = 4"));
    assert!(domain[0].contains("ELIMINATED · 2  3  4  5  6"));
    assert!(!domain[0].contains("data-block-type=\"list\""));

    let schedule = print_pages_for_slide(&print, "section-3-main");
    assert_eq!(schedule.len(), 1);
    assert!(schedule[0].contains("zpres-slide-class-sv-evidence-resolved-outcome"));
    assert_eq!(schedule[0].matches("data-block-type=\"table\"").count(), 2);
    assert_eq!(
        schedule[0].matches("data-block-type=\"fit-text\"").count(),
        1
    );
    assert!(schedule[0].contains("Causal change · the x event"));
    assert!(schedule[0].contains("2 revisits removed"));

    let trace = print_pages_for_slide(&print, "section-4-main");
    assert_eq!(trace.len(), 2, "ACTIVE and FIXED need separate print pages");
    for page in &trace {
        assert!(page.contains("data-step-count=\"2\""));
        assert!(page.contains("QUEUED · STABLE PREMISE"));
        assert!(page.contains("x = 4 · D(y) = {1, 2, 3, 4, 5, 6, 7}"));
    }
    assert!(trace[0].contains("data-pdf-step=\"1\""));
    assert!(trace[0].contains("data-step-state=\"active\""));
    assert!(trace[0].contains("data-block-type=\"diagram\""));
    assert!(trace[1].contains("data-pdf-step=\"2\""));
    assert!(trace[1].contains("FIXED · RESOLVED DOMAIN"));
    assert!(trace[1].contains("Eliminated values"));

    let failed = print_pages_for_slide(&print, "section-4-detail-1");
    assert_eq!(failed.len(), 1);
    assert!(failed[0].contains("data-slide-role=\"detail\""));
    assert!(failed[0].contains("zpres-slide-class-sv-state-failed"));
    assert!(!failed[0].contains("zpres-slide-class-sv-state-trace"));
    assert!(failed[0].contains("SEPARATE SNAPSHOT · NOT A CONTINUATION"));
    assert!(failed[0].contains("not the next Derivation Step"));

    let native = print_pages_for_slide(&print, "section-5-main");
    assert_eq!(native.len(), 1);
    assert!(native[0].contains("data-block-type=\"diagram\""));
    assert!(native[0].contains("data-block-type=\"paragraph\""));
    assert!(native[0].contains("Path · event → revision → D(y) → branch"));
    assert!(!native[0].contains("data-block-type=\"figure\""));

    let inspectable_marker = screen
        .find("data-slide-id=\"section-5-detail-1\"")
        .expect("inspectable advisor Detail in the HTML presentation");
    let inspectable_start = screen[..inspectable_marker]
        .rfind("<section")
        .expect("inspectable advisor Detail opening element");
    let inspectable_screen = screen[inspectable_start..]
        .split_once("</section>")
        .expect("inspectable advisor Detail closing element")
        .0;
    assert!(inspectable_screen.contains("zpres-slide-class-sv-evidence-inspectable-trace"));
    assert!(inspectable_screen.contains("data-code-reveal-steps=\"2\""));
    assert!(inspectable_screen.contains("data-zpres-step-progress"));
    assert!(inspectable_screen.contains("Step 0 of 2"));
    assert!(inspectable_screen.contains("Pacing note ·"));
    assert!(!inspectable_screen.contains("zpres-slide-class-sv-state-"));

    let inspectable = print_pages_for_slide(&print, "section-5-detail-1");
    assert_eq!(inspectable.len(), 1);
    assert!(inspectable[0].contains("data-slide-role=\"detail\""));
    assert!(inspectable[0].contains("zpres-slide-class-sv-evidence-inspectable-trace"));
    assert!(inspectable[0].contains("data-block-type=\"code\""));
    assert!(inspectable[0].contains("data-block-type=\"paragraph\""));
    assert!(inspectable[0].contains("Pacing note ·"));
    assert!(!inspectable[0].contains("zpres-slide-class-sv-state-"));

    let imported = print_pages_for_slide(&print, "section-6-main");
    assert_eq!(imported.len(), 1);
    assert!(imported[0].contains("data-block-type=\"figure\""));
    assert!(imported[0].contains("sv-imported-search-profile.svg"));
    assert!(imported[0].contains("Takeaway · the resolved after-propagation outcome"));
    assert!(!imported[0].contains("zpres-slide-class-sv-causal-trace"));

    let result = print_pages_for_slide(&print, "section-7-main");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].matches("data-block-type=\"fit-text\"").count(), 1);
    for forbidden in [
        "paragraph",
        "list",
        "layout",
        "diagram",
        "figure",
        "table",
        "quote",
    ] {
        assert!(
            !result[0].contains(&format!("data-block-type=\"{forbidden}\"")),
            "signature Claim accumulated a competing {forbidden} block"
        );
    }
    assert!(result[0].contains("2,688 → 64 nodes"));
}

#[test]
fn wedding_v1_palettes_fonts_ornaments_and_role_parameters_are_effective() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/wedding/theme.toml")).unwrap();
    assert_eq!(manifest.api_version, 1);
    assert_eq!(manifest.modules, vec!["scientific-data"]);
    assert_eq!(manifest.style.family.as_deref(), Some("wedding-editorial"));
    assert!(manifest.style.inspiration.is_empty());
    let pile_scale = manifest.parameters.get("pile_scale").unwrap();
    assert_eq!(pile_scale.default.as_deref(), Some("0.92"));
    assert_eq!(pile_scale.min, Some(0.88));
    assert_eq!(pile_scale.max, Some(0.96));

    let garden = zpres::theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
    let slate = zpres::theme::render_theme(
        &manifest,
        &BTreeMap::from([
            ("variant".to_string(), "slate-rose".to_string()),
            ("ornament_style".to_string(), "bow".to_string()),
            ("density".to_string(), "compact".to_string()),
            ("footer".to_string(), "none".to_string()),
            (
                "font_heading".to_string(),
                "Baskerville, Georgia, serif".to_string(),
            ),
            (
                "font_body".to_string(),
                "Aptos, ui-sans-serif, sans-serif".to_string(),
            ),
            (
                "font_mono".to_string(),
                "Menlo, ui-monospace, monospace".to_string(),
            ),
            (
                "font_script".to_string(),
                "Apple Chancery, cursive".to_string(),
            ),
        ]),
    )
    .unwrap();

    for marker in [
        "--wedding-background: #f1eee8",
        "--wedding-surface: #fffdf8",
        "--wedding-accent: #7895a3",
        "assets/botanical.svg",
        "assets/wildflower.svg",
        "assets/vine.svg",
        "assets/bow.svg",
        "assets/minimal.svg",
        "data-theme-param-density=\"compact\"",
        "data-theme-param-density=\"spacious\"",
        ".zpres-slide-ornament::before",
        "data-background-split",
        "zpres-slide-class-result",
        "--wedding-insert-rule",
        "data-comparison-role",
        "data-derivation-role",
        "zpres-slide-class-ornament-frame",
        "zpres-slide-class-ornament-join",
        "zpres-slide-class-ornament-point",
        "zpres-slide-class-ornament-boundary",
        "zpres-slide-class-ornament-celebrate",
        "zpres-slide-class-figure-unmounted",
        "zpres-slide-class-title-wide",
        "zpres-slide-class-botanical-list",
        "--wedding-pile-scale: 0.92",
        "wedding-sheet-enter-backward",
        "wedding-pile-enter",
        ".zpres-block-chart::before",
        ".zpres-callout-title",
    ] {
        assert!(garden.screen_css.contains(marker), "missing {marker}");
    }
    for marker in [
        "--wedding-background: #e8e5df",
        "--wedding-surface: #fffdf8",
        "--wedding-ink: #20242a",
        "font-family: Baskerville, Georgia, serif",
        "font-family: Aptos, ui-sans-serif, sans-serif",
        "font-family: Menlo, ui-monospace, monospace",
        "font-family: Apple Chancery, cursive",
        "--zpres-param-ornament-style: bow",
        "--zpres-param-density: compact",
        "--zpres-param-footer: none",
    ] {
        assert!(slate.screen_css.contains(marker), "missing {marker}");
    }
    assert_ne!(garden.screen_css, slate.screen_css);
    assert!(slate.print_css.contains("#e8e5df"));

    for invalid in ["0.87", "0.97"] {
        let error = zpres::theme::render_theme(
            &manifest,
            &BTreeMap::from([("pile_scale".to_string(), invalid.to_string())]),
        )
        .unwrap_err();
        assert!(error.to_string().contains(if invalid == "0.87" {
            "at least 0.88"
        } else {
            "at most 0.96"
        }));
    }
}

#[test]
fn dark_splash_v1_palettes_fonts_and_role_parameters_are_effective() {
    let manifest =
        zpres::theme::load_theme_manifest(&repository_path("themes/dark-splash/theme.toml"))
            .unwrap();
    assert_eq!(manifest.api_version, 1);
    assert_eq!(manifest.modules, vec!["scientific-data"]);
    assert_eq!(
        manifest.style.family.as_deref(),
        Some("midnight-constraint-atlas")
    );

    let violet = zpres::theme::render_theme(&manifest, &BTreeMap::new()).unwrap();
    let cyan = zpres::theme::render_theme(
        &manifest,
        &BTreeMap::from([
            ("variant".to_string(), "cyan".to_string()),
            ("background".to_string(), "#071015".to_string()),
            ("surface".to_string(), "#102027".to_string()),
            ("text".to_string(), "#f4fbfc".to_string()),
            ("muted".to_string(), "#aac0c7".to_string()),
            ("accent".to_string(), "#45e3ff".to_string()),
            ("accent_alt".to_string(), "#f4ff66".to_string()),
            ("rule".to_string(), "#34535c".to_string()),
            ("density".to_string(), "compact".to_string()),
            ("footer".to_string(), "section-title".to_string()),
            (
                "font_heading".to_string(),
                "Impact, ui-sans-serif, sans-serif".to_string(),
            ),
            (
                "font_body".to_string(),
                "Aptos, ui-sans-serif, sans-serif".to_string(),
            ),
            (
                "font_mono".to_string(),
                "Menlo, ui-monospace, monospace".to_string(),
            ),
        ]),
    )
    .unwrap();

    for marker in [
        "--atlas-background: #08090d",
        "--atlas-accent: #9b6cff",
        "Avenir Next Condensed",
        "Avenir Next",
        "SFMono-Regular",
        "data-slide-variant=\"claim\"",
        "data-slide-variant=\"comparison\"",
        "data-slide-variant=\"derivation\"",
        "data-slide-variant=\"figure\"",
        "data-slide-variant=\"dense\"",
        "data-slide-role=\"detail\"",
        "data-background-split",
    ] {
        assert!(violet.screen_css.contains(marker), "missing {marker}");
    }
    for marker in [
        "--atlas-background: #071015",
        "--atlas-surface: #102027",
        "--atlas-text: #f4fbfc",
        "--atlas-muted: #aac0c7",
        "--atlas-accent: #45e3ff",
        "--atlas-result: #f4ff66",
        "--atlas-rule: #34535c",
        "font-family: Impact, ui-sans-serif, sans-serif",
        "font-family: Aptos, ui-sans-serif, sans-serif",
        "font-family: Menlo, ui-monospace, monospace",
        "--zpres-param-density: compact",
        "--zpres-param-footer: section-title",
    ] {
        assert!(cyan.screen_css.contains(marker), "missing {marker}");
    }
    assert_ne!(violet.screen_css, cyan.screen_css);
    assert!(cyan.print_css.contains("#071015"));
    assert!(cyan.print_css.contains("#45e3ff"));
}

#[test]
fn comparison_pattern_exposes_stable_roles_and_redundant_cues_in_debug_and_science() {
    let temp = tempdir().unwrap();
    for theme in ["debug", "science"] {
        let source = temp.path().join(format!("{theme}-comparison.zp.md"));
        let output = temp.path().join(format!("{theme}-html"));
        let print_html = temp.path().join(format!("{theme}-print.html"));
        fs::write(
            &source,
            format!(
                "---\ntheme: \"{theme}\"\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Solver comparison\n\n::::: comparison\n:::: primary label=\"Baseline\"\n::: fit\n42 nodes\n:::\n\nChronological branching.\n::::\n:::: supporting label=\"Candidate\"\n::: fit\n11 nodes\n:::\n\nImpact-guided branching.\n::::\n:::::\n",
                repository_path("themes").display()
            ),
        )
        .unwrap();

        zpres(temp.path())
            .args([
                "build",
                source.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
            ])
            .assert()
            .success();
        zpres(temp.path())
            .args([
                "export",
                source.to_str().unwrap(),
                "--print-html",
                print_html.to_str().unwrap(),
            ])
            .assert()
            .success();

        let publication = current_html_generation(&output);
        let screen = fs::read_to_string(publication.presentation_index).unwrap();
        let print = fs::read_to_string(print_html).unwrap();
        for document in [&screen, &print] {
            assert!(document.contains("data-slide-variant=\"comparison\""));
            assert!(document.contains("data-comparison-role=\"primary\""));
            assert!(document.contains("data-comparison-role=\"supporting\""));
            assert!(document.contains("data-comparison-cue=\"circle\""));
            assert!(document.contains("data-comparison-cue=\"square\""));
            assert!(document.contains("class=\"zpres-layout-region-content\""));
            let baseline = document.find("Baseline").unwrap();
            let candidate = document.find("Candidate").unwrap();
            assert!(baseline < candidate, "comparison reading order changed");
        }
        let theme_css =
            fs::read_to_string(publication.generation_path.join("assets/theme.css")).unwrap();
        assert!(theme_css.contains("data-slide-variant=\"comparison\""));
        assert!(theme_css.contains("data-comparison-role"));
    }
}

#[test]
fn derivation_pattern_exposes_context_stages_and_deterministic_step_states() {
    let temp = tempdir().unwrap();
    for theme in ["debug", "science"] {
        let source = temp.path().join(format!("{theme}-derivation.zp.md"));
        let output = temp.path().join(format!("{theme}-html"));
        let print_html = temp.path().join(format!("{theme}-print.html"));
        fs::write(
            &source,
            format!(
                "---\ntheme: \"{theme}\"\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Derive the invariant\n\n::::: derivation pdf=\"pages\"\n:::: context label=\"Invariant\"\nFor every t, x_t is in D.\n::::\n:::: stage label=\"Transition\"\nApply f to x_t.\n::::\n:::: stage label=\"Consequence\"\nTherefore x_(t+1) is in D.\n::::\n:::::\n",
                repository_path("themes").display()
            ),
        )
        .unwrap();

        zpres(temp.path())
            .args([
                "build",
                source.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
            ])
            .assert()
            .success();
        zpres(temp.path())
            .args([
                "export",
                source.to_str().unwrap(),
                "--print-html",
                print_html.to_str().unwrap(),
            ])
            .assert()
            .success();

        let publication = current_html_generation(&output);
        let screen = fs::read_to_string(&publication.presentation_index).unwrap();
        let print = fs::read_to_string(print_html).unwrap();
        assert!(screen.contains("data-slide-variant=\"derivation\""));
        assert!(screen.contains("data-derivation-role=\"context\""));
        assert_eq!(screen.matches("data-derivation-role=\"stage\"").count(), 2);
        assert!(screen.contains("data-step-count=\"2\""));
        assert!(screen.contains("Step 0 of 2"));
        assert!(screen.contains("aria-live=\"polite\""));
        assert!(!screen.contains("tabindex=\"0\" class=\"zpres-step-progress"));

        assert_eq!(print.matches("data-pdf-step=\"1\"").count(), 1);
        assert_eq!(print.matches("data-pdf-step=\"2\"").count(), 1);
        assert!(print.contains("data-step-state=\"active\""));
        assert!(print.contains("data-step-state=\"complete\""));
        assert!(print.contains("Step 1 of 2"));
        assert!(print.contains("Step 2 of 2"));

        let theme_css =
            fs::read_to_string(publication.generation_path.join("assets/theme.css")).unwrap();
        assert!(theme_css.contains("data-slide-variant=\"derivation\""));
        assert!(theme_css.contains("data-derivation-role"));
    }
}

#[test]
fn dense_detail_exposes_technical_roles_and_non_color_table_highlighting() {
    let temp = tempdir().unwrap();
    for theme in ["debug", "science"] {
        let source = temp.path().join(format!("{theme}-dense.zp.md"));
        let output = temp.path().join(format!("{theme}-dense-html"));
        fs::write(
            &source,
            format!(
                "---\ntheme: \"{theme}\"\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Result\n\nThe Main path states the result.\n\n--\n\n## Technical detail\n\n::: variant dense\n:::\n\n| Model | Nodes | Runtime |\n| --- | ---: | ---: |\n| Baseline | 42 | 18.4 s |\n| **Candidate** | **11** | **6.2 s** |\n\n```rust\nlet bound = propagate(&model);\nassert!(bound <= incumbent);\n```\n\n$$\nz^* = min c^T x\n$$\n",
                repository_path("themes").display()
            ),
        )
        .unwrap();

        zpres(temp.path())
            .args([
                "build",
                source.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
            ])
            .assert()
            .success();

        let publication = current_html_generation(&output);
        let screen = fs::read_to_string(&publication.presentation_index).unwrap();
        assert!(screen.contains("data-slide-role=\"detail\""));
        assert!(screen.contains("data-slide-variant=\"dense\""));
        assert_eq!(
            screen.matches("data-zpres-type-role=\"technical\"").count(),
            3
        );
        assert!(screen.contains("data-table-role=\"header\""));
        assert!(screen.contains("scope=\"col\""));
        assert!(screen.contains("<strong>Candidate</strong>"));
        let foundation = fs::read_to_string(
            publication
                .generation_path
                .join("assets/zpres-theme-api-v1.css"),
        )
        .unwrap();
        assert!(foundation.contains("white-space: pre"));
        assert!(!foundation.contains("white-space: pre-wrap"));
    }
}

#[test]
fn unknown_theme_api_version_reports_its_manifest_location() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("future-theme");
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(
        theme_dir.join("theme.toml"),
        "[theme]\nname = \"future-theme\"\nversion = \"0.1.0\"\napi_version = 99\n",
    )
    .unwrap();

    zpres(temp.path())
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--no-fixture",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("theme.toml:4:")
                .and(predicate::str::contains("unsupported api_version 99"))
                .and(predicate::str::contains("supported api_version is 1")),
        );
}

#[test]
fn v1_reference_build_uses_semantic_regions_and_compiled_css() {
    let temp = tempdir().unwrap();
    let source = repository_path("themes/reference/specimen.zp.md");
    let html_dir = temp.path().join("html");
    let print_html_path = temp.path().join("print.html");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            html_dir.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&html_dir);
    let index_html = fs::read_to_string(publication.presentation_index).unwrap();
    let print_html = fs::read_to_string(print_html_path).unwrap();
    let foundation = fs::read_to_string(
        publication
            .generation_path
            .join("assets/zpres-theme-api-v1.css"),
    )
    .unwrap();
    for marker in [
        "data-zpres-theme-api=\"1\"",
        "zpres-slide-frame",
        "zpres-slide-header",
        "zpres-slide-title",
        "zpres-slide-body",
        "zpres-slide-primary",
        "zpres-slide-sources",
        "zpres-slide-footer",
    ] {
        assert!(
            index_html.contains(marker),
            "missing screen marker {marker}"
        );
        assert!(print_html.contains(marker), "missing print marker {marker}");
    }
    assert!(index_html.contains("assets/zpres-theme-api-v1.css"));
    assert!(!index_html.contains("assets/reveal.css"));
    assert!(!foundation.contains("@import"));
    assert!(!foundation.contains("@theme"));
    assert!(!foundation.contains("cdn.tailwindcss.com"));
    assert!(!index_html.contains("cdn.tailwindcss.com"));
}

#[test]
fn theme_api_v1_spacing_scale_defines_every_foundation_and_builtin_use() {
    let source = fs::read_to_string(repository_path("styles/theme-api-v1.css")).unwrap();
    let generated = fs::read_to_string(repository_path("src/generated/theme-api-v1.css")).unwrap();

    for (name, value) in [
        ("--spacing-slide-1", "0.25rem"),
        ("--spacing-slide-2", "0.5rem"),
        ("--spacing-slide-3", "0.75rem"),
        ("--spacing-slide-4", "1rem"),
        ("--spacing-slide-5", "1.25rem"),
        ("--spacing-slide-6", "1.5rem"),
        ("--spacing-slide-8", "2rem"),
        ("--spacing-slide-10", "2.5rem"),
        ("--spacing-slide-12", "3rem"),
        ("--spacing-slide-16", "4rem"),
    ] {
        let definition = format!("{name}: {value};");
        assert!(
            source.contains(&definition),
            "source foundation is missing {definition}"
        );
        assert!(
            generated.contains(&definition),
            "generated foundation is missing {definition}"
        );
    }

    let spacing_tokens = |css: &str| {
        let prefix = "--spacing-slide-";
        css.match_indices(prefix)
            .filter_map(|(offset, _)| {
                let suffix = &css[offset + prefix.len()..];
                let digits = suffix.bytes().take_while(u8::is_ascii_digit).count();
                (digits > 0).then(|| format!("{prefix}{}", &suffix[..digits]))
            })
            .collect::<BTreeSet<_>>()
    };

    for path in [
        "styles/theme-api-v1.css",
        "src/generated/theme-api-v1.css",
        "themes/dark-splash/theme.css.tmpl",
        "themes/dark-splash/print.css.tmpl",
        "themes/debug/theme.css.tmpl",
        "themes/debug/print.css.tmpl",
        "themes/paper-chalk/theme.css.tmpl",
        "themes/paper-chalk/print.css.tmpl",
        "themes/science/theme.css.tmpl",
        "themes/science/print.css.tmpl",
        "themes/sv/theme.css.tmpl",
        "themes/sv/print.css.tmpl",
        "themes/reference/theme.css.tmpl",
        "themes/reference/print.css.tmpl",
        "themes/wedding/theme.css.tmpl",
        "themes/wedding/print.css.tmpl",
    ] {
        let css = fs::read_to_string(repository_path(path)).unwrap();
        for token in spacing_tokens(&css) {
            assert!(
                source.contains(&format!("{token}:")),
                "{path} uses undefined spacing token {token}"
            );
        }
    }
}

#[test]
fn v1_component_foundation_paints_diagrams_with_role_tokens() {
    let source = fs::read_to_string(repository_path("styles/theme-api-v1.css")).unwrap();
    let generated = fs::read_to_string(repository_path("src/generated/theme-api-v1.css")).unwrap();
    for marker in [
        ":where([data-zpres-theme-api=\"1\"]) .zpres-diagram-surface",
        "fill: var(--zpres-color-surface, #fffdf8);",
        ".zpres-diagram-node rect",
        "stroke: var(--zpres-color-accent, #0a6c67);",
        ".zpres-diagram-edge-label, .zpres-diagram-node text",
        "fill: var(--zpres-color-text, #17211f);",
        "font-size: var(--text-slide-technical);",
        "paint-order: stroke;",
    ] {
        for (surface, css) in [
            ("source", source.as_str()),
            ("generated", generated.as_str()),
        ] {
            assert!(
                css.contains(marker),
                "Theme API v1 {surface} component foundation is missing Diagram marker {marker}"
            );
        }
    }
    assert!(
        !generated.contains(
            "body.zpres-module-scientific-data[data-zpres-theme-api=\"1\"] .zpres-diagram-surface"
        ),
        "core Diagram paint must reach reference without the scientific-data module"
    );
}

#[test]
fn v1_diagram_branches_have_raster_contrast_and_clearance_on_light_and_dark_themes() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("diagram-role-tokens.zp.md");
    fs::write(
        &source,
        r#"---
title: Diagram role-token contrast
aspect: "16:9"
---

## Diagram labels remain visible

```mermaid
flowchart LR
  A[WWWWWWWWWWWW] --> B[Branch one]
  A -->|branch label WWWWWWWW| C[Branch two]
  B --> D[Converged result]
  C --> D
  A -->|long shortcut WWWWWWWWWWWW| D
```
"#,
    )
    .unwrap();

    for theme in ["reference", "science", "dark-splash"] {
        let output_dir = temp.path().join(theme);
        let output = zpres(temp.path())
            .args([
                "theme",
                "check",
                repository_path(format!("themes/{theme}")).to_str().unwrap(),
                "--visual",
                "--fixture",
                source.to_str().unwrap(),
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
            eprintln!("skipping browser-backed v1 Diagram contrast regression: {stderr}");
            return;
        }

        let report_debug = fs::read_to_string(output_dir.join("visual-report.json"))
            .unwrap_or_else(|_| "<visual report unavailable>".to_string());
        let report: Value = serde_json::from_str(&report_debug).unwrap();
        for (surface, observation, image_path) in [
            (
                "screen",
                &report["screen_states"][0],
                output_dir.join("screen-pages/state-001.png"),
            ),
            (
                "print",
                &report["pages"][0],
                output_dir.join("pages/page-001.png"),
            ),
        ] {
            assert_eq!(
                observation["clip_marker"], false,
                "{theme} {surface} Diagram fixture clipped: {observation}"
            );
            assert!(
                observation["geometry_violations"]
                    .as_array()
                    .is_none_or(Vec::is_empty),
                "{theme} {surface} Diagram fixture retained geometry violations: {}",
                observation["geometry_violations"]
            );
            assert!(
                observation["unresolved"]
                    .as_array()
                    .is_none_or(Vec::is_empty),
                "{theme} {surface} Diagram fixture was unresolved: {}",
                observation["unresolved"]
            );
            let body = observation["semantic_text_regions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|region| region["region"] == "body")
                .unwrap_or_else(|| panic!("missing {theme} {surface} Diagram body region"));
            let text_bounds = body["text_bounds"].as_array().unwrap();
            assert!(
                text_bounds.len() >= 6,
                "{theme} {surface} omitted Diagram label bounds: {body}"
            );
            for bound in text_bounds {
                let contrast_pixels = text_bound_contrast_pixels(&image_path, bound);
                assert!(
                    contrast_pixels >= 20,
                    "{theme} {surface} Diagram label has no raster contrast ({contrast_pixels} pixels): {bound}"
                );
            }
        }
        if !output.status.success() {
            eprintln!(
                "{theme} retained unrelated full-Theme gate findings while Diagram raster contrast passed:\n{stderr}"
            );
        }
    }
}

#[test]
fn debug_theme_passes_wide_sweep_visual_characterization() {
    let temp = tempdir().unwrap();
    let output_dir = temp.path().join("debug-wide-review");
    let output = zpres(temp.path())
        .args([
            "theme",
            "check",
            repository_path("themes/debug").to_str().unwrap(),
            "--visual",
            "--fixture",
            repository_path("fixtures/canonical/wide-sweep.zp.md")
                .to_str()
                .unwrap(),
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
        eprintln!("skipping browser-backed Debug wide-sweep regression: {stderr}");
        return;
    }

    let report_debug = fs::read_to_string(output_dir.join("visual-report.json"))
        .unwrap_or_else(|_| "<visual report unavailable>".to_string());
    assert!(
        output.status.success(),
        "Debug wide-sweep characterization failed\nstdout:\n{stdout}\nstderr:\n{stderr}\nreport:\n{report_debug}"
    );

    let report: Value = serde_json::from_str(&report_debug).unwrap();
    assert_eq!(report["objective_gate_status"], "passed");
    for field in ["screen_failures", "print_failures"] {
        assert!(
            report[field].as_array().unwrap().is_empty(),
            "Debug wide-sweep reported {field}: {}",
            report[field]
        );
    }

    for (surface, observations) in [("screen", "screen_states"), ("print", "pages")] {
        let spot = report[observations]
            .as_array()
            .unwrap()
            .iter()
            .find(|observation| observation["slide_id"] == "section-10-main")
            .unwrap_or_else(|| panic!("missing {surface} spot-directive observation"));
        for region in ["header", "body"] {
            let ink_pixels = spot["semantic_text_regions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|sample| sample["region"] == region)
                .unwrap_or_else(|| panic!("missing {surface} spot-directive {region} ink"))
                ["raster_ink"]["ink_pixels"]
                .as_u64()
                .unwrap();
            assert!(
                ink_pixels >= 2_000,
                "{surface} spot-directive {region} retained too little semantic raster ink: {ink_pixels}"
            );
        }

        let chart = report[observations]
            .as_array()
            .unwrap()
            .iter()
            .find(|observation| observation["slide_id"] == "section-4-main")
            .unwrap_or_else(|| panic!("missing {surface} Runtime chart observation"));
        assert_eq!(
            chart["clip_marker"], false,
            "{surface} Runtime chart retained clipped content"
        );
    }
}

#[test]
fn v1_foundation_paints_authored_backgrounds_on_screen_and_print() {
    let temp = tempdir().unwrap();
    let asset_dir = temp.path().join("assets");
    fs::create_dir_all(&asset_dir).unwrap();
    fs::write(
        asset_dir.join("field.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720"><rect width="1280" height="720" fill="#0f766e"/></svg>"##,
    )
    .unwrap();
    let source = temp.path().join("background.zp.md");
    fs::write(
        &source,
        format!(
            r#"---
title: "V1 background foundation"
theme: "reference"
theme_dirs:
  - "{}"
aspect: "16:9"
background_image:
  src: "assets/field.svg"
  intent: contextual
  alt: "Abstract field"
  position: "right 40%"
  fit: "contain"
  split: "left:35%"
  dim: 42
  grayscale: 11
  saturate: 93
  blur: 3
  splash: true
---

# Title omits the atmospheric image

The generated splash follows this slide.

---

# Content uses the authored image

The frame moves beside the split background.
"#,
            repository_path("themes").display()
        ),
    )
    .unwrap();

    let html_dir = temp.path().join("html");
    let print_html_path = temp.path().join("print.html");
    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            html_dir.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&html_dir);
    let index_html = fs::read_to_string(publication.presentation_index).unwrap();
    let print_html = fs::read_to_string(print_html_path).unwrap();
    for document in [&index_html, &print_html] {
        assert!(document.contains("data-background-phase=\"title\""));
        assert!(document.contains("data-background-phase=\"splash\""));
        assert!(document.contains("data-background-phase=\"content\""));
        assert!(document.contains("data-background-split=\"left\""));
        assert_eq!(
            document.matches("class=\"zpres-slide-background\"").count(),
            2,
            "only the generated splash and content slide should paint a background"
        );
        for declaration in [
            "--zpres-background-position: right 40%",
            "--zpres-background-fit: contain",
            "--zpres-background-split-size: 35%",
            "--zpres-background-dim: 0.42",
            "--zpres-background-gray: 0.11",
            "--zpres-background-saturate: 0.93",
            "--zpres-background-blur: 3px",
        ] {
            assert!(document.contains(declaration), "missing {declaration}");
        }
    }

    let title_start = index_html.find("data-background-phase=\"title\"").unwrap();
    let splash_start = index_html.find("data-background-phase=\"splash\"").unwrap();
    assert!(
        !index_html[title_start..splash_start].contains("zpres-slide-background"),
        "the title phase must not grow a background layer"
    );

    let foundation = fs::read_to_string(
        publication
            .generation_path
            .join("assets/zpres-theme-api-v1.css"),
    )
    .unwrap();
    for marker in [
        "background-image: var(--zpres-background-image)",
        "background-position: var(--zpres-background-position, center center)",
        "background-size: var(--zpres-background-fit, cover)",
        "background-repeat: no-repeat",
        "filter: grayscale(var(--zpres-background-gray, 0.3)) saturate(var(--zpres-background-saturate, 0.7)) blur(var(--zpres-background-blur, 0px))",
        ".zpres-slide:is([data-background-phase=\"content\"], [data-background-phase=\"title\"]) > .zpres-slide-background::after",
        ".zpres-slide:is([data-background-phase=\"content\"], [data-background-phase=\"title\"])[data-background-split=\"left\"] > .zpres-slide-background",
        ".zpres-slide:is([data-background-phase=\"content\"], [data-background-phase=\"title\"])[data-background-split=\"right\"] > .zpres-slide-frame",
        ".zpres-background-splash-slide > .zpres-slide-background",
    ] {
        assert!(
            foundation.contains(marker),
            "missing foundation marker {marker}"
        );
    }
}

#[test]
fn v1_background_semantics_fixture_keeps_meaning_and_decoration_distinct() {
    let temp = tempdir().unwrap();
    let source = repository_path("fixtures/theme-api-v1/background-semantics.zp.md");
    let output = temp.path().join("background-semantics");

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let html = fs::read_to_string(publication.presentation_index).unwrap();
    assert!(html.contains("data-zpres-background-intent=\"contextual\""));
    assert!(html.contains("data-zpres-background-intent=\"evidence\""));
    assert!(html.contains("Feasible region contracts after propagation"));
    assert!(html.contains("showing how propagation removes most candidate assignments"));
    assert!(html.contains("data-generated-slide=\"background-image\""));
    assert!(html.contains("aria-hidden=\"true\""));
    assert!(!html.contains("data-zpres-background-intent=\"decorative\""));
}

#[test]
fn v1_reference_passes_screen_and_print_visual_characterization() {
    let temp = tempdir().unwrap();
    let output_dir = temp.path().join("review");
    let pdf_path = temp.path().join("reference.pdf");
    let print_html_path = temp.path().join("export-print.html");
    let output = zpres(temp.path())
        .args([
            "theme",
            "check",
            repository_path("themes/reference").to_str().unwrap(),
            "--visual",
            "--fixture",
            repository_path("themes/reference/specimen.zp.md")
                .to_str()
                .unwrap(),
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
        eprintln!("skipping browser-backed v1 characterization: {stderr}");
        return;
    }
    let report_debug = fs::read_to_string(output_dir.join("visual-report.json"))
        .unwrap_or_else(|_| "<visual report unavailable>".to_string());
    assert!(
        output.status.success(),
        "v1 visual characterization failed\nstdout:\n{stdout}\nstderr:\n{stderr}\nreport:\n{report_debug}"
    );

    let report: Value = serde_json::from_str(&report_debug).unwrap();
    assert_eq!(report["schema_version"], 9);
    assert_eq!(
        report["room_profile"]["selection"],
        "projected-room-default"
    );
    assert_eq!(report["room_profile"]["decision_status"], "provisional");
    assert_eq!(report["room_profile"]["enforcement_status"], "report_only");
    for field in ["visual_status", "screen_status", "print_status"] {
        assert_eq!(report[field], "passed", "unexpected {field}");
    }
    assert_eq!(report["objective_gate_status"], "passed");
    assert_eq!(report["composition_review_status"], "required");
    assert_eq!(report["final_source_release_approval"], "pending-review");
    assert_eq!(report["expected_screen_slides"], 11);
    assert_eq!(report["captured_screen_slides"], 11);
    assert_eq!(report["expected_screen_states"], 24);
    assert_eq!(report["captured_screen_states"], 24);
    assert_eq!(report["expected_pages"], 13);
    assert_eq!(report["captured_pages"], 13);
    for field in ["warnings", "failures", "screen_failures", "print_failures"] {
        assert!(
            report[field].as_array().unwrap().is_empty(),
            "v1 characterization reported {field}: {}",
            report[field]
        );
    }

    let screen = &report["screen_states"][0];
    let print = &report["pages"][0];
    for surface in [screen, print] {
        assert_eq!(surface["slide_bounds"]["width"], 1280.0);
        assert_eq!(surface["slide_bounds"]["height"], 720.0);
        assert_eq!(surface["canvas_bounds"], surface["slide_bounds"]);
        assert_eq!(surface["content_bounds"]["x"], 72.0);
        assert_eq!(surface["content_bounds"]["y"], 52.0);
        assert_eq!(surface["content_bounds"]["right"], 1208.0);
        assert_eq!(surface["content_bounds"]["bottom"], 628.0);
        assert_eq!(surface["footer_bounds"]["x"], 72.0);
        assert_eq!(surface["footer_bounds"]["y"], 644.0);
        assert_eq!(surface["footer_bounds"]["right"], 1208.0);
        assert_eq!(surface["footer_bounds"]["bottom"], 672.0);
        assert!(surface["overflow_elements"].as_array().unwrap().is_empty());
        assert!(
            surface["geometry_violations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(surface["autoscale_factor"].is_null());
        for field in [
            "focus_visible",
            "reduced_motion_preserves_state",
            "increased_contrast_preserves_state",
            "forced_colors_preserves_state",
            "current_step_has_non_color_cue",
            "non_color_state_cues_present",
        ] {
            assert_eq!(
                surface["accessibility_preferences"][field], true,
                "missing accessibility preference evidence for {field}: {surface}"
            );
        }
    }

    let screen_path = output_dir.join("screen-pages/state-001.png");
    let print_path = output_dir.join("pages/page-001.png");
    assert_eq!(image::open(&screen_path).unwrap().dimensions(), (1280, 720));
    assert_eq!(image::open(&print_path).unwrap().dimensions(), (1280, 720));
    let screen_print_rmse = normalized_rgb_rmse(&screen_path, &print_path);
    assert!(
        screen_print_rmse <= 0.005,
        "screen/print characterization exceeded normalized RGB RMSE tolerance 0.005: {screen_print_rmse:.6}"
    );

    zpres(temp.path())
        .args([
            "export",
            repository_path("themes/reference/specimen.zp.md")
                .to_str()
                .unwrap(),
            "--pdf",
            pdf_path.to_str().unwrap(),
            "--print-html",
            print_html_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let print_html = fs::read_to_string(&print_html_path).unwrap();
    for authored_text in [
        "Semantic regions stay put",
        "docs/theme-authoring.md",
        "Figure evidence should dominate the body",
        "Page-per-Step export is opt-in",
        "Detail: keep Technical evidence readable",
        "reference · teaching Deck",
        "1 / 11",
    ] {
        assert!(
            print_html.contains(authored_text),
            "print HTML lost authored text {authored_text:?}"
        );
    }
    let pdf = lopdf::Document::load(&pdf_path).unwrap();
    assert_eq!(pdf.get_pages().len(), 13);
    let page_id = *pdf.get_pages().get(&1).unwrap();
    let pdf_content = pdf.get_page_content(page_id).unwrap();
    assert!(!pdf_content.is_empty(), "PDF page has no content stream");

    match std::process::Command::new("pdftotext")
        .args([pdf_path.to_str().unwrap(), "-"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let pdf_text = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            for authored_text in [
                "Semantic regions stay put",
                "docs/theme-authoring.md",
                "Figure evidence should dominate the body",
                "Page-per-Step export is opt-in",
                "Detail: keep Technical evidence readable",
                "REFERENCE · TEACHING DECK",
                "1 / 11",
            ] {
                assert!(
                    pdf_text.contains(authored_text),
                    "independent PDF text extraction lost authored text {authored_text:?}: {pdf_text:?}"
                );
            }
        }
        Ok(output) => panic!(
            "pdftotext failed while inspecting the exported PDF: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping independent PDF text inspection: pdftotext was not found")
        }
        Err(error) => panic!("failed to run pdftotext: {error}"),
    }

    let independent_pdf_page = temp.path().join("independent-pdf-page.png");
    let poppler = std::process::Command::new("pdftoppm")
        .args([
            "-singlefile",
            "-png",
            "-r",
            "96",
            pdf_path.to_str().unwrap(),
            independent_pdf_page.with_extension("").to_str().unwrap(),
        ])
        .output();
    match poppler {
        Ok(output) if output.status.success() => {
            assert_eq!(
                image::open(&independent_pdf_page).unwrap().dimensions(),
                (1280, 720)
            );
            let print_pdf_rmse = normalized_rgb_rmse(&print_path, &independent_pdf_page);
            assert!(
                print_pdf_rmse <= 0.11,
                "direct Chromium print capture and independent PDF raster exceeded normalized RGB RMSE tolerance 0.11: {print_pdf_rmse:.6}"
            );
            for (region, x, y, width, height, minimum_ink) in [
                ("title", 72, 44, 670, 66, 1_000),
                ("body", 72, 180, 775, 295, 5_000),
                ("source", 72, 600, 815, 30, 500),
                ("footer", 72, 647, 290, 26, 200),
                ("page number", 1165, 647, 43, 26, 100),
            ] {
                let ink = dark_rgb_pixels(&independent_pdf_page, x, y, width, height);
                assert!(
                    ink >= minimum_ink,
                    "independent PDF raster lost visible {region} ink: observed {ink} dark pixels, expected at least {minimum_ink}"
                );
            }
        }
        Ok(output) => panic!(
            "pdftoppm failed while independently rasterizing the exported PDF: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping independent PDF raster equivalence: pdftoppm was not found")
        }
        Err(error) => panic!("failed to run pdftoppm: {error}"),
    }

    let provenance: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["schema_version"], 6);
    assert_eq!(provenance["capture_mode"], "normal");
    assert_eq!(
        provenance["room_profile"]["selection"],
        "projected-room-default"
    );
    assert_eq!(provenance["theme_name"], "reference");
    assert_eq!(provenance["theme_api_version"], 1);
    assert!(provenance["presentation_plan_sha256"].is_string());

    let theme_api = fs::read_to_string(output_dir.join("theme-api.txt")).unwrap();
    assert!(theme_api.contains(".zpres-api-v1"));
    assert!(theme_api.contains(".zpres-slide-frame"));
    assert!(theme_api.contains(".zpres-slide-body"));
    let stable_v1 = theme_api
        .split("## Stable Theme API v1 selectors\n\n")
        .nth(1)
        .unwrap()
        .split("## Internal selectors")
        .next()
        .unwrap();
    assert!(!stable_v1.contains(".zpres-slide-ornament\n"));
    assert!(stable_v1.contains(".zpres-block-layout\n"));
    assert!(theme_api.contains(".zpres-slide-ornament\n"));
}

#[test]
fn v1_reference_shaped_print_text_loss_is_a_retained_objective_failure() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("reference-inkless-print");
    fs::create_dir_all(&theme_dir).unwrap();
    for name in ["theme.toml", "theme.css.tmpl", "print.css.tmpl"] {
        fs::copy(
            repository_path("themes/reference").join(name),
            theme_dir.join(name),
        )
        .unwrap();
    }
    let print_css = theme_dir.join("print.css.tmpl");
    let mut css = fs::read_to_string(&print_css).unwrap();
    css.push_str(
        r#"
.zpres-slide-header, .zpres-slide-header *,
.zpres-slide-body, .zpres-slide-body *,
.zpres-slide-footer, .zpres-slide-footer * {
  color: transparent !important;
  -webkit-text-fill-color: transparent !important;
  text-shadow: none !important;
}
"#,
    );
    fs::write(&print_css, css).unwrap();

    let output_dir = temp.path().join("review");
    let output = zpres(temp.path())
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--visual",
            "--fixture",
            repository_path("themes/reference/specimen.zp.md")
                .to_str()
                .unwrap(),
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
        eprintln!("skipping browser-backed semantic ink regression: {stderr}");
        return;
    }
    assert!(
        !output.status.success(),
        "inkless print unexpectedly passed"
    );

    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["objective_gate_status"], "failed");
    assert_eq!(report["screen_status"], "passed");
    assert_eq!(report["print_status"], "failed");
    let finding = report["print_failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["code"] == "missing-semantic-text-ink")
        .expect("retained missing semantic text ink finding");
    assert_eq!(finding["surface"], "print");
    assert_eq!(finding["slide_id"], "section-1-main");
    assert_eq!(finding["page"], 1);
    let evidence = finding["evidence"].as_str().unwrap();
    assert_eq!(evidence, "pages/page-001.png");
    assert!(output_dir.join(evidence).is_file());
    assert!(
        finding["message"]
            .as_str()
            .unwrap()
            .contains("original-size evidence")
    );
}

#[test]
fn audited_science_section_title_composition_does_not_overlap_screen_or_print() {
    let temp = tempdir().unwrap();
    let output_dir = temp.path().join("science-review");
    let output = zpres(temp.path())
        .args([
            "theme",
            "check",
            repository_path("themes/science").to_str().unwrap(),
            "--visual",
            "--fixture",
            repository_path("fixtures/canonical/wide-sweep.zp.md")
                .to_str()
                .unwrap(),
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
        eprintln!("skipping browser-backed Science overlap regression: {stderr}");
        return;
    }
    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("visual-report.json")).unwrap())
            .unwrap();

    let chart_state = report["screen_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["route"]["hash"] == "#/4/0")
        .expect("retained browser-backed Runtime chart state");
    let chart_body_ink = chart_state["semantic_text_regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|region| region["region"] == "body")
        .expect("Runtime chart body semantic text region")["raster_ink"]
        .clone();
    assert!(
        chart_body_ink["ink_pixels"].as_u64().unwrap() >= 12,
        "visible SVG chart labels did not produce semantic raster ink: {chart_body_ink}"
    );
    assert!(
        report["screen_failures"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| !(finding["code"] == "missing-semantic-text-ink"
                && finding["route"] == "#/4/0")),
        "visible SVG chart labels produced a false missing-ink failure"
    );

    for surface in ["screen", "print"] {
        let findings = report[format!("{surface}_failures")].as_array().unwrap();
        assert!(
            findings
                .iter()
                .all(|finding| !(finding["code"] == "header-body-overlap"
                    && finding["slide_id"] == "section-1-main")),
            "repaired Science Section-title still overlaps on {surface}: {findings:?}"
        );
    }

    for (surface, observations) in [("screen", "screen_states"), ("print", "pages")] {
        let observation = report[observations]
            .as_array()
            .unwrap()
            .iter()
            .find(|observation| observation["slide_id"] == "section-1-main")
            .unwrap_or_else(|| panic!("missing audited {surface} Section-title observation"));
        let title = observation["design_measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|measurement| measurement["element"] == "h1.zpres-slide-title")
            .expect("audited Section-title measurement");
        assert_eq!(
            title["line_count"], 3,
            "{surface} title did not use three lines"
        );

        let regions = observation["semantic_text_regions"].as_array().unwrap();
        let header_bottom = regions
            .iter()
            .find(|region| region["region"] == "header")
            .expect("audited Section-title Header region")["text_bounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|bounds| bounds["bottom"].as_f64())
            .fold(f64::NEG_INFINITY, f64::max);
        let body_top = regions
            .iter()
            .find(|region| region["region"] == "body")
            .expect("audited Section-title Body region")["text_bounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|bounds| bounds["y"].as_f64())
            .fold(f64::INFINITY, f64::min);
        assert!(
            body_top > header_bottom,
            "{surface} Section-title Header ended at {header_bottom}, Body began at {body_top}"
        );
    }
}

#[test]
fn v1_authored_background_gate_is_independent_on_screen_and_print() {
    fn prepare_case(root: &Path, hidden_target: &str) -> PathBuf {
        let theme_dir = root.join("themes/reference");
        fs::create_dir_all(&theme_dir).unwrap();
        for file in ["theme.toml", "theme.css.tmpl", "print.css.tmpl"] {
            fs::copy(
                repository_path(format!("themes/reference/{file}")),
                theme_dir.join(file),
            )
            .unwrap();
        }
        let stylesheet = if hidden_target == "screen" {
            theme_dir.join("theme.css.tmpl")
        } else {
            theme_dir.join("print.css.tmpl")
        };
        let mut css = fs::read_to_string(&stylesheet).unwrap();
        let discard = if hidden_target == "screen" {
            "background-image: none !important;"
        } else {
            "display: none !important;"
        };
        css.push_str(&format!(
            "\n.zpres-theme-reference[data-zpres-output-target=\"{hidden_target}\"] .zpres-slide-background {{ {discard} }}\n"
        ));
        fs::write(stylesheet, css).unwrap();

        let assets = root.join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(
            assets.join("evidence.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="720"><rect width="448" height="720" fill="#0f766e"/><circle cx="224" cy="250" r="130" fill="#f4b942"/></svg>"##,
        )
        .unwrap();
        let source = root.join("background-gate.zp.md");
        fs::write(
            &source,
            r##"---
title: "V1 authored background gate"
theme: "reference"
theme_dirs:
  - "themes"
aspect: "16:9"
background_image:
  src: "assets/evidence.svg"
  intent: contextual
  alt: "Evidence field"
  split: "right:35%"
  splash: true
---

# Clean title

The first Slide has no authored image background, but it has live Step states.

::: steps pdf="pages"
1. First state.
2. Second state.
:::

--

## Detail evidence

This Detail slide receives the Deck background after the clean title.

---

![bg right:35% intent="evidence" alt="Evidence field" description="The evidence field occupies the right side and must remain available in the reading order as well as in the painted background."](assets/evidence.svg)

# Background evidence

The semantic image must survive independently on screen and print.
"##,
        )
        .unwrap();
        source
    }

    fn visual_report(root: &Path, source: &Path, hidden_target: &str) -> Option<Value> {
        let output_dir = root.join("review");
        let output = zpres(root)
            .args([
                "theme",
                "check",
                root.join("themes/reference").to_str().unwrap(),
                "--visual",
                "--fixture",
                source.to_str().unwrap(),
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
            eprintln!("skipping v1 background gate test: {stderr}");
            return None;
        }
        let report_text = fs::read_to_string(output_dir.join("visual-report.json"))
            .unwrap_or_else(|_| "<visual report unavailable>".to_string());
        assert!(
            !output.status.success(),
            "the {hidden_target}-hidden v1 Theme should fail its visual gate\nstdout:\n{}\nstderr:\n{stderr}\nreport:\n{report_text}",
            String::from_utf8_lossy(&output.stdout),
        );
        Some(serde_json::from_str(&report_text).unwrap())
    }

    for hidden_target in ["screen", "print"] {
        let temp = tempdir().unwrap();
        let source = prepare_case(temp.path(), hidden_target);
        let Some(report) = visual_report(temp.path(), &source, hidden_target) else {
            return;
        };
        assert_eq!(report["schema_version"], 9);
        let visible_target = if hidden_target == "screen" {
            "print"
        } else {
            "screen"
        };
        assert_eq!(report[format!("{hidden_target}_status")], "failed");
        assert_eq!(report[format!("{visible_target}_status")], "passed");

        let screen_states = report["screen_states"].as_array().unwrap();
        assert!(screen_states.iter().any(|state| {
            state["route"]["slide_id"] == "section-1-main" && state["route"]["step"] == 2
        }));
        assert!(screen_states.iter().any(|state| {
            state["role"] == "detail"
                && state["authored_background_expectation"]["origin"] == "deck"
        }));
        assert!(screen_states.iter().any(|state| {
            state["generated"] == "background-image"
                && state["authored_background_expectation"]["origin"] == "generated_splash"
        }));
        assert!(report["pages"].as_array().unwrap().iter().any(|page| {
            page["generated"] == "background-image"
                && page["authored_background_expectation"]["phase"] == "splash"
        }));
        let pages = report["pages"].as_array().unwrap();
        assert_eq!(pages[0]["slide_id"], "section-1-main");
        assert_eq!(pages[1]["slide_id"], "section-1-main");
        assert!(pages[0].get("authored_background_expectation").is_none());
        assert!(pages[1].get("authored_background_expectation").is_none());
        assert_eq!(pages[2]["slide_id"], "section-1-detail-1");
        assert_eq!(
            pages[2]["authored_background_expectation"]["origin"],
            "deck"
        );
        assert_eq!(pages[3]["generated"], "background-image");
        assert_eq!(
            pages[3]["authored_background_expectation"]["phase"],
            "splash"
        );
        assert_eq!(pages[4]["slide_id"], "section-2-main");

        let failures = report[format!("{hidden_target}_failures")]
            .as_array()
            .unwrap();
        assert!(failures.iter().any(|failure| {
            failure["code"] == "discarded-authored-background"
                && failure["slide_id"] == "section-2-main"
                && failure["element"] == ".zpres-slide-background"
        }));
        assert!(
            report[format!("{visible_target}_failures")]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let states = if visible_target == "screen" {
            report["screen_states"].as_array().unwrap()
        } else {
            report["pages"].as_array().unwrap()
        };
        let preserved = states
            .iter()
            .find(|state| {
                state["authored_background_expectation"]["origin"] == "slide"
                    && state["slide_id"] == "section-2-main"
            })
            .expect("slide-local background expectation on the positive surface");
        assert_eq!(
            preserved["authored_background_expectation"]["source"],
            "assets/evidence.svg"
        );
        assert_eq!(preserved["authored_background"]["layer_count"], 1);
        assert_eq!(preserved["authored_background"]["visible"], true);
        assert_eq!(preserved["authored_background"]["intersects_slide"], true);
        assert_eq!(preserved["authored_background"]["source_preserved"], true);
        assert_eq!(preserved["authored_background"]["bounds"]["width"], 448.0);
        assert_eq!(preserved["canvas_bounds"]["width"], 832.0);
        assert_eq!(
            preserved["authored_background"]["bounds"]["x"],
            preserved["canvas_bounds"]["right"]
        );

        if hidden_target == "print" {
            let pdf = temp.path().join("blocked.pdf");
            zpres(temp.path())
                .args([
                    "export",
                    source.to_str().unwrap(),
                    "--pdf",
                    pdf.to_str().unwrap(),
                ])
                .assert()
                .failure()
                .stderr(predicate::str::contains("discarded-authored-background"));
            assert!(!pdf.exists(), "a failed v1 preflight published a PDF");
        }
    }
}

#[test]
fn v1_reference_rejects_non_widescreen_decks_before_writing_output() {
    let temp = tempdir().unwrap();
    let theme_dir = repository_path("themes");
    let source = temp.path().join("aspect.zp.md");
    let output = temp.path().join("aspect-out");
    fs::write(
        &source,
        format!(
            "---\ntheme: \"reference\"\ntheme_dirs:\n  - \"{}\"\naspect: \"4:3\"\n---\n\n# Unsupported aspect\n\nOrdinary body.\n",
            theme_dir.display()
        ),
    )
    .unwrap();

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("supports only 16:9"));
    assert!(!output.exists(), "failed v1 build published aspect output");
}

#[test]
fn v1_columns_render_typed_nested_content_on_screen_and_print() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("columns.zp.md");
    let output = temp.path().join("html");
    let print_html = temp.path().join("print.html");
    fs::write(
        &source,
        format!(
            "---\ntheme: \"reference\"\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Typed Columns\n\n:::: columns widths=\"2/1\" gap=\"4\" align=\"start\"\nEvidence column:\n> [!NOTE] Bound\n> Propagation preserves the invariant.\n\nReasoning column:\n1. Establish the model.\n2. State the consequence.\n::::\n",
            repository_path("themes").display()
        ),
    )
    .unwrap();

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    for document in [&screen, &print] {
        assert!(document.contains("data-layout-kind=\"columns\""));
        assert!(document.contains("grid-template-columns:2fr 1fr"));
        assert!(document.contains("data-block-type=\"callout\""));
        assert!(document.contains("data-block-type=\"list\""));
        assert!(document.contains("<ol"));
    }
}

#[test]
fn v1_grid_and_stack_render_distinct_typed_structures_on_screen_and_print() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("grid-stack.zp.md");
    let output = temp.path().join("html");
    let print_html = temp.path().join("print.html");
    fs::write(
        &source,
        format!(
            "---\ntheme: \"reference\"\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Evidence grid\n\n::::: grid tracks=\"2/1/1\" gap=\"4\" align=\"start\"\n:::: cell name=\"Primary\" column=\"1\" span=\"2\"\n> [!NOTE] Bound\n> Propagation preserves the invariant.\n::::\n\n:::: cell name=\"Metric\" column=\"3\"\n1. First value.\n2. Second value.\n::::\n:::::\n\n---\n\n# Reasoning stack\n\n::::: stack gap=\"4\" align=\"stretch\"\n:::: item name=\"Question\"\nWhat must remain invariant?\n::::\n\n:::: item name=\"Answer\"\n::: steps\n1. Preserve source order.\n2. Preserve static output.\n:::\n::::\n:::::\n",
            repository_path("themes").display()
        ),
    )
    .unwrap();

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    for document in [&screen, &print] {
        assert!(document.contains("data-layout-kind=\"grid\""));
        assert!(document.contains("data-layout-tracks=\"2/1/1\""));
        assert!(
            document.contains("grid-template-columns:minmax(0,2fr) minmax(0,1fr) minmax(0,1fr)")
        );
        assert!(document.contains("data-grid-column=\"1\""));
        assert!(document.contains("data-grid-column-span=\"2\""));
        assert!(document.contains("grid-column:1/span 2"));
        assert!(document.contains("data-layout-kind=\"stack\""));
        assert!(document.contains("display:flex;flex-direction:column"));
        assert!(document.contains("data-block-type=\"steps\""));
    }
}

#[test]
fn v1_overlay_and_aside_preserve_semantic_roles_on_screen_and_print() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("overlay-aside.zp.md");
    let output = temp.path().join("html");
    let print_html = temp.path().join("print.html");
    fs::write(
        &source,
        format!(
            "---\ntheme: \"reference\"\ntheme_dirs:\n  - \"{}\"\naspect: \"16:9\"\n---\n\n# Annotated evidence\n\n::::: overlay overlap=\"edge-only\"\n:::: base name=\"Evidence\"\n| n | result |\n|---:|:-------|\n| 1 | feasible |\n| 2 | fixed |\n::::\n:::: annotation name=\"Consequence\" anchor=\"top-end\" width=\"compact\"\nThe fixed point is unique.\n::::\n:::::\n\n---\n\n# Primary with context\n\n::::: aside supporting=\"standard\" gap=\"4\" align=\"start\"\n:::: primary name=\"Argument\"\nThe primary region retains at least two thirds of the available width.\n::::\n:::: supporting name=\"Context\"\n> [!NOTE] Scope\n> Supporting material remains subordinate and visible.\n::::\n:::::\n",
            repository_path("themes").display()
        ),
    )
    .unwrap();

    zpres(temp.path())
        .args([
            "build",
            source.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    zpres(temp.path())
        .args([
            "export",
            source.to_str().unwrap(),
            "--print-html",
            print_html.to_str().unwrap(),
        ])
        .assert()
        .success();

    let publication = current_html_generation(&output);
    let foundation = fs::read_to_string(
        publication
            .generation_path
            .join("assets/zpres-theme-api-v1.css"),
    )
    .unwrap();
    let screen = fs::read_to_string(publication.presentation_index).unwrap();
    let print = fs::read_to_string(print_html).unwrap();
    for document in [&screen, &print] {
        assert!(document.contains("data-layout-kind=\"overlay\""));
        assert!(document.contains("data-overlay-policy=\"edge-only\""));
        assert!(
            document
                .contains("data-overlay-forbidden-boundaries=\"title footer caption safe-area\"")
        );
        assert!(document.contains("data-region-role=\"base\""));
        assert!(document.contains("data-region-role=\"annotation\""));
        assert!(document.contains("data-overlay-anchor=\"top-end\""));
        assert!(document.contains("grid-area:1/1;place-self:start end"));
        assert!(document.contains("data-layout-kind=\"aside\""));
        assert!(document.contains("data-region-role=\"primary\""));
        assert!(document.contains("data-region-role=\"supporting\""));
        assert!(document.contains("grid-template-columns:minmax(0,3fr) minmax(0,1.4fr)"));
    }
    assert!(foundation.contains("@container body (max-width: 38rem)"));
    assert!(foundation.contains("data-layout-kind=\"overlay\""));
    assert!(foundation.contains("flex-direction: column"));
    assert!(foundation.contains("data-layout-kind=\"aside\""));
}
