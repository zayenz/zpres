# Require browser-rendered release gates for production themes

`zpres theme check` remains the fast contract check for a Theme package. It verifies manifests, parameters, templates, declared hooks, fixture coverage, and static readiness without requiring Chromium. Passing that check means that zpres can understand the Theme. It does not mean that the Theme is ready for a talk.

A Theme used for a real presentation must also pass a browser-rendered release gate. The gate renders the actual Source file and the wide-sweep fixture as an HTML presentation and as static pages, waits for fonts, images, charts, and autoscaling, and produces page images plus a contact sheet. It records clipping, unresolved content, content outside the safe area, and other objective geometry failures. A human still reviews composition, hierarchy, legibility, and whether different Slide variants look intentionally different.

The first production matrix is `dark-splash`, `paper-chalk`, and `wedding`, with `sv` as a candidate replacement for `dark-splash`. Each Theme has an independent release result. A passing result for one Theme says nothing about the others, and a canonical fixture pass cannot substitute for rendering the real Deck.

Generated review artifacts may stay outside version control, but the command, fixture, result summary, and reviewer decision must be reproducible. The Chromium-free Theme check and browser-rendered release gate must report contract, objective surface, and human-review states separately so they are not confused again.

The implemented state model is `contract_status`, aggregate `visual_status`, `screen_status`, `print_status`, `human_review`, and `release_status`. A successful run reports `contract_status = valid`, `screen_status = passed`, and `print_status = passed`, but still produces `human_review = required` and `release_status = pending-review`, never automatic presentation approval. The gate retains `print.html`, waits for its browser-owned readiness promise, and captures every page from that same executed document. PDF, PNG, and JPEG export use the same observation path before publishing artifacts.

the release gate also traverses the offline HTML presentation through the production navigation runtime. It records `screen_status` and `print_status` separately, visits every Main slide, Detail slide, generated slide, and Step, and retains full-size screen-state captures beside the print pages. Route identity, actual rendered Slide visibility, descendant and text geometry, canvas/footer containment, Step visibility, assets, and document scroll are objective checks. Autoscaling and composition measurements remain review signals until their thresholds are calibrated. This expands the gate; it does not replace the human full-size review required above.

visual report schema 3 also records the v1 Deck-derived
expectation and browser-observed semantic layer for each authored image
background. A v1 screen state or print page fails when the required direct
layer is missing, hidden, outside the Slide, cleared by CSS, or no longer
contains the authored source. This is structural survival evidence, not
approval of crop, focal content, image-backed contrast, alternative text, or
composition.

visual report schema 4 also records Chrome's actual platform
faces and glyph counts for visible text on every observed screen state and
print page. Per-observation bridge attributes prevent Theme CSS from targeting
the measurement hook. The report does not infer fallback cause, missing-glyph
cause, or synthesized weight/style where CDP supplies no such evidence; those
fields remain explicitly unavailable unless typed Theme expectations can support them.
