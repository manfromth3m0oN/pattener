# latexpat

Pattern drafting for latex (natural rubber sheet) garments.

Enter the wearer's measurements, start from a template drafted to them,
reshape the pieces in a flat 2D editor, and print the pattern. The design is
drawn at **body size**; making the pattern reduces each piece by its
**ease** `s` (e.g. 0.85), so it stretches by about `1/s` when pulled on,
then adds the glue laps, hems, numbered notches, trim strips, sheet
layouts, an assembly guide and a PDF tiled for a home printer.

## Web app

Everything runs in the browser (WebAssembly, no server-side code). Every
push to `main` deploys it to GitHub Pages:
https://manfromth3m0on.github.io/pattener/

New to it? The [tutorial](docs/tutorial.md) walks through making a
sleeveless top, from measurements to the printed pattern.

The app is a design workspace:
- **Top bar:** the project (name, New, Open, Save) and **Make pattern**.
- **Left panel:** the **Body** (measurements), the **Garment** (templates, pieces, seams) and the latex **Sheets**.
- **Middle:** the **Design** editor with its tools (Select, Point, Piece, Split, Join, Undo, Redo, Delete, Fit), and the flat **Pattern** pieces beside it. Tick **Live** to remake the pattern after every change.
- **Right panel:** the **Inspector** for what is selected, the pattern **Settings**, and the **Report**: warnings, downloads, materials, assembly guide, piece and seam tables.
- **Status bar:** progress, plus a warnings counter that opens them.

It is dark, or light when the device is set to light mode. On a phone the
same screens are chosen from a bottom tab bar: pinch to zoom the design,
tap a measurement's name for how to take it.

To run it locally:

```sh
web/build.sh                          # needs: rustup target add wasm32-unknown-unknown
python3 -m http.server -d web 8000    # then open http://localhost:8000
```

`node web/smoke-test.cjs <url>` checks the page with Playwright.

## Measurements

Any of 17 tape measurements can be given: height, inseam, waist height,
back length (nape to waist), shoulder width, arm length, and girths of the
neck, bust, underbust, waist, hips, thigh, knee, calf, ankle, upper arm and
wrist. The rest are estimated from a typical **women's** or **men's**
figure (`body::Figure`): lengths scale with the height, girths with the
girths that were given (the geometric mean of their ratios). The app shows
the estimates greyed in each empty field. Measurements that cannot belong
to one body (an inseam above the waist, say) are explained, not drafted.

The body's **levels** (neck, nape, shoulder, underarm, bust, underbust,
waist, hip, crotch, knee, calf, ankle) are worked out from the lengths with
common anthropometric proportions and drawn across the editor as guides;
moved points snap to them.

```sh
latexpat body --measurements measurements.json   # every value, estimates marked, and the levels
```

## Designing flat

A design is the **right half** of the garment, as in flat pattern
drafting, made of **pieces**. Each piece is an outline through its
**points**, in millimetres: `x` across the body from the centre line,
`y` the height above the floor.

- **Points** are *corners* (sharp; an edge starts there), *smooth* (the
  outline curves through) or *joints* (smooth, but an edge starts there,
  like the top of a sleeve cap). The outline between points is a
  centripetal Catmull–Rom curve, which never loops or cusps.
- **Edges** run from one corner or joint to the next. An edge is either
  sewn in a seam, a **fold** (the piece is cut on the fold there and printed
  unfolded, since latex is cut single-layer), or open with a **finish**:
  the pattern's hem allowance, raw (cut on the line), a folded hem, or a
  trim strip. Open edges with the same name and a trim finish share one
  strip.
- A piece can be **cut as a mirrored pair**, one for each side.
- Each piece has an **arrow up the body**. Pieces are usually drawn
  upright (the arrow points up the page), but a piece drawn turned, such as
  a strap lying on its side or an angled panel, has its arrow turned to
  match: drag the round handle at its head (15° steps with Snap on), or type
  the angle in the Inspector. The arrow is printed on the pattern.
