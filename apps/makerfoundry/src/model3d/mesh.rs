//! A triangle mesh read from a model file (MF-36 onwards): what the readers give back, with the
//! limits every reader shares.

/// The largest model file read, in bytes (the owner's decision, 2026-10-30: 100 MB).
pub const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
/// The most triangles kept for drawing; a file with more is still read and counted, and drawn
/// from the first ones kept (docs/3d-hub-plan.md § 3).
pub const MAX_TRIANGLES: usize = 5_000_000;
/// The largest a model may be along any side, in millimetres: far beyond any printer, and a sign
/// the file is in other units or damaged.
pub const MAX_SIZE_MM: f64 = 5000.0;

pub type Vertex = [f32; 3];
pub type Triangle = [Vertex; 3];

/// The triangles kept, and how many the file had in all.
#[derive(Debug, Default, PartialEq)]
pub struct Mesh {
    pub triangles: Vec<Triangle>,
    pub total: u64,
}

impl Mesh {
    /// Adds a triangle: refused if a number isn't finite; counted but not kept past
    /// `MAX_TRIANGLES`.
    pub fn push(&mut self, t: Triangle) -> Result<(), String> {
        if !t.iter().flatten().all(|n| n.is_finite()) {
            return Err("the file has a number that isn't a number (NaN or infinite)".into());
        }
        self.total = self.total.saturating_add(1);
        if self.triangles.len() < MAX_TRIANGLES {
            self.triangles.push(t);
        }
        Ok(())
    }

    /// The box around every kept triangle, as (smallest, largest) corners; `None` when empty.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut vertices = self.triangles.iter().flatten();
        let first = vertices.next()?;
        let mut lo = first.map(f64::from);
        let mut hi = lo;
        for v in vertices {
            for (i, n) in v.iter().enumerate() {
                let n = f64::from(*n);
                if let (Some(l), Some(h)) = (lo.get_mut(i), hi.get_mut(i)) {
                    *l = l.min(n);
                    *h = h.max(n);
                }
            }
        }
        Some((lo, hi))
    }

    /// Its size along x, y and z in millimetres.
    pub fn size_mm(&self) -> [f64; 3] {
        match self.bounds() {
            Some((lo, hi)) => [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
            None => [0.0; 3],
        }
    }

    /// The checks every reader ends with: some triangles, and a size a printer could make.
    pub fn checked(self) -> Result<Self, String> {
        if self.triangles.is_empty() {
            return Err("the file has no triangles".into());
        }
        let size = self.size_mm();
        if !size.iter().all(|s| s.is_finite() && *s <= MAX_SIZE_MM) {
            return Err(format!("the model is larger than {MAX_SIZE_MM} mm (check its units)"));
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mesh_measures_itself_and_refuses_bad_numbers() {
        let mut m = Mesh::default();
        assert!(m.push([[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 20.0, 5.0]]).is_ok());
        assert_eq!(m.size_mm(), [10.0, 20.0, 5.0]);
        assert!(m.push([[f32::NAN, 0.0, 0.0], [0.0; 3], [0.0; 3]]).is_err());
        assert!(m.push([[0.0; 3], [f32::INFINITY, 0.0, 0.0], [0.0; 3]]).is_err());
        assert_eq!(m.total, 1);
        assert!(Mesh::default().checked().is_err());
        let mut big = Mesh::default();
        big.push([[0.0; 3], [5000.0, 0.0, 0.0], [0.0; 3]]).unwrap();
        assert!(big.checked().is_ok(), "just at the limit");
        let mut bigger = Mesh::default();
        bigger.push([[0.0; 3], [5001.0, 0.0, 0.0], [0.0; 3]]).unwrap();
        assert!(bigger.checked().is_err(), "just over");
    }
}
