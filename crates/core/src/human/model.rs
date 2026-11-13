//! The morphable body model: a base mesh, landmark joints and morph targets.
//!
//! A body is `base + Σ_k w_k Δ_k`: the base vertex positions plus a
//! weighted sum of sparse per-vertex displacement fields ("targets"). The
//! data comes from the MakeHuman base mesh and targets (CC0), packed by
//! `tools/build_body_model.py`; see that script for the file format.
//!
//! Coordinates are converted to millimetres on load. `Y` is up, the body
//! faces `+Z`, and its left side is `+X`.

use crate::error::{Error, Result};
use glam::DVec3;
use std::collections::HashMap;

/// MakeHuman units (decimetres) to millimetres.
const UNIT_MM: f64 = 100.0;

/// A sparse displacement field.
#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub indices: Vec<u32>,
    /// Displacement at weight 1 (mm), parallel to `indices`.
    pub deltas: Vec<DVec3>,
}

/// The loaded body model.
#[derive(Debug, Clone)]
pub struct BodyModel {
    /// Base positions (mm): body vertices, then 8 helper vertices per joint.
    pub base: Vec<DVec3>,
    /// Number of body vertices.
    pub n_body: usize,
    /// Body triangles (counter-clockwise seen from outside).
    pub triangles: Vec<[u32; 3]>,
    pub joint_names: Vec<String>,
    pub targets: Vec<Target>,
    target_index: HashMap<String, usize>,
    /// Skeleton and skinning weights, if the file has them (see
    /// [`crate::human::pose`]).
    pub skin: Option<Skin>,
}

/// One bone of the posing skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Parent bone (bones are listed parents first).
    pub parent: Option<usize>,
    /// The landmark joint it turns about (index into
    /// [`BodyModel::joint_names`]).
    pub joint: usize,
}

/// Skeleton and per-vertex bone weights (linear blend skinning).
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub bones: Vec<Bone>,
    /// Per body vertex: up to four (bone, weight) pairs, weights summing
    /// to 1.
    pub weights: Vec<Vec<(u8, f32)>>,
}

/// Vertex positions of one morphed body.
#[derive(Debug, Clone)]
pub struct Shape {
    /// Same layout as [`BodyModel::base`].
    pub positions: Vec<DVec3>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or_else(|| Error::Body("body model data is truncated".into()))?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn name(&mut self) -> Result<String> {
        let n = self.u8()? as usize;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| Error::Body("bad name in body model".into()))
    }
}

impl BodyModel {
    /// Parse a model packed by `tools/build_body_model.py`.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader { data, at: 0 };
        if r.take(8)? != b"LPBODY1\0" {
            return Err(Error::Body("not a latexpat body model file".into()));
        }
        let (n_body, n_joints, n_tris, n_targets) = (
            r.u32()? as usize,
            r.u32()? as usize,
            r.u32()? as usize,
            r.u32()? as usize,
        );
        let n = n_body + 8 * n_joints;
        let mut base = Vec::with_capacity(n);
        for _ in 0..n {
            let p = [r.f32()?, r.f32()?, r.f32()?];
            base.push(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) * UNIT_MM);
        }
        let mut triangles = Vec::with_capacity(n_tris);
        for _ in 0..n_tris {
            let t = [r.u16()? as u32, r.u16()? as u32, r.u16()? as u32];
            if t.iter().any(|&i| i as usize >= n_body) {
                return Err(Error::Body("body model triangle index out of range".into()));
            }
            triangles.push(t);
        }
        let joint_names = (0..n_joints)
            .map(|_| r.name())
            .collect::<Result<Vec<_>>>()?;
        let mut targets = Vec::with_capacity(n_targets);
        let mut target_index = HashMap::new();
        for k in 0..n_targets {
            let name = r.name()?;
            let scale = r.f32()? as f64 * UNIT_MM;
            let count = r.u32()? as usize;
            let mut indices = Vec::with_capacity(count);
            let mut deltas = Vec::with_capacity(count);
            for _ in 0..count {
                let i = r.u16()? as u32;
                if i as usize >= n {
                    return Err(Error::Body("body model target index out of range".into()));
                }
                indices.push(i);
                deltas.push(DVec3::new(r.i16()? as f64, r.i16()? as f64, r.i16()? as f64) * scale);
            }
            target_index.insert(name.clone(), k);
            targets.push(Target {
                name,
                indices,
                deltas,
            });
        }
        // Optional skinning section.
        let skin = if r.at < data.len() {
            if r.take(4)? != b"SKIN" {
                return Err(Error::Body("unknown section in body model".into()));
            }
            let n_bones = r.u8()? as usize;
            let mut bones = Vec::with_capacity(n_bones);
            for b in 0..n_bones {
                let name = r.name()?;
                let parent = r.u8()? as i8;
                let joint = r.u8()? as usize;
                if joint >= n_joints || (parent >= 0 && parent as usize >= b) {
                    return Err(Error::Body("bad bone in body model".into()));
                }
                bones.push(Bone {
                    name,
                    parent: (parent >= 0).then_some(parent as usize),
                    joint,
                });
            }
            let mut weights = Vec::with_capacity(n_body);
            for _ in 0..n_body {
                let count = r.u8()? as usize;
                let mut w = Vec::with_capacity(count);
                for _ in 0..count {
                    let (b, x) = (r.u8()?, r.u8()?);
                    if b as usize >= n_bones {
                        return Err(Error::Body("bad bone weight in body model".into()));
                    }
                    w.push((b, x as f32 / 255.0));
                }
                weights.push(w);
            }
            Some(Skin { bones, weights })
        } else {
            None
        };
        Ok(Self {
            base,
            n_body,
            triangles,
            joint_names,
            targets,
            target_index,
            skin,
        })
    }

    /// The model bundled with the crate (MakeHuman base mesh, CC0).
    #[cfg(feature = "bundled-body")]
    pub fn bundled() -> Self {
        static DATA: &[u8] = include_bytes!("../../../../assets/body/makehuman.lpbody");
        Self::from_bytes(DATA).expect("bundled body model is valid")
    }

    /// Index of a target by name.
    pub fn target(&self, name: &str) -> Result<usize> {
        self.target_index
            .get(name)
            .copied()
            .ok_or_else(|| Error::Body(format!("body model has no target '{name}'")))
    }

    /// Index of a joint by name.
    pub fn joint(&self, name: &str) -> Result<usize> {
        self.joint_names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error::Body(format!("body model has no joint '{name}'")))
    }

    /// Apply `(target index, weight)` pairs to the base mesh.
    pub fn morph(&self, weights: &[(usize, f64)]) -> Shape {
        let mut positions = self.base.clone();
        for &(k, w) in weights {
            if w != 0.0 {
                let t = &self.targets[k];
                for (&i, d) in t.indices.iter().zip(&t.deltas) {
                    positions[i as usize] += w * *d;
                }
            }
        }
        Shape { positions }
    }
}

impl Shape {
    /// Position of joint `j`: the mean of its 8 helper vertices.
    pub fn joint(&self, model: &BodyModel, j: usize) -> DVec3 {
        let s = model.n_body + 8 * j;
        self.positions[s..s + 8].iter().sum::<DVec3>() / 8.0
    }

    /// Body vertex positions (without joint helpers).
    pub fn body<'a>(&'a self, model: &BodyModel) -> &'a [DVec3] {
        &self.positions[..model.n_body]
    }
}
