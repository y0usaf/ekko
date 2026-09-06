# Independent analysis retained with recovery attempt

The comparator's workflow stages explicitly settle each key and require the
actual Zellij red failed-split signal before capturing the approximately
300-ms `failed-split-flash` checkpoint. A timeout cannot substitute for that
signal. The attempted run stopped before the first fixture readiness event,
so it provides no functional or pixel parity claim.

The existing paired visual evidence shows that the settled Escape checkpoint
has equal pyte cursor coordinates while 290 cells differ. Sampled cells differ
in chrome attributes (Zellij bold `eeeeee` on black versus Ekko default
attributes). The apparent cursor pixel style mismatch from the lost workflow
10 capture remains unresolved: matching pyte coordinates does not establish
matching cursor color or raster style, and the relevant PNG is unavailable.
Startup also has cursor inequality because the release screen is still present
on one side.

Titles have a separate policy difference. The Zellij reference can derive an
initial title from the command representation and displays the generated
`Pane #4` fallback, while Ekko's current profile decoration path uses the pane
label/display label and falls back to a basename-oriented label. This changes
title text and therefore glyph pixels even when pane geometry and pyte cursor
state match. The profile and default decoration code were not modified during
recovery.
