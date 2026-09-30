use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn write_theme_manifest(root: &std::path::Path, name: &str) {
    let theme_dir = root.join(name);
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(
        theme_dir.join("theme.toml"),
        format!(
            r##"[theme]
name = "{name}"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim"]

[parameters.accent]
type = "color"
default = "#0891b2"
"##
        ),
    )
    .unwrap();
}

fn current_html_generation(output_root: &Path) -> zpres::html::CurrentHtmlBundlePublication {
    zpres::html::current_html_bundle_publication(output_root)
        .unwrap()
        .expect("the command should publish an HTML generation")
}

fn assert_local_review_links_exist(output_root: &Path, html: &str) {
    let mut rest = html;
    while let Some(start) = rest.find("href=\"") {
        rest = &rest[start + "href=\"".len()..];
        let Some(end) = rest.find('"') else {
            panic!("unterminated href in review HTML");
        };
        let href = &rest[..end];
        rest = &rest[end + 1..];
        if href.starts_with('#') || href.contains("://") {
            continue;
        }
        let path = href.split(['?', '#']).next().unwrap();
        assert!(
            output_root.join(path).is_file(),
            "local review link does not exist: {href}"
        );
    }
}

fn write_switchable_theme_manifest(
    root: &Path,
    name: &str,
    palette_parameter: &str,
    default_palette: &str,
    palette_values: &[&str],
) {
    let theme_dir = root.join(name);
    fs::create_dir_all(&theme_dir).unwrap();
    let palette_values = palette_values
        .iter()
        .map(|value| format!(r#""{value}""#))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        theme_dir.join("theme.toml"),
        format!(
            r##"[theme]
name = "{name}"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "comparison"]
palette_parameter = "{palette_parameter}"

[parameters.{palette_parameter}]
type = "enum"
default = "{default_palette}"
values = [{palette_values}]

[parameters.density]
type = "enum"
default = "normal"
values = ["compact", "normal", "spacious"]

[parameters.footer]
type = "enum"
default = "slide-number"
values = ["none", "slide-number", "section-progress"]
"##
        ),
    )
    .unwrap();
}

fn write_checkable_theme(root: &Path, name: &str) {
    let theme_dir = root.join(name);
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(
        theme_dir.join("theme.toml"),
        format!(
            r##"[theme]
name = "{name}"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]

[presets.spotlight]
variant = "claim"
classes = ["lead"]
autoscale = true
transition = "fade"

[presets.spotlight.theme_params]
accent = "#0891b2"

[parameters.accent]
type = "color"
default = "#0891b2"

[parameters.background]
type = "color"

[parameters.text]
type = "color"
"##
        ),
    )
    .unwrap();
    fs::write(
        theme_dir.join("theme.css.tmpl"),
        ".zpres-slide { color: {{param.accent}}; }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
    )
    .unwrap();
    fs::write(
        theme_dir.join("print.css.tmpl"),
        ".zpres-print-slide {}\n.zpres-slide { color: {{param.accent}}; }\n.zpres-slide-canvas {}\n.zpres-block {}\n",
    )
    .unwrap();
}

#[test]
fn theme_check_validates_theme_packages() {
    let temp = tempdir().unwrap();
    write_checkable_theme(temp.path(), "cli-theme");
    let theme_dir = temp.path().join("cli-theme");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["theme", "check", theme_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Theme 'cli-theme' 0.1.0 is valid")
                .and(predicate::str::contains("checked_files = 2"))
                .and(predicate::str::contains(
                    "theme_parameters = accent:color=#0891b2",
                ))
                .and(predicate::str::contains("rendered_variants = defaults"))
                .and(predicate::str::contains("declared_feature_hooks = none"))
                .and(predicate::str::contains("fixture = "))
                .and(predicate::str::contains(
                    "fixture_variants = claim, comparison, dense, derivation, figure, section-title",
                ))
                .and(predicate::str::contains("fixture_presets = none"))
                .and(predicate::str::contains(
                    "fixture_classes = hero, lead, result",
                ))
                .and(predicate::str::contains(
                    "fixture_blocks = callout, chart, code, diagram, figure, fit-text, footnotes, gallery, heading, html-only, layout, list, math, media, paragraph, quote, speaker-notes, steps, table",
                ))
                .and(predicate::str::contains("fixture_layouts = columns"))
                .and(predicate::str::contains(
                    "fixture_media = audio, iframe, video",
                ))
                .and(predicate::str::contains(
                    "fixture_features = autoscale, background-image, background-splash, background-split, background-treatment, chart-local-data, code-reveal, detail-slides, figure-align, figure-fit, figure-radius, figure-size, figure-treatment, fit-text, footer, footnotes, gallery-columns, html-only, image-gallery, list-reveal, media-align, media-autoadvance, media-fit, media-hidden, media-poster, media-size, media-start, mermaid-diagram, slide-classes, speaker-notes, steps-final-state, steps-pages, transitions",
                ))
                .and(predicate::str::contains(
                    "fixture_uncovered_features = slide-presets",
                ))
                .and(predicate::str::contains("wide-sweep.zp.md"))
                .and(predicate::str::contains("12 section(s), 18 pdf page(s)")),
        );
}

