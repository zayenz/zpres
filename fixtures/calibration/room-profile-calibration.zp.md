---
title: "Room-profile calibration"
theme: "debug"
theme_dirs:
  - "../../themes"
aspect: "16:9"
footer: "Room-profile calibration · controlled fixture"
slide_numbers: true
autoscale: false
---

# Room-profile calibration

::: variant section-title
:::

Use this controlled Deck at the delivered venue resolution. Record physical
readability and contrast decisions in the room observation sheet; do not treat
the renderer measurements as room approval.

---

# Type-role ramp

::: html
<div style="display:grid;gap:10px;background:#ffffff;color:#111111;padding:16px">
  <div data-zpres-type-role="display" style="font-size:72px;line-height:1">Display · 72px</div>
  <div data-zpres-type-role="title" style="font-size:48px;line-height:1.05">Title · 48px</div>
  <div data-zpres-type-role="heading" style="font-size:36px;line-height:1.1">Heading · 36px</div>
  <div data-zpres-type-role="body" style="font-size:32px;line-height:1.2">Body · 32px · essential explanatory prose</div>
  <div data-zpres-type-role="technical" style="font:24px/1.2 Menlo,monospace">Technical · 24px · x[i] ≤ upper_bound</div>
  <div data-zpres-type-role="micro" style="font-size:18px;line-height:1.2">Micro · 18px · nonessential source metadata only</div>
</div>
:::

---

# Body-size bracket

::: html
<div style="display:grid;gap:16px;background:#ffffff;color:#111111;padding:20px">
  <p data-zpres-type-role="body" style="margin:0;font-size:36px;line-height:1.2">36px — Constraint propagation removes values that have no support.</p>
  <p data-zpres-type-role="body" style="margin:0;font-size:32px;line-height:1.2">32px — Constraint propagation removes values that have no support.</p>
  <p data-zpres-type-role="body" style="margin:0;font-size:28px;line-height:1.2">28px — Constraint propagation removes values that have no support.</p>
</div>
:::

Judge the smallest comfortable sample from the farthest likely viewing position.

---

# Technical-size bracket

::: html
<div style="display:grid;gap:18px;background:#ffffff;color:#111111;padding:20px;font-family:Menlo,monospace">
  <div data-zpres-type-role="technical" style="font-size:28px;line-height:1.2">28px · ∀i ∈ V: x[i] ≤ upper_bound[i]</div>
  <div data-zpres-type-role="technical" style="font-size:24px;line-height:1.2">24px · ∀i ∈ V: x[i] ≤ upper_bound[i]</div>
  <div data-zpres-type-role="technical" style="font-size:20px;line-height:1.2">20px · ∀i ∈ V: x[i] ≤ upper_bound[i]</div>
</div>
:::

Judge symbols, subscripts, punctuation, and similar glyphs—not only words.

---

# Micro-size bracket

::: html
<div style="display:grid;gap:20px;background:#ffffff;color:#111111;padding:20px">
  <div data-zpres-type-role="micro" style="font-size:22px;line-height:1.2">22px · Source: Example et al. (2026), §4.2</div>
  <div data-zpres-type-role="micro" style="font-size:18px;line-height:1.2">18px · Source: Example et al. (2026), §4.2</div>
  <div data-zpres-type-role="micro" style="font-size:16px;line-height:1.2">16px · Source: Example et al. (2026), §4.2</div>
</div>
:::

Micro is nonessential. A result, qualification, axis label, or legend may not be
reclassified as Micro to make it fit.

---

# Ordinary contrast bracket

::: html
<div style="display:grid;grid-template-columns:1fr 1fr;gap:18px">
  <div style="background:#ffffff;color:#595959;padding:24px">
    <p data-zpres-type-role="body" style="margin:0;font-size:32px;line-height:1.2">Candidate projection contrast near 7:1</p>
  </div>
  <div style="background:#ffffff;color:#767676;padding:24px">
    <p data-zpres-type-role="body" style="margin:0;font-size:32px;line-height:1.2">Lower ordinary contrast near 4.5:1</p>
  </div>
</div>
:::

The report computes solid-color contrast from the rendered foreground and
nearest opaque background. The room reviewer decides whether either sample has
enough projected headroom.

---

# Large-text contrast bracket

::: html
<div style="display:grid;grid-template-columns:1fr 1fr;gap:18px">
  <div style="background:#ffffff;color:#767676;padding:24px">
    <p data-zpres-type-role="heading" style="margin:0;font-size:36px;line-height:1.1">Candidate large-text contrast near 4.5:1</p>
  </div>
  <div style="background:#ffffff;color:#949494;padding:24px">
    <p data-zpres-type-role="heading" style="margin:0;font-size:36px;line-height:1.1">Lower large-text contrast near 3:1</p>
  </div>
</div>
:::

---

# Evidence-occupancy bracket

::: html
<div style="display:grid;grid-template-columns:1fr 1fr;gap:24px;align-items:start">
  <figure data-zpres-content-role="evidence" style="box-sizing:border-box;width:100%;height:260px;margin:0;padding:20px;background:#dbeafe;border:4px solid #2563eb;display:grid;place-items:center">
    <div data-zpres-type-role="technical" style="font-size:24px">Larger evidence surface</div>
  </figure>
  <figure data-zpres-content-role="evidence" style="box-sizing:border-box;width:55%;height:145px;margin:0;padding:12px;background:#ffedd5;border:4px solid #ea580c;display:grid;place-items:center">
    <div data-zpres-type-role="technical" style="font-size:24px">Smaller evidence surface</div>
  </figure>
</div>
:::

The report records element and aggregate bounding-box occupancy. It does not
decide whether the evidence is informative, appropriately cropped, or readable.

---

# Autoscale warning candidate

[.autoscale: true]

1. Autoscale warning-boundary stimulus.
2. The content is deliberately repetitive.
3. Judge the measured factor, not this label.
4. Check symbols: `x[i] <= upper_bound[i]`.
5. Check prose at the farthest seat.
6. Check the live HTML Output target.
7. Check the PDF Output target.
8. Record delivered resolution.
9. Record ambient-light conditions.
10. Record the viewing position.

---

# Autoscale hard-floor candidate

[.autoscale: true]

1. Autoscale hard-floor stimulus.
2. The content is deliberately repetitive.
3. Judge the measured factor, not this label.
4. Check symbols: `x[i] <= upper_bound[i]`.
5. Check prose at the farthest seat.
6. Check the live HTML Output target.
7. Check the PDF Output target.
8. Record delivered resolution.
9. Record ambient-light conditions.
10. Record the viewing position.
11. Record the connection path.

---

# Calibration decision

For each candidate, record:

- delivered resolution and connection path;
- viewing position and ambient-light condition;
- smallest comfortable Body, Technical, and Micro sample;
- ordinary and large-text contrast judgment;
- measured autoscale factors and physical readability;
- reviewer, date, and accept/change decision.

The profile remains provisional until those physical observations are complete.
