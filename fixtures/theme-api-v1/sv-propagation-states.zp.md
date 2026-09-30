---
title: "SV semantic propagation states"
author: "zpres"
theme: "sv"
theme_dirs:
  - "../../themes"
footer: "SV state grammar · one authored bound update"
slide_numbers: true
aspect: "16:9"
---

# One bound update has a visible state

::: variant section-title
:::

QUEUED → ACTIVE → FIXED is one authored causal trace. FAILED remains an alternative terminal snapshot.

::: notes
This specimen demonstrates the opt-in SV state grammar. Slide order is not
treated as a solver trace unless the Source explicitly selects the state hooks.
:::

---

# A domain event enters one queue

::: slide
variant: claim
classes: [sv-state-queued]
:::

[fit] Revising x makes the relation x → y stale.

- queue entry: revise(x → y)
- current domain: D(y) = {1, 2, 3, 4, 5, 6, 7}

::: notes
QUEUED is an authored state, not a default inferred from this slide's position.
The outline diamond and dashed line remain visible without colour.
:::

---

# The propagator selects one causal relation

::: slide
variant: claim
classes: [sv-state-active, sv-causal-trace]
:::

[fit] With x = 4, revise only x → y against |x − y| ≥ 3.

- the filled triangle and solid line mark current work
- unrelated queue entries remain neutral

::: notes
Violet is reserved for the relation being processed now. It is not generic
trim, a section colour, or a synonym for importance.
:::

---

# State survives without colour

::::: comparison
:::: primary label="Stable label"
QUEUED · ACTIVE · FIXED · FAILED

- direct state words
- one stable order
::::
:::: supporting label="Redundant cue"
◇ outline + dash · ▶ fill + solid · ■ fill + double · × crossing + dots

- shape and fill
- line style and position
::::
:::::

::: notes
The two lanes describe one encoding grammar. They are not before and after
solver snapshots, so neither lane receives violet or gold.
:::

---

# One bound update removes five values

::: slide
variant: derivation
classes: [sv-state-trace, sv-causal-trace]
:::

::::: derivation pdf="pages"
:::: context label="QUEUED · domain event"
x = 4; D(y) = {1, 2, 3, 4, 5, 6, 7}; enqueue revise(x → y).
::::
:::: stage label="ACTIVE · causal selection"
Apply |x − y| ≥ 3; values 2–6 lose support.
::::
:::: stage label="FIXED · bound update"
| Domain state | D(y) |
| --- | --- |
| Before | 1 2 3 4 5 6 7 |
| **Eliminated** | **2 3 4 5 6** |
| After | 1 7 |
::::
:::::

::: notes
The Step sequence is intentionally causal: queued domain event, current
revision, then the resolved smaller domain. The print Output target uses one
page per authored transition so ACTIVE and FIXED both remain explicit.
:::

---

# Unannotated evidence stays neutral

::: variant figure
:::

::: figure src="assets/science-orbit.svg" alt="Neutral nested feasible regions with a trajectory, shown without an authored SV propagation-state hook" caption="A Figure does not become current or resolved merely because it appears later in the Deck." fit="contain" radius="4"
:::

::: notes
The Source has not said that this Figure is the current causal relation or a
resolved outcome. Its Theme furniture therefore remains neutral.
:::

---

# Generic Steps do not invent a solver trace

```text reveal="2|3"
pair-check snapshot
forward-bound snapshot
matching-bound snapshot
```

Each line names an independent solver snapshot; reveal order is presentation pacing only.

::: notes
This slide covers generic code reveal without a state class. The Theme must not
turn its Step order into QUEUED, ACTIVE, FIXED, or FAILED, and must not spend
violet or gold on that invented relation.
:::

---

# The queue closes on a smaller domain

::: slide
variant: claim
classes: [sv-state-fixed]
:::

[fit] D(y) = {1, 7} is stable for this revision.

- five values are eliminated
- no stale copy of x → y remains

::: notes
Gold marks the resolved outcome. The filled square and double line preserve
the meaning in grayscale and on the static Output target.
:::

--

## FAILED is a separate snapshot—not the next Step

::: slide
variant: dense
classes: [sv-state-failed]
:::

SEPARATE SNAPSHOT · NOT A CONTINUATION

At another search node, x = 4 and D(y) = {4, 5} use the same rule.

| Check | Result |
| --- | --- |
| Supported values | none |
| Domain after revision | ∅ |
| Solver state | contradiction |

The dotted line and × marker mean failure; gold marks the contradiction.

::: notes
This Detail slide is deliberately an alternative solver snapshot. It does not
follow causally from the FIXED Main slide, and the visible band says so on both
screen and print.
:::
