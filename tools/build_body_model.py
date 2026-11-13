#!/usr/bin/env python3
"""Build assets/body/makehuman.lpbody from a MakeHuman checkout.

The MakeHuman base mesh and targets are CC0 (see assets/body/README.md).
This script only reads those asset files; it uses no MakeHuman code.

    git clone https://github.com/makehumancommunity/makehuman
    git -C makehuman checkout a8bc2d54ff0ac92e78ff71431b1023eda42bf482
    python3 tools/build_body_model.py makehuman assets/body/makehuman.lpbody

File format (little endian), units decimetres as in MakeHuman, Y up, front +Z:
    magic        8 bytes  "LPBODY1\\0"
    n_body       u32      body vertices (indices 0..n_body)
    n_joints     u32      landmark joints; joint j owns 8 helper vertices
                          n_body + 8j .. n_body + 8j + 8 (their mean is the joint)
    n_tris       u32
    n_targets    u32
    positions    f32[3 * (n_body + 8 * n_joints)]
    triangles    u16[3 * n_tris]          (body only, counter-clockwise outside)
    joints       n_joints x (u8 len, name)
    targets      n_targets x (u8 len, name, f32 scale, u32 count,
                              count x (u16 vertex, i16[3] delta / scale))
    skin         (optional) "SKIN", u8 n_bones,
                 n_bones x (u8 len, name, i8 parent or -1, u8 pivot joint),
                 n_body x (u8 count, count x (u8 bone, u8 weight / 255))

The skin folds MakeHuman's default rig (CC0, data/rigs) into the bones
below, each turning about one of the landmark joints; a vertex keeps its
four largest weights.
"""
import struct
import sys
from pathlib import Path

JOINTS = [
    "ground", "pelvis", "spine-4", "spine-3", "spine-2", "spine-1", "neck", "head",
    "l-clavicle", "l-shoulder", "l-elbow", "l-hand", "l-upper-leg", "l-knee", "l-ankle",
    "r-clavicle", "r-shoulder", "r-elbow", "r-hand", "r-upper-leg", "r-knee", "r-ankle",
]
MEASURES = [
    "neck-circ", "neck-height", "upperarm-circ", "upperarm-length", "lowerarm-length",
    "wrist-circ", "frontchest-dist", "bust-circ", "underbust-circ", "waist-circ",
    "napetowaist-dist", "waisttohip-dist", "shoulder-dist", "hips-circ", "upperleg-height",
    "thigh-circ", "lowerleg-height", "calf-circ", "ankle-circ", "knee-circ",
]
# Reduced skeleton: (name, parent, pivot joint, MakeHuman bones folded in;
# a MakeHuman bone not listed folds into its nearest listed ancestor).
BONES = [
    ("hips", None, "pelvis", ["root", "pelvis.L", "pelvis.R", "spine05"]),
    ("spine-lower", "hips", "spine-4", ["spine04"]),
    ("spine-mid", "spine-lower", "spine-3", ["spine03"]),
    ("chest", "spine-mid", "spine-2", ["spine02", "breast.L", "breast.R"]),
    ("upper-chest", "chest", "spine-1", ["spine01"]),
    ("neck", "upper-chest", "neck", ["neck01", "neck02", "neck03"]),
    ("head", "neck", "head", ["head"]),
]
for side, s_ in (("l", "L"), ("r", "R")):
    BONES += [
        (f"{side}-clavicle", "upper-chest", f"{side}-clavicle", [f"clavicle.{s_}", f"shoulder01.{s_}"]),
        (f"{side}-upper-arm", f"{side}-clavicle", f"{side}-shoulder", [f"upperarm01.{s_}", f"upperarm02.{s_}"]),
        (f"{side}-forearm", f"{side}-upper-arm", f"{side}-elbow", [f"lowerarm01.{s_}", f"lowerarm02.{s_}"]),
        (f"{side}-hand", f"{side}-forearm", f"{side}-hand", [f"wrist.{s_}"]),
        (f"{side}-thigh", "hips", f"{side}-upper-leg", [f"upperleg01.{s_}", f"upperleg02.{s_}"]),
        (f"{side}-shin", f"{side}-thigh", f"{side}-knee", [f"lowerleg01.{s_}", f"lowerleg02.{s_}"]),
        (f"{side}-foot", f"{side}-shin", f"{side}-ankle", [f"foot.{s_}"]),
    ]

M = "macrodetails/"
U = M + "universal-{g}-young-{m}muscle-{w}weight"
# output name -> list of source targets to average
TARGETS = {}
for g, s in (("female", "f"), ("male", "m")):
    TARGETS[g] = [f"{M}{race}-{g}-young" for race in ("african", "asian", "caucasian")]
    TARGETS[f"{s}-weight-incr"] = [U.format(g=g, m="average", w="max")]
    TARGETS[f"{s}-weight-decr"] = [U.format(g=g, m="average", w="min")]
    TARGETS[f"{s}-muscle-incr"] = [U.format(g=g, m="max", w="average")]
    TARGETS[f"{s}-muscle-decr"] = [U.format(g=g, m="min", w="average")]
    TARGETS[f"{s}-height-incr"] = [f"{M}height/{g}-young-averagemuscle-averageweight-maxheight"]
    TARGETS[f"{s}-height-decr"] = [f"{M}height/{g}-young-averagemuscle-averageweight-minheight"]
