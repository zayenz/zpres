# Figure authoring

Use a Figure for a plot, photograph, or diagram that belongs in the slide's
content. zpres checks local image files, bundles them with HTML, watches them
during preview, and verifies them before static export. For an image behind
other content, see [background authoring](background-authoring.md).

Start with a Markdown image and a short alternative description. Add sizing,
a caption, or a gallery only when the slide needs it.

## Markdown figures

```markdown
![Phase-space sketch](assets/phase-space.svg "Synthetic phase-space sketch")
```

The image alt text becomes the figure `alt`, and the optional Markdown title
becomes the caption.

For Theme API v1, a non-empty `alt` marks a meaningful Figure. Keep it to one
paragraph of at most 160 characters. A visible caption or directive body is
optional; use one when it helps the audience interpret the Figure, not merely
to satisfy validation. An empty `alt=""` with no caption is explicitly
decorative. A caption paired with an empty alternative is rejected because
zpres cannot tell whether the missing alternative was intentional. The same
rule applies to every gallery item. The gate checks this structure; a reviewer
still judges whether the short alternative conveys the Figure's purpose and
whether a complex relation also needs nearby prose, notes, or an audible
description.

You can also put the caption on the next line as a standalone emphasized
sentence:

```markdown
![Phase-space sketch](assets/phase-space.svg)
*Synthetic phase-space sketch.*
```

This is only treated as a caption when the image did not already provide a
Markdown title or `caption=` attribute.

For fast layout hints, put figure attributes in the image label:

```markdown
![width=70% fit=contain align=center grayscale=35 saturate=72 corner-radius(14) alt="Runtime comparison"](figures/runtime.svg "Runtime comparison")
```

zpres also accepts Pandoc/Quarto-style image attributes after the Markdown
image, which makes imported technical decks and existing notes easier to use:

```markdown
![Runtime comparison](figures/runtime.svg){out-width="70%" fig-align="center" fig-alt="Line chart comparing solver runtime." fig-cap="Runtime comparison"}
```

The common aliases map onto the same typed figure model: `width` and
`out-width` set the displayed width, `height` and `out-height` set the displayed
height, `fig-align` maps to `align`, `fig-alt` maps to `alt`, `fig-cap` or
`fig-caption` maps to `caption`, and `fig-fit` maps to `fit`. Local
`pdf-src`/`static-src` fallbacks and visual treatment aliases such as
`fig-radius`, `fig-dim`, `fig-grayscale`, `fig-saturate`, and `fig-blur` work
in the same attribute block. CSS classes and ids inside the block, such as
`.wide` or `#fig-runtime`, are ignored for now.

For the common "make this image fill the usable slide area without cropping"
case, use the shorter fit shorthand:

```markdown
![fit alt="Runtime comparison"](figures/runtime.svg "Runtime comparison")
```

This expands to `width=100%`, `height=100%`, `fit=contain`, and
`align=center`. Explicit attributes still win, so `![fit width=80%](...)`
keeps the 80% width while using the other fit defaults.

Markdown figures can also appear inside layout regions such as `columns` and
`aside`. They keep the same asset validation, live dependency tracking, bundle
copying, and PDF preflight behavior as top-level figures.

Live HTML renders figure and gallery images with browser lazy-loading hints so
large decks do not eagerly fetch every image at startup. Print/PDF/PNG/JPEG
exports keep normal eager image `src` attributes so Chromium captures a
deterministic static artifact.

Remote figures can declare a checked local fallback for static export:

```markdown
![pdf-src="assets/plot-fallback.png" alt="Remote plot"](https://example.com/plot.png "Remote plot")
```

Live HTML keeps the remote URL. Print/PDF/PNG/JPEG export uses the local
`pdf-src` image instead and checks that it exists before capture. `static-src`,
`pdf_src`, and `static_src` are accepted aliases. Remote figures without a
local fallback fail PDF/static readiness because they are not reliable export
dependencies.

Animated GIFs use the same fallback path. Live HTML keeps the `.gif`, while
print/PDF/PNG/JPEG export requires a deterministic local `pdf-src` or
`static-src` still image:

```markdown
![static-src="assets/demo-still.png" alt="Animated solver trace"](assets/demo.gif "Animated solver trace")
```

## Inline image galleries

Use `inline` when several images should live together as one visual group
instead of separate stacked figures:

```markdown
![inline fill columns=2 alt="Before state"](figures/before.png "Before")
![inline fill alt="After state"](figures/after.png "After")
```

Consecutive `inline` images become a typed gallery block. The gallery keeps the
same local asset checks, live reload, bundle copying, static fallback handling,
and PDF preflight behavior as normal figures. Use `columns=2` or `columns(2)`
on any item to choose a stable grid from one to six columns. Without an explicit
column count, zpres uses one column for one image, two columns for two images,
and three columns for larger galleries.

Each gallery item accepts the normal figure visual hints such as `fit`, `fill`,
percentage width, `dim`, `grayscale`, `saturate`, `blur`, and
`corner-radius(...)`.

## Directive figures

Use the directive form when the figure needs more explicit presentation intent:

```markdown
::: figure src="figures/runtime.svg" alt="Runtime comparison" width="70%" height="48vh" fit="contain" align="center" dim="12" grayscale="20" saturate="85" blur="1" radius="14"
Runtime comparison for the three solvers.
:::
```

Supported attributes:

- `src`: required local image path or image URL.
- `pdf-src` or `static-src`: checked local image used for PDF/PNG/JPEG/print
  export when `src` is remote.
- `alt`: accessible image description.
- `caption`: explicit caption. If omitted, directive body text becomes caption.
- `width`: CSS size token such as `70%`, `640px`, `42rem`, or `80vw`.
- `height`: CSS size token such as `48vh`, `360px`, or `auto`.
- `fit`: `contain`, `cover`, or `fill`.
- `align`: `start`, `center`, `end`, or `stretch`.
- `dim`: brightness reduction from `0` to `100`; higher values quiet the image.
- `grayscale`: grayscale treatment from `0` to `100`.
- `saturate`: saturation treatment from `0` to `100`.
- `blur`: blur in pixels from `0` to `24`.
- `radius` or `corner-radius`: rounded image corner radius as a CSS size token
  such as `14`, `14px`, `0.75rem`, or `8%`.

Markdown images also accept Deckset-style `corner-radius(...)` and
`radius(...)` tokens:

```markdown
![fit corner-radius(18) alt="Architecture sketch"](figures/architecture.png)
```

To set a default corner radius for all plain figures and gallery images in a
deck, add a Deckset-style command near the top of the source file:

```markdown
image-corner-radius: 12
```

You can also use front matter:

```yaml
---
image_corner_radius: 12
---
```

The default applies only when an individual image does not set `radius`,
`corner-radius`, `radius(...)`, or `corner-radius(...)` itself.

## Theme hooks

Themes receive these hints as `data-figure-fit`, `data-figure-align`,
`data-figure-radius`, `data-figure-treatment`, and CSS variables such as
`--zpres-figure-width`, `--zpres-figure-height`, `--zpres-figure-fit`,
`--zpres-figure-radius`, `--zpres-figure-brightness`, `--zpres-figure-gray`,
`--zpres-figure-saturate`, and `--zpres-figure-blur`. The intent stays in the
deck model; the theme still owns the final visual treatment.

Gallery blocks render as `.zpres-block-gallery` with `data-gallery-count`,
`data-gallery-columns`, `--zpres-gallery-columns`, and nested
`.zpres-gallery-item` figures. Item-level figure hooks stay available inside
the gallery.