#[test]
fn theme_check_can_write_html_specimen() {
    let temp = tempdir().unwrap();
    write_checkable_theme(temp.path(), "specimen-theme");
    let theme_dir = temp.path().join("specimen-theme");
    let output_dir = temp.path().join("specimen-out");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--write-specimen",
            output_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Theme 'specimen-theme' 0.1.0 is valid")
                .and(predicate::str::contains("specimen = "))
                .and(predicate::str::contains(
                    "review.html, index.html, print.html, print-notes.html, speaker-notes.txt, theme-check.txt, theme-api.txt",
                )),
        );

    assert!(output_dir.join("review.html").is_file());
    assert!(output_dir.join("index.html").is_file());
    assert!(output_dir.join("print.html").is_file());
    assert!(output_dir.join("print-notes.html").is_file());
    assert!(output_dir.join("speaker-notes.txt").is_file());
    assert!(output_dir.join("theme-check.txt").is_file());
    assert!(output_dir.join("theme-api.txt").is_file());
    let publication = current_html_generation(&output_dir);
    assert!(
        publication
            .generation_path
            .join("assets/theme.css")
            .is_file()
    );
    let review_html = fs::read_to_string(output_dir.join("review.html")).unwrap();
    assert_local_review_links_exist(&output_dir, &review_html);
    let index_html = fs::read_to_string(publication.presentation_index).unwrap();
    let print_html = fs::read_to_string(output_dir.join("print.html")).unwrap();
    let print_notes_html = fs::read_to_string(output_dir.join("print-notes.html")).unwrap();
    let notes_text = fs::read_to_string(output_dir.join("speaker-notes.txt")).unwrap();
    let report = fs::read_to_string(output_dir.join("theme-check.txt")).unwrap();
    let theme_api = fs::read_to_string(output_dir.join("theme-api.txt")).unwrap();
    assert!(review_html.contains("zpres theme specimen review"));
    assert!(review_html.contains("href=\"index.html\""));
    assert!(review_html.contains("src=\"print-notes.html\""));
    assert!(review_html.contains("href=\"speaker-notes.txt\""));
    assert!(review_html.contains("href=\"theme-api.txt\""));
    assert!(review_html.contains("Theme API"));
    assert!(review_html.contains("fixture_uncovered_features"));
    assert!(index_html.contains("zpres-theme-specimen-theme"));
    assert!(print_html.contains(r#"data-zpres-ready="pending""#));
    assert!(print_html.contains("zpres-print-slide"));
    assert!(print_notes_html.contains(r#"data-generated-slide="speaker-notes""#));
    assert!(print_notes_html.contains("zpres-speaker-notes-print-body"));
    assert!(notes_text.contains("# Speaker Notes"));
    assert!(report.contains("specimen_variant = defaults"));
    assert!(report.contains("notes_pdf_pages = "));
    assert!(report.contains("fixture_blocks = "));
    assert!(report.contains("fixture_uncovered_features = "));
    assert!(report.contains("unknown_selectors = "));
    assert!(report.contains("fixture_dead_selectors = "));
    assert!(theme_api.contains("# zpres theme authoring reference"));
    assert!(theme_api.contains("## Resolved CSS variables"));
    assert!(theme_api.contains("--zpres-color-accent"));
    assert!(theme_api.contains("## Stable Theme API v1 selectors"));
    assert!(theme_api.contains("## Internal selectors (not a Theme API contract)"));
    assert!(theme_api.contains("unknown_selectors = "));
    assert!(theme_api.contains("fixture_dead_selectors = "));
    assert!(theme_api.contains(".zpres-slide-canvas"));
    assert!(theme_api.contains(".zpres-slide-frame"));
    assert!(theme_api.contains(".zpres-block-layout"));
    assert!(theme_api.contains(".zpres-api-v1"));
    assert!(theme_api.contains(".zpres-slide-frame"));
    assert!(theme_api.contains(".zpres-slide-body"));
    assert!(theme_api.contains(".zpres-chart-svg"));
    assert!(theme_api.contains("## Supported feature hook names"));
}

#[test]
fn theme_check_write_specimen_requires_fixture() {
    let temp = tempdir().unwrap();
    write_checkable_theme(temp.path(), "fast-theme");
    let theme_dir = temp.path().join("fast-theme");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--no-fixture",
            "--write-specimen",
            temp.path().join("specimen-out").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--write-specimen requires a fixture",
        ));
}

#[test]
fn theme_check_all_variants_requires_specimen_output() {
    let temp = tempdir().unwrap();
    write_checkable_theme(temp.path(), "variant-theme");
    let theme_dir = temp.path().join("variant-theme");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--all-variants",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--all-variants requires --write-specimen",
        ));
}

#[test]
fn theme_check_inspection_requires_visual_capture() {
    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["theme", "check", "themes/debug", "--inspection"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--inspection requires --visual and --write-specimen",
        ));
}

#[test]
fn theme_check_can_skip_fixture_rendering() {
    let temp = tempdir().unwrap();
    write_checkable_theme(temp.path(), "fast-theme");
    let theme_dir = temp.path().join("fast-theme");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--no-fixture",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Theme 'fast-theme' 0.1.0 is valid")
                .and(predicate::str::contains("rendered_variants = defaults"))
                .and(predicate::str::contains("fixture = ").not()),
        );
}

#[test]
fn theme_check_fails_on_broken_theme_packages() {
    let temp = tempdir().unwrap();
    write_checkable_theme(temp.path(), "broken-theme");
    fs::write(
        temp.path().join("broken-theme").join("theme.css.tmpl"),
        ".zpres-slide { color: {{param.missing}}; }",
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            temp.path().join("broken-theme").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "references missing parameter 'missing'",
        ));
}

