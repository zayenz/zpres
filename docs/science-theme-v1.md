# Science Theme API v1

Science uses serif-led headings, quiet surfaces,
and teal and rust accents. It supports light and dark palettes. Select it in
Source front matter or try it with:

```sh
zpres serve talk.zp.md --theme science --theme-param mode=light
```

Science uses Theme API v1 and the shared `scientific-data` module. This guide
explains its compositions and the fixtures used to review them.

The Theme uses the renderer-owned Frame, Header, Body, Primary, Sources, and
Footer regions. It provides compositions for Section-title, Claim, Figure,
Comparison, Derivation, Dense, Main, and Detail slides.

The narrow reference Deck is
`fixtures/theme-api-v1/science-reference.zp.md`. It covers:

- an ordinary explanation with a fragmented list, source, and speaker note;
- Section-title and Claim variants;
- a Figure variant whose local SVG expands through the available Body;
- aligned value and figure Comparisons with labeled primary/supporting roles,
  redundant shape cues, and a narrow stacked fallback;
- connected Main and Detail slides;
- a final coherent Step state;
- mathematical and non-math Derivations with final-state and per-Step PDF policy;
- Dense Detail evidence with a table, code, equation, and captioned Figure;
- light and dark palettes, footer behavior, and the renderer-owned type roles.

The focused argument-trace specimen is
`themes/science/argument-trace-specimen.zp.md`. It shows how
Science marks the argument:

- teal marks the primary explanatory path or accepted evidence;
- rust marks an exception, caveat, or competing result;
- every semantic color cue also carries a second authored cue through
  line style, marker shape, position, or direct labels.

Run the complete review with:

```bash
cargo run -- theme check themes/science \
  --visual \
  --fixture fixtures/theme-api-v1/science-reference.zp.md \
  --write-specimen dist/science-v1-specimen \
  --all-variants
```

## Historical fixture review

The following results describe the 2026-07-12 review, not a fresh verification
of the current checkout. Run the command above for current evidence.

On that date, defaults, light, and dark each passed 15 Slides, 26 live states,
and 17 print pages with zero failures. Each palette reports the same two
review warnings for the deliberately sparse Section-title on screen and print. Full-size and contact
sheet review found the Figure appropriately dominant, the Claim recognizably
different from ordinary explanation, Comparison labels and evidence aligned on
shared guides, the narrow Comparison stacked without shrinking its roles, the
Detail treatment subordinate but complete, and the screen/print compositions
consistent. Derivation kept its invariant visible, marked the active stage with
shape and border weight as well as color, and produced coherent cumulative
per-Step pages.
The Dense Detail kept its essential table, code, and equation text at 24 px on
screen and print. Its highlighted row used an inset rule and underline in
addition to color, while the companion Figure retained its caption and evidence
area.

Science's visual gate measures essential and Technical text separately from
Figure containers and footnote markers. Wrapped titles and image-backed
backgrounds remain explicit human-review notes. Browser checks cover the
default, dark, and light palettes as well as reduced motion, increased contrast,
forced colors, focus, current Step cues, and structural alternative routes.

The palette pairs meet WCAG contrast minimums on their surfaces:

| Palette | Pair | Contrast |
| --- | --- | ---: |
| Light | text / surface | 17.52:1 |
| Light | muted / surface | 5.90:1 |
| Light | accent / surface | 5.33:1 |
| Dark | text / surface | 14.70:1 |
| Dark | muted / surface | 7.96:1 |
| Dark | accent / surface | 11.07:1 |

These are display contrast measurements, not calibrated projection targets.

Science deliberately ships no external font file. Its declared stacks use
Charter and Avenir Next when available, followed by documented serif and
sans-serif fallbacks. The local SVG evidence asset is bundled with the Theme.

## Shared module

Science uses the renderer-owned `scientific-data` module. Technical
blocks, data/evidence primitives, and reusable Claim/Figure geometry live in
the module layer; Science keeps the serif-led typography, teal/rust palette,
accent rail, surfaces, Detail treatment, and other visual identity. Its only
module-token override retains the established 54rem Dense evidence measure.

The small `scientific-data-consumer` fixture separately proves that a v1 Theme
can consume the module with palette-only Theme CSS. It exercises the module
without Science's visual treatment, so shared behavior can be checked separately.
