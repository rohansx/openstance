//! `HashEmbedder`: deterministic vectors from token hashes. Used by CI and
//! tests so search and fusion are fully exercised without a model download
//! (models.md section 2.1). Not a real semantic model: texts that share
//! words (or 5-character word prefixes) land closer together.

use crate::{EmbedKind, Embedder, Result};

/// Feature hashing of lowercase alphanumeric tokens and their 5-character
/// prefixes into `dims` buckets with a sign bit, then L2-normalized. Empty
/// text maps to a fixed unit vector so cosine stays defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashEmbedder {
    id: String,
    dims: usize,
}

impl HashEmbedder {
    /// A hash embedder with `dims` dimensions (id `hash:<dims>`).
    pub fn new(dims: usize) -> Self {
        let dims = dims.max(1);
        HashEmbedder {
            id: format!("hash:{dims}"),
            dims,
        }
    }

    /// One vector.
    pub fn vector(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dims];
        let mut any = false;
        for tok in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let tok = tok.to_lowercase();
            self.add(&mut v, tok.as_bytes(), 1.0);
            let prefix: String = tok.chars().take(5).collect();
            if prefix.len() < tok.len() {
                self.add(&mut v, format!("p:{prefix}").as_bytes(), 0.5);
            }
            any = true;
        }
        if !any {
            v[0] = 1.0;
        }
        super::l2_normalize(&mut v);
        v
    }

    fn add(&self, v: &mut [f32], bytes: &[u8], w: f32) {
        let h = fnv1a(bytes);
        let i = usize::try_from(h % self.dims as u64).unwrap_or(0);
        let sign = if (h >> 63) & 1 == 1 { -1.0 } else { 1.0 };
        v[i] += sign * w;
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Embedder for HashEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn embed(&self, texts: &[&str], _kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.vector(t)).collect())
    }
}
