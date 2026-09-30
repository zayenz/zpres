# Theme and example inventory

This inventory records the Themes and examples shipped in the repository. It
cannot describe external Theme packages installed on another machine.

## Choose a built-in Theme

All built-in Themes use API v1 and are embedded in the installed binary.
Try one with `zpres serve talk.zp.md --theme NAME`.

| Name | Treatment | Palettes |
| --- | --- | --- |
| `science` | Serif headings with restrained teal and rust accents | Light, Dark |
| `dark-splash` | Dark technical slides with luminous accents and bounded evidence regions | Cyan, Violet, Magenta, Ember |
| `paper-chalk` | Field notebook with a grid, margin line, and annotation marks | Light, Dark |
| `sv` | Technical workbench with condensed headings and squared evidence regions | Light, Dark |
| `wedding` | Formal stationery with serif type and botanical or ribbon ornament | Garden, Ivory, Slate Rose |
| `signal` | Swiss poster typography, heavy vermilion rules, square comparison regions | Warm white |
| `tidal` | Deep ocean surface, seafoam headings, contour arcs, rounded technical regions | Midnight teal |
| `kiln` | Warm paper, editorial serif headings, terracotta printmaking marks | Bone |
| `prism` | Geometric headings, violet facets, asymmetric corner treatments | Lavender |
| `blueprint` | Cobalt technical type, drafting rules, registration ticks, mono labels | Ice blue |
| `debug` | Inspection of layout regions, Step state, and overflow | Diagnostic surface |
| `reference` | Reference compositions for Theme authors | Reference surface |

Science, Paper Chalk, and SV select palettes with `--theme-param mode=dark`
or `mode=light`. Dark Splash and Wedding use the `variant` parameter; for
example, `--theme-param variant=violet` or `variant=ivory`.

All built-in Theme packages are version 0.1.0. Package versions are separate
from the zpres release and Theme API version. Packages live under
`themes/NAME/` and are maintained with zpres.

Signal, Tidal, Kiln, Prism, and Blueprint each include a local
`specimen.zp.md`, an illustrative Figure, and local chart data. The packages
use local system-font stacks with fallbacks and accept the standard color and
font Theme parameters. Check the actual fonts and layout on the presentation
machine before use. In these five Themes, explicit slide background and text
colors also supply the surface and muted colors unless those are set separately.

For example:

```sh
zpres serve themes/tidal/specimen.zp.md
zpres theme check themes/tidal --all-variants --visual --write-specimen dist/tidal
```

Each Theme has an ordinary composition for slides without a special variant
or annotation. Use that as the starting point, then add semantic treatment
when it helps explain the content. See [Theme authoring](theme-authoring.md)
for custom packages and [the design contract](presentation-design.md) for
review criteria.

## User-facing example Decks

| Source file | Theme | API | Aspect | Owner | Status |
| --- | --- | ---: | --- | --- | --- |
| `examples/quickstart/talk.zp.md` | Science | 1 | 16:9 | zpres maintainer | maintained |
| `examples/overlapping-intervals/talk.zp.md` | Science | 1 | 16:9 | zpres maintainer | maintained |

## Contract fixtures

The canonical and wide-sweep Decks, Debug specimen, production Theme references, layout/background fixtures, and `scientific-data-consumer` are maintained test material. Their owner is the zpres test suite. The consumer package uses API v1; Theme-specific reference Decks resolve to their corresponding v1 built-in.
