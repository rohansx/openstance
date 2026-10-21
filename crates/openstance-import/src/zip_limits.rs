//! Zip-bomb guard shared by the importers that read archives (`x_archive`,
//! `linkedin_csv`): a reader that fails once one entry or the whole archive
//! has produced more decompressed bytes than allowed. Entries are streamed
//! from the archive and never written to disk.

use std::cell::Cell;
use std::io::Read;
use std::rc::Rc;

/// A reader that fails once an entry or the whole archive exceeds its cap.
pub(crate) struct Limited<R> {
    inner: R,
    entry_left: u64,
    total: Rc<Cell<u64>>,
    total_max: u64,
}

impl<R: Read> Limited<R> {
    /// Wrap `inner`, allowing `entry_max` bytes from it and `total_max`
    /// bytes across every reader sharing `total`.
    pub(crate) fn new(inner: R, entry_max: u64, total: Rc<Cell<u64>>, total_max: u64) -> Self {
        Limited {
            inner,
            entry_left: entry_max,
            total,
            total_max,
        }
    }
}

impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let n64 = n as u64;
        if n64 > self.entry_left {
            return Err(std::io::Error::other("archive entry too large"));
        }
        self.entry_left -= n64;
        let t = self.total.get() + n64;
        if t > self.total_max {
            return Err(std::io::Error::other("archive too large"));
        }
        self.total.set(t);
        Ok(n)
    }
}
