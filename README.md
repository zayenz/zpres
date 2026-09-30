# zpres

zpres builds presentations from Markdown-based `.zp.md` Source files.
Write slide content, images, and speaker notes together, preview the talk in a
browser, then publish an HTML presentation or PDF export.
PNG and JPEG page sets are available for review and sharing.

A Deck has a main talk path and optional Detail slides for supporting
material, worked examples, or questions. Themes control its appearance. The
same Source file supplies the content for live and static output.

The 0.1 series supports macOS and Linux. Chromium or Google Chrome is required
for `zpres check`, visual Theme checks, and PDF or raster export. HTML builds,
print HTML, and speaker-note text do not require a browser.

## Install

zpres requires Rust 1.88 or newer:

```sh
cargo install zpres --locked
zpres --version
```

Or install from a repository checkout:

```sh
git clone https://github.com/zayenz/zpres.git
cd zpres
cargo install --path . --locked
zpres --version
```

zpres searches the usual Chrome and Chromium installation paths. Set
`ZPRES_CHROMIUM` when the browser is installed elsewhere:

```sh
export ZPRES_CHROMIUM=/path/to/chromium
```

Windows and BSD are not supported in 0.1 because HTML and raster publication
use native atomic filesystem operations.

## Start a Deck

```sh
zpres init talk.zp.md --title "My talk" --author "Ada"
zpres check talk.zp.md --strict
zpres serve talk.zp.md
```

The starter uses Debug unless configuration selects another Theme. Set
`theme: science` in its YAML front matter to try a restrained, serif-led Theme.

Open the local URL printed by `serve`. The server watches the Source file,
local assets, and Theme files. It keeps the last valid Deck visible when an
edit prevents a rebuild.

The [getting started guide](docs/getting-started.md) explains the Source
format, navigation, Theme selection, and export workflow with a complete example.

Publish an offline HTML presentation or static artifacts with:

```sh
zpres build talk.zp.md --out dist/talk
zpres build talk.zp.md --out dist/public-talk --exclude-speaker-notes
zpres export talk.zp.md --pdf dist/talk.pdf
zpres export talk.zp.md --png dist/talk-pages \
  --png-contact-sheet dist/contact-sheet.png
zpres export talk.zp.md --notes-txt dist/speaker-notes.txt
```

HTML builds include speaker notes by default. Use `--exclude-speaker-notes`
with a fresh output directory when sharing a presentation that should not
include them. Older retained builds can still contain notes. Copy the whole HTML
output directory when publishing it, including `zpres-html-generations/`.

Run `zpres <command> --help` for all options. The
[documentation index](docs/README.md) links to the authoring and reference guides.

## Examples

The repository contains two self-contained examples:

- [`examples/quickstart/talk.zp.md`](examples/quickstart/talk.zp.md) is a small
  authoring-loop example.
- [`examples/overlapping-intervals/talk.zp.md`](examples/overlapping-intervals/talk.zp.md)
  develops an interval-overlap test from a two-case separation argument.

Both include [rendered HTML, PDFs, and previews](examples/README.md).

From a checkout:

```sh
cargo run -- check examples/overlapping-intervals/talk.zp.md --strict
cargo run -- serve examples/overlapping-intervals/talk.zp.md
```

## Deck model and Output targets

A Source file parses into a Deck containing Sections, Main slides, optional
Detail slides, typed Content blocks, Steps, and speaker notes. HTML and PDF are
peer Output targets: both are rendered from the Deck model. PDF export uses
a separate print HTML document captured by Chromium.

The authoring language includes:

- KaTeX math, highlighted code, tables, figures, galleries, a subset of
  Vega-Lite charts and Mermaid flowcharts, media, and local assets;
- Columns, Grid, Stack, Overlay, and Aside Layouts;
- semantic Slide variants and per-Slide Theme parameters;
- live Steps with a final-state or page-per-Step policy for static output.

Theme API v1 supports 16:9 presentations. Static chart rendering currently
supports line charts backed by local CSV or JSON data. zpres does not run
Python, R, or Julia during a build; generate other plots separately and include
them as figures.

The [text](docs/text-authoring.md), [figure](docs/figure-authoring.md),
[background](docs/background-authoring.md), [diagram](docs/diagram-authoring.md),
and [media](docs/media-authoring.md) guides describe the Source format.
The [chart guide](docs/chart-authoring.md) defines the supported numeric line-chart subset and data validation.
[Charts and math](docs/scientific-content.md) covers equations, code, and
chart export limits.

## Themes

Science provides a restrained, serif-led starting point. Dark Splash, Paper Chalk,
SV, and Wedding offer other visual treatments. Signal, Tidal, Kiln, Prism,
and Blueprint provide poster, ocean, editorial, geometric, and drafting
treatments. Debug and the v1 reference Theme help inspect and develop Themes. The
[Theme inventory](docs/theme-inventory.md) describes each choice.
Built-in Themes are embedded in the installed binary;
they do not depend on a repository checkout or Cargo's source cache.

Deck-local and configured Theme packages override a built-in Theme with the
same name. Create and inspect a package with:

```sh
zpres theme init themes/my-theme
zpres theme check themes/my-theme
zpres theme check themes/my-theme \
  --write-specimen dist/theme-specimen --all-variants --visual
```

All Themes use Theme API 1. See the [Theme authoring guide](docs/theme-authoring.md)
for the manifest, supported selectors, and renderer contract.

## Output ownership

`build` publishes complete retained HTML generations below the requested output
directory and updates its root `index.html` to select one generation. It leaves
peer PDFs, notes, reports, and review files untouched.

PNG and JPEG page sets each own a separate marked directory. zpres refuses to
replace an unmarked non-empty directory. Keep contact sheets, PDFs, and notes
outside raster page-set directories. The
[HTML](docs/html-publication.md) and [raster](docs/raster-publication.md)
publication guides describe these boundaries.

## Compatibility

The command-line interface and Source format are the supported public surfaces
for 0.1. The Rust library exists to implement the binary and remains
experimental; its modules may change before 1.0.

Theme API 1 is the sole Theme contract in the first release. Theme APIs and
TOML configuration schemas are versioned separately from zpres.

See the [configuration guide](docs/configuration.md) for supported defaults and precedence.

## Development

The [system overview](docs/current-system.md) maps the implementation, and
[CONTEXT.md](CONTEXT.md) defines the project vocabulary. From a checkout:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets
```

Theme API v1 CSS is compiled offline with a pinned JavaScript toolchain. After
changing the shared foundation:

```sh
npm ci
npm run build:theme-api-v1-css
```

Visual or Theme work requires fresh screen and print captures as well as
original-size inspection. [AGENTS.md](AGENTS.md) records the repository's
verification rules.

## License

Licensed under either the [Apache License 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.
