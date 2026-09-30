---
title: "Typed Grid and Stack reference"
theme: "reference"
theme_dirs:
  - "../../themes"
aspect: "16:9"
footer: "Theme API v1 · typed Grid and Stack"
slide_numbers: true
---

# Evidence arranged by role

::::: grid tracks="2/1/1" gap="4" align="start"
:::: cell name="Primary evidence" column="1" span="2"
::: figure src="assets/science-orbit.svg" alt="Nested feasible regions and a search trajectory" caption="A primary cell spans two of the three declared tracks." fit="contain" radius="10"
:::
::::

:::: cell name="Checks" column="3" row="1"
> [!NOTE] Grid contract
> Checked tracks and spans preserve source reading order.
::::
:::::

---

# Stacked reasoning sequence

::::: stack gap="3" align="stretch"
:::: item name="Model"
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

:::: item name="Conclusion"
The vertical sequence stays explicit in screen and static output.
::::
:::::

---

# Progression inside a Stack item

::::: stack gap="4" align="stretch"
:::: item name="Invariant"
Every Output target sees the same typed child blocks and dependencies.[^layout]
::::

:::: item name="Argument"
::: steps
1. Parse each region through the normal Content-block pipeline.
2. Preserve source order while applying checked visual placement.
3. Collapse to the final state for static output.
:::
::::
:::::

[^layout]: Visual placement never changes the authored reading order.
