---
title: "Patterns become predictions"
author: "zpres"
theme: "blueprint"
footer: "Blueprint · illustrative research"
slide_numbers: true
aspect: "16:9"
---

# Patterns become predictions

::: variant section-title
:::

A new language for scientific discovery.

::: notes
The values in this Deck are illustrative, not experimental findings.
:::

---

# Structure makes the difference

::: variant claim
:::

42% fewer steps on the same task.

The representation changes the work; the question stays the same.

---

# Compare work, not just time

::::: comparison
:::: primary label="Structured"
58 search steps

- reuse known relations
- revisit changed state
::::
:::: supporting label="Baseline"
100 search steps

- recompute each relation
- scan every candidate
::::
:::::

---

# A smaller search preserves the result

::: variant figure
:::

::: figure src="evidence.svg" alt="A baseline tree explores seven nodes; the structured tree explores four nodes while retaining the same result." caption="Pruning avoids redundant branches without changing the feasible result." fit="contain"
:::

---

# More structure, less repeated work

::: vega-lite
{"data":{"url":"runtime.csv"},"mark":"line","encoding":{"x":{"field":"structure","type":"quantitative"},"y":{"field":"steps","type":"quantitative"},"color":{"field":"method","type":"nominal"}}}
:::

--

## Exact counts make the claim auditable

::: variant dense
:::

| Method | Steps | Relative work |
| --- | ---: | ---: |
| Baseline | 100 | 1.00 |
| **Structured** | **58** | **0.58** |

$$
1 - \frac{58}{100} = 0.42
$$

---

# Update only what changed

::: variant derivation
:::

```text reveal="2|3|4"
queue ← changed variables
take the next variable
update affected relations
enqueue newly changed variables
```

---

# Keep the result and its boundary

::: variant claim
:::

Structure can remove redundant work.

Measure maintenance cost before choosing the representation.[^scope]

[^scope]: Illustrative counts; confirm against your own workload.
