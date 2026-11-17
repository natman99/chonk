# Chonk — feature roadmap (idea evaluation)

User ideas first (all preserved, overlaps unified), then agent-proposed ideas,
then build order with dependencies. Legend: **[U]** = user idea, **[A]** = agent idea.

## Cluster 1 — Degradation / "deep fry" engine
The signature feature. A *non-destructive filter chain* per layer, previewed
live at reduced resolution, baked at full res on export.
- [U] Authentic fry: real JPEG encode/decode chain (multi-stage, quality
  ramp, chroma subsampling, upscale-downscale rounds). The image crate's
  baseline JPEG codec is authentic enough; chain it like a camera + WhatsApp
  + repost pipeline would. Deterministic seed so a fry is reproducible.
- [U] Artifacts: banding, mosquito noise, color bleed — fall out of the
  JPEG chain for free; add explicit artifact brushes (scanlines, chromatic
  aberration, VHS wobble) as presets on top.
- [U] Blurs: box/gaussian/motion; already have blur — add directional +
  "upscale blur" (the repost smear).
- [U] Social-media watermarks: NOT a filter — an asset pack of vector/raster
  overlay stickers (Instagram/TikTok/Twitter/X/Tumblr-style), draggable
  layers with the platform's typical placement/opacity. Legal note: parody
  style, not exact logo copies.
- [U] Efficient live filter preview: filter chain runs on the worker at
  preview scale (already the architecture); bake at 1.0 scale only for
  exports. Filter chain stored per layer = "effects stack" panel later.
- Dependency: filter chains are layer properties → do after Cluster 2
  (undo + serialization make chains editable/safe).

## Cluster 2 — Document foundations (boring, load-bearing, do FIRST)
- [U] First-class undo/redo. Command pattern over Document, but cheaper:
      Document snapshots with image data behind Rc (Pixmap clone is the only
      expensive part; Rc it). History panel + Ctrl+Z/Ctrl+Shift+Z.
- [U] .chonk save format: serde (ron or compressed JSON + zip for embedded
      images). Compact = PNG-encode embedded images inside the container.
      Text-only diffability (ron) makes templates reviewable/shareable.
- [U] Templates = .chonk files with placeholder text slots. "Save as
      template" + "new from template" + open-template-file. Zero extra format.
- Template registry (approved design):
  - Slots promoted from layers: TEXT slots (anchor region + fit rules +
    locked/tweakable style, named) and IMAGE slots (placeholder layer with
    target rect + cover/contain policy). Non-slot layers locked.
  - Slots are LIVE: auto-fit re-runs per keystroke inside the anchor;
    contrast sampling re-picks outline/fill against the actual image slot
    content; placement hints ("stay in this band, dodge faces").
  - LAYOUT-ONLY templates are the shareable unit (anchors + styles + fry
    chain, image slot empty) — legally clean, tiny, the layout IS the craft.
    Ship a starter set of layout-only classics.
  - Registry = watched folder (~/.local/share/chonk/templates) + index with
    name/tags/auto-thumbnail. Sharing = send the file. A GIT REPO of
    layout-only templates is a first-class registry source (git pull =
    registry update; text templates diff in review). No server, no account.
  - Per-slot STYLE PRESETS: cycle a slot's look (impact / clean /
    handwritten / ...) with a keypress — templates as instruments.
  - Template creation is a mode: mark slot/lock per layer, name slots, drag
    anchors; "strip images" for layout-only export.
  - Batch mode: template + folder = image slot auto-filled, re-fit +
    re-sampled per image (quote cards = the quote template, batched).
  - v1 scope (this sprint): .chonk save/load + undo/redo. Slot metadata +
    picker + registry later; auto-fit already exists to hang off them.
- [A] Autosave journal + crash recovery (same machinery as undo).
- [A] Clipboard paste (Ctrl+V) for images — Wayland primary selection.
- Everything else regresses without undo; everything else wants .chonk.

## Cluster 3 — Layer model completion
- [U] Per-layer blend mode (tiny-skia has full Porter-Duff + separable
  blend modes natively — cheap win, big aesthetic range: Multiply for
  watermarks, Screen for glows, Difference for fry).
- [U] Per-layer transform/scale/opacity — exist; add rotation UI (model
  has rotation? verify) and flip.
