# Unicode pane-title width investigation

This note records the current public-profile gap against the pinned Zellij
source at `/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source` (0.43.1). The investigation below preceded implementation; the current result is recorded
in [Unicode evidence](../evidence/zellij/unicode-titles/README.md). The profile
now uses public `display-width`; its previous ASCII rejection described below
is historical.

## Observed in the current profile

`examples/profiles/zellij-frames.lisp:59-74` defines the title width used by
the frame helper, but `zellij-frame-char-width` raises an error for every code
point above ASCII `~` (`:59-70`). Consequently a pane title supplied by rename,
OSC 0/2, or the title fallback cannot reach the frame renderer if it contains a
wide character such as `界` or a combining sequence such as `e` followed by
U+0301. The file documents this as an explicit ASCII contract at `:6-19`.

The profile's title fitting path (`:98-122`) already has the same shape as
Zellij's middle truncation: measure the decorated title, retain equal-width
prefix and suffix portions, and select `[..]` versus `[...]`. Its scroll
indicator fitting (`:124-139`) uses Lisp character counts; the current strings
are ASCII, so this is equivalent for the existing indicator data.

The terminal core already has a reusable cell-width operation in
`src/vt.lisp:136-138`. It returns zero for combining/format categories and two
for East Asian wide/full-width characters. The command copy path calls that
operation through an internal package reference at `src/commands.lisp:600-605`;
that is evidence of reuse, but not a public renderer API.

## Observed in pinned Zellij source

`zellij-server/src/ui/pane_boundaries_frame.rs:10` imports both
`UnicodeWidthChar` and `UnicodeWidthStr` from the `unicode_width` crate.
`render_title_left_side` at `:410-458` measures the complete decorated title
with `full_text.width()`, computes half budgets from the display width of
`[..]`, and accepts prefix/suffix characters only when
`first_part.width() + char.width().unwrap_or(0)` stays within each budget
(`:419-437`). The final short/long marker decision again uses display widths
(`:439-458`). Thus a two-cell character consumes two columns and a combining
character contributes zero to these budgets; the source does not use byte
length or Rust `String::len()` for title fitting.

The title-line assembly advances its cursor by the returned measured lengths
(`:461-509`, and the related two/one-sided functions at `:511-610`). This is
why an adapter that merely accepts Unicode but continues to count Lisp
characters would misplace the frame boundary around wide titles.

The same source has a separate, currently ASCII-only detail: scroll and pin
indications use `.chars().count()` (`:205-247`) while focus indications use
`.width()` (`:250-376`). This does not affect the standard numeric scroll text,
but it is a reason to keep the reusable width primitive available to all frame
decoration fitting code rather than adding a title-only special case.

## Smallest reusable vertical slice

First compare the existing VT width rules with the pinned `unicode_width`
crate and inspect decoration clipping and client rendering. A public pure
measurement function may suffice: start/end fitting already belongs to the
ordinary profile. Adding a second generic fitting API is not yet justified.
The current VT scalar function alone does not establish equivalence with
Zellij's string-width operation, particularly for emoji sequences, variation
selectors, and Unicode-version differences.

Preserve the reference's exact scalar iteration and repeated string-width
measurements when porting truncation. Do not introduce grapheme-preserving
truncation without reference evidence: the reverse suffix loop can select
combining marks independently of their preceding base. Title selection and
metadata already use public APIs; width behavior and the complete decoration
rendering path still need validation before selecting the smallest mechanism.
This is a proposed investigation sequence, not an established parity fix.

## Tests for the next slice

Extend `tests/zellij-frames.lisp` beside the existing title and truncation
checks (around `run-zellij-frame-tests` and its current 20x8 assertions):

* assert ordinary ASCII width remains unchanged;
* assert `"界"` consumes two cells and the sequence `"e"` + U+0301 consumes one;
* exercise mixed wide/combining titles at budgets that would split a wide
  scalar or leave a combining mark at an edge;
* assert exact Zellij marker and frame-line strings at both a full-width and a
  truncating budget; and
* retain rejection of controls and invalid control-containing titles.

The live pane-title check should add one renamed or OSC title containing a
wide scalar and one combining sequence after the profile primitive exists, so
the metadata-to-rendered-frame path is covered in addition to the pure helper.

The exact expected strings should be generated from the pinned algorithm and
cell widths, not from host character counts. Whether a combining mark is
rendered in the preceding cell is an observed VT behavior to verify in the
live test; the source evidence here establishes its width contribution and
truncation budget, not every terminal emulator rendering detail.

## Width dependency check (bounded follow-up)

The Zellij workspace dependency is declared as `unicode-width = { version =
"0.1.8", default-features = false }` in the pinned root `Cargo.toml:91` and
resolves to 0.1.10 in `Cargo.lock:4701-4707`. The 0.1.10 crate source was
inspected from its crates.io archive. Its `UnicodeWidthStr::width` implementation
is scalar additive (`src/lib.rs:123-126`): it maps each character through the
width table and treats `None` as zero. Its `UnicodeWidthChar::width` contract
uses one cell for ambiguous characters and has a separate `width_cjk` method
(`src/lib.rs:70-95`). Therefore the pinned version does not establish a
grapheme-aware or emoji-ZWJ-specific string-width rule.

The crate's own vectors provide useful oracle cases in `src/tests.rs:110-188`:
full-width `ｈ` is two cells; U+2081 is one cell in ordinary mode and two in
CJK mode; U+0300 is zero; U+00A1 is one ordinary/two CJK; and emoji scalars
such as U+1F469 are two. The test also records `"👩‍🔬"` as four cells, which is
the additive result (2 + zero-width U+200D + 2), rather than a two-cell ZWJ
ligature. These are source-observed vectors, not guesses about a terminal
font.

Ekko's `src/vt.lisp:136-138` agrees with these broad scalar cases (combining
categories zero, East Asian W/F two, and default one), but its table/version
and handling of control categories are not proven identical to the crate.
The profile already rejects controls, so that difference should remain an
explicit validation boundary. A direct SBCL comparison should use the crate's
test vectors plus generated Unicode scalar samples, with the expected values
checked against the pinned 0.1.10 table; it should not use a newer
`unicode-width` release as the oracle.

The least risky implementation path is consequently a public scalar width
operation that reproduces the pinned ordinary (`width`, not `width_cjk`) table
and the existing profile's control rejection. Keep the profile's current
per-scalar start/end loops until exact vectors pass. Do not add grapheme or
ZWJ grouping: the pinned 0.1.10 source supplies no such behavior. A tiny
standalone Rust oracle compiled against the archived 0.1.10 crate can emit
`codepoint,width` and string totals for fixtures; checked-in Lisp tests can
then use those fixed vectors without a runtime Zellij process.

The checked-in table is regenerated with:

```
python3 scripts/generate-text-width.py /path/to/unicode-width-0.1.10 \
  --rustc /path/to/rustc --output src/text-width.lisp
```

The generator verifies the SHA-256 of `src/lib.rs`, `src/tables.rs`, and
`LICENSE-MIT` in addition to the archive hash recorded above. The Nix check
regenerates into a temporary file, compares it with the checked-in table, and
then compares all scalar outputs with the Rust oracle.

## Implemented boundary

The pure shared CL implementation now reproduces all scalar widths against an
exhaustive pinned Rust oracle. Frame titles retain combining scalars; reference
frame ANSI bypasses `Grid::add_character`, whose zero-width drop applies to
application output. Twenty per-key CJK-font screenshot checkpoints match.
That does not establish emoji shaping or application-content combining parity.
Batched DEL remains a separate observed mismatch; see the evidence and
[batch investigation](rename-batched-input.md).