#[test]
fn theme_init_creates_a_checkable_theme_package() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("fresh-theme");

    let mut init_command = Command::cargo_bin("zpres").unwrap();
    init_command
        .args(["theme", "init", theme_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created theme 'fresh-theme'"));

    assert!(theme_dir.join("theme.toml").is_file());
    assert!(theme_dir.join("theme.css.tmpl").is_file());
    assert!(theme_dir.join("print.css.tmpl").is_file());
    assert!(theme_dir.join("specimen.zp.md").is_file());
    assert!(theme_dir.join("assets").is_dir());
    assert!(theme_dir.join("assets/specimen-visual.svg").is_file());
    assert!(theme_dir.join("assets/specimen-clip.mp4").is_file());
    assert!(theme_dir.join("assets/specimen-voice.mp3").is_file());
    assert!(theme_dir.join("data/specimen-runtime.csv").is_file());

    let manifest = fs::read_to_string(theme_dir.join("theme.toml")).unwrap();
    assert!(manifest.contains("api_version = 1"));
    assert!(manifest.contains("modules = [\"scientific-data\"]"));
    let source = fs::read_to_string(theme_dir.join("specimen.zp.md")).unwrap();
    assert!(source.contains("aspect: \"16:9\""));
    let screen_css = fs::read_to_string(theme_dir.join("theme.css.tmpl")).unwrap();
    assert!(screen_css.contains(".zpres-theme-fresh-theme .zpres-slide-frame"));
    assert!(screen_css.contains(".zpres-slide-header"));
    assert!(screen_css.contains(".zpres-slide-title"));
    assert!(screen_css.contains(".zpres-slide-body"));
    assert!(screen_css.contains(".zpres-slide-primary"));
    assert!(screen_css.contains(".zpres-slide-sources"));
    assert!(screen_css.contains(".zpres-slide-footer"));
    assert!(!screen_css.contains("@tailwind"));
    assert!(!screen_css.contains("@import"));
    assert!(!screen_css.contains("https://"));
    let print_css = fs::read_to_string(theme_dir.join("print.css.tmpl")).unwrap();
    assert!(print_css.contains(".zpres-theme-fresh-theme .zpres-print-slide"));

    let mut check_command = Command::cargo_bin("zpres").unwrap();
    check_command
        .args(["theme", "check", theme_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Theme 'fresh-theme' 0.1.0 is valid")
                .and(predicate::str::contains(
                    "declared_modules = scientific-data",
                ))
                .and(predicate::str::contains(
                    "font_heading:font=ui-sans-serif",
                ))
                .and(predicate::str::contains(
                    "rendered_variants = defaults, dark, light",
                ))
                .and(predicate::str::contains(
                    "declared_feature_hooks = autoscale, background-image, background-splash, background-split, background-treatment, chart-local-data, code-reveal, detail-slides, figure-align, figure-fit, figure-radius, figure-size, figure-treatment, fit-text, footer, footnotes, gallery-columns, html-only, image-gallery, list-reveal, media-align, media-autoadvance, media-fit, media-hidden, media-poster, media-size, media-start, mermaid-diagram, slide-classes, slide-presets, speaker-notes, steps-final-state, steps-pages, transitions",
                ))
                .and(predicate::str::contains("7 section(s), 17 pdf page(s)"))
                .and(predicate::str::contains(
                    "fixture_layouts = aside, columns, grid, overlay, stack",
                ))
                .and(predicate::str::contains("fixture_media = audio, video"))
                .and(predicate::str::contains("fixture_uncovered_features = none")),
        );

    let specimen_output = temp.path().join("specimen-out");
    let mut specimen_check = Command::cargo_bin("zpres").unwrap();
    specimen_check
        .args([
            "theme",
            "check",
            theme_dir.to_str().unwrap(),
            "--fixture",
            theme_dir.join("specimen.zp.md").to_str().unwrap(),
            "--write-specimen",
            specimen_output.to_str().unwrap(),
            "--all-variants",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("fixture = ")
                .and(predicate::str::contains("specimen.zp.md"))
                .and(predicate::str::contains("fixture_features = "))
                .and(predicate::str::contains("background-image"))
                .and(predicate::str::contains("background-treatment"))
                .and(predicate::str::contains("chart-local-data"))
                .and(predicate::str::contains("detail-slides"))
                .and(predicate::str::contains("figure-radius"))
                .and(predicate::str::contains("image-gallery"))
                .and(predicate::str::contains("media-poster"))
                .and(predicate::str::contains("mermaid-diagram"))
                .and(predicate::str::contains("fit-text"))
                .and(predicate::str::contains("footnotes"))
                .and(predicate::str::contains("html-only"))
                .and(predicate::str::contains("list-reveal"))
                .and(predicate::str::contains("slide-classes"))
                .and(predicate::str::contains("speaker-notes"))
                .and(predicate::str::contains("steps-final-state"))
                .and(predicate::str::contains("steps-pages"))
                .and(predicate::str::contains(
                    "fixture_uncovered_features = none",
                ))
                .and(predicate::str::contains("specimens = "))
                .and(predicate::str::contains("defaults, dark, light"))
                .and(predicate::str::contains("specimen_variant = defaults"))
                .and(predicate::str::contains("specimen_variant = dark"))
                .and(predicate::str::contains("specimen_variant = light")),
        );
    for variant in ["defaults", "dark", "light"] {
        let variant_output = specimen_output.join(variant);
        assert!(variant_output.join("review.html").is_file());
        assert!(variant_output.join("index.html").is_file());
        assert!(variant_output.join("print.html").is_file());
        assert!(variant_output.join("print-notes.html").is_file());
        assert!(variant_output.join("speaker-notes.txt").is_file());
        assert!(variant_output.join("theme-api.txt").is_file());
        let report_path = variant_output.join("theme-check.txt");
        assert!(report_path.is_file());
        let report = fs::read_to_string(report_path).unwrap();
        assert!(report.contains(&format!("specimen_variant = {variant}")));
        assert!(report.contains("notes_pdf_pages = "));
        assert!(report.contains("fixture_uncovered_features = none"));
        let review = fs::read_to_string(variant_output.join("review.html")).unwrap();
        assert!(review.contains(&format!("Variant <strong>{variant}</strong>")));
        assert!(review.contains("href=\"theme-check.txt\""));
        assert!(review.contains("href=\"theme-api.txt\""));
        assert!(review.contains("href=\"speaker-notes.txt\""));
        let theme_api = fs::read_to_string(variant_output.join("theme-api.txt")).unwrap();
        assert!(theme_api.contains(&format!("specimen_variant = {variant}")));
        assert!(theme_api.contains("feature_hooks = autoscale"));
        assert!(theme_api.contains(".zpres-block-media"));
        let notes = fs::read_to_string(variant_output.join("speaker-notes.txt")).unwrap();
        assert!(notes.contains("# Speaker Notes"));
    }
    let dark_publication = current_html_generation(&specimen_output.join("dark"));
    let dark_css =
        fs::read_to_string(dark_publication.generation_path.join("assets/theme.css")).unwrap();
    assert!(dark_css.contains("--zpres-color-background: #111816"));
    assert!(dark_css.contains("--zpres-param-mode: dark"));
}

#[test]
fn theme_init_refuses_existing_package_unless_forced() {
    let temp = tempdir().unwrap();
    let theme_dir = temp.path().join("force-theme");

    let mut init_command = Command::cargo_bin("zpres").unwrap();
    init_command
        .args(["theme", "init", theme_dir.to_str().unwrap()])
        .assert()
        .success();

    let mut second_init = Command::cargo_bin("zpres").unwrap();
    second_init
        .args(["theme", "init", theme_dir.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    let mut force_init = Command::cargo_bin("zpres").unwrap();
    force_init
        .args(["theme", "init", theme_dir.to_str().unwrap(), "--force"])
        .assert()
        .success();
}

#[test]
fn deck_init_creates_a_checkable_starter_deck() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("starter-talk.zp.md");

    let mut init_command = Command::cargo_bin("zpres").unwrap();
    init_command
        .args([
            "init",
            source_path.to_str().unwrap(),
            "--title",
            "Starter Deck",
            "--author",
            "Zayenz",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created starter deck"));

    let source = fs::read_to_string(&source_path).unwrap();
    assert!(source.contains(r#"title: "Starter Deck""#));
    assert!(source.contains("::::: comparison"));
    assert!(source.contains("::: class lead"));
    assert!(source.contains("::: steps pdf=\"pages\""));
    assert!(source.contains(":::: primary label=\"Baseline\""));
    assert!(source.contains(":::: supporting label=\"Alternative\""));
    assert!(source.contains("::: notes"));

    let mut check_command = Command::cargo_bin("zpres").unwrap();
    check_command
        .args(["check", source_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Deck is ready")
                .and(predicate::str::contains("theme = debug"))
                .and(predicate::str::contains("sections = 5"))
                .and(predicate::str::contains("static_pages = 7"))
                .and(predicate::str::contains("checked_math_blocks = 1")),
        );
}

#[test]
fn deck_init_refuses_existing_source_unless_forced() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("force-talk.zp.md");

    let mut init_command = Command::cargo_bin("zpres").unwrap();
    init_command
        .args(["init", source_path.to_str().unwrap()])
        .assert()
        .success();

    let mut second_init = Command::cargo_bin("zpres").unwrap();
    second_init
        .args(["init", source_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    let mut force_init = Command::cargo_bin("zpres").unwrap();
    force_init
        .args([
            "init",
            source_path.to_str().unwrap(),
            "--title",
            "Forced Deck",
            "--force",
        ])
        .assert()
        .success();

    let source = fs::read_to_string(&source_path).unwrap();
    assert!(source.contains(r#"title: "Forced Deck""#));
}

#[test]
fn deck_init_requires_zp_markdown_path() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.md");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["init", source_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must end with .zp.md"));
}

#[test]
fn authoring_and_config_initializers_refuse_owned_publication_roots() {
    let temp = tempdir().unwrap();
    let pages = temp.path().join("pages");
    fs::create_dir(&pages).unwrap();
    fs::write(
        pages.join(".zpres-raster-page-set.json"),
        b"reserved ownership marker",
    )
    .unwrap();
    fs::write(pages.join("page-001.png"), b"preserved page").unwrap();

    let source = pages.join("new-talk.zp.md");
    let mut deck_init = Command::cargo_bin("zpres").unwrap();
    deck_init
        .args(["init", source.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inside exclusively owned raster page directory",
        ));

    let theme = pages.join("new-theme");
    let mut theme_init = Command::cargo_bin("zpres").unwrap();
    theme_init
        .args(["theme", "init", theme.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inside exclusively owned raster page directory",
        ));

    let mut config_init = Command::cargo_bin("zpres").unwrap();
    config_init
        .env("XDG_CONFIG_HOME", &pages)
        .args(["config", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inside exclusively owned raster page directory",
        ));

    assert!(!source.exists());
    assert!(!theme.exists());
    assert!(!pages.join("zpres").exists());
    assert_eq!(
        fs::read(pages.join("page-001.png")).unwrap(),
        b"preserved page"
    );
}

#[test]
fn deck_check_reports_static_readiness_summary() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("theme-api-v1")
        .join("wedding-reference.zp.md");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["check", fixture.to_str().unwrap(), "--theme", "wedding"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Deck is ready")
                .and(predicate::str::contains("theme = wedding"))
                .and(predicate::str::contains("sections = 12"))
                .and(predicate::str::contains("static_pages = 13"))
                .and(predicate::str::contains("checked_chart_blocks = 1")),
        );
}

#[test]
fn deck_check_fails_on_unreliable_static_media() {
    let temp = tempdir().unwrap();
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(deck_root.join("assets")).unwrap();
    fs::write(deck_root.join("assets/clip.mp4"), "video").unwrap();
    let source_path = deck_root.join("media.zp.md");
    fs::write(
        &source_path,
        r#"# Media

::: video src="assets/clip.mp4"
Video without fallback.
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["check", source_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "requires a local poster image for reliable PDF export",
        ));
}

#[test]
fn deck_check_fails_on_invalid_slide_theme_params() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let theme_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
    fs::write(
        &source_path,
        format!(
            r##"---
theme: "paper-chalk"
theme_dirs:
  - "{}"
---

# Bad local theme

::: theme mode=neon
:::
"##,
            theme_dir.display()
        ),
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args(["check", source_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("slide 'section-1-main'")
                .and(predicate::str::contains("invalid theme parameter override"))
                .and(predicate::str::contains("mode"))
                .and(predicate::str::contains("neon")),
        );
}

#[test]
fn export_contact_sheet_requires_png_pages() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    fs::write(&source_path, "# Talk\n").unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png-contact-sheet",
            temp.path().join("contact.png").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--png-contact-sheet requires --png",
        ));
}

#[test]
fn export_rejects_shared_or_nested_exclusive_raster_directories_before_rendering() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let pages = temp.path().join("pages");
    let aliased_pages = pages.join("unused").join("..");
    fs::write(&source_path, "# Talk\n").unwrap();

    let mut shared = Command::cargo_bin("zpres").unwrap();
    shared
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            pages.to_str().unwrap(),
            "--jpg",
            aliased_pages.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("raster Output target --png")
                .and(predicate::str::contains("overlaps --jpg"))
                .and(predicate::str::contains("choose disjoint paths")),
        );
    assert!(!pages.exists());

    let mut nested = Command::cargo_bin("zpres").unwrap();
    nested
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            pages.to_str().unwrap(),
            "--jpg",
            pages.join("jpeg").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps --jpg"));
    assert!(!pages.exists());

    {
        let case_alias = temp.path().join("PAGES");
        let nested_case_alias = case_alias.join("jpeg");
        let mut case_insensitive = Command::cargo_bin("zpres").unwrap();
        case_insensitive
            .args([
                "export",
                source_path.to_str().unwrap(),
                "--png",
                pages.to_str().unwrap(),
                "--jpg",
                nested_case_alias.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("overlaps --jpg"));
        assert!(!pages.exists());
        assert!(!case_alias.exists());

        #[cfg(target_os = "macos")]
        for (left, right) in [
            ("caf\u{e9}", "cafe\u{301}"),
            ("stra\u{df}e", "STRASSE"),
            ("\u{fb03}", "FFI"),
        ] {
            let png_alias = temp.path().join(left);
            let jpg_alias = temp.path().join(right).join("jpeg");
            let mut unicode_alias = Command::cargo_bin("zpres").unwrap();
            unicode_alias
                .args([
                    "export",
                    source_path.to_str().unwrap(),
                    "--png",
                    png_alias.to_str().unwrap(),
                    "--jpg",
                    jpg_alias.to_str().unwrap(),
                ])
                .assert()
                .failure()
                .stderr(predicate::str::contains("overlaps --jpg"));
            assert!(!png_alias.exists());
            assert!(!jpg_alias.exists());
        }
    }
}

#[test]
fn export_rejects_file_outputs_inside_an_exclusive_raster_directory() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let pages = temp.path().join("pages");
    fs::write(&source_path, "# Talk\n").unwrap();

    for (flag, filename) in [
        ("--png-contact-sheet", "contact.png"),
        ("--pdf", "deck.pdf"),
        ("--notes-txt", "notes.txt"),
        ("--print-html", "print.html"),
    ] {
        let output_path = pages.join(filename);
        let mut command = Command::cargo_bin("zpres").unwrap();
        command
            .args([
                "export",
                source_path.to_str().unwrap(),
                "--png",
                pages.to_str().unwrap(),
                flag,
                output_path.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("raster Output target --png")
                    .and(predicate::str::contains(format!("overlaps {flag}"))),
            );
        assert!(!pages.exists());
    }
}

#[test]
fn later_export_commands_refuse_an_existing_raster_owned_ancestor() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let pages = temp.path().join("pages");
    fs::write(&source_path, "# Talk\n").unwrap();
    fs::create_dir(&pages).unwrap();
    fs::write(
        pages.join(".zpres-raster-page-set.json"),
        b"reserved ownership marker",
    )
    .unwrap();
    fs::write(pages.join("page-001.png"), b"preserved page").unwrap();

    for (flag, filename) in [
        ("--notes-txt", "notes.txt"),
        ("--print-html", "print.html"),
        ("--pdf", "deck.pdf"),
    ] {
        let output = pages.join(filename);
        let mut command = Command::cargo_bin("zpres").unwrap();
        command
            .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
            .args([
                "export",
                source_path.to_str().unwrap(),
                flag,
                output.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "inside exclusively owned raster page directory",
            ));
        assert!(!output.exists());
    }

    let contact = pages.join("contact.png");
    let outside_pages = temp.path().join("new-pages");
    let mut contact_command = Command::cargo_bin("zpres").unwrap();
    contact_command
        .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            outside_pages.to_str().unwrap(),
            "--png-contact-sheet",
            contact.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inside exclusively owned raster page directory",
        ));
    assert!(!contact.exists());
    assert!(!outside_pages.exists());
    assert_eq!(
        fs::read(pages.join("page-001.png")).unwrap(),
        b"preserved page"
    );
}

#[test]
fn later_html_build_refuses_an_existing_raster_owned_ancestor() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let pages = temp.path().join("pages");
    fs::write(&source_path, "# Talk\n").unwrap();
    fs::create_dir(&pages).unwrap();
    fs::write(
        pages.join(".zpres-raster-page-set.json"),
        b"reserved ownership marker",
    )
    .unwrap();
    fs::write(pages.join("page-001.png"), b"preserved page").unwrap();

    let output = pages.join("html");
    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inside exclusively owned raster page directory",
        ));

    assert!(!output.exists());
    assert_eq!(
        fs::read(pages.join("page-001.png")).unwrap(),
        b"preserved page"
    );
}