- [A] Layer groups/folders (needed once effects stacks + bubbles multiply).
- [A] Per-layer mask (paint-to-hide) — pairs with cutouts (Cluster 6).
- [A] Snap guides + alignment (edges/centers) — meme layouts are centered;
  make "correct" the default, not the user's job.

## Cluster 4 — Smart placement engine (the "pop" cluster)
One analysis pass per image, several consumers. All on the worker.
- [U] Face + eye detection: pure Rust first (e.g. `frankenstein`/Viola-Jones
  OpenCV-style cascade port, or `face-detection` crate); ONNX (ort +
  ultra-light YOLO-face/UltraFace ~250KB) if pure-Rust quality disappoints.
- [U] Busy/high-contrast area detection: saliency proxy = local variance +
  edge density (Sobel) on a downsampled grid. Pure Rust, ~100 lines.
- [U] Low-contrast text zones → auto text adjustment: sample the region
  under the caption; pick fill/stroke for max WCAG-ish contrast; nudge
  toward outline-only when background is busy.
- Consumers (all read the same analysis cache):
  - [U] Smart speech-bubble/quote placement around faces (bubble tail
    points at nearest face; avoids busy zones).
  - [U] One-click classic meme layouts (top/bottom caption zones chosen by
    content, not blind placement).
  - [U] Caption placement suggestions (ghost preview + Tab to accept).
- [A] "Clear zone" heat overlay (V key) showing where text will read well.
- [A] Face-aware crop suggestion for portrait sources.

## Cluster 5 — Text autopilot
- [U] Auto text fitting (exists: binary-search fit_box) + outline
  generation that looks correct: stroke BEHIND fill (two-pass render),
  stroke width proportional to font size (~size/12), corner-joined —
  the "impact look" is mostly correct stroke geometry, not the font.
- [U] Balanced line wrapping: cosmic-text doesn't balance; post-pass that
  adjusts max_width to minimize line-length variance. Small, very visible.
- [U] Auto font size to fill available space, readable floor, optional
  toggle per caption.
- [U] Impact mode button: ALL CAPS + white fill + black stroke + centered
  + Impact-ish bundled font (Anton/Archivo Black). A preset = named style
  object; presets are shareable via templates.
- [U] Auto kerning/tracking at large sizes (optical: tighten tracking as
  size grows; swash kerning already in shaping — expose tracking slider).
- [U] Variable width/weight fonts: swash supports variations, cosmic-text
  does not expose axes. Risky: may need pre-instantiated named instances
  (fontmake offline) as the pragmatic path. Flag: investigate before
  promising.
- [U] Per-character/per-word styling (maybe): needs the per-glyph text
  model (Cluster 8) — same foundation as explosion physics and warping.
- [A] Style presets panel + font pairing suggestions from bundled set.
- [A] Contrast-aware outline color auto-pick (shares Cluster 4 sampling).

## Cluster 6 — Vision: cutouts, outlines, editable masks
- [U] Subject auto-trace / background removal: small ONNX segmentation
  (u2netp / isnet ~4MB) via ort; pure-Rust fallback = GrabCut on the
  saliency grid (rough but dependency-free). Output = layer mask, not a
  flattened cutout — user can paint it up/down.
- [U] Vision-model outlines/edits as first-class editable objects: masks
  become vector-ish contours (simplify with ramer-peucker) stored in the
  document, editable nodes. This is the UX layer on top of the mask.
- [U] Edge-aware text warping: thin-plate spline or homography over the
  mask/edge map; caption "sticks" to surfaces. Needs per-glyph placement
  (Cluster 8) + edge map (Cluster 4 Sobel or Cluster 6 contour).
- Order: mask → editable contour → warp. Each step feeds the next.

## Cluster 7 — Motion & physics (stills AND gifs)
- [U] Exploding/falling text, jitter/shake layers. Model as *deterministic
  effect functions* over time t: explosion(t), shake(t, seed), fall(t,
  gravity). Still export = render at frozen t; GIF = animate t. No physics
  engine needed — closed-form motion curves beat a simulator for memes.
- Per-glyph transforms (Cluster 8) make "exploding text" per-letter.
- [U] Physics feel for layers: keyframe-free — effect params (seed,
  intensity, t) are sliders; "randomize" button.
