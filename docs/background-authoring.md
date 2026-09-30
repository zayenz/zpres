# Background authoring

Use a background when an image should sit behind content or occupy one side
of the slide. Use a [Figure](figure-authoring.md) when the image belongs in the
ordinary content flow.

A background can apply to the whole Deck or to one slide:

- `background_image` in front matter applies an atmospheric Deck background and
  can also appear on the title slide or a generated full-image Splash slide.
- `::: background` inside a slide applies a checked background image only to
  that slide.
- `![bg](...)` is shorthand for a checked slide background when Markdown-native
  image syntax is more convenient.

Use slide backgrounds when a visual is part of the story, but the slide still
needs foreground text, math, code, or figures.

## Background meaning and reading order

Theme API v1 requires the author to say what an image does:

- `decorative` repeats atmosphere or ornament. It must not have `alt` or
  `description`, and it stays out of the accessibility tree on every Slide.
- `contextual` establishes setting or identity without carrying the argument.
  It requires a short `alt`; `description` is optional.
- `evidence` contributes to the argument. It requires both a short `alt` and a
  longer `description`.

The rendered pixels still come from `.zpres-slide-background`. For contextual
and evidence-bearing images, v1 also emits a visually hidden semantic Figure
before the Slide frame. Its image carries the short alternative, and its
caption carries the longer description when one exists. This keeps meaningful
split imagery in reading order without exposing the painted copy twice.

The visual report records the declared intent, alternative and description
presence, focal-crop status, and whether local image-backed contrast needs
review. Missing structure or discarded semantics block v1. Wording quality,
focal-content preservation, and photographic contrast remain explicit human
checks.

## Apply a background

For a simple theme-colored slide background, use a local theme color override:

```markdown
[.background-color: #101820]
[.accent-color: "#ff00ff"]

# One dark slide
```

These commands set the slide-local `background` and `accent` theme parameters,
so they are validated against the active theme and render in HTML, PDF, PNG,
and JPEG through the same CSS variable path as `::: theme`.

When a slide needs color, classes, and a visual background together, use the
compact slide metadata block:

```markdown
::: slide
classes: [hero]
colors:
  background: "#101820"
  accent: "#ff00ff"
background:
  src: assets/phase-space.svg
  intent: evidence
  alt: "Feasible region after propagation"
  description: "The blue feasible region is roughly one quarter of the grey baseline region."
  split: right:35%
  dim: 86
:::

# One visual slide
```

This still uses the checked background model and the same manifest-validated
theme parameter path as the shorter color commands.

```markdown
# Visual result

::: background src="assets/phase-space.svg" intent="contextual" alt="Phase-space detail" position="center 42%" dim="86" grayscale="35" saturate="72"
:::

The foreground content stays on a protected canvas.
```

The same slide-local background can be written with Markdown image syntax:

```markdown
# Visual result

![bg contain intent="contextual" position="center 42%" dim="86" grayscale="35" saturate="72" alt="Phase-space detail"](assets/phase-space.svg)

The foreground content stays on a protected canvas.
```

When importing Marp/Marpit decks, zpres also accepts the common comment
directive form. Non-underscore directives apply to the current and following
slides, while underscore-prefixed directives apply only to the current slide:

```markdown
<!--
backgroundImage: url('assets/phase-space.svg')
backgroundPosition: center 42%
backgroundSize: contain
-->

# Visual result

---

<!--
_backgroundImage: url('assets/backup.svg')
_backgroundPosition: right bottom
_backgroundSize: cover
-->

# Backup visual
```

Marpit comments do not carry zpres background intent or alternative routes.
Before using an imported background
with a v1 Theme, rewrite it as `::: background`, `![bg]`, or `::: slide` and
declare its intent. This is deliberate: zpres does not guess whether an
imported image is decoration or evidence.

Supported attributes:

- `src`: required local image path.
- `intent`: required by Theme API v1; `decorative`, `contextual`, or `evidence`.
- `alt`: short alternative for a contextual or evidence-bearing image. It must
  be a single paragraph of at most 160 characters.
- `description`: longer explanation. Evidence backgrounds require it;
  contextual backgrounds may use it when the short alternative is not enough.
- `position`: safe CSS background-position tokens such as `center 42%`.
- `fit`: `cover` or `contain`.
- `dim`: scrim strength from `0` to `100`; higher values protect foreground
  text more strongly.
- `grayscale`: grayscale treatment from `0` to `100`.
- `saturate`: saturation treatment from `0` to `100`.
- `blur`: blur in pixels from `0` to `24`.

Deck-level front matter also accepts two Theme API v1 policy fields:

```markdown
---
background_image:
  src: assets/phase-space.svg
  intent: contextual
  alt: "Abstract phase-space field"
  title: clean
  splash: true
---
```

- `title`: `clean` by default, or `paint` to paint the Deck background while
  retaining the Title phase.
- `splash`: omitted and `false` produce no v1 Splash; only explicit `true`
  generates one.

These fields are Deck-only. Using them in `::: slide`, `::: background`, or
`![bg]` is a Source error.

For `![bg]` shorthand, put these attributes in the image alt text after the
`bg` keyword. `cover` and `contain` may also be written as bare keywords, as in
`![bg contain intent="contextual" alt="Diagram overview"](assets/diagram.svg)`.

## Split backgrounds

Use a split background when an image should occupy one side of the slide and
the foreground content should use the remaining side:

```markdown
![bg left:40% intent="contextual" alt="Portrait of the speaker"](assets/portrait.jpg)

# About me

- Constraint programming
- Scientific talks
```

Supported split keywords are `left`, `right`, `left:40%`, and `right:40%`.
Without a percentage, the image uses half the slide. Percentages must be between
`10%` and `90%`. zpres exposes split backgrounds through
`data-background-split` and `--zpres-background-split-size`, so themes can
refine the default side-by-side layout.

Local background images are resolved relative to the deck root, copied into
HTML bundles, watched by the live server, and checked before PDF or PNG export.
The defaults protect foreground text, but the actual image and room still
need visual review.

## Renderer checks

Theme API v1 treats an authored image background as renderer-owned structure.
Each Content or generated Splash Slide must retain exactly one direct
`.zpres-slide-background` layer with the authored source. `zpres theme check
--visual` checks screen and print independently. Static PDF, PNG, and JPEG
preflight applies the same print check before publishing an artifact. A hidden
layer, `background-image: none`, a replaced source, or a layer outside the
Slide is an objective failure.

The Theme API v1 Presentation plan keeps the first Section's Main slide in the
Title phase. It is clean unless `title: paint` or a slide-local background
paints it. Later Slides inherit the Deck image unless they provide a local
background. An explicit Splash follows the complete first Section, including
Detail slides and static notes pages; each notes page immediately follows its
owning Slide. Screen, print, page counts, readiness, and background validation
consume this same order.

A Deck background that no page paints is valid dormant metadata. zpres warns,
but still validates, bundles, watches, and hashes the file. Unknown, malformed,
duplicate, or context-invalid v1 fields fail at their Source location.

The survival check enforces the declared semantic class and alternative route,
but it does not approve crop, focal-content preservation, image-backed
contrast, or alternative-text adequacy. It also enforces the source and split
side, but not an exact split percentage, position, fit, dim, grayscale,
saturation, or blur value; the v1 reference foundation has separate
screen/print regressions for those rendering hooks.
