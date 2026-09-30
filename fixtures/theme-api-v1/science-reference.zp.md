---
title: "Science Theme API v1 reference"
theme: "science"
theme_dirs:
  - "../../themes"
aspect: "16:9"
footer: "Science v1 · reference"
slide_numbers: true
build_lists: true
theme_params:
  mode: "light"
  type_scale: 1
---

# A scientific argument has shape

::: variant section-title
:::

Claims, evidence, and caveats should remain visible as distinct roles.

---

# Ordinary explanation

The renderer supplies the geometry; Science supplies a calm editorial voice.[^contract]

- One readable measure
- One restrained accent
- One stable footer lane

[^contract]: Sources remain separate from the primary argument.

^ Check the source lane and the final list state.

---

# Constraint propagation changes the claim

::: variant claim
:::

The useful result is the invariant, not the amount of text used to derive it.

---

# Evidence should occupy the field

::: variant figure
:::

::: figure src="assets/science-orbit.svg" alt="Three nested feasible regions connected by a search trajectory" caption="A local vector figure expands through the available evidence region." fit="contain" radius="12"
:::

---

# Stronger propagation changes the search

::::: comparison
:::: primary label="Baseline"
::: fit
42 nodes
:::

Chronological branching

> [!NOTE] Qualification
> Same instance and stopping rule.
::::

:::: supporting label="Candidate"
::: fit
11 nodes
:::

Impact-guided branching

> [!NOTE] Qualification
> Same instance and stopping rule.
::::
:::::

---

# Compare evidence on shared guides

::::: comparison
:::: primary label="Before propagation"
::: figure src="assets/science-orbit.svg" alt="Baseline feasible regions marked with a circular comparison cue" caption="Larger feasible region" fit="contain" radius="10"
:::
::::

:::: supporting label="After propagation"
::: figure src="assets/science-orbit.svg" alt="Reduced feasible regions marked with a square comparison cue" caption="Smaller feasible core" fit="contain" radius="10"
:::
::::
:::::

---

# Narrow comparison keeps readable roles

![bg left:55% intent="evidence" alt="Feasible regions" description="Nested orbit-like regions show the feasible area narrowing from the baseline to the candidate state."](assets/science-orbit.svg)

::::: comparison
:::: primary label="Baseline"
42 nodes
::::

:::: supporting label="Candidate"
11 nodes
::::
:::::

---

# Main result and supporting detail

The Main slide carries the argument used in the talk.

--

## Detail: boundary case

The Detail slide is visually subordinate but remains a complete Slide.

---

# Final coherent Step state

::: steps
1. Establish the model.
2. Isolate the invariant.
3. State the consequence.
:::

---

# Derive the preserved invariant

::::: derivation
:::: context label="Assumption"
For every \(t\), the current state satisfies \(x_t \in D\).
::::
:::: stage label="Apply transition"
$$
x_{t+1}=f(x_t,u_t)
$$
::::
:::: stage label="Conclude"
The transition preserves \(D\), so \(x_{t+1} \in D\).
::::
:::::

---

# Refine the experiment in stages

::::: derivation pdf="pages"
:::: context label="Fixed protocol"
Use the same instance set, stopping rule, and hardware.
::::
:::: stage label="Measure"
Record the baseline search tree.
::::
:::: stage label="Change"
Replace only the propagation rule.
::::
:::: stage label="Compare"
Report runtime and node count together.
::::
:::::

---

# Keep the Main path focused on the result

The exact solver trace is available as compact Detail evidence.

--

## Detail: Technical evidence remains readable

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

$$
z^\star = \min_{x \in D} c^\mathsf{T}x
$$

--

## Detail: Evidence keeps its caption

::: variant dense
:::

::: figure src="assets/science-orbit.svg" alt="Nested feasible regions connected by a search trajectory" caption="The feasible core remains legible beside its compact explanation." fit="contain" radius="10"
:::
