# Charts, math, and code

Write equations, charts, and code directly in the Source file where possible.
Equations use KaTeX-compatible LaTeX, charts use a supported subset of
Vega-Lite JSON,
and code uses fenced blocks. Generate more complex plots in your usual tools
and include the resulting SVG or image as a [Figure](figure-authoring.md).

## Math

Use `\(...\)` for inline math and `$$...$$` for display math:

```markdown
The state satisfies \(x_t \in D\).

$$
x_{t+1} = f(x_t, u_t)
$$
```

KaTeX resolves the math before static capture. Unsupported commands fail
readiness checks. For a multi-stage argument, use a
[Derivation](text-authoring.md#derivation-pattern) to keep the invariant visible
while revealing each transformation.

## Charts with local data

The reliable static chart path supports line charts with `x` and `y` field
encodings and local CSV or JSON data. It is a limited renderer, not a complete
Vega-Lite implementation. Other marks and inline-only data do not pass static
readiness checks.

Save this as `data/runtime.csv` beside the Source file:

```csv
size,runtime_ms,model
10,4,baseline
20,9,baseline
30,16,baseline
10,3,candidate
20,5,candidate
30,8,candidate
```

Then add a Chart block to a slide:

```markdown
# The candidate uses less time as the model grows

::: vega-lite
{
  "data": { "url": "data/runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "size", "type": "quantitative" },
    "y": { "field": "runtime_ms", "type": "quantitative" },
    "color": { "field": "model", "type": "nominal" }
  }
}
:::
```

The live server watches the data file, and HTML builds copy it with the Deck's
other dependencies. zpres checks that the file exists and contains data before
static export. Inspect the rendered chart to confirm labels, units, ordering,
and the intended comparison. Passing validation does not establish that every
Vega-Lite property has an effect.

zpres does not execute Python, R, Julia, or other authoring scripts. Run those
separately when preparing data or figures.

## Code and tables

Use ordinary fenced code with a language label:

````markdown
```rust
let next = variables.iter().min_by_key(|v| v.domain_size());
```
````

Keep excerpts short enough to read at presentation size. Code preserves its
line structure, so a long line may overflow. Move supporting code to a Detail
slide or split the excerpt instead of relying on smaller type.

Use a Markdown table when exact values matter:

```markdown
| Model | Nodes | Runtime |
| --- | ---: | ---: |
| Baseline | 42 | 18.4 s |
| **Candidate** | **11** | **6.2 s** |
```

Numeric columns can be right-aligned. Under Theme API v1, a fully bold cell
marks its row for emphasis using a rule and underline as well as color. The
[Dense Detail pattern](text-authoring.md#dense-technical-detail-slides) combines
code, tables, and equations for material outside the Main path.
