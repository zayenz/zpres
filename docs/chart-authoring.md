# Chart blocks

zpres renders a small, explicit subset of Vega-Lite JSON as offline SVG on both
HTML and print surfaces. It does not run the Vega-Lite runtime. Unsupported
options produce a Source diagnostic instead of being silently ignored.

```text
::: vega-lite
{
  "data": { "url": "data/runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "size", "type": "quantitative", "title": "Problem size" },
    "y": { "field": "ms", "type": "quantitative", "title": "Runtime (ms)" },
    "color": { "field": "method", "type": "nominal", "title": "Method" }
  }
}
:::
```

The directive's `data="data/runtime.csv"` attribute can supply or override the
local data path. CSV and JSON paths are relative to the Deck root.

## Supported specification

- `mark` is `"line"` or an object containing `"type": "line"` and optionally
  `"point": true`. Lines include point marks and connect each series in ascending
  numeric x order.
- `encoding.x` and `encoding.y` require non-empty `field` names. Their optional
  `type` must be `"quantitative"`.
- `encoding.color` optionally selects a categorical series field. Its optional
  `type` must be `"nominal"`.
- `x`, `y`, and `color` accept string `title` values for the axis or legend label.
  Without a title, the field name is used.
- `encoding.yError` and `encoding.yError2` optionally select **absolute lower and
  upper bounds**, respectively, following the existing zpres convention. Both
  fields must be present; their optional type is `"quantitative"`. These are not
  offsets from y. State what the interval means in nearby slide text.
- `$schema`, `description`, `name`, and `usermeta` are accepted as metadata. A
  `$schema` URL does not enable additional Vega-Lite features.

Axes use linear scales over the observed numeric extent, including uncertainty
bounds on y. Constant values receive a one-unit margin. The renderer chooses
series colors and point treatment. It does not support authored scales or
domains, transforms, aggregates, binning, stacking, sorting, axis configuration,
additional encodings, layered specifications, or other mark types. Use an
externally prepared Figure when the chart needs those features.

Theme API v1 labels both numeric axes. For a narrow range far from zero, it
shows an additive offset above the y scale or in parentheses beside the x axis
title. Add that signed offset to each tick value to recover the original value:
with `+100000003`, a tick of `2.5` means `100000005.5`. Ordinary and scientific
notation are selected automatically; the offset does not change the data or
scale.

## Data validation

CSV supports quoted delimiters, escaped quotes, and multiline fields. Headers
must be non-empty and unique, and each record must have the same number of
fields as the header. Leading and trailing field whitespace is trimmed.

JSON data must be a non-empty array of objects. Numeric columns may contain
numbers or numeric strings. Series values may be strings, numbers, or booleans.
Field names select literal top-level columns; nested JSON field expressions are
not supported.

Every row must contain finite numeric x/y values and, when requested, both
uncertainty bounds. Bounds must be ordered. Numeric ranges that cannot be
represented reliably by the renderer are rejected. Missing or malformed
selected fields are errors; rows are never silently dropped. Diagnostics identify
the data file, data-row number, and field where applicable. Data-row numbers
exclude the CSV header and count a multiline record once.

Source checking, HTML publication, and static-export readiness use the same
validator. Static readiness and publication recheck the current data files,
including when a caller parsed the Deck before those files changed.
