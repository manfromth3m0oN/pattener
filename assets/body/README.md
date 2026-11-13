# Body model data

`makehuman.lpbody` is built from the [MakeHuman](https://github.com/makehumancommunity/makehuman)
base mesh and morph targets at commit `a8bc2d54ff0ac92e78ff71431b1023eda42bf482`
by `tools/build_body_model.py`.

MakeHuman's `LICENSE.md` (section C) releases its assets, explicitly
including "the base mesh" and "targets and modifiers", under
[CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/).
This file contains only data derived from those assets: the body part of
the base mesh, 22 landmark joints, 60 targets (two base shapes averaged
over MakeHuman's three default ones, weight, muscle, height, cup, belly,
buttocks and the 20 measurement targets), and a posing skeleton: the
default rig (`data/rigs/default.mhskel`, `default_weights.mhw`, also CC0)
folded into 21 bones that turn about the landmark joints, with each
vertex's four largest weights. No MakeHuman source code is used or
included.

To rebuild:

```sh
git clone https://github.com/makehumancommunity/makehuman
git -C makehuman checkout a8bc2d54ff0ac92e78ff71431b1023eda42bf482
python3 tools/build_body_model.py makehuman assets/body/makehuman.lpbody
```