- A **seam** joins two edges. Coloured dots in the editor mark the ends
  that meet (Swap ends if they don't). A seam can be glued (with the lap on
  either side, both, or neither) or a zip. Joining an edge of a pair piece
  to itself joins it to its mirror image, like a centre back seam.

Editor tools:

| Tool | Key | What it does |
|---|---|---|
| Select | V | Click pieces, edges, points or seams; drag points or whole pieces; double-click an edge to add a point; drag the background to pan, scroll or pinch to zoom |
| Point | A | Click on an edge to add a point |
| Piece | P | Click corners; click the first again (or Enter) to close a new piece |
| Split | S | Click two points on a piece's outline: it is cut along the line and the two parts seamed (a princess line, a yoke, a panel in another colour) |
| Join | J | Click an edge, then the edge it is sewn to |
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z | Every change is undoable |
| Delete | Del | The selected point, piece or seam |
| Fit | F | The whole design in view |

The Inspector edits names, point kinds and positions, notch marks, fold
lines, finishes, sheets, seam laps and zips, and shows each edge's length
and each seam's two sides, so you can see when they differ.

### Templates

**Start from a template** drafts a garment for the measurements in use:

| Template | Pieces | Construction |
|---|---|---|
| Sleeveless top | front, back (both on the fold) | shoulder and side seams; neckline and armholes trimmed; folded hem |
| Top with long sleeves | front, back, sleeve pair | set-in sleeves whose caps are drafted to the armholes; underarm seams |
| Waist cincher | front (fold), back pair | back zip, side seams; trimmed top and bottom |
| Pencil skirt | front (fold), back pair | zip at the top of the centre back seam; folded waist |
| Leggings | one pair (front and back of each leg in one) | centre front and back seams, inside leg seams; folded waist |
| Sleeveless catsuit | front pair, back pair | centre front seam, back zip, shoulder, side and inside leg seams |
| Catsuit with sleeves | front, back and sleeve pairs | as above, with set-in sleeves |

How they are drafted (`blocks`): the side seam sits a quarter of each girth
from the centre line (moved towards the back on a women's figure, by the
same amount at every level, so the front and back side seams have the same
shape and length). Necklines are quarter ellipses from the side of the neck;
the shoulder slopes to half the shoulder width; the armhole runs in to the
across-front or across-back width. Latex has no darts, so a women's front
is lengthened at the centre for the bust. Sleeve caps are solved so their
front and back halves are exactly as long as the armholes. Crotch
extensions take up whatever the thigh needs beyond half the hips, and the
legs are drafted so the front and back inside-leg seams match. Every
template is tested on women's and men's figures at several sizes: every
seam matches to within 0.5 mm at body size and every cut line is clean.

### Following new measurements

A design drafted from a template **follows the measurements**: change one
and the design is redrafted, keeping your edits. Each point the template
drew moves by as much as the template's own point moved; points you added
move with their neighbours along the outline. Pieces you drew yourself stay
as they are. (`blocks::redraft`, or `latexpat redraft`.)

## Making the pattern

1. **Unfold** pieces cut on the fold.
2. **Reduce** each piece by its ease: `s` around the body and
   `1 − r(1 − s)` along it, `r` being the lengthwise share
   ("Lengthwise (% of around)"; latex is usually reduced less along the
   body, 30–50 % is typical). "Along" is the direction of the piece's
   arrow, "around" the direction across it. The ease is the piece's sheet's
   own, else its swatch calibration, else the pattern's. With the
   lengthwise share at 100 % the reduction is the same every way, and the
   arrow makes no difference.
3. **Allowances**: a glued seam's lap on the side chosen, a strip on both
   sides of a zip (`--zip-allowance`, default 10 mm), and each open edge's
   finish. The cut line is the seam line offset edge by edge, mitred at
   corners.
