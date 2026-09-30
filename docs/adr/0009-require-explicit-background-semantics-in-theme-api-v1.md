# Require explicit background semantics in Theme API v1

A background image can be ornament, context, or evidence. One optional `alt`
string cannot distinguish those cases: an empty value may mean deliberate
decoration or a missing alternative, while a non-empty value says nothing
about whether the image needs a longer scientific explanation.

Theme API v1 therefore requires every background to declare `decorative`,
`contextual`, or `evidence` intent. Decorative backgrounds reject alternative
text and remain silent. Contextual backgrounds require a short alternative.
Evidence backgrounds require both the short alternative and a longer
description. The short form is one paragraph of at most 160 characters; this
is a project authoring boundary, not a claim that length establishes quality.

The painted background remains renderer-owned visual structure. A meaningful
background also receives a visually hidden semantic Figure in the Slide's
reading order. The Figure image carries the short alternative, and its caption
carries the longer description. The painted layer stays `aria-hidden`, so the
same image is not announced twice. This representation applies independently
to screen and print HTML.

The browser gate compares declared and rendered semantics. Missing intent,
missing required text, decoration with contradictory text, or a discarded
semantic Figure blocks a v1 Theme. The report records alternative presence,
crop-review status, and image-backed contrast review. It does not claim to
judge wording quality, an arbitrary crop, photographic contrast, or PDF/UA
conformance.

Imported Marpit
background comments have no channel for the v1 semantic contract, so they must
be rewritten in a native zpres background form before a v1 Theme accepts them.
