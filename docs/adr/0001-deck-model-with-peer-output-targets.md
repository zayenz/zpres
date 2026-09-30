# Use the deck model as the canonical representation

zpres will treat the deck model, not the generated HTML presentation, as the canonical representation of a deck. The HTML presentation remains the preferred live-presenting output because it can support richer navigation, interaction, and animation, but PDF export is a peer output target that must be good enough to present from when a venue only accepts PDF. This avoids baking Chromium print output into the architecture while still allowing a browser-based PDF renderer if it proves reliable enough.
