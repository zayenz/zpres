# Media authoring

Video and audio play in the HTML presentation. Static exports use a poster or
media card, so give the audience enough text to understand what the clip
contributes without playing it. Use media directives or the Markdown shorthand
below to get local asset checks, bundling, and live reload.

For quick authoring, Markdown image syntax can also create typed media:

```markdown
![video right 50% fill loop mute autoadvance poster="media/demo-poster.png" title="Solver demo"](media/demo.mp4?t=30s "A short solver trace")
![audio hide](media/ambient.mp3)
![audio title="Narration"](media/narration.mp3 "Optional narration")
![iframe poster="media/demo-poster.png" title="Interactive demo"](https://example.com/demo "Interactive demo fallback")
![youtube poster="media/talk-poster.png" title="Conference talk"](https://youtu.be/VIDEO_ID?t=45s "Conference talk clip")
```

The first label word can be `video`, `audio`, `iframe`, `youtube`, or `vimeo`.
Common video and audio file extensions such as `.mp4`, `.webm`, `.mov`, `.mp3`,
`.wav`, and `.ogg` are also inferred when no kind is given. YouTube and Vimeo
watch URLs are inferred as iframe media and normalized to embeddable player URLs.
The Markdown title becomes the caption, while `title`, `alt`, `poster`,
`controls`, `autoplay`, and `autoadvance` can be provided as label attributes.

Media accepts the same compact visual modifiers as figures for common slide
composition: `left`, `right`, `center`, `fit`, `fill`, and bare percentage
sizes such as `50%`. Use `hide` when the media should remain in the live deck
without reserving visible slide space.

Theme API v1 requires visible media to have a short alternative and adjacent
static explanatory text. Set `alt` explicitly, or let zpres use the caption or
title as the alternative. A caption, directive body, or title supplies the
explanatory text. Video and iframe media still require a local poster.
Hidden media may only be decorative:
if it has an alternative, title, or caption, v1 rejects it because that
information would disappear from PDF, PNG, and JPEG Output targets. Keep
meaningful narration visible as an audio card, or write its contribution into
the Slide content.

## Video

```markdown
::: video src="media/demo.mp4?t=1m30s" poster="media/demo-poster.png" title="Solver demo" loop=true muted=true autoadvance=true
A short solver trace used as a live presentation demo.
:::
```

The `poster` image is required for reliable PDF export. HTML presentations use
the native browser video element.

## Audio

```markdown
::: audio src="media/narration.mp3" title="Narration" start="45"
Optional narration for the worked example.
:::

::: audio src="media/ambient.mp3" hide=true
:::
```

Audio is interactive in HTML presentations. PDF export renders a static media
card and emits a warning so the author knows the audio itself is not embedded.
Hidden audio remains a live-deck element but is omitted visually from static
exports. Theme API v1 therefore accepts hidden audio only when it is
decorative and has no alternative, title, or caption.

## Start offsets

Video and audio can start partway through the file. Use Deckset-style `?t=`
offsets in the media path, or use `start` on a directive:

```markdown
![video poster="media/demo-poster.png" title="Solver trace"](media/demo.mp4?t=90s)

::: audio src="media/narration.mp3" title="Worked example narration" start="1m30s"
:::
```

Offsets may be plain seconds (`45`), seconds with a suffix (`90s`), minutes and
seconds (`1m30s`), or hours/minutes/seconds (`1h2m3s`). zpres stores the real
local asset path separately from the playback offset, so dependency checks,
HTML bundling, live reload, and PDF fallback checks still use `media/demo.mp4`
or `media/narration.mp3`.

## Playback progression

Use `autoadvance` when a live video or audio clip should advance to the next
step or section as soon as playback ends:

```markdown
![video autoplay mute autoadvance poster="media/demo-poster.png" title="Solver trace"](media/demo.mp4)

::: audio src="media/narration.mp3" autoplay=true autoadvance=true hide=true
:::
```

This is a live HTML behavior. Static PDF/PNG/JPEG exports keep the same visual
fallbacks and page order they would use without `autoadvance`.

## Layout

Use layout modifiers when a clip or audio card should share the slide with
text:

```markdown
![video right 50% fill poster="media/demo-poster.png" title="Solver trace"](media/demo.mp4)

::: video src="media/demo.mp4" poster="media/demo-poster.png" title="Solver trace" width="62%" fit="cover" align="center"
:::
```

`left` and `right` map to theme alignment hooks. A bare percentage such as
`50%` maps to media width. `fit` uses contain-style fitting; `fill` uses
fill-style fitting. Use explicit `width`, `height`, `fit`, and `align`
attributes when the shorthand would be unclear. `hide` maps to
`data-media-hidden="true"` for themes and static export policy.

## Iframes

Replace `VIDEO_ID` and the example Vimeo number with the clip you want to use.

```markdown
::: iframe src="https://example.com/demo" title="Interactive demo" poster="media/demo-poster.png"
Interactive demo fallback.
:::

::: youtube src="https://www.youtube.com/watch?v=VIDEO_ID&t=45s" title="Conference talk" poster="media/talk-poster.png"
Conference talk clip.
:::

::: vimeo src="https://vimeo.com/123456" title="Demo clip" poster="media/demo-poster.png"
Demo clip.
:::

![youtube poster="media/talk-poster.png" title="Conference talk"](https://www.youtube.com/watch?v=VIDEO_ID&t=45s "Conference talk clip")

![vimeo poster="media/demo-poster.png" title="Demo clip"](https://vimeo.com/123456 "Demo clip")
```

Remote iframes are allowed for HTML presentations. A local `poster` image is
required for reliable PDF export. YouTube watch, short, embed, and `youtu.be`
URLs normalize to `https://www.youtube.com/embed/...`; Vimeo page and player
URLs normalize to `https://player.vimeo.com/video/...`. Use either the explicit
`::: youtube` / `::: vimeo` directive form or the compact Markdown shorthand.
YouTube `?t=` and `start=` offsets become embed `start` offsets in live HTML.

## Generic form

```markdown
::: media kind="video" src="media/demo.mp4" poster="media/demo-poster.png"
Caption text.
:::
```

Common attributes:

- `src`: required media URL or local path.
- `poster`: local fallback image for video and iframe PDF export.
- `title`: accessible title for embeds and static PDF cards.
- `caption`: explicit caption. If omitted, directive body text becomes caption.
- `alt`: poster alt text. If omitted, zpres falls back to caption or title.
- `start`: video/audio start offset, using seconds, `90s`, `1m30s`, or
  `1h2m3s`.
- `loop`: `false` by default; set to `true` or use bare `loop` in Markdown
  media shorthand.
- `muted`: `false` by default; set `muted=true`, `mute=true`, or use bare
  `mute`/`muted` in Markdown media shorthand.
- `autoadvance`: `false` by default; set `autoadvance=true` or use bare
  `autoadvance` in Markdown media shorthand to advance the live deck when the
  media element ends.
- `hide`: `false` by default; set `hide=true` or use bare `hide` in Markdown
  media shorthand to keep media in the live deck without a visible slide box.
- `width`: CSS size token such as `50%`, `640px`, or `80vw`.
- `height`: CSS size token such as `42vh`, `360px`, or `auto`.
- `fit`: `contain`, `cover`, or `fill`.
- `align`: `start`, `center`, `end`, or `stretch`.
- `controls`: `true` by default for video and audio.
- `autoplay`: `false` by default.

Local `src` and `poster` paths are resolved relative to the deck root, copied
into HTML bundles, watched by the live server, and checked before build/export
output is written.