4. **Notches**: numbered pen marks (snipping latex starts tears), the same
   number on both sides of a seam at the same fraction of each side's body
   length: one every ~50 mm, plus one at every point marked as a notch.
5. **Check**: a seam whose sides differ by more than the tolerance
   (1 mm) after the reduction is reported, with which side is longer and
   why: the sides already differ at body size, or the pieces have different
   eases, or the two edges run at different angles to the body so the
   lengthwise reduction shortens them differently (a sleeve cap against its
   armhole, for example: ease it in, or check the arrows).
6. **Output**: one 1:1 SVG per piece (cut line solid, seam line dashed where
   there is an allowance, fold line, an arrow up the body, notches, edge
   labels and a 50 mm scale bar), trim strips with quarter marks, a layout
   of every copy of every piece on each sheet's roll width (and optionally
   on one sheet), the materials list with the length of each roll to buy
   and its cost, the assembly guide, a `report.json`, and a **printable
   PDF** tiled over A4 or Letter pages with overlaps, corner marks, page
   labels and a 50 mm check square.

The **assembly guide** orders the work: cut and mark notches, join the
pieces into one (shortest seams first), close the remaining seams, then fit
zips, hem, and bind edges. Each step names the pieces, which one carries
the lap, and the notches to match.

## Command line

```sh
latexpat templates
latexpat template top-sleeves --measurements m.json -o top.latexpat.json
latexpat pattern top.latexpat.json -o out/ --pdf a4 --sheet-width 1000
latexpat redraft top.latexpat.json --measurements new.json
latexpat calibrate swatches.json --thickness 0.5
```

`m.json` is `{"height": 1680, "bust": 940, ...}` in millimetres. Project
files are the same `.latexpat.json` the web app saves: the body, the
design, the sheets and the pattern settings. Projects from the earlier 3D
editor keep their measurements and sheets; start a new design from a
template with them (`latexpat template ID --project old.latexpat.json`).

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
latexpat pattern top.latexpat.json -o out/ \
  --calibration swatches.json --sheet "0.40mm black"
```

The tool fits a neo-Hookean rubber model to each sheet (shear modulus μ),
works out the wearing tension of the reference fit, and gives every sheet
the ease that produces that same tension. When making a pattern, each
piece's sheet is looked up in the calibration by name, then by thickness;
`--sheet` covers the rest. A sheet's own Ease, if set, comes first. `tests/data/swatches.json` is a synthetic example, not real measurements.

## Continuous integration

`.github/workflows/ci.yml` runs on every pull request and push to `main`:
`cargo fmt --check`, `clippy` and `rustdoc` with warnings as errors, all
tests, the WebAssembly build of the core and of the web app, and the
Playwright smoke test of the page. On `main` it then publishes the web app
to GitHub Pages (Settings → Pages → Source must be "GitHub Actions").

## Layout

- `crates/core` (`latexpat-core`): measurements, drafting and pattern
  making. No file I/O, no threads; builds for `wasm32-unknown-unknown`.
  - `body`: measurements, estimates and levels;
  - `design`: pieces, points, edges and seams, and their outlines (`geom`);
  - `blocks`: the templates and `redraft`;
  - `pattern`: unfolding, reduction, allowances (`allowance`), notches and
    trims;
  - `job`: one call from design to SVGs (`svg`), layouts (`nest`,
    `materials`), the assembly guide (`assembly`), the PDF (`pdf`) and a
    report; `project`: the project file; `calibrate`: swatch tests.
- `crates/cli` (`latexpat`): argument parsing and file I/O.
- `crates/wasm` (`latexpat-wasm`): WebAssembly module with a JSON-in,
  JSON-out `call` export; no binding generator needed.
- `web/`: the static web app. `geometry.js` repeats the outline curves of
  `geom`/`design` so the editor draws exactly what the pattern is made from
  (the smoke test and a unit check compare the two).

Each stage documents its formulas in rustdoc (`cargo doc --open -p latexpat-core`).