- [A] GIF timeline strip: per-frame delays, onion-skin, freeze-frame pick.
- [A] Repeat-last-effect (Ctrl+Y) + effect macros (record a chain).

## Cluster 8 — Per-glyph text model (foundation, not a feature)
Refactor RenderedText from one path to glyph runs with per-glyph transforms.
Unlocks simultaneously: [U] per-char/word styling, [U] explosion per-letter,
[U] text warping, [U] tracking control, [A] text-on-path, [A] bubble-tail
aware line breaking. Do this BEFORE Cluster 6-warp and 7-glyph-explosion.

## Cluster 9 — Performance targets
- [U] 120fps pan/zoom: already GPU-only blits; verify frame pacing on
  120Hz panel, keep composite off the UI thread (done), prefetch neighbor
  tiles at high zoom if needed.
- [U] 0-lag text editing: text cache exists; add incremental per-glyph
  cache (Cluster 8 makes typing O(changed glyphs) not O(text)).
- [U] Heavy tasks on background threads: standing principle (done; keep).
- [A] Lossy-export preview toggle (see the JPEG artifacts before export).

## Cluster 10 — Palette & color
- [U] Palette extraction: median-cut/k-means on downsampled image (pure
  Rust, ~80 lines); swatch strip in the panel; one-click "recolor text/
  overlays to match" (pick max-contrast palette color for text, harmony
  colors for fills).
- [A] Accent-from-image: UI accent follows the document palette.

## Cluster 11 — Annotation layer family
- [U] "Label this": click a region → arrow layer + text, auto-placed away
  from busy zones (Cluster 4), tail snaps to click point.
- [A] Bubble shapes as first-class layers: speech/thought/shout with
  draggable tail; vector-rendered so they scale crisp; auto-tail-to-face
  (Cluster 4). Bubbles > baked-in bubble images for the "correct" look.
- [A] Arrow styles: hand-drawn wobble variant (meme-authentic).

## Agent-only additions (make Chonk pop)
- [A] Meme template registry: templates carry *anchor slots* (labeled text
  zones); drop text in, it auto-fits/positions. The killer half of
  templates — file sharing without a server.
- [A] Export presets with byte budgets ("under 5MB for Discord", "Twitter
  3:2") — auto-pick quality; pairs with lossy preview.
- [A] Batch mode: apply a document's text/style chain across a folder of
  images (quote cards especially).
- [A] Deterministic seeds everywhere (fry, shake, jitter) — share a .chonk,
  get the identical meme.
- [A] First-run shortcut overlay (README's key map, in-app).
- [A] Command palette (Ctrl+P) once action count justifies it.
- [A] On this machine: niri window rules for a fixed-size editor window.

## Build order (dependency-respecting)
1. **Undo/redo + .chonk + templates** (Cluster 2) — everything else is
   unsafe and unsharable without it. **DONE** (undo/redo, .chonk format,
   named text/image slots with `S` tagging, `chonk-fill` CLI with
   `--list`/`--set`/`--out`, roundtrip + fill tests).
2. **Blend modes + palette extraction** (3, 10) — cheap, immediate range.
   **DONE** (16 tiny-skia blend modes via `,`/`.`; median-cut palette
   extraction with swatch recolor; `best_contrast` helper ready for
   smart text fill in step 4).
3. **Fry engine + artifact/blur presets** (1) — the signature feature,
   pure codecs, no ML. **DONE** (real JPEG round-trip ladder mild..ultra,
   contrast/saturation/noise, `F` in app + `--fry` in chonk-fill, fake
   platform watermarks tiktok/ig/yt/snap/reddit via `--watermark`).
4. **Saliency/contrast analysis + placement suggestions + impact mode +
   balanced wrap + stroke-correct outlines** (4-lite, 5) — smart feel
   without ML.
5. **Face detection + bubble layers + label-this + smart placement**
   (4-full, 11).
6. **Per-glyph text model** (8) → **warping, per-char styling** (6-warp, 5).
7. **Segmentation cutouts + editable contours** (6).
8. **Motion effects + GIF timeline** (7).
9. **Variable fonts** (5-risky) — investigate early, schedule late.

Performance targets (9) are continuous constraints, not a phase.