TARGETS["cup-incr"] = ["breast/female-young-averagemuscle-averageweight-maxcup-averagefirmness"]
TARGETS["cup-decr"] = ["breast/female-young-averagemuscle-averageweight-mincup-averagefirmness"]
TARGETS["belly-incr"] = ["stomach/stomach-pregnant-incr"]
TARGETS["belly-decr"] = ["stomach/stomach-pregnant-decr"]
TARGETS["buttocks-incr"] = ["buttocks/buttocks-volume-incr"]
TARGETS["buttocks-decr"] = ["buttocks/buttocks-volume-decr"]
for name in MEASURES:
    for d in ("incr", "decr"):
        TARGETS[f"measure-{name}-{d}"] = [f"measure/measure-{name}-{d}"]

MIN_DELTA = 2e-4  # decimetres (0.02 mm); smaller displacements are dropped


def main(src, dst):
    data = Path(src) / "makehuman" / "data"
    verts, faces, group = [], {}, None
    for line in open(data / "3dobjs" / "base.obj"):
        p = line.split()
        if not p:
            continue
        if p[0] == "v":
            verts.append(tuple(float(x) for x in p[1:4]))
        elif p[0] == "g":
            group = p[1]
        elif p[0] == "f":
            faces.setdefault(group, []).append([int(t.split("/")[0]) - 1 for t in p[1:]])

    n_body = 1 + max(i for f in faces["body"] for i in f)
    assert n_body < 65536
    remap = {i: i for i in range(n_body)}
    for j, name in enumerate(JOINTS):
        ids = sorted({i for f in faces["joint-" + name] for i in f})
        assert len(ids) == 8, name
        for k, i in enumerate(ids):
            remap[i] = n_body + 8 * j + k
    n = n_body + 8 * len(JOINTS)
    pos = [None] * n
    for old, new in remap.items():
        pos[new] = verts[old]

    tris = []
    for f in faces["body"]:
        for k in range(1, len(f) - 1):
            tris.append((f[0], f[k], f[k + 1]))

    out = bytearray(b"LPBODY1\0")
    out += struct.pack("<4I", n_body, len(JOINTS), len(tris), len(TARGETS))
    for p in pos:
        out += struct.pack("<3f", *p)
    for t in tris:
        out += struct.pack("<3H", *t)
    for name in JOINTS:
        out += struct.pack("<B", len(name)) + name.encode()

    for name, sources in TARGETS.items():
        acc = {}
        for s in sources:
            for line in open(data / "targets" / (s + ".target")):
                if line.startswith("#") or not line.strip():
                    continue
                p = line.split()
                i = remap.get(int(p[0]))
                if i is None:
                    continue
                d = acc.setdefault(i, [0.0, 0.0, 0.0])
                for a in range(3):
                    d[a] += float(p[1 + a]) / len(sources)
        items = sorted((i, d) for i, d in acc.items() if max(abs(x) for x in d) >= MIN_DELTA)
        scale = max((abs(x) for _, d in items for x in d), default=1.0) / 32767.0
        out += struct.pack("<B", len(name)) + name.encode()
        out += struct.pack("<fI", scale, len(items))
        for i, d in items:
            out += struct.pack("<H3h", i, *(round(x / scale) for x in d))
        print(f"{name:32} {len(items):6} vertices")

    out += skin(data, n_body)
    Path(dst).write_bytes(out)
    print(f"wrote {dst}: {len(out)} bytes, {n_body} body vertices, {len(tris)} triangles")


def skin(data, n_body):
    import json
    rig = json.load(open(data / "rigs" / "default.mhskel"))["bones"]
    weights = json.load(open(data / "rigs" / "default_weights.mhw"))["weights"]
    ours = {mh: k for k, (_, _, _, mhs) in enumerate(BONES) for mh in mhs}

    def fold(mh):
        while mh not in ours:
            mh = rig[mh]["parent"]
        return ours[mh]

    acc = [dict() for _ in range(n_body)]
    for mh, pairs in weights.items():
        b = fold(mh)
        for v, w in pairs:
            if v < n_body:
                acc[v][b] = acc[v].get(b, 0.0) + w
    names = [b[0] for b in BONES]
    out = bytearray(b"SKIN") + struct.pack("<B", len(BONES))
    for name, parent, pivot, _ in BONES:
        out += struct.pack("<B", len(name)) + name.encode()
        out += struct.pack("<bB", -1 if parent is None else names.index(parent), JOINTS.index(pivot))
    unweighted = 0
    for v in range(n_body):
        top = sorted(acc[v].items(), key=lambda kv: -kv[1])[:4]
        total = sum(w for _, w in top)
        if total <= 0:
            unweighted += 1
            top, total = [(0, 1.0)], 1.0
        q = [(b, round(255 * w / total)) for b, w in top]
        q = [(b, w) for b, w in q if w > 0]
        q[0] = (q[0][0], q[0][1] + 255 - sum(w for _, w in q))
        out += struct.pack("<B", len(q))
        for b, w in q:
            out += struct.pack("<BB", b, w)
    print(f"skin: {len(BONES)} bones, {unweighted} unweighted vertices (given to the hips)")
    return out


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
