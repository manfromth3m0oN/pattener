# latexpat

Pattern generation for latex (natural rubber sheet) garments.

A garment is a set of panels bounded by seams on a 3D body mesh. Each panel
is flattened to 2D at a target **ease factor** `s` (e.g. 0.85): the flat piece
is `s ×` the size of the body, so it stretches by about `1/s` when pulled on.
Latex absorbs curvature by stretching, so the goal is not a distortion-free
layout but low, even strain, which the tool measures and reports.

## Web app

Everything also runs in the browser (WebAssembly, no server-side code).
Every push to `main` deploys it to GitHub Pages:
https://manfromth3m0on.github.io/pattener/

To run it locally:

```sh
web/build.sh                          # needs: rustup target add wasm32-unknown-unknown
python3 -m http.server -d web 8000    # then open http://localhost:8000
```

Enter measurements (or upload an OBJ and seams), set the ease and pattern
options, and download the pieces. `node web/smoke-test.cjs <url>` checks the
page with Playwright.

## Full body from measurements

The web app opens on a full body fitted to your tape measurements (any of
15: height, inseam, waist height, arm length and 11 girths), shown in 3D
with the tape positions on it. Body type, muscle, cup, belly and buttocks
are set by eye. Projects (name, measurements, shape, units) save to a
`.latexpat.json` file and autosave in the browser.

```sh
latexpat human tests/data/project.json -o body/   # body.obj + measurements.json
```

The body is the [MakeHuman](https://github.com/makehumancommunity/makehuman)
base mesh (CC0, see `assets/body/README.md`) morphed by its shape targets.
`human::fit` solves height, weight and 17 local shape parameters so the
body's own tape measurements (convex-hull girths of plane cuts at the
narrowest or fullest point, as with a real tape) match yours, typically to
well under a millimetre. Combinations the model cannot reach are reported
with how far off they are.

### Drawing the garment

Below the 3D view, draw the garment on the body:

- **Edges** (necklines, hems, cuffs, leg openings): click points around the
  body; close the loop.
- **Seams** split the garment into pieces that lie flat. Their ends snap
  onto an edge or another seam within 2 cm.
- Curves pass smoothly through the points you click; drag a point of the
  selected curve to move it. Tick **Mirror** to get the same curve on the
  other side.
- **Pick inside** chooses which side of the edges is the garment.

### Fit tools

- **Lengthwise reduction** (`--length-relief`, or "% of around" in the web
  app): latex is usually reduced less along the body than around it.
  The reference shapes are scaled by `s` around and `1 − r(1 − s)` along
  the body; "along" follows the skeleton on the full body (bone
  directions blended as orientation tensors) and the up axis otherwise.
- **Ease zones**: click a point on the body and give it its own ease
  within a radius (looser at the crotch or a cuff, firmer at the waist),
  blending back to the rest; zones can be mirrored and follow the body.
- **Pressure map**: after Make pattern the garment on the body is coloured
  by how hard it presses, `p = T₁κ₁ + T₂κ₂` (neo-Hookean tensions in the
  principal stretch directions times the body's curvature in those
  directions, at a latex-scale of a few centimetres), in mmHg. Blue marks
  where it **bridges** a hollow instead of touching the skin. Each piece
  reports its mean and 95th-percentile pressure, bridging and slack
  (wrinkle-prone) area; pieces where 5 % exceeds 40 mmHg are flagged.
  Pressure uses the calibrated sheets, or `--thickness` (default 0.40 mm)
  with μ = 0.45 MPa.

### Latex sheets and colour

A project has a catalogue of sheets: name, colour, thickness, roll width,
and optionally its own ease, rubber stiffness and price per metre. Every
piece uses the default sheet unless you **paint** it another one on the
body (the piece containing the point you click; points rather than piece
numbers, so the choice survives changes to the seams). The 3D preview
shows each piece in its sheet's colour.

Each piece then takes its sheet's ease (the sheet's own, else its swatch
calibration by name or thickness, else the global ease) and stiffness
(for seam matching and the pressure map), and each sheet's pieces are
laid out on its roll width (`layout_<sheet>.svg`). The materials list
gives the length of each roll to buy, the area of the pieces, how much of
the roll is used, and the cost.

Each piece's report gives its stretch range and the stretch that 95 % of
its area stays under, which is not thrown off by a few tiny triangles at
sharp seam corners the way the maximum is.

The preview shows the garment in latex colour and says how many flat
pieces the seams make, or which piece still wraps around and needs a seam.
**Load example top** gives a short-sleeved top to start from. Designs are
stored as positions on the body model's triangles, which are the same for
every body, so a design follows the body when the measurements change.

```sh
latexpat flatten --project tests/data/top.project.json -o pattern/ --heatmap max
```

Under the hood (`garment::build`): curves become Catmull–Rom splines
sampled every 15 mm on the skin, are cut exactly into the mesh by walking
the plane section between samples (with a shortest-path fallback where the
plane cannot join two points, as at the armpit), the mesh is cleaned of
tiny edges and slivers, and the garment area is flood-filled from the
inside point without crossing edges. Seam matching never folds a panel to
make seams equal: a seam that needs shaping keeps its difference and is
reported (the example top, with no bust shaping, warns about this).

