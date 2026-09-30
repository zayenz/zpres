---
title: "How to check for overlapping intervals"
author: "Example Deck"
theme: "science"
aspect: "16:9"
footer: "Overlapping intervals"
slide_numbers: true
build_lists: true
---

# How to check for overlapping intervals

::: variant section-title
:::

The easiest overlap test starts by asking when overlap is impossible.[^source]

[^source]: Adapted from Zayenz, “How to check for overlapping intervals,” zayenz.se, 2025.

^ Give the short answer first: describe separation, negate it, and simplify.

---

# Use half-open intervals

For this talk, an interval is written as

$$
[s,e) \quad\text{with}\quad s < e.
$$

The start belongs to the interval. The end does not.

::::: comparison
:::: primary label="Contains 3"
$$
[1,4)
$$
::::

:::: supporting label="Does not contain 4"
$$
[1,4)
$$
::::
:::::

^ Half-open intervals make adjacent ranges meet at a boundary without overlapping.

---

# Listing overlap cases gets awkward

::::: comparison
:::: primary label="Endpoints cross"
- \(A\) starts first
- \(B\) starts first
::::

:::: supporting label="One contains the other"
- \(A\) contains \(B\)
- \(B\) contains \(A\)
::::
:::::

Four valid cases are already too many for a memorable test.

---

# Separation has only two cases

::: variant claim
:::

Two intervals do not overlap when one ends before the other starts.

::: steps
1. A is before B: A.end ≤ B.start
2. B is before A: B.end ≤ A.start
:::

^ Pause on the reduction from four overlap shapes to two separation directions.

---

# Negate separation

::::: derivation
:::: context label="No overlap"
$$
(a_e \le b_s) \lor (b_e \le a_s)
$$
::::

:::: stage label="Negate"
$$
\neg\big((a_e \le b_s) \lor (b_e \le a_s)\big)
$$
::::

:::: stage label="Apply De Morgan"
$$
(b_s < a_e) \land (a_s < b_e)
$$
::::
:::::

---

# The implementation follows the derivation

```python
@dataclass
class Interval:
    start: int
    end: int

    def overlaps(self, other: "Interval") -> bool:
        return (
            other.start < self.end
            and self.start < other.end
        )
```

The two comparisons say that each interval starts before the other one ends.

---

# Boundary cases follow from the notation

::::: comparison
:::: primary label="Adjacent"
$$
[1,4) \quad [4,7)
$$

$$
4 \le 4 \quad\Longrightarrow\quad \text{no overlap}
$$
::::

:::: supporting label="Shared range"
$$
[1,5) \quad [4,7)
$$

$$
4 < 5 \land 1 < 7 \quad\Longrightarrow\quad \text{overlap}
$$
::::
:::::

--

## Detail: closed intervals change the boundary

For closed intervals \([s,e]\), touching at one endpoint counts as overlap.
The corresponding comparisons use \(\le\) rather than \(<\).

---

# Boxes repeat the test on each axis

Two axis-aligned boxes overlap when their horizontal intervals overlap and
their vertical intervals overlap.

$$
\begin{aligned}
b_l &< a_r &\land\quad a_l &< b_r \\
b_b &< a_t &\land\quad a_b &< b_t
\end{aligned}
$$

The interval argument avoids sixteen geometric cases.

---

# Check the complement first

::: variant claim
:::

When a Boolean property has many positive cases, its negation may have a much
smaller case analysis.

For interval overlap:

$$
\boxed{\;b_s < a_e \;\land\; a_s < b_e\;}
$$

^ End on the method, not only the formula: characterize failure, negate, then simplify.
