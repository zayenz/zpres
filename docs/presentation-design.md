# Presentation design contract

This contract defines how a zpres presentation should support a live talk.
Use it when designing a Theme, changing the renderer, or reviewing a Deck.
The [Theme authoring guide](theme-authoring.md) supplies package syntax and
commands; the [evidence and limits](#evidence-and-limits) below explain the
research and standards behind these choices.

A slide should make its communicative job apparent in one glance, then give the audience enough visual evidence to follow the speaker. The renderer owns the conditions that make that possible. A Theme gives those conditions a particular voice.

The rules are deliberately stricter about Main slides than Detail slides. A Main slide belongs to the talk's primary path and must work at room distance, under time pressure, while the audience is listening. A Detail slide may carry a derivation, code excerpt, exact table, or qualification, but it must remain legible.

## How to read the rules

Read every rule on two axes. The first is its normative force:

- **Invariant**: part of the renderer, Theme, or Output target contract. A Theme cannot opt out.
- **Default**: the normal choice. A Slide variant or explicit choice in a Source file may replace it when the alternative still satisfies the invariants.
- **Review criterion**: a human judgment that automated checks can support but cannot settle.
- **Theme choice**: visual expression that can vary freely inside the other constraints.

In normative prose, **must** and **cannot** mark invariants; **should** and an explicit “default” mark defaults; a request to inspect, rehearse, or judge marks a review criterion; and **may** marks a Theme choice unless the sentence says otherwise.

The second axis is the basis for the rule:

- **Standard**: required or defined by an external normative specification.
- **Research synthesis**: a practical deduction from the cited evidence, with its limitations preserved.
- **Calibrated project value**: measured against named zpres fixtures, machines, fonts, or rooms.
- **House rule**: chosen to keep zpres coherent where research and standards do not select one value.

The [evidence and limits](#evidence-and-limits) below distinguish research findings from project choices. The distinction matters: a practical threshold should not be presented as a law of cognition.

Implementation status is separate from both the rule and its basis:

| Status | Meaning in this document |
| --- | --- |
| **Current** | Implemented authoring, rendering, or review behavior in Theme API v1 |
| **Planned** | Structure, tooling, or enforcement that does not exist yet |
| **Calibrating** | A proposed numeric value that remains report-only until real Deck and room tests select it |

Audience, composition, Content-block, renderer-region, Layout, Step, background,
publication, accessibility, and manual release rules are **Current** Theme API
v1 guidance. All built-in Themes use this contract. The browser gate measures
live routes and print pages, and selectable room profiles carry provenance and
can enforce approved floors. The typed face/glyph contract remains **Planned**
and is not enforced yet. Physical-room approval of the provisional default profile and
its projection type, contrast, occupancy, and autoscale values remains
**Calibrating**.

The main concrete decisions are registered here so their force, basis, and status remain visible:

| Decision | Force | Basis | Status |
| --- | --- | --- | --- |
| One communicative job per Main slide | Default | Research synthesis plus house rule | Current |
| Claim or question followed by nearby evidence | Default | Research synthesis | Current |
| 1280 by 720 logical field for 16:9 | Invariant | House rule | Current |
| Safe-area and spacing values | Default | House rule | Current; room-facing floors calibrating |
| Role ordering and selected room-profile floor | Invariant once selected | Research synthesis plus calibrated project value | Calibrating |
| WCAG text and non-text minimums | Invariant | Standard | Current enforcement |
| `7:1` body and `4.5:1` large-text projection targets | Default | House rule | Calibrating |
| Autoscale warning and hard-fail thresholds | Review criterion, then invariant hard floor | Calibrated project value | Calibrating |

## The audience task

A projected slide supports a live explanation. It is not a paper page enlarged onto a wall.

An audience member needs to do three things quickly:

1. identify what this slide is trying to establish;
2. find the evidence or explanation that supports it;
3. connect that evidence to what the speaker is saying now.

The design should minimize unrelated visual search, transcription, and decoding. A person who looks away briefly should be able to recover the current point from the title, the dominant evidence, and the visible state of the slide.

This leads to the central default for scientific Main slides:

> State the claim or question in the title region, then use the body for evidence, explanation, or a worked step.

This is broader than a rigid assertion-evidence template. A Section slide can orient rather than argue. A derivation can state the transformation being made. A figure slide can pose a question before revealing the conclusion. One communicative job is the default, not a requirement to use one grammatical form.

## Ownership boundary

Theme API v1 separates renderer-owned geometry from Theme-owned visual treatment:

| Owner | Owns | Does not own |
| --- | --- | --- |
| Deck and Source file | Message, evidence, Slide variant, Layout directive, Steps, captions, alternatives, and Detail-slide structure | Raw CSS positioning or Theme-specific DOM assumptions |
| Renderer | Semantic DOM, logical canvas, safe area, title/body/footer regions, reading order, layout semantics, type floors, overflow policy, accessibility preferences, and target-independent geometry | Theme personality, ornamental language, or talk-specific emphasis |
| Shared module | Opt-in semantic composition for a named family of Themes, including technical blocks and scientific/data evidence geometry | Palette, font personality, ornament, logical canvas, type floors, or Output-target policy |
| Theme | Font families and weights within the metric limits, role colors, surface treatment, accent placement, radius, limited depth, ornament, and declared variant compositions | Minimum legibility, semantic order, contrast floors, static fallbacks, or silent structural reinterpretation |
| Output target | Navigation, Step realization, pagination, static media handling, and target-specific readiness | Changing what the Deck says or shrinking content to evade a failed layout |

The renderer should produce a presentable neutral composition before a Theme adds personality. Theme CSS should therefore set tokens and variant composition rather than infer semantic roles from child order.

## Deck-level composition

### Build a coherent Main path

The Main slides should tell the talk without requiring a trip through any Detail slide. A useful sequence for a scientific talk is often:

- the problem or observation;
- the question or constraint;
- the approach at the level needed to interpret the result;
- the result and its evidence;
- the limitation or boundary;
- the conclusion and consequence.

This is a pattern, not a mandatory section taxonomy. The test is whether each Main slide makes the next one feel earned.

### Give each Main slide one communicative job

One job can contain several facts or evidence marks. A comparison needs both sides. A theorem may need its conditions and conclusion. A chart may need context, uncertainty, and a highlighted result. They belong together when the audience must consider them together to understand the claim.

Split the material when the speaker must explain independent ideas in sequence, when a figure needs several unrelated callouts, or when the design must reduce essential text below the technical-content floor.

Do not encode a universal word count, bullet count, or slide-per-minute rule. Rehearsed dwell time, computed size, overflow, and reading burden are better signals.

### Use titles as navigation and interpretation

The default Main-slide title is a short sentence that states the claim, result, contrast, or question. Prefer “Symmetry cuts the search tree in half” to “Symmetry results.”

A good title:

- is specific enough to guide interpretation;
- still makes sense in a contact sheet or title-only outline;
- fits in one or two lines at the title role;
- uses sentence case;
- avoids a trailing period unless punctuation helps the meaning.

Section titles, cover titles, short quotations, and deliberate questions are valid exceptions to the sentence-assertion default.

### Put detail where it belongs

Use Detail slides for:

- worked derivations and proofs;
- full code excerpts;
- exact lookup tables;
- methodological qualifications;
- sensitivity analyses;
- secondary results;
- backup material for questions.

A Detail slide should visibly remain attached to its Section, but the indicator is metadata. It must not compete with the title or evidence.

### End on the conclusion

The final Main slide should keep the main result, its boundary, and the next useful action visible. A bare “Questions?” slide discards the strongest visual support during discussion. Contact details and acknowledgements can be present without replacing the conclusion.

## Theme-neutral slide anatomy

**Status: Current in Theme API v1.** The renderer implements frame, background,
header, title, body, primary, supporting, sources, footer, ornament, and typed
Layout regions. Browser checks cover authored backgrounds on live and static
surfaces. All Themes use this contract.

Theme API v1 renders explicit regions:

```text
section.zpres-slide
├── .zpres-slide-background
├── .zpres-slide-frame
│   ├── header.zpres-slide-header
│   │   ├── .zpres-slide-kicker
│   │   └── .zpres-slide-title
│   ├── main.zpres-slide-body
│   │   ├── .zpres-slide-primary
│   │   └── .zpres-slide-supporting?
│   ├── .zpres-slide-sources?
│   └── footer.zpres-slide-footer?
└── .zpres-slide-ornament
```

This diagram shows the principal roles. The generated `theme-api.txt` lists
the stable selectors for a checked package.

- The painted background sits outside reading order. Contextual and evidence
  images also have a semantic Figure carrying their alternative text; see
  [background authoring](background-authoring.md).
- The frame establishes the safe area and composition grid.
- The header owns the slide's navigational statement.
- The body owns evidence and explanation.
- Sources remain attached to the evidence without entering the main reading path.
- The footer has reserved space and cannot overlap the body.
- Ornament is Theme-owned, non-semantic, and unable to consume required content area.

The `.zpres-slide-content` wrapper remains available for content sizing.
New Themes should use named regions rather than `:first-child`, block-count selectors, or fixture-specific `:has(...)` rules to identify these roles.

## Canvas and positioning

**Status: Current in Theme API v1.** The renderer uses a 1280 by 720 field,
fills or letterboxes the live viewport, and rejects non-16:9 Decks before
publication.

### Use one logical composition field

The HTML presentation and the print-HTML surface used for a 16:9 PDF export should share a logical 1280 by 720 composition field. The browser may scale or letterbox that field, but it should not independently reflow the same slide into a different composition during PDF capture.

Other aspect ratios are rejected. Supporting another ratio requires matching
Deck, Theme, live, and static rendering behavior; see
[ADR 0012](adr/0012-retain-the-distinct-v1-theme-portfolio-and-16-9-contract.md).

A reflowed handout would need a separate Output target; it is not implemented.
Shrinking presentation slides does not provide a reading view.

### Reserve a safe area

The initial house defaults on a 1280 by 720 field are:

| Token | Initial value | Purpose |
| --- | ---: | --- |
| `--zpres-safe-inline` | `72px` | Keeps content away from crop, overscan, and ornamental edges |
| `--zpres-safe-block-start` | `52px` | Creates a stable top landmark |
| `--zpres-safe-block-end` | `48px` | Protects the footer and bottom crop |
| `--zpres-footer-band` | `28px` | Reserves footer/source metadata outside the body |
| `--zpres-grid-columns` | `12` | Gives direct Layout directives a shared alignment system |
| `--zpres-grid-gap` | `24px` | Keeps columns related without collapsing their boundary |

These are house defaults, not perception constants. A Theme may make the safe area more generous. It may reduce it only within a validated API v1 range and only when ornament or a full-bleed figure still preserves the content floor.

Essential text and evidence should stay out of the outer five percent of the slide. Full-bleed media and background art may cross it.

### Keep landmarks stable

On ordinary Main slides, keep the title in a consistent upper region and align multi-line content to the logical start edge. Stability lets the audience spend attention on new information rather than finding the layout again.

Centered composition is appropriate for short cover, Section, quotation, and closing treatments. It is a poor default for paragraphs, lists, tables, or multi-line technical content.

Visual change should communicate a real state change. A Section boundary may change composition. A Step may highlight the term that changed. Arbitrary shifts in title location, type family, background, or transition imply meaning where none exists.

### Use the available body

Whitespace separates and prioritizes; it should not become an excuse to leave a small chart or tiny text in one corner of an otherwise empty slide.

The primary evidence region should normally occupy the remaining body height. Figure and chart variants should let the visual grow. A short claim can use a centered body. A comparison should align both sides to a shared baseline. Ordinary content should not sit in an upper-left stack while most of the body remains unused.

### Make local components respond to their region

Use viewport breakpoints for the application shell and presentation chrome. Use named container queries for Layout regions and reusable blocks.

Suggested container names are:

- `slide` for the logical composition field;
- `body` for the available evidence area;
- `region` for a Layout region;
- `figure`, `table`, and `code` only when their internals genuinely need distinct thresholds.

Keep thresholds sparse and semantic. A comparison region may switch its internal label placement when it becomes narrow; it should not acquire a dozen micro-breakpoints. Use `cqi` and `cqb` only when a stepped token cannot prevent an awkward wrap.

## Hierarchy

Every Main slide needs one dominant element. It can be the assertion, a result figure, a numerical result, or a worked transformation. The dominant element should be obvious without every other element becoming faint.

Use these mechanisms in order:

1. placement and occupied area;
2. proximity and whitespace;
3. type weight and contrast;
4. a restrained accent or enclosure;
5. motion, only when the change itself is explanatory.

Do not put every block in a card. Cards imply separate, comparable units and consume space. Prefer alignment, proximity, and surface contrast. Use a border or accent rule when it establishes a relationship the spacing alone cannot show.

Metadata, citations, slide numbers, and Detail indicators are secondary. They must remain readable, but they should not compete with the title or evidence.

## Typography

### Size for the room

Point sizes are a poor cross-medium contract. What the audience sees depends on the projected screen height, farthest viewing distance, font metrics, ambient light, and projector quality.

zpres defines type roles within the logical slide field and uses room profiles
to record venue-specific measurements. The provisional `projected-room-default` profile uses this ramp on a 720-high logical field:

| Role | Size / line height | Typical use |
| --- | --- | --- |
| Display | `72px / 0.98` | Cover, Section, single-number result |
| Title | `48px / 1.06` | Ordinary Main-slide assertion or question |
| Heading | `36px / 1.12` | Supporting region heading |
| Body | `32px / 1.30` | Main explanatory text and lists |
| Technical | `24px / 1.32` | Code, table cells, chart labels, compact Detail content |
| Micro | `18px / 1.25` | sources, slide numbers, nonessential metadata |

At the browser's standard conversion, `32px` equals `24pt`. Physical legibility
still depends on viewing distance and room calibration, not that unit conversion.

These values scale with the whole logical canvas. Within `projected-room-default`, they are both nominal role sizes and floors for essential content assigned to those roles. The profile is a provisional engineering baseline, not a claim that any logical size works in every venue. A selected room profile records the physical screen, farthest viewing position, font metrics, projection machine, and rehearsal result; it may raise any floor while preserving the role order.

The browser gate reports these values without blocking releases until the calibration phase approves a profile. The selected profile and its content hash are stored in the visual report and provenance artifact. After approval, preserving that selected profile's role ordering and floor becomes an invariant for v1 Themes. These are calibrated zpres project values, not universal findings about presentation typography.

Rules for the floor:

- Main-slide prose should use the Body role.
- Code, table cells, chart labels, and dense Detail content may use Technical.
- Micro cannot carry a claim, qualification, axis label, legend item, or any information the audience must read to understand the result.
- A Theme may increase the ramp. It must not lower a role below the selected profile floor once enforcement begins.
- Autoscale must not silently turn Body into Technical or Technical into Micro.

The browser gate records computed size, a cap-height proxy, and autoscale factor. A room rehearsal remains required because automated CSS measurements do not include the physical screen and audience distance.

### Use a fixed role ramp

Map size, weight, and leading to roles. Do not invent a new size for every selector.

Short headings benefit from `text-balance`. Paragraphs and captions can use `text-pretty` where browser support is reliable. Long URLs, identifiers, and emails need `overflow-wrap: anywhere` in narrow regions. Code needs deliberate wrapping or line selection rather than indiscriminate word breaking.

Use a controlled reading measure:

- title: normally no more than about `28ch` and two lines;
- body prose: about `32ch` to `48ch` on a projected slide;
- long descriptions and notes: a separate reflowed surface, not the presentation canvas.

These measures are review guides. Equations, code, and tables have their own geometry.

### Choose fonts by rendered behavior

A Theme may use serif, sans serif, or a deliberate pairing. The family label alone does not determine readability.

Check:

- clear `I`, `l`, and `1`, and clear `O` and `0` where technical content needs them;
- open counters and adequate x-height;
- strokes that survive projector washout;
- real regular, medium, semibold, and italic faces rather than browser synthesis;
- a fallback stack with similar metrics;
- math and code coverage;
- stable font loading before static capture.

Decorative or script faces are appropriate for short, large accents. They do not belong in paragraph text, code, tables, axes, or captions. This is especially important for the Wedding Theme: the script is personality, not a body face.

Use sentence case for titles and body text. Reserve uppercase and tracking for brief kickers or labels. Avoid wide tracking in technical headings and never apply it to paragraph text.

### Keep related type together

Place figure annotations beside the feature they explain. Put units with values, definitions with symbols, and a table note with the table. A distant legend or paragraph forces the audience to hold one item in memory while searching for another.

## Spacing and depth tokens

Use one small scale for layout and block rhythm:

| Token | Value | Role |
| --- | ---: | --- |
| `slide-2` | `0.5rem` | label/icon gap, tight table padding |
| `slide-4` | `1rem` | within a text or annotation group |
| `slide-6` | `1.5rem` | ordinary block and region gap |
| `slide-8` | `2rem` | between related groups |
| `slide-12` | `3rem` | title-to-body or major group separation |
| `slide-16` | `4rem` | large composition separation and safe insets |

Prefer logical properties such as `padding-inline`, `margin-inline-start`, and `text-align: start`. This preserves the path toward RTL and vertical-writing support without requiring Theme rewrites.

Use a finite depth ladder:

- `shadow-0`: flat;
- `shadow-1`: a quiet separation for a figure or raised surface;
- `shadow-2`: a prominent modal, media frame, or physical-card Theme treatment.

Most slide blocks should remain at `shadow-0`. Projection reduces subtle shadow detail, and many independent elevations make the slide look like an application dashboard.

## Color and contrast

### Define semantic roles first

Every Theme needs these roles before it adds named aesthetic colors:

- background;
- surface;
- raised surface, if the Theme uses one;
- strong text;
- ordinary text;
- muted text;
- rule;
- accent and accent-on-accent ink;
- code surface and code text;
- success, warning, and danger where interactive content needs them;
- categorical, sequential, and diverging chart palettes.

Use OKLCH while designing and validating perceptual relationships, then emit a dependable browser representation. Theme parameter validation should explicitly support the required CSS Color 4 syntax; accepting arbitrary CSS through a color parameter is not an acceptable substitute.

### Treat WCAG as the floor

For the HTML presentation:

- ordinary text and images of text need at least `4.5:1` contrast;
- WCAG-defined large text needs at least `3:1`;
- meaningful graphics and interactive indicators need at least `3:1` against adjacent colors;
- color cannot be the only carrier of category, state, or emphasis.

Use `7:1` for ordinary body text and `4.5:1` for large text as initial projection targets, subject to room calibration. Projection and ambient light reduce effective contrast, and thin fonts can look fainter than their declared color pair suggests.

Muted text still needs to be readable. Do not put muted gray text on a saturated background. If an accent color cannot carry text, use it for a rule, fill, or large mark and provide a separate readable ink color.

### Make data encodings redundant

Pair categorical color with direct labels, marker shapes, line styles, patterns, or stable spatial grouping. Test grayscale and representative color-vision-deficiency simulations.

Sequential data needs a perceptually ordered scale. Diverging data needs a meaningful center. Rainbow scales are not the default: their uneven lightness can invent boundaries and hide others.

### Protect text over imagery

Text shadows do not make an arbitrary photograph safe. Use a mostly opaque surface, a tested scrim, or a protected text region. The existing conservative background treatment is the right default.

Automated contrast checks can handle solid colors. Image-backed regions require review at every sampled slide and palette variant.

## Content-block rules

### Figures

- Give a figure enough area to be inspected from the back of the room.
- Redraw or crop a paper figure for the talk instead of pasting a multi-panel manuscript figure unchanged.
- Preserve only the panels, labels, and precision needed for this claim.
- Put a short caption or interpretive annotation beside the relevant feature.
- Prefer SVG or another vector representation. Check raster assets at their rendered size.
- Provide useful alternative text. A complex figure also needs a longer description or nearby prose that states the principal relation and implication.
- Alt text does not replace an audible description of meaningful visual content during the live talk.

### Charts

- Choose the geometry for the comparison: common-position and length judgments are usually more precise than angle, area, or volume.
- Use a table for exact lookup and a chart for pattern, trend, or comparison.
- Direct-label series where practical. A distant legend adds search and memory work.
- Highlight the result being discussed and quiet the context without deleting necessary baselines, units, denominators, sample sizes, or uncertainty.
- Label uncertainty as SD, SE, confidence interval, credible interval, model interval, or the actual quantity shown.
- Bar length normally needs a meaningful zero. A line or point plot may use a restricted domain when the choice is explicit and honest.
- Chart labels use at least the Technical type role. A chart library's ten-pixel default is not acceptable on a projected slide.

### Tables

- Use a table when the audience needs exact values or categorical lookup.
- Order rows and columns to support the comparison.
- Use alignment, spacing, light rules, or row surfaces to preserve grouping. Avoid both a border around every cell and an unstructured field of numbers.
- Use tabular numerals for aligned quantitative columns.
- Highlight a row, column, or small set of cells when the speaker discusses them, using more than color alone.
- Split a manuscript-sized table into several views rather than shrinking it.

### Equations and derivations

- State what the equation establishes, not only its name.
- Define symbols next to their first meaningful use.
- Align transformations and preserve stable context between Steps.
- Highlight the changed term with weight, enclosure, or annotation as well as color.
- Use one derivation stage per Step or slide when simultaneous display would force technical text below the floor.

### Code

- Show the smallest excerpt that supports the explanation.
- Keep surrounding context visible but de-emphasized when the active lines need focus.
- Use line numbers only when the speaker or annotation refers to them.
- Avoid semantic line wrapping. Shorten names only in a didactic transcription, crop irrelevant context, or split the example.
- Use the Technical role or larger. If an excerpt does not fit, make it a Detail slide or reveal it in meaningful groups.

### Lists and prose

- Use short, parallel phrases as signposts. Do not put the speaker script on the slide.
- Reveal items only when order or pacing changes understanding.
- Keep enough prior context visible that the audience knows where the new item belongs.
- Break a dense explanatory paragraph into a diagram, annotated example, or several slides when that better matches the explanation.
- Do not ban visible text: key terms, captions, qualifications, second-language support, and poor-audio conditions can make it essential.

### Citations and sources

- Attach credit when the figure or claim first appears.
- Use a consistent compact form on the slide and put full bibliographic detail in notes or a Detail slide.
- Keep sources secondary but readable. Micro is the floor, not a target for every citation.
- Preserve links and selectable text in HTML and PDF where the Output target supports them.

### Empty and fallback states

An empty or failed block should never look like a successfully sparse slide.

- In live authoring, name the missing content or failed dependency and give the author the next useful action.
- In a release build, fail on unresolved charts, missing figures, unreliable media, and unsupported portable content instead of leaving a blank region.
- An intentional visual pause needs an explicit semantic treatment and an accessible title; it is not an accidental empty slide.
- Static media fallbacks should state what they represent. A poster frame with no context is not a complete fallback for an explanatory video.

## Slide patterns

### Unlabelled/default explanation

Use a stable title region and one primary body flow. The body can contain a short explanation, a diagram, or a small set of signposts. Start-align by default.

### Claim

Put the assertion in the title or dominant body statement. Give the supporting figure, equation, comparison, or example most of the body. Avoid a second competing takeaway.

### Figure

Let the figure fill the primary region. Keep the title interpretive and the caption spatially attached. A figure variant should warn when the visual remains a small island inside a large empty body.

### Comparison

Use parallel region structure, shared baselines, and consistent labels. Two columns are the ordinary case. Three or more regions need a real comparison task, not a desire to fit more cards.

### Derivation

Preserve the previous state, reveal one meaningful change, and make the changed relation obvious. Use Steps when the intermediate states are pedagogically important; use Detail slides when the derivation is optional to the Main path.

### Section title

Orient the audience with a short phrase, question, or transition. This variant has the widest Theme freedom after the cover slide. It should remain legible and should not introduce a new ornamental system unrelated to the Theme.

### Dense

Dense is an explicit exception for content whose value depends on simultaneous detail. It is not a general fit switch.

- Prefer it on Detail slides.
- Keep essential content at Technical or larger.
- Record the use in visual review.
- Warn when a Main slide uses it.
- Fail rather than autoscale below the agreed floor.

## Steps, motion, and state

Motion is functional and opt-in.

Use it when it explains a process, transformation, path, or changing spatial relation. Keep it presenter-controlled, pausable where continuous media is involved, and deterministic in static output.

Avoid:

- decorative fly-ins and zooms;
- looping GIFs beside unrelated content;
- parallax or continuous background motion;
- arbitrary transitions between ordinary Main slides;
- hiding context that the audience still needs to interpret the new state.

Every Theme must respect `prefers-reduced-motion`. Reduced motion should remove nonessential translation, scaling, and looping without removing the information or current-state cue.

Communicate state with more than color. The current Step can use a combination of weight, opacity, position, outline, label, or icon. Keyboard focus needs a consistent `focus-visible` ring and offset. Interactive charts and media must remain understandable in forced-colors and increased-contrast modes.

## Output target rules

The **HTML presentation** and **PDF export** are peer Output targets. Print HTML is the internal static rendering surface used to produce the PDF export. PNG or JPEG pages and contact sheets are derived review or sharing artifacts, not peer Output targets.

### HTML presentation

- Preserve the logical composition field.
- Support keyboard navigation, focus visibility, semantic headings, alternative text, and reduced motion.
- Use Steps for meaningful progression.
- Keep the offline bundle self-contained for presentation use.

### Print HTML and PDF export

In zpres, print HTML is the static rendering surface for the PDF export. It is not a document handout.

- Use the same composition tokens and type ramp as the live slide.
- Linearize each Section as its Main slide followed by Detail slides.
- Default to the final coherent Step state; use one page per Step only when the progression needs to survive as pages.
- Disable motion without discarding state.
- Use static fallbacks for media and fail when no reliable fallback exists.
- Preserve selectable text, links, language, and reading order where the browser PDF path allows it. Do not claim tagged-PDF or PDF/UA conformance without testing it.

### Notes and handouts

Notes pages and a future reflowed handout can include longer explanations, complete citations, definitions, and text alternatives. They should have their own layout. The presentation slide should not become self-contained by reducing every element.

### PNG, JPEG, and contact sheets

These derived artifacts support visual review and sharing. They do not replace an accessible HTML presentation or PDF export.

- Use contact sheets to review pacing, repetition, dominant elements, and title continuity.
- Use full-size pages to review type, chart labels, code, and fine contrast.
- Use an actual presentation machine and room-distance test before release.

## Tailwind v4 implementation shape

**Status: Current in Theme API v1.** The renderer foundation is compiled with
pinned Tailwind `4.3.2` and committed as ordinary CSS. Built-in Themes and
`theme init` use v1. Source files and generated presentations have no Tailwind
runtime dependency.

Tailwind is an internal build-time authoring tool. Source files contain no Tailwind classes, and generated presentations load no Tailwind runtime or CDN asset. Built-in styles compile to ordinary CSS and ship in the offline bundle.

A compact foundation can start like this:

```css
@theme static {
  --spacing-slide-2: 0.5rem;
  --spacing-slide-4: 1rem;
  --spacing-slide-6: 1.5rem;
  --spacing-slide-8: 2rem;
  --spacing-slide-12: 3rem;
  --spacing-slide-16: 4rem;

  --text-slide-display: 4.5rem;
  --text-slide-display--line-height: 0.98;
  --text-slide-title: 3rem;
  --text-slide-title--line-height: 1.06;
  --text-slide-heading: 2.25rem;
  --text-slide-heading--line-height: 1.12;
  --text-slide-body: 2rem;
  --text-slide-body--line-height: 1.3;
  --text-slide-technical: 1.5rem;
  --text-slide-technical--line-height: 1.32;
  --text-slide-micro: 1.125rem;
  --text-slide-micro--line-height: 1.25;

  --radius-slide-1: 0.5rem;
  --radius-slide-2: 1rem;
  --shadow-slide-1: 0 1px 2px rgb(0 0 0 / 0.10);
  --shadow-slide-2: 0 12px 32px rgb(0 0 0 / 0.16);

  --container-slide-region-sm: 28rem;
  --container-slide-region-lg: 48rem;
}

@theme inline {
  --color-slide-background: var(--zpres-color-background);
  --color-slide-surface: var(--zpres-color-surface);
  --color-slide-text: var(--zpres-color-text);
  --color-slide-muted: var(--zpres-color-muted);
  --color-slide-accent: var(--zpres-color-accent);
  --font-slide-heading: var(--zpres-font-heading);
  --font-slide-body: var(--zpres-font-body);
  --font-slide-mono: var(--zpres-font-mono);
}
```

One top-level `@theme` block defines the utility and token vocabulary because Tailwind Theme variables must be top-level. Theme scopes set the `--zpres-*` values; a Theme package cannot define a nested `@theme` under its body class.

Use cascade layers so ownership is visible:

1. reset and browser normalization;
2. renderer foundation and semantic geometry;
3. Theme tokens and variant treatment;
4. Deck-local validated overrides;
5. output-target adjustments.

The renderer may use CSS utility classes in generated HTML, but stable `.zpres-*` classes and `data-*` attributes remain the public Theme API.

## Automated quality gate

**Status: Structural and standards gates implemented; room profile calibrating.** The browser-rendered gate navigates every live route and Step, measures visible descendants and declared boundaries, and checks print independently. v1 compares authored image backgrounds with their executed semantic layers and enforces room-independent contrast, interaction, state, and alternative-structure checks. The provisional room profile adds report-only type, enhanced-contrast, and autoscale findings. Its approved-profile path blocks v1 type-floor and hard-autoscale failures, but no profile is approved without physical room evidence.

The criteria below define what a review must catch. Automated coverage is
narrower: the [Theme check reference](theme-authoring.md#browser-rendered-release-review)
describes the measurements and enforced checks. Type and autoscale floors
become blocking only with an approved room profile. Composition, wording, and
image-backed contrast still require human judgment.

### Fail

- clipped content or overlap across a declared forbidden boundary;
- missing assets, fonts, or static fallbacks;
- a missing, hidden, cleared, source-replaced, or off-Slide authored v1 background layer;
- unresolved or inaccessible content markers;
- ordinary text below WCAG contrast minimum;
- meaningful non-text content below the non-text contrast minimum when it can be computed;
- essential text below the agreed role floor;
- a missing meaningful figure alternative;
- an autoscale factor below the hard floor;
- a mismatch between declared aspect and static canvas;
- a Step policy that loses necessary information in static output.

### Warn

- autoscale below the normal range;
- a Main slide using `dense`;
- a title wrapping beyond two lines;
- chart, code, table, or caption text below its preferred role;
- a figure variant whose figure occupies too little of the body;
- a sparse upper-stacked body with a small primary element;
- long prose measure or estimated reading burden;
- color categories without a detected redundant cue;
- image-backed text that needs manual contrast review;
- a Theme parameter that does not change any rendered result;
- an unregistered or dead Theme selector.

The provisional profile flags values below `0.92` and records `0.80` as a candidate hard floor. The hard floor must be accepted or changed in physical calibration before it becomes a failure; it must prevent a slide from reaching the observed `0.62` extreme. Autoscale cannot be a silent substitute for editing.

### Human review

Automation cannot decide whether:

- the title states the right claim;
- the evidence actually supports it;
- a chart domain or omitted baseline misleads;
- decorative treatment competes with the talk;
- the visual sequence has good pacing;
- a figure is understandable from the room;
- the speaker describes visual content adequately.

The release report keeps objective status and human approval separate.

## Theme-authoring workflow

1. Define the audience, room, and expected content shapes.
2. Choose one or two signature visual moves. A Theme needs a recognizable voice, not decoration on every block.
3. Choose heading, body, and mono families and test their actual metrics.
4. Fill the semantic color roles and validate every palette variant.
5. Start with the neutral safe area, type ramp, and spacing scale.
6. Style the default explanation, Claim, Figure, Comparison, Derivation, Section-title, and Dense patterns.
7. Style figures, charts, code, tables, math, media, captions, sources, and Detail indicators.
8. Add interaction states, reduced motion, forced colors, and static fallbacks.
9. Run the wide specimen and a real talk-shaped Source file through live HTML, PDF, page images, and contact-sheet review.
10. Rehearse on the presentation machine at room distance.

## Built-in Theme treatments

The supported portfolio is recorded in
[ADR 0012](adr/0012-retain-the-distinct-v1-theme-portfolio-and-16-9-contract.md).
These descriptions explain each Theme's role. A supported Theme still needs
review with the actual Deck and presentation room.

### Science

Science uses serif-led headings, restrained teal and rust accents, and a
shared screen/print frame. Its compositions cover ordinary explanations and
scientific evidence; see [the Science guide](science-theme-v1.md).

### SV

SV is an adopted Theme API v1 package on the shared scientific/data module. Its
Propagation Workbench treatment uses condensed sans headings, mono evidence
labels, a sparse technical grid, segmented trace-bus accents, squared panels,
and violet/gold data treatment. The ignored local inspiration subtree is not a
product dependency; [ADR 0010](adr/0010-adopt-sv-as-a-supported-theme.md) names
the exact three-file package boundary. SV/light and Dark Splash/cyan remain
complementary light-first and dark-first technical Themes.

### Paper Chalk

Paper Chalk uses a field-notebook grid, condensed headings, a margin line,
and annotation marks. Named annotations indicate judgment or consequence;
ordinary evidence keeps the same grid without added marks.

### Wedding

Wedding uses a stationery surface, botanical ornament, formal serif, and
restrained script accents. Ornament frames the content within the shared
regions. Technical text retains the normal role sizes.

### Dark Splash

Dark Splash uses a dark surface, luminous accents, and bounded regions for
technical evidence. It remains supported alongside SV, providing a dark-first
alternative to SV's light-first treatment.

### Debug

Debug exposes layout regions, Step state, and overflow. Its inspection overlay
adds labels without changing content geometry; see [Debug Theme](debug-theme.md).

## Release checklist

A Theme is ready for a real Deck only when all of these are true:

- The Main path remains coherent without Detail slides.
- Each Main slide has one identifiable communicative job.
- Title, primary evidence, supporting material, sources, and footer have a clear order.
- The role ramp passes the chosen room profile and no essential content uses Micro.
- Spacing comes from the shared scale except for documented artwork geometry.
- Text, graphics, states, and image-backed regions pass contrast review.
- Color categories and interactive states have non-color cues.
- Figures, charts, tables, equations, and code are readable at full size.
- HTML respects keyboard focus, reduced motion, contrast preferences, and semantic reading order.
- PDF uses coherent static states and reliable media fallbacks.
- Authored image backgrounds survive on both Output targets; crop, contrast,
  and semantic intent receive their separate review.
- Contact sheets look paced and varied without moving landmarks arbitrarily.
- Full-size pages have no clipping, accidental autoscale, or tiny islands of content.
- The Theme passes the wide specimen, every palette variant, and the actual talk's Source file.
- The final HTML presentation and PDF export have been rehearsed on the presentation machine.

## Evidence and limits

### Scope

Research on live scientific talks is limited. Much of the useful evidence comes
from educational multimedia, visual perception, accessibility standards, and
controlled slide lessons. Those settings support defaults, but they do not fix
one universal word count, slide rate, font size, or color scheme.

Venue size, projector quality, ambient light, audience knowledge, and visual
acuity can dominate a nominal CSS measurement. zpres therefore combines a
structural browser gate with original-size human review.

### Claims and structure

Garner and Alley found better comprehension for an assertion-evidence redesign
than for conventional topic/subtopic slides. The redesign changed several
properties together, so the result supports the combined pattern more strongly
than any single rule. [Garner and Alley, 2013](https://www.writing.engr.psu.edu/ae_comprehension.pdf)

Alley and colleagues found improved retention with sentence-headline slides,
again in a setting where typography and evidence changed alongside the title
form. [Alley et al., 2006](https://www.writing.engr.psu.edu/alley_et_al_2006.pdf)

zpres uses a claim, explicit question, or interpretive transformation as the
Main-slide default. Covers, Section titles, quotations, and other semantic roles
may use a different title form.

### Capacity, coherence, and segmentation

Reviews of multimedia learning support complementary visuals, spatial
integration, removal of nonessential material, signaling, and meaningful
segmentation. [Castro-Alonso et al., 2021](https://doi.org/10.1007/s10648-021-09606-9),
[Mayer and Moreno, 2003](https://doi.org/10.1207/S15326985EP3801_6)

Written and spoken redundancy is not uniformly harmful. Labels, captions, key
terms, qualifications, and access support can help even when the speaker covers
the same subject. Full duplicate paragraphs usually compete for attention.
[Adesope and Nesbit, 2012](https://doi.org/10.1037/a0026147)

Steps should introduce a process stage, derivation step, comparison state, or
evidence layer. They should retain enough stable context for the audience to
stay oriented. [Rey et al., 2019](https://doi.org/10.1007/s10648-018-9456-4)

### Spatial relations and visual encoding

Corresponding text and visuals are easier to connect when they remain spatially
close. Selective signaling can direct attention, but a field full of competing
signals loses that benefit. [Schroeder and Cenkci, 2018](https://doi.org/10.1007/s10648-018-9435-9),
[Schneider et al., 2018](https://doi.org/10.1016/j.edurev.2017.11.001)

Position on a common scale and length generally support more accurate
quantitative comparison than angle, area, volume, or color saturation.
[Cleveland and McGill, 1984](https://doi.org/10.1080/01621459.1984.10478080),
[Heer and Bostock, 2010](https://doi.org/10.1145/1753326.1753357)

### Accessibility and projection

WCAG contrast criteria provide a useful HTML floor, but projection often needs
more headroom because ambient light and the display surface reduce effective
contrast. [WCAG 2.2 contrast guidance](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)

Computed size alone cannot certify legibility. Font metrics, line length,
weight, local contrast, and viewing distance all affect the result. Room
profiles report measurable conditions; they do not replace a room-scale check.
