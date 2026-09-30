---
title: "SV propagation-native evidence"
author: "zpres"
theme: "sv"
theme_dirs:
  - "../../themes"
footer: "SV evidence grammar · propagation changes the search"
slide_numbers: true
aspect: "16:9"
---

# Propagation makes consequences visible

::: variant section-title
:::

One premise · one selected relation · one visible search consequence

::: notes
This talk-shaped specimen uses constraint-programming evidence rather than
generic scientific cards. The memorable result moment appears once near the
end; the other Slides keep the workbench quiet.
:::

---

# One revision removes five values from the same domain

::: slide
variant: comparison
classes: [sv-evidence-domain-update, sv-causal-trace]
:::

::::: comparison
:::: primary label="BEFORE · D(y)"
::: fit
1  2  3  4  5  6  7
:::

Shared range · 1…7
::::
:::: supporting label="AFTER · D(y)"
::: fit
1                 7
:::

ELIMINATED · 2  3  4  5  6
::::
:::::

> Cause · fixing x = 4 in |x − y| ≥ 3 removes exactly the unsupported values 2–6.

::: notes
Both lanes show the same variable and baseline. The authored causal hook lets
the ornamental transition arrow use violet; the authored quotation names the
cause. Gold and the double/patterned rail mark the eliminated outcome.
:::

---

# One domain event wakes only the stale dependency

::: slide
variant: comparison
classes: [sv-evidence-causal-schedule, sv-evidence-resolved-outcome, sv-causal-trace]
:::

::::: comparison
:::: primary label="BEFORE · FULL SWEEP"
| Tick | Scheduled revision |
| ---: | --- |
| 1 | revise(x → y) |
| 2 | revise(y → z) |
| 3 | revise(x → z) |
| 4 | revisit(x → y) |
::::
:::: supporting label="AFTER · ADVISOR QUEUE"
| Tick | Scheduled revision |
| ---: | --- |
| 1 | **revise(x → y)** |
| 2 | revise(y → z) |

::: fit
2 revisits removed
:::
::::
:::::

> Causal change · the x event selects revise(x → y); cached support suppresses two unrelated revisits.

::: notes
The two schedules share tick rows and a time direction. Violet identifies only
the selected revision because this Slide explicitly opts into sv-causal-trace.
The gold metric is the authored eliminated work.
:::

---

# The premise stays fixed while the revision advances

::: slide
variant: derivation
classes: [sv-state-trace, sv-causal-trace, sv-evidence-causal-trace]
:::

::::: derivation pdf="pages"
:::: context label="QUEUED · STABLE PREMISE"
x = 4 · D(y) = {1, 2, 3, 4, 5, 6, 7} · |x − y| ≥ 3
::::
:::: stage label="ACTIVE · SELECTED RELATION"
```mermaid
flowchart LR
  X[x = 4] -->|revise| R[x→y]
  R -->|support| Y[D(y)]
```
::::
:::: stage label="FIXED · RESOLVED DOMAIN"
| Evidence | Result |
| --- | --- |
| Surviving values | 1, 7 |
| **Eliminated values** | **2, 3, 4, 5, 6** |
::::
:::::

::: notes
The context is unchanged on every Step. ACTIVE shows the selected relation;
FIXED shows its resolved consequence. Print keeps one page per transition.
:::

--

## FAILED belongs to another search node

::: slide
variant: dense
classes: [sv-state-failed]
:::

SEPARATE SNAPSHOT · NOT A CONTINUATION

At another node, x = 4 and D(y) = {4, 5} use the same premise.

| Check | Result |
| --- | --- |
| Supported values | none |
| Domain after revision | ∅ |
| Solver state | contradiction |

FAILED is an alternative terminal snapshot, not the next Derivation Step.

::: notes
The visible band and Detail-slide boundary prevent the contradiction from being
read as a causal successor of the fixed domain above.
:::

---

# The current relation stays native to the workbench

::: slide
variant: figure
classes: [sv-evidence-native, sv-causal-trace]
:::

```mermaid
flowchart LR
  E[x event] -->|queues| R[revise]
  R -->|narrows| D[D(y)]
  D -->|unblocks| B[branch]
```

Path · event → revision → D(y) → branch

::: notes
This inline Diagram is the native path: transparent evidence, Theme-matched
labels and rules, and no imported white field. Every edge belongs to the one
authored selected path; the Diagram surface itself remains neutral.
:::

--

## Detail: the advisor trace stays inspectable

::: slide
variant: dense
classes: [sv-evidence-inspectable-trace]
:::

```text reveal="2|3"
domain_event(x)
enqueue(revise_x_to_y)
commit_domain(y, {1, 7})
```

Pacing note · Reveals explain the code; they do not author another solver state.

---

# The imported benchmark keeps its paper field on purpose

::: slide
variant: figure
classes: [sv-evidence-imported]
:::

::: figure src="assets/sv-imported-search-profile.svg" alt="Search nodes over four sizes: dark-ochre squares mark the lower resolved after-propagation outcome; circles mark before." caption="Takeaway · the resolved after-propagation outcome uses fewer search nodes; paper field retained." fit="contain" radius="0"
:::

::: notes
The Theme labels and frames this as imported evidence. Dark keeps the white
paper surface but never presents it as an unexplained island. The static
resolved-outcome series uses dark ochre squares, not current-state violet.
:::

---

# Propagation leaves forty-two times fewer search nodes

::: slide
variant: claim
classes: [sv-evidence-claim, sv-evidence-search-consequence]
:::

::: fit
2,688 → 64 nodes
:::

::: notes
This is the single signature result moment. One assertion and one before/after
metric carry the Slide; there is no paragraph or list stack competing below it.
:::