## Quick torso from measurements

```sh
latexpat body measurements.json -o body/ --style princess
latexpat flatten body/body.obj --seams body/body.seams.json -o pattern/ --sheet-width 1000
```

```json
{ "levels": [
    { "name": "hip",   "height_mm": 0,   "circumference_mm": 980 },
    { "name": "waist", "height_mm": 200, "circumference_mm": 740 },
    { "name": "bust",  "height_mm": 420, "circumference_mm": 940, "front_bias": 0.6 } ] }
```

Levels go bottom to top; heights are above the lowest level. Each
cross-section is an ellipse (`depth_ratio`, default 0.75) whose front holds
`front_bias` of the depth (0.5 = symmetric, more for bust or belly). The mesh
ring at each level has exactly the measured circumference, and the shape is
interpolated between levels without overshooting. `--style tube` gives
front and back panels; `princess` (default) adds four princess lines for six
panels. Every seam is notched at each inner level (e.g. the waist).

## Usage

```sh
cargo run --release -p latexpat -- flatten body.obj --seams seams.json --ease 0.85 -o out/ --heatmap max
```

| option | meaning |
|---|---|
| `--seams FILE` | seam polylines (below); omit if the mesh is already a single disc |
| `--ease S` | flat edge length = `S ×` body edge length (default 0.85) |
| `--calibration FILE --sheet NAME` | take the ease from a swatch calibration instead (below); `NAME` may also be a thickness like `0.5mm` |
| `--panel-sheet I=NAME` | make panel `I` from a different sheet (repeatable) |
| `--scale K` | multiply mesh coordinates by `K` to get mm (10 for cm, 1000 for m) |
| `--up AXIS` | body axis that points up the page: `x y z -x -y -z` (default `y`) |
| `--heatmap M` | `none`, `max` (σ_max) or `area` (σ_min·σ_max) |
| `--heatmap-range R` | stretch where colours saturate (default 1.5) |
| `--no-seam-matching` | keep each panel's independent flattening |
| `--seam-weight W` | strength of the seam-length constraints (default 10) |
| `--seam-tolerance MM` | warn when a seam's sides still differ by more than this (default 1 mm) |
| `--glue-allowance MM` | lap overlap added to one side of each seam (default 6 mm) |
| `--allowance-side S` | `left` (default), `right`, `both` or `none`; seams can override |
| `--hem-allowance MM` | allowance on open edges such as hems and necklines (default 0) |
| `--notch-spacing MM` | body distance between automatic notches (default 50; 0 = explicit only) |
| `--sheet-width MM` | lay all pieces out on a sheet this wide and write `layout.svg` |
| `--sheet-gap MM` | gap between pieces on the sheet (default 5) |
| `--no-rotate` | keep pieces upright on the sheet |

Output:
- `out/panel_NN.svg`: 1:1 scale in mm with a 50 mm scale bar to check
  printing. Solid line = cut line; dashed line = seam/glue line where the
  piece carries an allowance. Numbered ticks are notches: each number
  appears on exactly two pieces, at the same point of the body. Mark them
  with a pen (snipping latex starts tears). Seam edges are labelled with the
  panel they join.
- `out/layout.svg` (with `--sheet-width`): all pieces on the sheet, and the
  length of sheet needed.
- `out/report.json`: per-panel stretch range, areas, flat and cut sizes,
  notches; per-seam lengths before and after matching; layout placements. Heatmap colours: blue = slack (σ < 1), white = unstretched,
orange = stretched (σ > 1).

### Seam file

Seams are polylines of **0-based OBJ vertex indices** (file order, as shown
in Blender). Consecutive vertices must share a mesh edge; repeat the first
vertex to close a loop.

```json
{ "seams": [ { "name": "side_left", "vertices": [12, 40, 41, 77],
               "notches": [41], "allowance": "right" } ] }
```

`notches` (optional) are seam vertices that get a numbered matching mark on
both pieces, in addition to the automatic ones. `allowance` (optional)
overrides `--allowance-side` for this seam. *Left* is the panel on the left
when walking the seam in vertex order, seen from outside the body, so
reversing a seam's vertex list moves its lap to the other piece.

Every panel produced by the cut must be a topological disc; otherwise the
tool reports which panel fails and why (Euler characteristic and number of
boundary loops), e.g. a tube needs one lengthwise seam.

## Swatch calibration

Thicker latex pulls harder for the same stretch, so it needs less reduction.
Instead of guessing the ease for each sheet, measure it:

1. Cut a strip about 25 mm wide from each sheet and draw two marks 100 mm
   apart.
2. Hang a few known weights from it (e.g. 100, 200, 300 g) and measure the
   distance between the marks each time.
3. Note a fit you know you like, e.g. "0.40 mm at ease 0.85".

