//! sqlite-vec: statically linked and registered on every connection
//! (P2-CORE-01). The registration is the single `unsafe` call allowed in
//! our crates (docs/engineering/conventions.md, rule 11).

use std::sync::Once;

static REGISTER: Once = Once::new();

/// Register sqlite-vec as an auto-extension, once per process, so every
/// connection opened afterwards has `vec0`, `vec_distance_cosine`, ...
/// Called by [`super::open`] and [`super::open_in_memory`] before opening.
pub fn register() {
    REGISTER.call_once(|| {
        // SAFETY: `sqlite3_vec_init` is the extension entry point compiled
        // into this binary by the `sqlite-vec` crate. SQLite calls it with
        // the documented `(db, pzErrMsg, pApi)` signature for each new
        // connection; the fn-pointer transmute only restores that signature
        // (the crate declares it as `fn()`), which is the registration
        // pattern the sqlite-vec crate itself documents and tests.
        #[allow(unsafe_code)]
        unsafe {
            type Entry = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int;
            let entry: Entry =
                std::mem::transmute::<*const (), Entry>(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(entry));
        }
    });
}

/// `f32` vector -> the little-endian blob sqlite-vec expects.
pub fn to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Inverse of [`to_blob`].
pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn every_connection_has_sqlite_vec() {
        let c = open_in_memory().unwrap();
        let v: String = c
            .query_row("SELECT vec_version()", [], |r| r.get(0))
            .unwrap();
        assert!(v.starts_with('v'), "{v}");
    }

    #[test]
    fn vec_distance_cosine_smoke() {
        let c = open_in_memory().unwrap();
        let a = to_blob(&[1.0, 0.0, 0.0]);
        let b = to_blob(&[0.0, 1.0, 0.0]);
        let same: f64 = c
            .query_row("SELECT vec_distance_cosine(?1, ?1)", [&a], |r| r.get(0))
            .unwrap();
        let ortho: f64 = c
            .query_row("SELECT vec_distance_cosine(?1, ?2)", [&a, &b], |r| r.get(0))
            .unwrap();
        assert!(same.abs() < 1e-6);
        assert!((ortho - 1.0).abs() < 1e-6);
    }

    #[test]
    fn blob_round_trip() {
        let v = vec![0.25f32, -1.5, 3.0];
        assert_eq!(from_blob(&to_blob(&v)), v);
    }
}
