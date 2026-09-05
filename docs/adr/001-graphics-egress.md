# ADR-001: M01 graphics egress experiment

Status: provisional experiment; P0 remains open

M01 exercises native Kitty egress through three synthetic fixtures: colliding
red/blue child IDs, a scaled checkerboard cropped against pane edges and an
overlay, and a native-size checkerboard with fragments starting inside cells.
The scene module clips rational rectangles against pane/client/visibility
bounds and opaque overlays. Attachment-local allocation gives each derived
fragment its own outer image identity. The writer serializes complete
transactions and drains repeated short-write budgets without truncation.

The native fixture preserves exact pixel dimensions by omitting `c/r` and using
uppercase `X/Y` offsets within 8×16 pixel cells. Lowercase `x/y` are source crop
coordinates, not destination offsets. This follows the pinned Kitty protocol's
“Controlling displayed image layout” section in `docs/sources.json`. The fixture
uses a 26×18 checkerboard at local (-1,-1), 24×16 pane bounds, and an overlay at
(9,3) of size 6×9. Four fragments per pane exercise both horizontal and vertical
partial-cell origins. Fixed RGB row data represents only this fixture's crops;
it is not a general codec or production raster adapter.

The independent Python host rejects output crossing pane boundaries and checks
every canvas pixel against the original source-coordinate formula. It never
clips incorrect output on the producer's behalf. Its nearest-neighbor expansion
is an explicit synthetic assumption, not a claim about Kitty filtering. Native
one-to-one placement avoids that scaling assumption. Full reconstruction and
short-write output agree for all three fixtures.

Fixed fixture payloads were chosen instead of a general encoder/resampler:
these known pixels need neither a new runtime dependency nor a custom codec.
A real image path will need a maintained pinned decoder/resampler and bounded
resource ownership before arbitrary input is accepted.

Real Kitty remains a separate gate. Neither the fake host nor the red-pixel
precursor proves P0. Final native-versus-placeholder selection awaits calibrated
real-host evidence, fractional resampling, RGBA/text layering, and
resize-during-upload reconstruction. No runtime ingress, PTY, or general
renderer support is claimed by this decision.
