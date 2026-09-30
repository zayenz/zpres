# Start with Chromium PDF rendering behind an abstraction

zpres will start with Chromium as the first PDF renderer because it can share much of the HTML and theme rendering path and should get the project to a usable authoring loop faster. PDF is still a peer output target, not a print fallback from HTML, so Chromium must sit behind a replaceable renderer abstraction and be checked against PDF quality fixtures. If Chromium cannot produce presentation-quality PDFs for the fixture decks, zpres should pivot before depending on it too deeply.