#[test]
fn peer_file_exports_preserve_html_publication_metadata_and_entrypoint() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let output = temp.path().join("html");
    fs::write(&source_path, "# First build\n").unwrap();

    let mut build = Command::cargo_bin("zpres").unwrap();
    build
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    let index = output.join("index.html");
    let marker = output
        .join("zpres-html-generations")
        .join(".zpres-output.json");
    let index_before = fs::read(&index).unwrap();
    let marker_before = fs::read(&marker).unwrap();

    for target in [&index, &marker] {
        let mut export = Command::cargo_bin("zpres").unwrap();
        export
            .args([
                "export",
                source_path.to_str().unwrap(),
                "--notes-txt",
                target.to_str().unwrap(),
            ])
            .assert()
            .failure();
    }
    assert_eq!(fs::read(&index).unwrap(), index_before);
    assert_eq!(fs::read(&marker).unwrap(), marker_before);

    fs::write(&source_path, "# Second build\n").unwrap();
    let mut rebuild = Command::cargo_bin("zpres").unwrap();
    rebuild
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    let current = current_html_generation(&output);
    assert!(
        fs::read_to_string(current.presentation_index)
            .unwrap()
            .contains("Second build")
    );
}

#[test]
fn raster_export_refuses_the_html_managed_generation_tree_before_chromium() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let generations = temp.path().join("html").join("zpres-html-generations");
    fs::write(&source_path, "# Talk\n").unwrap();
    fs::create_dir_all(&generations).unwrap();
    fs::write(
        generations.join(".zpres-output.json"),
        b"reserved HTML ownership marker",
    )
    .unwrap();
    let output = generations.join("pages");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inside or over zpres-managed HTML publication",
        ));
    assert!(!output.exists());
}