```json
{
  "sheets": [
    { "name": "0.40mm black", "thickness_mm": 0.40,
      "samples": [ { "width_mm": 25, "gauge_mm": 100, "stretched_mm": 108.5, "mass_g": 100 },
                   { "width_mm": 25, "gauge_mm": 100, "stretched_mm": 116.0, "mass_g": 200 } ] },
    { "name": "0.60mm black", "thickness_mm": 0.60, "samples": [ ... ] }
  ],
  "target": { "reference": { "sheet": "0.40mm black", "ease": 0.85 } }
}
```

Loads can be `mass_g` (hanging weights) or `load_n`. The target can instead
be a wearing tension: `"target": { "tension_n_per_cm": 1.3 }`.

```sh
latexpat calibrate swatches.json --thickness 0.5
latexpat flatten body.obj --seams seams.json -o out/ \
  --calibration swatches.json --sheet "0.40mm black" --panel-sheet 3="0.60mm black"
```

The tool fits a neo-Hookean rubber model to each sheet (shear modulus μ),
works out the wearing tension of the reference fit, and gives every sheet
the ease that produces that same tension. Seams joining different sheets
settle where the two tensions balance, nearer the thicker side.
`tests/data/swatches.json` is a synthetic example, not real measurements.

## Continuous integration

`.github/workflows/ci.yml` runs on every pull request and push to `main`:
`cargo fmt --check`, `clippy` and `rustdoc` with warnings as errors, all
tests, the WebAssembly build of the core and of the web app, and the
Playwright smoke test of the page. On `main` it then publishes the web app
to GitHub Pages (Settings → Pages → Source must be "GitHub Actions").

## Layout

- `crates/core` (`latexpat-core`): all geometry. No file I/O, no threads;
  builds for `wasm32-unknown-unknown`.
- `crates/cli` (`latexpat`): argument parsing and file I/O.
- `crates/wasm` (`latexpat-wasm`): WebAssembly module with a JSON-in,
  JSON-out `call` export; no binding generator needed.
- `web/`: static web UI using that module, with a WebGL viewer.
- `assets/body/`: the packed body model and its licence notes;
  `tools/build_body_model.py` rebuilds it.
- `tests/data`: small test meshes, regenerated by
  `cargo run -p latexpat-core --example gen_test_data -- tests/data`.

## Pipeline (`latexpat-core`)

1. `obj::parse_obj`: OBJ text → validated `Mesh` (manifold, consistent winding).
2. `SeamSet::resolve` + `cut::cut_panels`: union-find over triangles and
   triangle corners → disc-shaped `Panel`s with maps back to the input mesh
   and a seam-tagged boundary loop.
3. `lscm::lscm`: least-squares conformal map, initial layout.
4. `arap::arap`: local/global ARAP towards reference triangles scaled by the
   per-triangle `Ease`.
5. `seam_match`: pairs the two sides of every seam (splitting where a seam
   crosses another) and equalises their flat lengths edge by edge by
   re-running ARAP with soft edge-length targets, correcting the targets
   each round until the sides agree.
6. `strain::measure`: deformation gradient `F = P Q⁻¹` of flat → body,
   principal stretches, flipped triangles.
7. `pattern::build_pattern`: per-edge glue allowance (lap seams: one side),
   cut line via `allowance::offset_polygon`, and notches placed by body
   arc length so both pieces mark the same body point.
8. `nest::nest`: bottom-left skyline packing of the cut pieces on a
   fixed-width sheet (rotation only, never mirrored, since latex has a
   shiny and a matte face).
9. `flatten::orient` + `svg::panel_svg` / `svg::layout_svg`: upright
   placement and SVG export.

`body::torso` builds the mesh and seams from measurements, and `job::run`
wraps steps 2–9 into one call returning SVG strings, a report and warnings;
both the CLI and the web UI use it.

`calibrate` (separate from the pipeline) fits swatch tests and returns the
ease per sheet, which feeds step 4 via `Ease::per_panel`, and the stiffness
`μt` per panel, which weights step 5.

Each stage documents its formulas in rustdoc (`cargo doc --open -p latexpat-core`).

## Validation

`cargo test` checks the pipeline against analytic cases: a flat patch
(stretch exactly `1/s`), a tube opened by one seam and an open cylinder
panel (unroll with stretch `1/s`), and a spherical cap (per-triangle
stretches within 1.5 % of an independent axisymmetric ARAP solution). Every
case also checks for positive flat areas and no flipped triangles. Seam
matching is tested on a cap cut around a ring, where the dome and the band
flatten to sides 3.1 mm apart; matching removes the gap edge by edge
(< 0.01 mm) while changing stretch by under 0.01.

## Roadmap

1. ✅ Cut, flatten (LSCM + ease-scaled ARAP), strain report, SVG.
2. ✅ Seam-length matching between paired panel edges, with mismatch warnings.
3. ✅ Glue-overlap allowance, notches, sheet-width nesting (bounding-box
   packing; polygon nesting later).
4. ✅ Swatch calibration → per-sheet ease, stiffness-weighted seam matching.
5. ✅ Parametric torso from measurements; web UI via WASM.

Next, towards designing whole garments: full body from measurements with a
3D view and project files (done); garments drawn on the body (done); fit
tools (done); materials and colour (done); construction details
and print output (zips, trims, tiled PDF, assembly guide); templates.
