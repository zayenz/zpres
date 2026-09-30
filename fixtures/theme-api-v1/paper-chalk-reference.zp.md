---
title: "Paper Chalk Theme API v1 reference"
author: "zpres"
theme: "paper-chalk"
theme_dirs:
  - "../../themes"
footer: "Paper Chalk v1 · benchmark notebook"
slide_numbers: true
aspect: "16:9"
---

# Scaling controlled benchmarks

::: variant section-title
:::

Record the experiment, show the evidence, and keep the technical workings legible.

::: notes
The ruled field, rose margin, and evidence labels should survive on both screen and print surfaces.
:::

---

# One notebook, three kinds of evidence

The construction combines generated instances, solver traces, and controlled comparisons.

- Each family changes one structural parameter.
- Every result retains its generation provenance.
- Detail slides hold the audit trail.

---

# Comparison needs controlled families

::: variant claim
:::

[fit] A useful benchmark changes one structural property at a time.

---

# Difficulty is not just board size

::: variant claim
:::

[fit] Input size predicts cost only after workload shape and solver settings are held fixed.

::: columns widths="1/1" gap="4" align="start"
Generation:

- nested board families
- fixed clue policies

Measurement:

- identical solver settings
- complete trace provenance
:::

---

# A longer claim steps down deliberately

::: variant claim
:::

[fit] Benchmark conclusions are credible only when instance generation, solver configuration, and the mapping from raw traces to reported measurements remain inspectable together.

---

# Two sampling strategies expose different bias

::::: comparison
:::: primary label="Independent samples"
Broad coverage

- simple confidence intervals
- weak local comparability
::::
:::: supporting label="Paired families"
Controlled change

- shared construction history
- stronger causal reading
::::
:::::

---

# Build each family in stages

::: variant derivation
:::

```text reveal="3|4|5-6"
choose a base workload B
select a scaling factor k
lift B onto the k-input
apply the sampling policy C
validate the instance
record generator and seed
```

---

# The construction remains visible

::: variant figure
:::

::: figure src="assets/science-orbit.svg" alt="Nested feasible regions used as a stand-in for a generated family" caption="A Technical caption records what changed between members of the family." fit="contain" radius="2"
:::

---

# Runtime follows the generated scale

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/columns-runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "n", "type": "quantitative" },
    "y": { "field": "ms", "type": "quantitative" }
  }
}
:::

--

## Detail: the evidence ledger

::: variant dense
:::

| Family | Instances | Median nodes | Runtime |
| --- | ---: | ---: | ---: |
| Baseline | 80 | 1,284 | 2.8 s |
| **Paired scale** | **80** | **3,911** | **8.7 s** |
| Sparse clues | 80 | 12,604 | 31.2 s |

$$
H(F_k) = \operatorname{median}_{x \in F_k} N(x)
$$