#[test]
fn export_refuses_to_claim_a_directory_containing_the_deck_source() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    fs::write(&source_path, "# Talk\n").unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            temp.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("raster Output target --png")
                .and(predicate::str::contains("overlaps the Deck source")),
        );
    assert_eq!(fs::read_to_string(source_path).unwrap(), "# Talk\n");
}

#[cfg(unix)]
#[test]
fn export_resolves_existing_symlink_aliases_before_comparing_raster_targets() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let pages = temp.path().join("pages");
    let alias = temp.path().join("page-alias");
    fs::write(&source_path, "# Talk\n").unwrap();
    fs::create_dir(&pages).unwrap();
    symlink(&pages, &alias).unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            pages.to_str().unwrap(),
            "--jpg",
            alias.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps --jpg"));
    assert!(pages.read_dir().unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn export_uses_physical_file_identity_to_protect_hard_link_aliases() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let output_alias = temp.path().join("notes-alias.txt");
    fs::write(&source_path, "# Talk\n").unwrap();
    fs::hard_link(&source_path, &output_alias).unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--notes-txt",
            output_alias.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps the Deck source"));
    assert_eq!(fs::read_to_string(source_path).unwrap(), "# Talk\n");
    assert_eq!(fs::read_to_string(output_alias).unwrap(), "# Talk\n");
}

