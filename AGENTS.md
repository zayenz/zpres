# Agent Notes

## Repository Context

zpres is a Rust presentation builder for source-driven talks on any subject. Its core workflow is:

1. Parse a `.zp.md` source file into a typed deck model.
2. Render the peer HTML presentation and PDF export Output targets. Use print HTML as the static rendering surface, and derive PNG/JPEG page sets and contact sheets for review or sharing.
3. Keep Theme behavior explicit through Theme packages, manifests, Theme parameters, API versions, and documented rendering hooks.

All top-level CLI commands in the README are implemented. Built-in Themes use
Theme API 1, the sole supported Theme contract.
Treat `docs/current-system.md` as the implementation map and the ADRs as the
record of durable architectural decisions.

Use the project vocabulary in [CONTEXT.md](CONTEXT.md). In particular, prefer terms such as **Deck**, **Section**, **Main slide**, **Detail slide**, **Source file**, **Theme**, **Theme parameter**, **Slide variant**, and **Output target**.

## Development Commands

Use the normal Rust checks before handing work back:

```sh
cargo fmt --check
cargo test
```

For theme work, also run the relevant theme check. For example:

```sh
cargo run -- theme check themes/wedding --write-specimen dist/wedding-theme-specimen --all-variants
```

## Working Guidelines

- Read the surrounding code and existing docs before changing behavior.
- Keep theme changes scoped to the theme package unless renderer behavior must change.
- Treat print and screen output as separate surfaces; a CSS change can fix one and break the other.
- Verify rendered output visually when working on layout, themes, PDF export, PNG export, or contact sheets.
- Do not rely on CSS inspection alone for visual regressions.
- For visual work, render fresh screen and print screenshots to the acting
  agent's own artifact path and inspect the generated images at original size
  before reading automated visual reports or declaring the work acceptable.
- Check text clearance from rules, borders, ornaments, panel edges, and adjacent
  registers. A label that sits on or visually touches a rule is a failure even
  when automated containment passes.
- Check clearance on every side, including text beside vertical rails. If a
  label remains inside a rule or ornament's immediate visual field instead of
  having at least one clear small-scale spacing step, score spacing/clearance
  and ornament integration no higher than 2, even without pixel overlap.
- Apply the same immediate-field rule to inline markers, bullets, badges, cue
  shapes, and other ornaments before or after text. A marker that touches or
  visually fuses with the first or last glyph caps spacing/clearance and
  ornament integration at 2 even when its bounding box does not overlap text.
- Score every representative and boundary-risk page from 1–5 for hierarchy,
  spacing and clearance, use of vertical space, typography and readability,
  evidence dominance, ornament integration, and projector legibility. Record
  separate screen and print scores when the surfaces differ. Any category below
  4 requires repair; do not average away a failing category.
- Implementation agents, the primary reviewer, and independent verification
  agents must each inspect the generated images and report their own scorecard.
  For an independent verifier, screenshot generation, inspection, and scoring
  are the first substantive task, before automated reports or prior conclusions.
- Use contact sheets to judge Deck rhythm, but never as a substitute for
  original-size inspection of representative and boundary-risk pages.
- Before Theme, Layout, renderer-composition, or visual-gate work, read the
  presentation design contract, including its evidence and limits.
- Preserve user or generated changes already present in the worktree unless explicitly asked to revert them.

## Useful Docs

- [docs/presentation-design.md](docs/presentation-design.md): theme-neutral presentation design contract, role tokens, slide patterns, and release criteria.
- [docs/theme-authoring.md](docs/theme-authoring.md): theme package structure and theme contract.
- [docs/figure-authoring.md](docs/figure-authoring.md): figure block behavior.
- [docs/background-authoring.md](docs/background-authoring.md): slide background behavior.
