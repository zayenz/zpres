---
title: "Science evidence-compositions specimen"
theme: "science"
theme_dirs:
  - ".."
aspect: "16:9"
footer: "Science · evidence compositions"
slide_numbers: true
---

# Comparable evidence belongs on one measure

::::: comparison
:::: primary label="Unscreened search"
::: fit
1.00×
:::

Median normalized search effort

240 protein-design instances · identical stopping rule
::::

:::: supporting label="Propagation first"
::: fit
0.31×
:::

Median normalized search effort

240 protein-design instances · identical stopping rule
::::
:::::

[^benchmark]: Synthetic benchmark protocol adapted from a constraint-propagation ablation: paired instances, fixed hardware, and a 600 s cutoff.

---

# The invariant stays fixed

::::: derivation
:::: context label="Invariant context"
The feasible set Dₜ and incumbent z⋆ remain fixed through every transformation.
::::
:::: stage label="Screen"
$$
D_t \longrightarrow \widetilde D_t
$$
::::
:::: stage label="Bound"
$$
L(\widetilde D_t) \le z^\star
$$
::::
:::: stage label="Branch"
$$
B_{t+1}=\operatorname{argmin}_{x\in\widetilde D_t}|D(x)|
$$
::::
:::::

---

# Screening removes work before search begins

The paired benchmark falls from 18.4 s to 6.2 s median runtime while preserving the optimum on all 240 instances.

The exact protocol and solver trace remain attached as Detail evidence.

--

## Detail: the conclusion stays separate from exact evidence

::: variant dense
:::

Conclusion — propagation removes 69% of normalized search effort without changing the optimum.

| Protocol | Nodes | Runtime | Optimum |
| --- | ---: | ---: | ---: |
| Unscreened | 42,180 | 18.4 s | −137.6 |
| **Propagation first** | **13,076** | **6.2 s** | **−137.6** |

```rust
let d = propagate(domain);
assert!(same_optimum(d));
```

$$
\Delta N / N_0 = (13\,076 - 42\,180) / 42\,180 = -0.690
$$

---

# Carry the accepted path into the next experiment

::: variant claim
:::

Propagation earns its place before branching.

1. Screen unsupported pairs.
2. Bound the surviving domain.
3. Branch only on accepted evidence.

The rust boundary stays visible: the result is established for the paired benchmark, not yet for every protein-design family.