#[cfg(target_os = "macos")]
#[test]
fn export_rejects_macos_firmlink_aliases_before_any_output() {
    let temp = tempfile::tempdir_in("/private/tmp").unwrap();
    let direct_root = temp.path();
    let data_root = Path::new("/System/Volumes/Data").join(
        direct_root
            .strip_prefix(Path::new("/"))
            .expect("private temporary path is absolute"),
    );
    let source_path = direct_root.join("talk.zp.md");
    fs::write(&source_path, "# Talk\n").unwrap();
    let source_alias = data_root.join("talk.zp.md");

    let mut source_overwrite = Command::cargo_bin("zpres").unwrap();
    source_overwrite
        .args([
            "export",
            source_alias.to_str().unwrap(),
            "--notes-txt",
            source_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps the Deck source"));
    assert_eq!(fs::read_to_string(&source_path).unwrap(), "# Talk\n");

    let png_dir = direct_root.join("Pages");
    let aliased_contact = data_root.join("pages").join("contact.png");
    let mut nested_output = Command::cargo_bin("zpres").unwrap();
    nested_output
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            png_dir.to_str().unwrap(),
            "--png-contact-sheet",
            aliased_contact.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps --png-contact-sheet"));
    assert!(!png_dir.exists());
}

#[test]
fn export_rejects_output_paths_that_overlap_deck_or_theme_dependencies() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let asset_dir = temp.path().join("assets");
    fs::create_dir(&asset_dir).unwrap();
    fs::write(
        asset_dir.join("plot.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    )
    .unwrap();
    fs::write(&source_path, "# Talk\n\n![Plot](assets/plot.svg)\n").unwrap();

    let mut deck_dependency = Command::cargo_bin("zpres").unwrap();
    deck_dependency
        .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            asset_dir.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps a Deck dependency"));
    assert!(asset_dir.join("plot.svg").is_file());

    let theme_root = temp.path().join("themes");
    write_theme_manifest(&theme_root, "protected-theme");
    let theme_dir = theme_root.join("protected-theme");
    fs::write(theme_dir.join("theme.css.tmpl"), ":root {}\n").unwrap();
    fs::write(theme_dir.join("print.css.tmpl"), ":root {}\n").unwrap();
    let theme_output = theme_dir.join("generated-pages");
    let mut theme_dependency = Command::cargo_bin("zpres").unwrap();
    theme_dependency
        .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--theme",
            "protected-theme",
            "--theme-dir",
            theme_root.to_str().unwrap(),
            "--png",
            theme_output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps the Theme package"));
    assert!(!theme_output.exists());

    let print_output = theme_dir.join("dist").join("print.html");
    let mut safe_theme_file_output = Command::cargo_bin("zpres").unwrap();
    safe_theme_file_output
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--theme",
            "protected-theme",
            "--theme-dir",
            theme_root.to_str().unwrap(),
            "--print-html",
            print_output.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(print_output.is_file());
}

#[test]
fn export_rejects_colliding_file_outputs_and_protects_the_deck_source() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let shared_output = temp.path().join("artifact.txt");
    fs::write(&source_path, "# Talk\n").unwrap();

    let mut colliding_outputs = Command::cargo_bin("zpres").unwrap();
    colliding_outputs
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--notes-txt",
            shared_output.to_str().unwrap(),
            "--print-html",
            shared_output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("export Output target --notes-txt")
                .and(predicate::str::contains("overlaps --print-html")),
        );
    assert!(!shared_output.exists());

    let mut overwrite_source = Command::cargo_bin("zpres").unwrap();
    overwrite_source
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--notes-txt",
            source_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlaps the Deck source"));
    assert_eq!(fs::read_to_string(source_path).unwrap(), "# Talk\n");
}

#[test]
fn export_notes_txt_writes_speaker_notes_without_chromium() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let notes_path = temp.path().join("dist").join("notes.txt");
    fs::write(
        &source_path,
        r#"# Opening

::: notes
Pause before the claim.
:::

---

# Detail

^ Mention the backup result.
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--notes-txt",
            notes_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported speaker notes"));

    let notes = fs::read_to_string(notes_path).unwrap();
    assert!(notes.contains("# Speaker Notes"));
    assert!(notes.contains("## 1. Opening"));
    assert!(notes.contains("Pause before the claim."));
    assert!(notes.contains("## 2. Detail"));
    assert!(notes.contains("Mention the backup result."));
}

#[test]
fn export_print_html_writes_static_export_without_chromium() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let print_html_path = temp.path().join("dist").join("print.html");
    fs::write(
        &source_path,
        r#"# Print HTML

::: notes
Presenter note.
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("ZPRES_CHROMIUM", temp.path().join("missing-chromium"))
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--print-html",
            print_html_path.to_str().unwrap(),
            "--notes",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Exported print HTML with")
                .and(predicate::str::contains("page(s)")),
        );

    let print_html = fs::read_to_string(print_html_path).unwrap();
    assert!(print_html.contains(r#"data-zpres-ready="pending""#));
    assert!(print_html.contains(r#"data-zpres-ready-target="pdf""#));
    assert!(print_html.contains("zpres-print-slide"));
    assert!(print_html.contains(r#"data-generated-slide="speaker-notes""#));
}

#[test]
fn rebuild_preserves_unrelated_files_in_the_shared_output_root() {
    let temp = tempdir().unwrap();
    let source_path = temp.path().join("talk.zp.md");
    let output_root = temp.path().join("dist");
    let report_dir = output_root.join("review-data");
    fs::create_dir_all(&report_dir).unwrap();
    fs::write(&source_path, "# First version\n").unwrap();
    fs::write(output_root.join("deck.pdf"), b"existing pdf bytes").unwrap();
    fs::write(
        output_root.join("speaker-notes.txt"),
        b"existing speaker notes",
    )
    .unwrap();
    fs::write(report_dir.join("visual-report.json"), b"{\"kept\":true}").unwrap();

    let mut first_build = Command::cargo_bin("zpres").unwrap();
    first_build
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            output_root.to_str().unwrap(),
        ])
        .assert()
        .success();
    let first_publication = current_html_generation(&output_root);
    assert!(first_publication.generation_path.is_dir());

    fs::write(&source_path, "# Second version\n").unwrap();
    let mut second_build = Command::cargo_bin("zpres").unwrap();
    second_build
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            output_root.to_str().unwrap(),
        ])
        .assert()
        .success();
    let second_publication = current_html_generation(&output_root);

    assert_ne!(first_publication.generation, second_publication.generation);
    assert!(first_publication.generation_path.is_dir());
    assert!(
        fs::read_to_string(second_publication.presentation_index)
            .unwrap()
            .contains("Second version")
    );
    assert_eq!(
        fs::read(output_root.join("deck.pdf")).unwrap(),
        b"existing pdf bytes"
    );
    assert_eq!(
        fs::read(output_root.join("speaker-notes.txt")).unwrap(),
        b"existing speaker notes"
    );
    assert_eq!(
        fs::read(report_dir.join("visual-report.json")).unwrap(),
        b"{\"kept\":true}"
    );
}

#[test]
fn config_path_and_init_use_xdg_config_home() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let expected_path = config_home.join("zpres").join("zpres.toml");

    let mut path_command = Command::cargo_bin("zpres").unwrap();
    path_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(format!("{}\n", expected_path.display()));

    let mut init_command = Command::cargo_bin("zpres").unwrap();
    init_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["config", "init"])
        .assert()
        .success();
    assert!(expected_path.exists());

    let mut second_init = Command::cargo_bin("zpres").unwrap();
    second_init
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["config", "init"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));

    let mut force_init = Command::cargo_bin("zpres").unwrap();
    force_init
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["config", "init", "--force"])
        .assert()
        .success();
}

