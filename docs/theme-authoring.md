# Theme authoring

A Theme is a package containing a `theme.toml` manifest, screen and print CSS
templates, and any local fonts or artwork. The renderer supplies the slide
geometry and content regions; the Theme supplies typography, color, surfaces,
and ornament.

To use an existing Theme, start with the [Theme inventory](theme-inventory.md).
To build one, [create a package](#package-shape), edit its templates, then
[check it](#checking-a-theme) with a specimen Deck. The API and selector
sections below are the reference for that work. New Themes use the stable
`zpres-*` classes; `debug-*` classes are internal hooks.

Use [the presentation design contract](presentation-design.md) for review
criteria and the Theme API v1 boundary. Built-in Themes and `theme init` use
v1. The contract distinguishes the peer HTML presentation and PDF export from
the print-HTML rendering surface and derived review artifacts.

## Theme API 1

`api_version = 1` selects the current renderer and cascade contract. Its
semantic regions include `.zpres-slide-frame`, `.zpres-slide-header`,
`.zpres-slide-title`, `.zpres-slide-body`, `.zpres-slide-primary`, supporting,
sources, footer, ornament, and typed Layout regions. The renderer owns the 1280
by 720 logical field, safe area, reading order, footer lane, fit/letterbox
behavior, and Output-target geometry. A Theme owns tokens, typography,
palette, surfaces, and bounded accent treatment.

V1 accepts 16:9 and equivalent ratios such as `1920:1080`. It rejects other
declared aspects before HTML, PDF, PNG, or JPEG publication. Grid, Stack,
Columns, Overlay, and Aside are typed renderer-owned Layout kinds. Use
[`themes/reference/specimen.zp.md`](../themes/reference/specimen.zp.md),
the generated Theme specimen, and the production Theme references as
implemented examples. The `reference` teaching Deck is the visual companion
to this guide: it keeps one named contract idea per page and is the place to
inspect ordinary Main, Detail, Claim, Figure, Comparison, Derivation, Dense,
Sources, Footer, and Step behavior on both the HTML presentation and PDF export.
Unknown future API values fail at the value's manifest location without silently selecting a different contract.

V1 compiles one Presentation plan for screen routes, print pages, page counts,
readiness, and authored-background checks. The first Section's Main slide is
the Title phase. A notes page follows its owning logical Slide, an explicit
Splash follows the complete first Section, and later Sections follow in source
order. See [ADR 0008](adr/0008-compile-theme-api-v1-output-from-one-presentation-plan.md)
and [background authoring](background-authoring.md) for the exact background
precedence and `title`/`splash` policy.

The built-in Debug Theme exposes the v1 regions for inspection. Its normal
rendering uses the same geometry as other v1 Themes. Add `?zpres-debug=1` to a served
presentation to label semantic regions and their computed canvas-relative
bounds. See [Debug Theme](debug-theme.md).

Science is a production v1 example built on the renderer-owned
`scientific-data` module. Its reference Deck demonstrates scientific Content
blocks, semantic variants, Main and Detail roles, Sources, Footer, and Step
treatments in light and dark palettes. See
[Science Theme API v1](science-theme-v1.md).

SV also consumes the renderer-owned `scientific-data` module. Its v1 reference covers Claim, Comparison,
Derivation, Figure, Gallery, Chart, Dense Detail, code reveal, and both
palettes. [ADR 0010](adr/0010-adopt-sv-as-a-supported-theme.md) defines the
three-file product package and excludes the ignored inspiration subtree.

## Package shape

Start a new package with:

```bash
zpres theme init themes/my-theme
zpres theme check themes/my-theme
```

`theme init` writes an editable v1 package that uses stable semantic regions,
the `scientific-data` shared module, standard color slots, light/dark palette
variants, a bounded print overlay, and a local specimen Deck with tiny generated
assets. The renderer foundation is compiled into offline presentations; the
package has no Tailwind runtime or CDN dependency. Initialization refuses to
overwrite an existing package unless you pass `--force`.

Check the generated local specimen and write inspectable HTML artifacts with:

```bash
zpres theme check themes/my-theme --fixture themes/my-theme/specimen.zp.md --write-specimen dist/theme-specimen
```

The generated specimen exercises the declared variants and common content:
text, math, code, charts, figures, media, Layouts, backgrounds, Steps, and
speaker notes. Use it while editing the Theme, then check a real talk to catch
compositions the specimen does not cover.

You can still create a separate deck to exercise a theme for a particular
presentation:

```bash
zpres init examples/theme-specimen.zp.md --title "Theme specimen"
zpres check examples/theme-specimen.zp.md --theme my-theme --theme-dir ../themes
```

```text
themes/my-theme/
  theme.toml
  theme.css.tmpl
  print.css.tmpl
  specimen.zp.md
  fonts/
    body.woff2              optional font you supply
  textures/
    paper.svg               optional artwork you supply
  assets/
    specimen-visual.svg
    specimen-clip.mp4
    specimen-voice.mp3
  data/
    specimen-runtime.csv
```

The optional font and texture paths below illustrate declared dependencies;
create those files or omit their declarations.

`theme.toml` must use a lowercase slug name such as `science` or
`paper-chalk`. Template, asset, and inspiration paths must stay inside the
theme directory.

```toml
[theme]
name = "my-theme"
version = "0.1.0"
api_version = 1
modules = ["scientific-data"]
stylesheet = "theme.css.tmpl"
print_stylesheet = "print.css.tmpl"
fonts = ["fonts/body.woff2"]
assets = ["textures/paper.svg"]
output_targets = ["html", "pdf"]
slide_variants = ["claim", "figure", "comparison", "derivation", "section-title", "dense"]
feature_hooks = ["fit-text", "list-reveal", "slide-presets", "speaker-notes"]

[presets.spotlight]
variant = "claim"
classes = ["lead"]
autoscale = true
transition = "fade"

[presets.spotlight.theme_params]
accent = "#0f766e"

[parameters.accent]
type = "color"
default = "#0f766e"

[parameters.density]
type = "enum"
default = "normal"
values = ["compact", "normal", "spacious"]

[parameters.font_heading]
type = "font"
default = '"Aptos Display", ui-sans-serif, system-ui, sans-serif'

[parameters.type_scale]
type = "number"
default = 1
min = 0.82
max = 1.24

[parameters.canvas_radius]
type = "size"
default = "1.25rem"
```

`fonts` and `assets` are the complete runtime dependency inventory for Theme
CSS. Each entry is a Theme-root-relative file path; directories, absolute
paths, `..`, backslashes, symlink escapes, duplicate portable spellings, and
renderer-owned asset names are rejected. Use the same path in CSS:

```css
@font-face {
  font-family: "My Theme Body";
  src: url("fonts/body.woff2") format("woff2");
  font-style: normal;
  font-weight: 400;
}

.zpres-theme-my-theme .zpres-slide-frame {
  background-image: url("textures/paper.svg");
}
```

zpres validates every `url(...)` against the manifest. It percent-encodes and
copies the dependency for retained HTML, serves the same path during live
authoring, and rebases static screen/print CSS to a local `file:` URL before
Chromium loads it. Query strings and fragments are preserved. Data URLs and
same-document fragments remain inline.

Themes must not use remote or absolute URLs, parent traversal, undeclared
files, `@import`, or escaped CSS identifiers to hide a dependency. The current
bounded grammar also rejects `image()`, `image-set()`, and
`-webkit-image-set()`; use an ordinary declared `url(...)`.
Supporting those forms requires a more complete CSS dependency parser.
The same dependency graph serves live and static output.

Presets let a theme package name reusable slide treatments. Authors can write
`[.preset: spotlight]` or a `::: preset spotlight` directive, and the theme can supply
defaults for `variant`, `classes`, `theme_params`, `autoscale`, and
`transition`. Explicit slide metadata still wins, so presets are a theme-owned
starting point rather than a hard lock. Rendered slides receive
`data-slide-preset` plus `.zpres-slide-preset-{name}` hooks.

## CSS templates

Templates may use:

- `{{theme.name}}`
- `{{theme.version}}`
- `{{param.name}}`

Parameter values are validated before templates render. Color, enum, boolean,
integer, number, font, and size parameters render as CSS tokens. String
parameters render as escaped CSS strings.

Every rendered Theme receives a generated CSS variable block scoped to
`:where(.zpres-api-v1.zpres-theme-{name}, .zpres-api-v1 .{name}-theme)`. All resolved
parameters are exposed as `--zpres-param-{name}`, with underscores converted to
hyphens. Color parameters are also exposed as `--zpres-color-{name}`. Font
parameters are exposed as `--zpres-font-{name}`; if the parameter starts with
`font_`, that prefix is removed, so `font_heading` becomes
`--zpres-font-heading`. Size parameters are exposed as `--zpres-size-{name}`;
if the parameter starts with `size_`, that prefix is removed. That means a
minimal theme can rely on variables such as:

```css
.zpres-slide {
  color: var(--zpres-color-text);
}

.zpres-slide-canvas {
  background: var(--zpres-color-surface);
  border-color: var(--zpres-color-rule);
  border-radius: var(--zpres-size-canvas-radius);
}

.zpres-block-heading h1 {
  font-family: var(--zpres-font-heading);
  font-size: var(--text-slide-heading);
}
```

Templates may still use `{{param.name}}` when a value must be inlined, but CSS
variables are the preferred surface for broad theme styling because palette
variants, front matter overrides, project config, and CLI overrides all resolve
through the same names.

For v1, generated variables target the body carrying both `.zpres-api-v1` and
`.zpres-theme-NAME`. Both templates remain in the `zpres-theme` cascade layer.
The renderer reserves `zpres-reset`, `zpres-foundation`, `zpres-module`, and
`zpres-target` for normalization, general semantic geometry, opt-in shared
behavior, and Output-target behavior. Every Theme
selector must remain under `.zpres-theme-NAME` or its declared Theme class. Theme authors are responsible for this scoping. Selector diagnostics report
selected unknown or unmatched class names; they do not enforce selector isolation
or restrict at-rules. The renderer does not add
a descendant `@scope`: the v1 body is itself the API and Theme root, so that
scope would suppress selectors which name the Theme root. The print template must remain a Theme overlay and must not redefine page size or
pagination. The browser-rendered gate checks the resulting output geometry.

V1 Themes may opt into a renderer-owned shared module through the closed
manifest registry:

```toml
[theme]
api_version = 1
modules = ["scientific-data"]
```

`scientific-data` is currently the only module. Unknown names and duplicate module declarations fail while loading the manifest. The
renderer marks both Output targets with `.zpres-module-scientific-data`; the
module CSS is compiled into the offline v1 foundation and runs before Theme
CSS. A package therefore consumes the behavior by name rather than copying a
stylesheet or reaching outside its package root.

The renderer owns the logical field, safe area, type ramp, accessibility, and
Layout invariants. The scientific/data module owns technical Code, Math, and
Table blocks; Figure, Chart, Diagram, Gallery, and Media geometry; Dense
technical behavior; and reusable Claim/Figure composition. Themes own palette,
font personality, surfaces, accent placement, and ornament. Module geometry
can be adjusted only through the documented `--zpres-data-*` boundary:

- `--zpres-data-primary-measure`
- `--zpres-data-claim-measure`
- `--zpres-data-claim-inset`
- `--zpres-data-dense-measure`
- `--zpres-data-figure-max-block`

Each token is defaulted once in `zpres-module`. Theme overrides belong on the
Theme root and must preserve the semantic role and calibrated type floor.

The renderer foundation is authored with the pinned Tailwind v4 developer
toolchain and committed as ordinary CSS:

```bash
npm ci
npm run build:theme-api-v1-css
```

Tailwind is not a runtime dependency. Generated presentations contain no
Tailwind script, CDN reference, Node process, or network requirement.

Use `type = "font"` for font-family stacks such as
`Inter, system-ui, sans-serif` or `"IBM Plex Mono", ui-monospace, monospace`.
Font parameters are validated as safe CSS font-family lists and render
unquoted, unlike generic string parameters.

Use `type = "size"` for one-token CSS sizes such as `0`, `1rem`, `24px`,
`50%`, or `12pt`. Size parameters are useful for theme-level radius, padding,
gap, border width, and similar layout knobs. They intentionally reject compound
CSS values and functions; use regular CSS in the template when a property needs
a multi-value expression.

## Semantic classes

`theme-api.txt` is the authoritative selector list for the checked package.
It separates stable Theme selectors from internal renderer selectors. Theme
packages should use the stable list. The region selectors include:

- `.zpres-api-v1`
- `.zpres-section-stack`
- `.zpres-slide`
- `.zpres-slide-background`
- `.zpres-slide-frame`
- `.zpres-slide-header`
- `.zpres-slide-title`
- `.zpres-slide-body`
- `.zpres-slide-primary`
- `.zpres-slide-sources`
- `.zpres-slide-footer`
- `.zpres-slide-footer-content`
- `.zpres-slide-footer-number`
- `.zpres-print-slide`
- `.zpres-slide-canvas`
- `.zpres-slide-content`

The typed Content-block and speaker-note selectors that the renderer actually
emits are also listed in the generated reference.

In v1, `.zpres-slide-background` is a direct renderer-owned child of the
Slide. A Theme may style its treatment, but it must not hide the layer, move it
outside the Slide, clear `background-image`, or replace the authored URL. The
renderer foundation owns the default fit, position, treatment, generated
splash, and split geometry on both Output targets. The
`data-zpres-background-source` attribute is validation metadata, not a Theme
selector. Visual report schema 3 records the expected and observed background
separately for screen and print.

Slide variant intent is exposed as `data-slide-variant`, and block kind is exposed as
`data-block-type`. Authors can also add checked slide classes:

```markdown
::: class lead result
:::
```

Those classes appear as `data-slide-classes="lead result"` and as prefixed CSS
hooks such as `.zpres-slide-class-lead` and `.zpres-slide-class-result`. Use
variants for the core theme vocabulary and classes for talk-specific accents
that should still be safe and visible to `zpres theme check`.

Individual slides can also override declared Theme parameters locally:

```markdown
::: theme mode=dark accent="#ff00ff"
:::
```

When one slide needs several presentational choices at once, use a slide
metadata block. This is the compact path for steering a theme without inline
CSS:

```markdown
::: slide
variant: claim
classes: [hero, branded]
theme:
  mode: dark
  accent: "#ff00ff"
colors:
  background: "#101820"
background:
  src: assets/phase-space.svg
  intent: contextual
  alt: "Phase-space overview"
  split: right:35%
  dim: 86
footer: "Main theorem"
slide_numbers: false
autoscale: true
transition: zoom
:::
```

The block writes to the same checked model as the smaller directives:
`variant` becomes `data-slide-variant`, `classes` become prefixed slide class
hooks, `theme` and `colors` become manifest-validated local theme parameters,
and `background` becomes a checked slide background with the normal split and
treatment hooks. Footer settings become `.zpres-slide-footer` content and
slide-number controls. Autoscale settings become `data-autoscale` hooks and
scale the `.zpres-slide-content` wrapper after layout. Transition settings
become live-only `data-transition` hooks.

For common color slots, shorter slide commands are available:

```markdown
[.background-color: #101820]
[.accent-color: "#ff00ff"]

::: text-color #f8fafc
:::
```

The names and values are validated against the active theme manifest during
`zpres check`, `build`, `export`, and fixture checks. In rendered HTML/PDF, the
slide receives `data-theme-param-*` attributes and local CSS variables such as
`--zpres-param-mode`, `--zpres-color-background`, and `--zpres-color-accent`.
This is the zpres-native way to make one slide use a dark palette, a different
accent, or a tighter density without creating a separate theme or writing
inline CSS. The shorthand commands compile to the same typed theme-parameter
path as `::: theme`.

Figures expose `data-figure-fit`, `data-figure-align`, and CSS variables such
as `--zpres-figure-width`, `--zpres-figure-height`, `--zpres-figure-fit`, and
`--zpres-figure-radius`.
Inline image galleries expose `.zpres-block-gallery`, `.zpres-gallery-item`,
`data-gallery-count`, `data-gallery-columns`, and
`--zpres-gallery-columns`; item-level figure hooks remain available inside each
gallery item.
Split backgrounds expose `data-background-split` and
`--zpres-background-split-size`.
Fragmented lists expose `data-list-reveal="fragments"`.
Footers expose `.zpres-slide-footer`, `.zpres-slide-footer-content`,
`.zpres-slide-footer-number`, `data-footer-hidden`, and `data-slide-numbers`.
Autoscaled slides expose `data-autoscale`, `data-autoscale-factor`, and the
`.zpres-slide-content` wrapper. If content still cannot fit, static export
preflight sees `data-zpres-overflow="clipped"` and fails before writing a bad
artifact.
Transitions expose `data-transition` on slides and `.is-entering` during live
navigation. Static export disables transition animations.
Media blocks expose `data-media-kind`, `data-media-fit`, `data-media-align`,
CSS variables such as `--zpres-media-width`, `--zpres-media-height`, and
`--zpres-media-fit`, and, when a video/audio start offset is set,
`data-media-start` in seconds. Hidden live media exposes
`data-media-hidden="true"`. Media that advances the live deck on playback end
exposes `data-media-autoadvance="true"`. Mermaid flowcharts expose
`data-diagram-language="mermaid"`, `.zpres-diagram-svg`,
`.zpres-diagram-node`, and `.zpres-diagram-edge`. Layout directives also expose
`data-layout-kind`, `data-layout-widths`, and `data-layout-tracks`. Grid regions
also expose `data-grid-column`, `data-grid-column-span`, `data-grid-row`, and
`data-grid-row-span`. These attributes describe checked model values; Themes
must not infer a different reading order from them.

Overlay and Aside regions expose `data-region-role`. Overlay annotations also
expose `data-overlay-anchor` and `data-overlay-width`; the shell declares
`data-overlay-policy="edge-only"` and its protected title, footer, caption, and
safe-area boundaries. The renderer owns stacking, Aside width floors, and the
linear narrow-container fallback. Themes may change surface treatment, borders,
and ornament, but must not reposition annotations or reverse region order.

Comparison Slides expose `data-slide-variant="comparison"`. Their regions use
`data-comparison-role="primary|supporting"`, `data-comparison-cue="circle|square"`,
and a stable `.zpres-layout-region-content` wrapper. The shared foundation owns
parallel label/content guides and the narrow stacked fallback. Themes should
reinforce the redundant cues, as Science does with teal/rust plus shape and
Debug does with solid/dashed borders plus shape; color alone is insufficient.

Derivation Slides expose `data-slide-variant="derivation"`. The Layout shell
exposes `data-derivation`, `data-step-count`, and `data-step-pdf-policy`.
Stable context uses `data-derivation-role="context"`; changing regions use
`data-derivation-role="stage"`, `data-step-index`, and
`data-step-state="future|active|complete"`. The shared foundation preserves
context, hides future stages on screen, and supplies a square/filled-circle
state cue plus border-weight change. Themes may reinforce these states, but
must retain a non-color cue and the authored reading order. The hidden
`[data-zpres-step-progress]` status reports `Step current of total` without
entering the focus order. Existing fragment transitions are disabled under
`prefers-reduced-motion`, while the border and shape state remain.

The v1 foundation owns the minimum interaction states as well. Keyboard
targets receive a three-pixel `:focus-visible` outline with a surface gap.
Presentation shortcuts do not consume keys from links, controls, media, or
explicitly focusable descendants, and the runtime calls `preventDefault()`
only for a shortcut it handles. The current Step carries `aria-current="step"`
and a leading rule in addition to color. `prefers-contrast: more` strengthens
that rule and marks completed Steps in the same channel; `forced-colors` uses
system colors for the same cues. `prefers-reduced-motion` removes
renderer-owned animation and transition duration without hiding state.

The visual gate exercises these preference states in Chrome for every v1
screen state and print page. It also blocks reliably computed solid text below
`4.5:1`, or `3:1` for WCAG-large text, and focus/current-Step indicators below
`3:1`. Image-backed contrast remains a human-review item. These standards are
blocking for all Themes.

Dense Slides expose `data-slide-variant="dense"`. Technical blocks use
`data-zpres-type-role="technical"`; figures, charts, diagrams, galleries, and
media use `data-zpres-content-role="evidence"`; captions use the `micro` type
role. Table sections expose `data-table-role="header|body"`, and column headers
retain `scope="col"`. Chart axes and legends expose `data-chart-role` and use
Technical rather than Micro text. Themes may tighten spacing for Dense Detail
slides, but must not reduce the Technical floor or turn Dense into automatic
fit-to-canvas behavior.

The v1 foundation preserves code lines with `white-space: pre`. Unrevealed code
hides line numbers; revealed code retains them as stable references. Gallery
items use the declared column count, and a gallery followed by media divides
the available evidence height between both blocks. Visually hidden media does
not reserve screen or print geometry. Captions and static fallback cards remain
inside the evidence region.

Layout regions can contain normal rendered content such as figures, charts,
math, code, tables, lists, media, Steps, and paragraphs. Themes should style the
layout shell with `.zpres-block-layout`, `.zpres-layout-regions`, and
`.zpres-layout-region`, while relying on the nested `.zpres-block-*` classes for
the content inside each region. The v1 foundation owns Grid tracks, checked
placement, Stack direction, Overlay stacking, Aside width floors, and the named
`body` container's `38rem` linear Layout fallbacks; Theme CSS may polish regions
but must preserve those semantics.

## Declaring feature hooks

`slide_variants` declares the presentation shapes a theme knows how to style.
`feature_hooks` declares the cross-cutting authoring features the theme
intentionally supports. This makes theme packages easier to review and safer to
reuse: the manifest says what the theme promises, and `zpres theme check`
verifies that the chosen fixture actually exercises those promises.

The supported feature hook names are:

- `autoscale`
- `background-image`
- `background-splash`
- `background-split`
- `background-treatment`
- `chart-local-data`
- `code-reveal`
- `detail-slides`
- `figure-align`
- `figure-fit`
- `figure-radius`
- `figure-size`
- `figure-treatment`
- `fit-text`
- `footer`
- `footnotes`
- `gallery-columns`
- `html-only`
- `image-gallery`
- `list-reveal`
- `media-align`
- `media-autoadvance`
- `media-fit`
- `media-hidden`
- `media-poster`
- `media-size`
- `media-start`
- `mermaid-diagram`
- `slide-classes`
- `slide-presets`
- `speaker-notes`
- `steps-final-state`
- `steps-pages`
- `transitions`

Keep `feature_hooks` honest. If a theme declares `media-poster` or
`mermaid-diagram`, the fixture passed to `zpres theme check` must include media
with a poster or a Mermaid diagram. Otherwise the checker fails with the missing
hook names. Use `--fixture` to point at a richer theme specimen while authoring:

```bash
zpres theme check themes/my-theme --fixture examples/theme-specimen.zp.md
```

## Color variants

A Theme can define palette variants with the standard color slots. Put
`palette_parameter` in the `[theme]` table:

```toml
[theme]
palette_parameter = "mode"

[color_variants.light]
background = "#f4f9ff"
surface = "#ffffff"
text = "#174ea6"
muted = "#5e7fb1"
accent = "#1f63d1"
accent_alt = "#ef6f8f"
rule = "#c4daf8"

[parameters.mode]
type = "enum"
default = "light"
values = ["light"]

[parameters.background]
type = "color"

[parameters.surface]
type = "color"

[parameters.text]
type = "color"

[parameters.muted]
type = "color"

[parameters.accent]
type = "color"

[parameters.accent_alt]
type = "color"

[parameters.rule]
type = "color"
```

Every color used by a variant must be declared as a color parameter. Authors can
override individual color slots from front matter, project config, or CLI flags.

## Checking a Theme

Run the theme checker before using a new package in a deck:

```bash
zpres theme check themes/my-theme
```

The checker loads `theme.toml`, verifies referenced templates, fonts, assets,
and inspiration files, validates the manifest contract, renders the screen and print CSS
templates, renders each declared palette variant, and renders a fixture deck
through the theme. If the package contains `specimen.zp.md`, that local
specimen is used by default; otherwise zpres falls back to the repository's
wide-sweep fixture when available. The fixture pass checks HTML rendering, print
HTML readiness, and PDF/static readiness without requiring Chromium. It also
fails if the fixture uses a `data-slide-variant` that the theme did not declare
in `slide_variants`, or if the theme declares a `feature_hooks` entry that the
fixture does not exercise.

The coverage report lists the variants, blocks, Layouts, media, and feature
hooks exercised by the fixture. Use it to find gaps in the specimen:

| Field | Meaning |
| --- | --- |
| `fixture_features` | Supported hooks the fixture exercises |
| `fixture_uncovered_features` | Supported hooks it does not exercise |
| `declared_feature_hooks` | Hooks the Theme promises to support |

An uncovered hook is advisory unless the Theme declares it in `feature_hooks`.
Every declared hook must be exercised or the check fails. Coverage alone does
not establish visual quality: a fixture may contain a Figure without giving
it enough room or readable labels.

Use a custom fixture while authoring a theme for a particular talk:

```bash
zpres theme check themes/my-theme --fixture talk.zp.md
```

Write an inspectable specimen while checking the theme:

```bash
zpres theme check themes/my-theme --fixture talk.zp.md --write-specimen dist/theme-specimen
```

Open `review.html` to browse the generated artifacts:

| File | Purpose |
| --- | --- |
| `index.html` | Live HTML presentation |
| `print.html` | Static document for PDF and page images |
| `print-notes.html` | Static document with speaker-note pages |
| `speaker-notes.txt` | Plain-text rehearsal script |
| `theme-check.txt` | Fixture coverage and page counts |
| `theme-api.txt` | Resolved CSS variables, stable selectors, and declared hooks |

These files come from the checked fixture and Theme package. Browser captures
and visual reports are added only with `--visual`.

When you review a v1 Theme, use the same inspection order that
[`themes/reference/specimen.zp.md`](../themes/reference/specimen.zp.md)
teaches:

1. Run with `--visual` to generate fresh screen and print captures.
2. Inspect representative and boundary-risk files under `screen-pages/` and
   `pages/` at original size before reading the automated reports. Check
   hierarchy, text clearance, evidence area, Step policy, and readability.
3. Use `screen-contact-sheet.png` and `contact-sheet.png` to judge Deck rhythm
   and the relationship between Main and Detail slides.
4. Read `theme-api.txt`, `theme-check.txt`, and the visual reports as supporting
   evidence. Record separate screen and print judgments where they differ.

Write one specimen per palette variant with:

```bash
zpres theme check themes/my-theme --fixture themes/my-theme/specimen.zp.md --write-specimen dist/theme-variants --all-variants
```

For a Theme whose `mode` parameter declares `light` and `dark`, this writes
`dist/theme-variants/defaults/`, `dist/theme-variants/light/`, and
`dist/theme-variants/dark/`. Each directory contains its own `review.html`,
`index.html`, `print.html`, `print-notes.html`, `speaker-notes.txt`,
`theme-check.txt`, and `theme-api.txt`. The resolved Theme CSS and declared
dependencies live inside the HTML generation selected by `index.html`; the
review page links to that exact CSS file. Do not assume that a mutable root
`assets/theme.css` exists. This lets light/dark or branded palette variants be
inspected side by side without publishing a partially updated bundle.

## Browser-rendered release review

The contract check above proves that zpres can load and render a Theme package. It does not approve the Theme for a presentation. Run the browser-backed review against the wide fixture and the current talk's Source file:

```bash
zpres theme check themes/my-theme \
  --visual \
  --fixture fixtures/canonical/wide-sweep.zp.md \
  --write-specimen dist/my-theme-wide

zpres theme check themes/my-theme \
  --visual \
  --fixture talk.zp.md \
  --room-profile projected-room-default \
  --write-specimen dist/my-theme-talk
```

The generated specimen and the v1 reference fixtures exercise all five typed
Layout kinds: Columns, Grid, Stack, Overlay, and Aside. Their regions accept
Content blocks, but nested Layout directives and slide-level metadata are
rejected by the Source parser. A Theme may give these compositions its own
visual treatment while preserving geometry, reading order, and pagination.

The review opens the retained offline `index.html` through the production navigation runtime. It visits every Main slide, Detail slide, generated slide, and Step state, waits for autoscaling, finite animations, and final paint, and writes full-size captures under `screen-pages/` plus `screen-contact-sheet.png`. It then opens `print.html` independently, waits for static readiness, and writes the print captures under `pages/` plus `contact-sheet.png`. `visual-report.json` and `visual-report.txt` retain the measurements and result; `provenance.json` records the generation inputs and environment.

The specimen directory is not one atomic Output target. Its HTML publication
and raster subdirectories can each be coherent while print HTML, notes,
reports, contact sheets, or provenance still belong to another or interrupted
run. Treat the directory as durable release evidence only after the complete
command succeeds and its files are reviewed together.

Both traversals fail on missing or clipped content, unresolved markers, failed fonts or images, scroll overflow, incorrect rendered state, and content outside the canvas or declared layout boundaries. The screen traversal also checks route identity, active Section and Slide counts, Step visibility, and footer placement. Descendant elements and text ranges are measured, so a fitting parent does not hide an overflowing code line, image, caption, or positioned child. Autoscaling below 1 is retained as a route- or page-specific calibration warning; content that still clips fails. Sparse and upper-stacked compositions remain warnings for the reviewer.

## Reading visual reports

`visual-report.json` schema version 8 also records design
measurements. Every visible text or evidence sample names its element, type and
content role, surface, resolved palette, Slide, and Step. Text samples include
computed size and line height, a named cap-height proxy, measured line count,
prose extent, solid-surface contrast or image-backed status, and sampled actual
Chrome font families. Evidence samples include canvas occupancy, raster
resolution, and Figure alternative status. The top-level `design_summary`
collects minimum essential and Technical type, Micro use, actual fonts, image
status, and autoscale distribution. `design_review_notes` retain their measured
basis but do not enter `warnings`, `failures`, or release status. The v1 gate
enforces the room-independent solid contrast, interaction, state, and
alternative-structure checks. The resolved room profile is
stored in the report and provenance artifact with its status and content hash.
The built-in `projected-room-default` remains provisional, so projection type,
enhanced contrast, and autoscale are report-only. Once a profile has complete
physical evidence and an approved decision, v1 essential-content type floors
and the hard autoscale floor become blocking.
`--room-profile` accepts the built-in name or a TOML file. Project-config paths
are relative to that config file, while direct `theme check` paths are relative
to the current directory. File-backed profiles retain their canonical path and
content hash in both artifacts.

`room-profile-calibration.json` is the compact calibration view of those raw
measurements. It keeps screen and print separate, then records per-role type
percentiles and floor counts, ordinary and large-text contrast distributions,
evidence occupancy, and autoscale distributions. Repeated Step states remain
repeated samples and the artifact says so explicitly. Use the summary to choose
what to inspect at room distance; do not treat its percentiles as room evidence.

The provisional thresholds select slides for full-size review; they are not
permission to treat a report-only number as a venue standard. A file-backed
profile should record the actual display, viewing distance, ambient conditions,
presentation machine, rehearsal date, reviewers, and approval decision.

The platform-font portion of the visual report records the faces and glyph
counts Chrome actually used for visible text on each observed screen state and
print page.
The evidence distinguishes “no browser evidence” from an observed face and
keeps representative probes bounded. Chrome does not identify why a fallback
was chosen or whether it synthesized a weight or style, so those fields report
`unavailable` instead of guessing.
Theme-specific pass/fail rules for expected font faces and glyph coverage
are not implemented.

The result separates `contract_status`, aggregate `visual_status`,
`screen_status`, `print_status`, `human_review`, and `release_status`. Screen
and print are independently traversed and measured: a passing screen surface
does not conceal print drift, and the report retains surface-specific failures
and captures. The browser review also emulates reduced motion, increased
contrast, forced colors, and keyboard focus visibility. These are bounded
contract probes, not substitutes for reviewing the retained full-size screen
and print captures. An objective pass remains `pending-review` until someone
has checked composition, contrast, text size, controls, and the actual talk
flow at the presentation viewport. Serve and rehearse the final offline bundle
on the presentation machine even after the automated traversal passes. A
talk-shaped fixture may report incomplete feature coverage because it does not
exercise the full Theme API; the wide fixture should remain the comprehensive
contract baseline.

The Chromium review enables local-file access and executes HTML authored in a Source file. Run it only on trusted Source files and Theme packages. Keep a separate review directory for each Theme, Source file revision, and palette variant so one candidate cannot overwrite another's evidence.

Use `--no-fixture` for a fast package-only check. Contract problems fail the
command before a deck build or PDF export depends on the theme.
