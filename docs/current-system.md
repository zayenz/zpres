# System overview

zpres parses one Source file into a typed Deck, then renders an HTML
presentation or PDF export from that model. This guide maps the implementation
for contributors. For authoring a talk, start with
[getting started](getting-started.md). [CONTEXT.md](../CONTEXT.md) defines the
project vocabulary, and the [ADRs](adr/README.md) explain the design decisions.

## Processing model

Configuration and YAML front matter select a Theme, its parameters, and
output defaults. The parser produces Sections, Main and Detail slides, Content blocks, Layouts, Steps, notes, dependencies, and
source-located diagnostics.

```text
Source file + configuration
  -> typed Deck + diagnostics + dependencies
  -> prepared Deck + Theme
     -> HTML presentation
     -> print HTML -> PDF export
                   -> PNG/JPEG pages -> contact sheet
```

HTML and PDF are peer Output targets. PDF uses its own print HTML rendering
surface rather than printing the live presentation. Theme API v1 compiles a
shared Presentation plan to keep slide order, Steps, notes, and backgrounds
consistent across the two surfaces.

## Code map

| Module | Responsibility |
| --- | --- |
| `src/lib.rs` | CLI, configuration, scaffolding, and command orchestration |
| `src/deck.rs` | Source parsing, the typed Deck model, and diagnostics |
| `src/theme.rs` | Theme discovery, manifests, parameters, and contract checks |
| `src/presentation_plan.rs` | Theme API v1 slide and Step order for live and static output |
| `src/html.rs`, `src/html/theme_api_v1.rs` | HTML, print HTML, and presentation runtime |
| `src/server.rs` | Live preview, dependency watching, and Last good deck behavior |
| `src/pdf.rs`, `src/chromium.rs` | Static readiness, browser capture, PDF, and page images |
| `src/visual.rs`, `src/browser_validation.rs` | Browser traversal, measurements, and visual review artifacts |
| `src/background_validation.rs` | Authored background checks |
| `src/room_profile.rs` | Room profile loading and enforcement policy |
| `src/publication.rs`, `src/raster_publication.rs`, `src/output_ownership.rs` | Output ownership, atomic publication, and recovery |

## Commands and browser requirements

| Command | What it does | Needs Chromium? |
| --- | --- | --- |
| `init` | Creates a starter Source file | No |
| `check` | Parses, validates, and checks executed static pages without publishing output | Yes |
| `build` | Publishes an HTML presentation | No |
| `serve` | Watches dependencies and serves successful builds | No; open its URL in a browser |
| `export --pdf`, `--png`, `--jpg` | Captures static output | Yes |
| `export --print-html`, `--notes-txt` | Writes print HTML or a speaker script | No |
| `config path`, `init`, `show` | Locates, creates, or inspects configuration | No |
| `theme init`, `theme check` | Creates or validates a Theme package | No |
| `theme check --visual` | Captures and measures live states and print pages | Yes |

The CLI supports Theme, Theme parameter, and search-path overrides. Room profiles
are selected explicitly for browser-rendered Theme review. Run a command with `--help` for its options. `--strict` makes Source
warnings fatal on commands that provide it.

## Content and Themes

The [authoring guides](README.md#write-a-talk) cover text, math, figures,
charts, diagrams, media, Layouts, and Steps. Content is validated
before static capture. The [chart renderer](chart-authoring.md) supports numeric line charts over local CSV/JSON; Mermaid supports a limited flowchart subset; zpres does not execute external authoring scripts.

Theme API v1 assigns geometry and semantic regions to the renderer. Themes
control typography, palettes, surfaces, and ornament. The logical field is
1280 by 720; only 16:9-equivalent aspects are supported. All built-in Themes
use the single API 1 contract.

Browser-rendered Theme checks traverse live routes and Steps, then validate
print pages independently. Reports and screenshots support human review;
an automated pass does not approve a talk for a room. The built-in room
profile is provisional. Approved profiles can enforce type and autoscale
floors, but approval requires physical room evidence.

## Publication

HTML builds write complete versions under `zpres-html-generations/` and
atomically update the root `index.html` to select one. Other files in the
output directory remain untouched. See [HTML publication](html-publication.md).

Each PNG or JPEG page set owns its whole directory. zpres replaces that
directory atomically and refuses to overwrite an unmarked non-empty one.
Contact sheets remain separate files. See [raster publication](raster-publication.md).

The publication implementation supports macOS and Linux. Native publication
operations for other platforms, richer font validation, and all-at-once
publication of Theme specimen artifacts remain outside the current support
boundary. The public CLI and Source format are supported in 0.1; the Rust
library API is experimental.