#[test]
fn config_show_prints_resolved_cli_overrides() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let global_config = config_home.join("zpres").join("zpres.toml");
    fs::create_dir_all(global_config.parent().unwrap()).unwrap();
    fs::write(
        &global_config,
        r#"schema_version = 1

[paths]
theme_dirs = ["global-themes"]
"#,
    )
    .unwrap();

    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    write_theme_manifest(&deck_root.join("cli-themes"), "cli");
    fs::write(
        deck_root.join("zpres.toml"),
        r#"schema_version = 1

[deck]
theme = "project"
"#,
    )
    .unwrap();
    fs::write(
        deck_root.join("talk.zp.md"),
        r#"---
theme: front
---

# Talk
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    let output = command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "config",
            "show",
            deck_root.join("talk.zp.md").to_str().unwrap(),
            "--theme",
            "cli",
            "--theme-dir",
            "cli-themes",
            "--theme-param",
            "accent=#00ffff",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();

    assert!(stdout.contains("theme = \"cli\""));
    assert!(stdout.contains("theme_manifest_path"));
    assert!(stdout.contains("accent = \"#00ffff\""));
    assert!(stdout.contains("cli-themes"));
}

#[test]
fn theme_check_accepts_a_file_backed_room_profile() {
    let temp = tempdir().unwrap();
    let profile = temp.path().join("auditorium.toml");
    let source = include_str!("../room-profiles/projected-room-default.toml")
        .replace("name = \"projected-room-default\"", "name = \"auditorium\"");
    fs::write(&profile, source).unwrap();
    let theme = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes/science");

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .args([
            "theme",
            "check",
            theme.to_str().unwrap(),
            "--no-fixture",
            "--room-profile",
            profile.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Theme 'science' 0.1.0 is valid"));
}

#[test]
fn config_show_theme_change_resets_inherited_deck_params_before_cli_params() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    let themes = deck_root.join("themes");
    fs::create_dir_all(&deck_root).unwrap();
    write_switchable_theme_manifest(
        &themes,
        "dark-splash",
        "variant",
        "violet",
        &["violet", "cyan"],
    );
    write_switchable_theme_manifest(&themes, "sv", "mode", "light", &["light", "dark"]);
    let source = deck_root.join("example-deck.zp.md");
    fs::write(
        &source,
        r#"---
theme: "dark-splash"
theme_params:
  variant: "cyan"
  density: "compact"
  footer: "section-progress"
aspect: "16:9"
---

# Example Deck
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    let output = command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "config",
            "show",
            source.to_str().unwrap(),
            "--theme",
            "sv",
            "--theme-param",
            "mode=dark",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();

    assert!(stdout.contains("theme = \"sv\""));
    assert!(stdout.contains("density = \"normal\""));
    assert!(stdout.contains("footer = \"slide-number\""));
    assert!(stdout.contains("mode = \"dark\""));
    assert!(
        !stdout
            .lines()
            .any(|line| line.trim_start().starts_with("variant ="))
    );
}

#[test]
fn build_fails_on_fatal_diagnostics_with_source_location() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("bad.zp.md");
    fs::write(
        &source_path,
        r#"# Bad

<div>raw html</div>
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["build", source_path.to_str().unwrap(), "--out", "dist/bad"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains(format!("{}:3:1", source_path.display()))
                .and(predicate::str::contains("raw HTML must be wrapped"))
                .and(predicate::str::contains("output not written")),
        );

    assert!(!deck_root.join("dist/bad/index.html").exists());
}

#[test]
fn build_rejects_unterminated_front_matter_at_the_opening_delimiter() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let source_path = temp.path().join("unterminated.zp.md");
    fs::write(
        &source_path,
        "\u{feff}---\r\ntheme: science\r\n# Missing closing delimiter\r\n",
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/unterminated",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains(format!("{}:1:1", source_path.display()))
                .and(predicate::str::contains("unterminated source front matter"))
                .and(predicate::str::contains("expected a closing --- delimiter")),
        );

    assert!(!temp.path().join("dist/unterminated/index.html").exists());
}

#[test]
fn build_allows_warnings_unless_strict_mode_is_enabled() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("warning.zp.md");
    fs::write(
        &source_path,
        r#"# Warning

::: unknown
still rendered as a placeholder
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/warning",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(format!(
            "{}:3:1",
            source_path.display()
        )));
    assert!(deck_root.join("dist/warning/index.html").exists());

    let mut strict_command = Command::cargo_bin("zpres").unwrap();
    strict_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/strict",
            "--strict",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("--strict treats warnings as fatal")
                .and(predicate::str::contains("output not written")),
        );
    assert!(!deck_root.join("dist/strict/index.html").exists());
}

