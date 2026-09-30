---
title: "Typed Overlay and Aside reference"
theme: "reference"
theme_dirs:
  - "../../themes"
aspect: "16:9"
footer: "Theme API v1 · typed Overlay and Aside"
slide_numbers: true
---

# Annotated evidence

::::: overlay overlap="edge-only"
:::: base name="Search geometry"
::: figure src="assets/science-orbit.svg" alt="Nested feasible regions and a search trajectory" caption="Annotations stay inside the evidence region and outside its caption." fit="contain" radius="10"
:::
::::

:::: annotation name="Fixed point" anchor="top-end" width="compact"
The final point lies in the feasible core.
::::
:::::

---

# Two restrained annotations

::::: overlay overlap="edge-only"
:::: base name="Runtime evidence"
::: vega-lite data="data/columns-runtime.csv"
{
  "mark": "line",
  "encoding": {
    "x": { "field": "n" },
    "y": { "field": "ms" }
  }
}
:::
::::

:::: annotation name="Start" anchor="top-start" width="compact"
Baseline
::::

:::: annotation name="Result" anchor="bottom-end" width="compact"
Measured growth
::::
:::::

---

# Primary evidence with subordinate context

::::: aside supporting="standard" gap="6" align="start"
:::: primary name="Argument"
The primary region keeps the dominant share of the canvas.

::: steps
1. Establish the invariant.
2. Apply the propagator.
3. State the consequence.
:::
::::

:::: supporting name="Context"
> [!NOTE] Scope
> Supporting material remains visible without competing with the argument.
::::
:::::

[^roles]: Aside preserves primary-before-supporting reading order on every Output target.

---

# Narrow Aside becomes linear

![bg left:55% intent="evidence" alt="Feasible regions" description="Nested orbit-like regions show the feasible area narrowing from the baseline to the candidate state."](assets/science-orbit.svg)

::::: aside supporting="compact" gap="4" align="start"
:::: primary name="Primary"
The argument remains first.
::::

:::: supporting name="Context"
The narrow `body` container stacks this context below it.[^roles]
::::
:::::
