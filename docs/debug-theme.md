# Debug Theme

The Debug Theme helps inspect slide structure, layout boundaries, Steps, and
overflow. It uses Theme API v1 geometry with a quiet grid and monospace headings.

Use the normal presentation to inspect renderer-owned geometry without Debug
labels:

```bash
zpres serve talk.zp.md --theme debug
```

Use `theme check --inspection` to retain the diagnostic overlay for every live
state and PDF page. The overlay labels each non-empty Frame, Header, Body,
Primary, Supporting, Sources, Footer, and Ornament region at its measured
boundary relative to the 1280 by 720 Slide canvas. Its orientation band exposes
the Slide identifier, Section and Main/Detail role, route and Step identity,
and Output target on both the HTML presentation and PDF export surfaces.

The renderer computes the bounds after layout and again after fonts load or the
viewport changes. Enabling the overlay adds data attributes, outlines, and
pseudo-element labels. It does not add padding, borders, margins, or content to
the semantic reading order, so the logical content box is unchanged.

Debug uses three semantic Step lifecycle states: `future` is labelled
`QUEUED` with a dashed diamond treatment, `active` is labelled `CURRENT` with
a pill/dot and heavy leading line, and `complete` is labelled `COMPLETED` with
a double-line/check treatment. There is no failed Step lifecycle state in the
Deck model or renderer. `FAILED` belongs to the inspection diagnostic rail and
means that the browser detected real remaining overflow. Print Steps expose
the same lifecycle values through a print-specific renderer hook; this is
renderer-owned Step/static policy, while the labels and line grammar are
Theme-owned.

Figure, Caption, Sources, and Footer are separate, boundary-attached lanes.
Their labels use reserved lane space and remain outside authored text. In
inspection mode, the print-only status rail occupies the top safe gutter and
reports PDF page, one-page-per-Step or final-state policy, page state,
autoscale, and detected overflow without changing semantic geometry.

The Debug Theme's local specimen is `themes/debug/specimen.zp.md`. It covers
Main and Detail slides, Steps, fragmented lists, footnotes, speaker notes,
autoscale, presets, transitions, and the footer without using Layout
directives. Run both Output-target checks with:

```bash
cargo run -- theme check themes/debug \
  --visual \
  --inspection \
  --fixture themes/debug/specimen.zp.md \
  --write-specimen dist/debug-theme-specimen
```

`provenance.json` records `capture_mode = "inspection"`. Omitting
`--inspection` records `capture_mode = "normal"` and retains the ordinary Debug
surface instead.

The default `specimen.zp.md` is the objective release specimen: recovered
autoscale may produce a review warning, but it must have no objective failure.
`diagnostic-specimen.zp.md` is explicitly report-only and deliberately
overflows. Use it only to verify that the objective gate fails and the rail
shows `FAILED`:

```bash
cargo run -- theme check themes/debug \
  --fixture themes/debug/diagnostic-specimen.zp.md \
  --visual --inspection \
  --write-specimen dist/debug-report-only-diagnostic
```

That command is expected to exit unsuccessfully. A passing result is a
regression; never substitute it for the valid objective specimen.

Debug deliberately uses a quiet grid and monospace headings. These make
contract boundaries legible; they are not a design recommendation for
production Themes.

## Historical maintainer review (2026-07-13)

The review below describes a past checkout. Its local `dist/` artifacts are
not shipped with the repository; use the commands above to generate current
evidence.

The review recorded normal and inspection evidence under
`dist/issue-0041/reverification/normal/defaults/` and
`dist/issue-0041/reverification/inspection/defaults/`. Each set contains
27 original-size HTML presentation states, 17 original-size PDF export pages,
and separate screen/print contact sheets. Both objective gates pass with zero
failure. The controlled warning appears on screen and print at 95.8% autoscale;
Chromium still measures 32 px minimum body text.

Original-size review covered the initial queued summary, current and completed
Step treatments, Main/Detail identity, Figure/Caption/Sources/Footer lane
boundaries, Comparison solid/dashed roles, Derivation progression, the Dense
Detail slide, the autoscale warning, and all three one-page-per-Step process
pages. The dominant reading order remains title, authored evidence, Sources,
then Footer. Boundary labels and the print status rail remain in reserved
gutters; no inspected authored text is occluded. State labels are readable at
1280 by 720, and authored body text remains projector-sized. The contact sheets
retain distinct Main/Detail, Figure, Comparison, Derivation, Dense, warning,
and print-page silhouettes.

The first independent review rejected a Body label that crossed authored text
on inspection state 27. The repair used measured Slide-level overlay
placements, and the final review found zero authored-text intersections across
all 276 screen and print labels. The Codex app preview again displayed false
black rectangles on some images; direct ImageMagick inspection found opaque
RGB PNGs and off-white pixels in those regions.

The deliberately invalid report-only capture is retained separately under
`dist/issue-0041/reverification/diagnostic/`; its command exit code is 1, its
objective screen and print gates fail, and four inspected states/pages show a
`FAILED` diagnostic rail. This evidence does not count as a valid Theme check.

This review approves the objective gate and this Debug composition only. It
does not approve a physical room profile, a production Theme portfolio, or a
final Source release.