#[test]
fn build_and_export_fail_on_missing_local_asset_dependency() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("missing.zp.md");
    fs::write(
        &source_path,
        r#"# Missing asset

::: figure src="missing.svg"
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/missing",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "figure references missing local asset 'missing.svg'",
        ));

    let mut export_command = Command::cargo_bin("zpres").unwrap();
    export_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--pdf",
            deck_root.join("dist/missing.pdf").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "figure references missing local asset 'missing.svg'",
        ));

    let mut png_command = Command::cargo_bin("zpres").unwrap();
    png_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--png",
            deck_root.join("dist/missing-pages").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "figure references missing local asset 'missing.svg'",
        ));
    assert!(!deck_root.join("dist/missing-pages").exists());
}

#[test]
fn build_and_export_fail_on_missing_local_media_dependency() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("missing-media.zp.md");
    fs::write(
        &source_path,
        r#"# Missing media

::: video src="media/missing.mp4" poster="media/missing.png"
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/missing-media",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "media src references missing local asset 'media/missing.mp4'",
        ));

    let mut export_command = Command::cargo_bin("zpres").unwrap();
    export_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--pdf",
            deck_root.join("dist/missing-media.pdf").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "media poster references missing local asset 'media/missing.png'",
        ));
}

#[test]
fn build_resolves_figure_assets_from_deck_root_not_process_cwd() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    let asset_dir = deck_root.join("assets");
    fs::create_dir_all(&asset_dir).unwrap();
    fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
    let source_path = deck_root.join("asset.zp.md");
    fs::write(
        &source_path,
        r#"# Asset

::: figure src="assets/plot.svg" caption="Resolved from the Deck root."
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .current_dir(temp.path())
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/assets",
        ])
        .assert()
        .success();

    assert!(deck_root.join("dist/assets/index.html").exists());
    let publication = current_html_generation(&deck_root.join("dist/assets"));
    assert!(publication.generation_path.join("assets/plot.svg").exists());
}

#[test]
fn build_fails_on_invalid_chart_json() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("bad-chart.zp.md");
    fs::write(
        &source_path,
        r#"# Bad chart

::: vega-lite
{ invalid json
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/bad-chart",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid Vega-Lite JSON")
                .and(predicate::str::contains("output not written")),
        );
}

#[test]
fn build_fails_on_missing_chart_data_dependency() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("missing-chart-data.zp.md");
    fs::write(
        &source_path,
        r#"# Missing chart data

::: vega-lite
{ "data": { "url": "data/missing.csv" }, "mark": "line" }
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/missing-chart-data",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "chart references missing local data dependency 'data/missing.csv'",
        ));
}

#[test]
fn build_and_export_fail_on_unsupported_chart_renderer_shape() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    let data_dir = deck_root.join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("runtime.csv"), "n,ms\n1,10\n2,20\n").unwrap();
    let source_path = deck_root.join("bad-chart-shape.zp.md");
    fs::write(
        &source_path,
        r#"# Bad chart shape

::: vega-lite
{ "data": { "url": "data/runtime.csv" }, "mark": "bar", "encoding": { "x": { "field": "n" }, "y": { "field": "ms" } } }
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/bad-chart-shape",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("unsupported Vega-Lite chart for HTML/PDF rendering")
                .and(predicate::str::contains("output not written")),
        );
    assert!(!deck_root.join("dist/bad-chart-shape/index.html").exists());

    let mut export_command = Command::cargo_bin("zpres").unwrap();
    export_command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "export",
            source_path.to_str().unwrap(),
            "--pdf",
            deck_root.join("dist/bad-chart-shape.pdf").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unsupported Vega-Lite chart for HTML/PDF rendering",
        ));
    assert!(!deck_root.join("dist/bad-chart-shape.pdf").exists());
}

#[test]
fn build_fails_on_invalid_layout_value() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("bad-layout.zp.md");
    fs::write(
        &source_path,
        r#"# Bad layout

::: columns widths="wide/narrow"
Left
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/bad-layout",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid Columns widths layout value")
                .and(predicate::str::contains("output not written")),
        );
}

#[test]
fn build_fails_on_invalid_step_syntax() {
    let temp = tempdir().unwrap();
    let config_home = temp.path().join("xdg");
    let deck_root = temp.path().join("deck");
    fs::create_dir_all(&deck_root).unwrap();
    let source_path = deck_root.join("bad-steps.zp.md");
    fs::write(
        &source_path,
        r#"# Bad steps

::: steps
- Not an ordered step.
:::
"#,
    )
    .unwrap();

    let mut command = Command::cargo_bin("zpres").unwrap();
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .args([
            "build",
            source_path.to_str().unwrap(),
            "--out",
            "dist/bad-steps",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("steps directive entries must be ordered list items")
                .and(predicate::str::contains("output not written")),
        );
}
