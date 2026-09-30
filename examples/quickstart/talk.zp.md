---
title: "A small zpres Deck"
author: "Example author"
theme: "science"
aspect: "16:9"
footer: "zpres quickstart"
slide_numbers: true
---

# A small zpres Deck

::: variant section-title
:::

Three slides are enough to exercise the authoring loop.

^ Introduce the Source file and explain that HTML and PDF come from the same Deck.

---

# One slide, one claim

::: variant claim
:::

The Source should keep the Main path visible while you edit.

::: steps
1. Write the claim.
2. Add the evidence.
3. Move optional detail off the Main path.
:::

--

## Detail: commands used while authoring

::: variant dense
:::

```sh
zpres check talk.zp.md --strict
zpres serve talk.zp.md
zpres export talk.zp.md --pdf talk.pdf
```

---

# Publish both Output targets

::::: comparison
:::: primary label="Live HTML"
Use navigation, Steps, and speaker notes while presenting.
::::

:::: supporting label="Static PDF"
Keep a portable fallback with the same authored argument.
::::
:::::
