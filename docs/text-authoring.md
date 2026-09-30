# Text authoring

Use ordinary Markdown for headings, paragraphs, lists, quotations, and tables.
Directives add presentation features such as Steps, notes, and Layouts. If you
are starting a Deck, read [getting started](getting-started.md) first for slide
separators, front matter, and navigation.

This guide covers [Steps](#steps), [text treatments](#fit-text),
[Layouts](#layouts), [speaker notes](#speaker-notes), and
[footers](#footers). Each example is a fragment to place inside a slide unless
it explicitly shows front matter. Theme hooks are included for authors who
need to style the result.

## Inline text

Theme API v1 renders `*emphasis*`, `**strong text**`, `_emphasis_`,
`__strong text__`, inline backtick code, and `[link text](target)` links in
headings, paragraphs, lists, tables, captions, and Steps. Code spans keep math,
HTML, and citation markers literal. Backslash escapes keep punctuation literal.
Inline math and footnotes can appear alongside these forms.

Links accept HTTP, HTTPS, mailto, and relative document targets. Unsafe URL
schemes remain visible text. Raw HTML is escaped; use an explicit HTML-only
block when HTML is intended. This is a bounded inline subset: reference-style
links, inline images, and the full CommonMark grammar are not supported.

## Steps

Use Steps to reveal an explanation in order:

```markdown
::: steps
1. Inspect the domains.
2. Choose a variable.
3. Branch on a value.
:::
```

The HTML presentation reveals one Step at a time. Static exports show the
final state on one page by default. To keep the progression as pages, declare
that policy explicitly:

```markdown
::: steps pdf="pages"
1. Inspect the domains.
2. Choose a variable.
3. Branch on a value.
:::
```

Use [Derivation](#derivation-pattern) when each Step needs a labeled region and
some context should remain visible. [Incremental lists](#lists) are a shorter
form for revealing individual points.

## Fit text

Use fit text when a slide needs one large, theme-owned statement:

```markdown
[fit] Search order is part of the model.
```

For longer statements, use the directive form:

```markdown
::: fit
Search becomes operational at \(x_t\).
:::
```

Fit text becomes a typed `fit-text` block with `.zpres-block-fit-text` and
`data-block-type="fit-text"`. Themes should treat it as a high-emphasis
statement, separate from ordinary headings and paragraphs. Inline math inside
fit text is checked before PDF/static export.

## Autoscale

Use autoscale when a normal slide should keep its structure but shrink dense
body content just enough to avoid clipping:

```yaml
autoscale: true
```

Turn it on or off for one slide with Deckset-style commands:

```markdown
[.autoscale: false]
```

or through slide metadata:

```markdown
::: slide
autoscale: true
:::
```

Autoscale renders as `data-autoscale="true"` and scales the stable
`.zpres-slide-content` wrapper in live HTML and print/export HTML. If content
still cannot fit at the minimum supported scale, zpres marks the slide with
`data-zpres-overflow="clipped"` so static export preflight can fail instead of
silently producing a bad PDF.

## Theme presets

Use a preset when a theme should supply a reusable slide treatment:

```markdown
[.preset: spotlight]
```

The directive form is equivalent:

```markdown
::: preset spotlight
:::
```

Presets are declared by the active theme. A preset can provide a slide variant,
classes, theme parameters, autoscale, and transition defaults. Slide-local
metadata still wins when it sets the same field explicitly.

In rendered HTML/PDF, zpres exposes the selected preset as
`data-slide-preset="spotlight"` and `.zpres-slide-preset-spotlight`.
`zpres check`, `build`, `serve`, and `export` fail or report diagnostics if a
slide uses a preset the active theme does not define.

## Transitions

Set a deck-wide live transition in front matter:

```yaml
transition: fade
```

Use `none`, `fade`, `slide`, or `zoom`. Deckset-style boolean syntax is also
accepted:

```yaml
slide-transition: true
```

Override one slide with a command:

```markdown
[.transition: zoom]
[.slide-transition: false]
```

or with slide metadata:

```markdown
::: slide
transition: slide
:::
```

Transitions render as `data-transition` hooks and animate only in live HTML.
Print/PDF/PNG/JPEG exports keep the final slide state with animations disabled.

## Layouts

Use columns when two ideas should stay visually related while the theme owns the
spacing and responsive behavior:

```markdown
::: columns widths="40/60" gap="4" align="start"
Left column:

- model
- assumptions

Right column:

![fit alt="Runtime plot"](figures/runtime.svg "Runtime plot")
:::
```

The native form uses `Left column:` / `Right column:` style headings to
split regions in source order. `widths` accepts fractions such as `40/60`,
scale steps such as `1/1`, and checked CSS sizes in brackets such as
`[35%]/[65%]`. `gap` uses the same checked size grammar (or a spacing-scale
number), and `align` is one of `start`, `center`, `end`, or `stretch`. These
values cannot inject arbitrary CSS declarations.

zpres also accepts the common Pandoc/Quarto fenced-div column form, which makes
many reveal.js and Quarto slides easier to port:

```markdown
:::: {.columns}
::: {.column width="40%" name="Evidence"}
![Evidence plot](figures/runtime.svg){out-width="85%" fig-align="center" fig-alt="Runtime plot"}
:::

::: {.column width="60%" name="Explanation"}
- first point
- second point
:::
::::
```

Nested `.column` divs become the same typed `columns` Layout block. Column
`width` attributes become layout widths when every column declares one.
Content inside every region follows the normal typed Content-block pipeline:
figures, charts and local data, math, code, media, callouts, lists, tables, and
Steps retain their ordinary validation, dependencies, live behavior, and
static fallbacks. Nested Layout blocks and slide-level metadata are rejected
at their Source location.

## Comparison pattern

Use an explicit Comparison when the audience must judge two alternatives on
the same terms:

```markdown
::::: comparison
:::: primary label="Baseline"
::: fit
42 nodes
:::

Chronological branching
::::

:::: supporting label="Candidate"
::: fit
11 nodes
:::

Impact-guided branching
::::
:::::
```

The block establishes the `comparison` Slide variant. It requires exactly one
labeled `primary` region followed by one labeled `supporting` region. Labels
are part of the semantic reading order, not generated decoration. Both regions
share label and content guides; circle and square markers provide a redundant
cue alongside label, color, and position. At a `body` width of `38rem` or less,
the regions stack primary-first instead of shrinking their text.

Existing Sources that declare `::: variant comparison` and contain exactly two
labeled Columns are promoted to the same roles. For three or more alternatives,
use a labeled Grid under the Comparison variant; the first region is primary
and the remaining regions are supporting peers.

## Derivation pattern

Use Derivation when stable context should remain visible while one meaningful
change is revealed at a time:

```markdown
::::: derivation pdf="final"
:::: context label="Invariant"
For every \(t\), \(x_t \in D\).
::::
:::: stage label="Transition"
$$x_{t+1}=f(x_t,u_t)$$
::::
:::: stage label="Consequence"
Therefore \(x_{t+1} \in D\).
::::
:::::
```

The block establishes the `derivation` Slide variant. It requires one labeled
`context` followed by one to six labeled `stage` regions. Each stage is one
semantic Step, so regions cannot contain nested Steps, fragmented lists, or
revealed code. Use unrevealed code inside a stage, or split a process across
stages instead.

`pdf="final"` is the default and prints one page containing the context and all
stages. `pdf="pages"` prints one cumulative page per stage: page two retains
the context and stage one before adding stage two. This makes every static page
understandable without a hidden prerequisite.

## Dense technical Detail slides

Dense is for compact technical evidence that belongs in a Detail slide. It is
not permission to shrink an overloaded Main slide:

````markdown
# The propagation rule cuts the search tree

The Main slide states the result and the comparison that supports it.

--

## Detail: Solver trace

::: variant dense
:::

| Model | Nodes | Runtime |
| --- | ---: | ---: |
| Baseline | 42 | 18.4 s |
| **Candidate** | **11** | **6.2 s** |

```rust
let bound = propagate(&model);
assert!(bound <= incumbent);
```
````

Dense content keeps the Technical type role as its floor. Code preserves
authored lines; long lines fail visual review instead of wrapping into a
different program. Line numbers appear for revealed code, where they help
identify the changing lines, and stay out of ordinary excerpts.

Table headers remain semantic headers. Right-aligned numeric columns use
tabular numerals. A row containing a fully bold cell receives an inset rule,
surface change, weight, and underline, so its emphasis does not depend on color
alone. Large manuscript tables still need to be split into several views.

A Main slide may declare `dense`, but Theme API v1 reports a design warning.
Use that exception only when the Main path genuinely requires exact technical
evidence at the provisional 24 px Technical floor.

## Grid and Stack Layouts

Grid is for evidence that belongs on shared visual tracks. Declare one to six
checked `tracks`, then write each cell as an explicit four-colon region inside
a five-colon Grid fence:

```markdown
::::: grid tracks="2/1/1" gap="4" align="start"
:::: cell name="Primary evidence" column="1" span="2"
![Search trajectory](figures/search.svg)
::::

:::: cell name="Checks" column="3" row="1"
> [!NOTE] Contract
> Source order remains reading order.
::::
:::::
```

`column` and `row` are one-based. `span` is an alias for `column-span`;
`row-span` is also available. A cell without explicit placement uses normal
row-major auto-placement. Explicit cells may not overlap or extend beyond the
declared tracks. Visual placement never reorders the cell elements in the DOM.
When the named `body` container is at most `38rem` wide, Grid becomes one
linear column in Source order.

Use Stack for a vertical argument or sequence:

```markdown
::::: stack gap="4" align="stretch"
:::: item name="Question"
What must remain invariant?
::::

:::: item name="Answer"
The typed model and static output must agree.
::::
:::::
```

Stack items always follow Source order. `gap` controls vertical spacing and
`align` controls horizontal item alignment (`start`, `center`, `end`, or
`stretch`). Grid placement attributes on Stack items are an error. Both Layout
kinds accept normal typed Content blocks inside their regions.

## Overlay and Aside Layouts

Overlay is for a small number of annotations attached to one evidence surface.
The base must come first, followed by one to three annotations:

```markdown
::::: overlay overlap="edge-only"
:::: base name="Search geometry"
![Feasible regions](figures/search.svg "Search geometry")
::::

:::: annotation name="Fixed point" anchor="top-end" width="compact"
The final point lies in the feasible core.
::::
:::::
```

The only overlap policy is `edge-only`. Annotation anchors are `top-start`,
`top-end`, `bottom-start`, and `bottom-end`; each anchor can be used once.
Annotation width is `compact` or `standard`. Center placement, arbitrary
coordinates, duplicate anchors, and annotations before the base are errors.
The visual gate rejects annotations that cross the title, footer, figure
caption, or body safe area. At a `body` width of `38rem` or less, the Overlay
becomes a linear base-then-annotations sequence.

Aside reserves subordinate context without letting it compete with the primary
evidence:

```markdown
::::: aside supporting="standard" gap="6" align="start"
:::: primary name="Argument"
The main result and its evidence.
::::

:::: supporting name="Context"
The boundary condition used in this example.
::::
:::::
```

Aside requires exactly one `primary` followed by one `supporting` region.
`supporting` is `compact` or `standard`; both choices leave the primary at
least two thirds of the available width. A narrow `body` container stacks the
supporting region below the primary. Screen and static output retain both
regions in the same semantic order.

## Speaker notes

Use the block form for presenter notes that stay out of the visible slide:

```markdown
::: notes
Pause before the result.
:::
```

Quarto/reveal.js-style notes blocks are accepted too:

```markdown
::: {.notes}
Pause before the result.
:::
```

When importing reveal.js Markdown, a `Note:` or `Notes:` marker starts
speaker notes for the rest of the current slide:

```markdown
Note:
Pause before the result.
Mention the backup slide.
```

When importing Slidev Markdown, the final HTML comment block in a slide becomes
speaker notes:

```markdown
Visible slide text.

<!-- This is a **note** -->
```

HTML comments that appear before visible slide content are treated as hidden
author comments rather than notes. reveal.js attribute comments such as
`<!-- .slide: data-background="#ff0000" -->` are also kept out of speaker
notes.

Slidev-style click markers inside notes become presenter-panel cues that track
the current step:

```markdown
<!--
Setup before the first reveal.
[click] Say this after the first reveal.
[click:3] Say this after the third reveal.
-->
```

The markers are not shown in exported speaker scripts; the note text remains.

For Deckset-style authoring, prefix note lines with a caret:

```markdown
^ Pause before the result.
^ Mention the backup slide.
```

All of these notes syntaxes become the same typed `speaker-notes` blocks. They
are hidden from the slide, shown in the live presenter notes panel, and included
as extra note pages in static exports only when you opt in with
`zpres export talk.zp.md --pdf rehearsal.pdf --notes`. Use
`zpres export talk.zp.md --notes-txt notes.txt` to write a plain-text speaker
script without requiring Chromium.

Notes are included in the HTML document by default, even when the notes panel
is closed. Use `zpres build talk.zp.md --exclude-speaker-notes` for a shared
copy that must omit them. Use a fresh output directory so older retained
builds cannot carry notes into that copy.

## Slide classes

Use a slide class when a theme needs a talk-specific hook that is more precise
than the built-in slide variants:

```markdown
::: class lead result
:::
```

Slide class names must use lowercase letters, numbers, and hyphens, starting
with a letter. zpres exposes them as `data-slide-classes` and prefixed CSS hooks
such as `.zpres-slide-class-lead`; the raw names are not injected directly into
the document.

Marpit-style HTML comment directives can import the same hooks:

```markdown
<!--
class: lead
backgroundColor: "#f8fafc"
color: "#111827"
backgroundImage: url('assets/background.jpg')
backgroundPosition: left top
backgroundSize: contain
-->
```

Without an underscore, supported Marpit local directives apply to the current
slide and following slides. Prefix the directive with `_` to make it a
single-slide spot directive:

```markdown
<!--
_class: result
_backgroundColor: "#222222"
_backgroundImage: url('assets/backup.jpg')
_backgroundPosition: right bottom
_backgroundSize: cover
-->
```

## Footers

Set a deck-wide footer in front matter:

```yaml
footer: "Constraint programming workshop"
slide_numbers: true
```

Override one slide with Deckset-style commands:

```markdown
[.footer: Backup result]
[.slidenumbers: false]
```

Hide the footer entirely on a title or full-bleed visual slide:

```markdown
[.hide-footer]
```

Marpit-style comment directives for pagination and footers are accepted too:

```markdown
<!--
paginate: true
footer: Constraint programming workshop
-->

# First slide

---

<!--
_paginate: false
_footer: Backup only
-->

# Backup slide
```

The same settings can live inside a compact slide metadata block:

```markdown
::: slide
footer: "Main theorem"
slide_numbers: false
:::
```

Footers render as `.zpres-slide-footer` in both live HTML and print/export
HTML. Themes can style `.zpres-slide-footer-content` and
`.zpres-slide-footer-number`, while `data-footer-hidden` and
`data-slide-numbers` expose slide-local controls.

## Quotes

Use standard Markdown blockquotes for quoted material, excerpts, definitions,
or a voice that should be visually distinct from the author's narration:

```markdown
> Search is where the model becomes operational: \(x_{t+1}=f(x_t)\).
> The quote can span multiple lines.
```

Quotes become typed `quote` blocks with the semantic class
`.zpres-block-quote` and `data-block-type="quote"`. Inline math inside a quote
is checked and rendered through the same path as paragraph math, so PDF export
can fail early if the math is not supported.

## Lists

Use normal Markdown lists for grouped points:

```markdown
- Model the state \(x_t\).
- Apply the branching rule.

1. Build the relaxation.
2. Check the bound.
```

Top-level unordered and ordered lists become typed `list` blocks with
`.zpres-block-list`, `data-block-type="list"`, and
`data-list-kind="unordered"` or `data-list-kind="ordered"`. Inline math inside
list items is checked before PDF export.

Use `*` bullets or `1)` ordered items when the list should reveal one item at a
time in the live presentation:

```markdown
* First point.
* Second point.
* Final point.

1) First ordered point.
2) Second ordered point.
```

For Pandoc/Quarto/reveal.js compatibility, you can also wrap a normal Markdown
list in an incremental fenced div:

```markdown
::: {.incremental}
- First point.
- Second point.
:::

::: incremental
1. First ordered point.
2. Second ordered point.
:::
```

For Deckset-style decks, enable build lists once for the whole file:

```markdown
build-lists: true

# Slide

- First point.
- Second point.
```

The same setting can be written in YAML front matter as `build-lists: true` or
`build_lists: true`. Deckset's `build-lists: all` spelling is accepted as an
alias for `true`, and `build-lists: notFirst` shows the first item immediately
while building the rest.

Override the default on one slide with Deckset-style slide commands:

```markdown
[.build-lists: false]

- This list appears all at once.

---

[.build-lists: all]

- This list reveals item by item again.

---

[.build-lists: not_first]

- This item is visible immediately.
- This item reveals first.
```

`::: nonincremental` and `::: {.nonincremental}` are accepted as explicit
all-at-once lists, which is useful when porting a deck that used a global
incremental default.

Fragmented lists expose `data-list-reveal="fragments"`. PDF, PNG, and JPEG
exports render the final-state list on one page.

## Footnotes

Use Markdown footnotes for citations, source notes, and brief context that
should stay visible on the citing slide:

```markdown
Search order changes the result[^search].

[^search]: The note can be defined on any slide and may contain \(x_t\).
```

Footnote definitions can appear anywhere in the source file. zpres numbers
references per slide in first-use order, including references in Layout regions,
removes definitions from normal slide flow, and appends a typed `footnotes` block
to each slide that cites them.
Missing references and duplicate definitions are fatal diagnostics. Footnotes
render in HTML, PDF, PNG, and JPEG output with `.zpres-block-footnotes`,
`.zpres-footnote-ref`, `.zpres-footnote-list`, and `.zpres-footnote` hooks.

## Callouts

Use GitHub-style alert blockquotes for notes, tips, warnings, and other
presentation callouts:

```markdown
> [!WARNING] Search caveat
> A greedy branching rule can hide \(2^n\) work.
```

Supported kinds are `note`, `tip`, `important`, `warning`, and `caution`.
Aliases `info`, `success`, `warn`, and `danger` are accepted. Callouts become
typed `callout` blocks with `.zpres-block-callout`,
`data-block-type="callout"`, and `data-callout-kind="{kind}"`. The optional
text after the marker becomes the callout title.
