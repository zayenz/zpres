---
title: "SV Detail-state boundaries"
author: "zpres"
theme: "sv"
theme_dirs:
  - "../../themes"
footer: "SV state grammar · Detail boundaries"
slide_numbers: true
aspect: "16:9"
---

# Detail slides keep the authored rail

::: variant section-title
:::

Queued, active, fixed, failed, and trace Details use the same four-row state grammar as Main slides.

---

# A queued Detail is opt-in

The Main slide remains neutral.

--

## Queued Detail

::: slide
variant: claim
classes: [sv-state-queued]
:::

[fit] This authored revision is waiting.

---

# An active Detail is opt-in

The Main slide remains neutral.

--

## Active Detail

::: slide
variant: claim
classes: [sv-state-active]
:::

[fit] This authored relation is current.

---

# A fixed Detail is opt-in

The Main slide remains neutral.

--

## Fixed Detail

::: slide
variant: claim
classes: [sv-state-fixed]
:::

[fit] This authored revision is resolved.

---

# A failed Detail is a separate snapshot

The Main slide remains neutral.

--

## Failed Detail

::: slide
variant: dense
classes: [sv-state-failed]
:::

SEPARATE SNAPSHOT · NOT A CONTINUATION

This authored snapshot contains a contradiction.

---

# A trace Detail advances one real relation

The Main slide remains neutral.

--

## Trace Detail

::: slide
variant: derivation
classes: [sv-state-trace, sv-causal-trace]
:::

::::: derivation pdf="pages"
:::: context label="QUEUED · domain event"
The domain event schedules one revision.
::::
:::: stage label="ACTIVE · causal selection"
Apply the propagator to the selected relation.
::::
:::: stage label="FIXED · bound update"
The smaller domain is stable for this revision.
::::
:::::
