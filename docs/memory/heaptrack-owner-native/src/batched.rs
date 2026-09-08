//! Bounded normalized-event SHA batching; no event history is retained.
use sha2::{Digest, Sha256};
#[derive(Clone)]
pub struct BufferedDigest {
    state: Sha256,
    bytes: Vec<u8>,
}
impl BufferedDigest {
    pub fn new() -> Self {
        Self {
            state: Sha256::new(),
            bytes: Vec::with_capacity(65536),
        }
    }
    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        let mut data = data.as_ref();
        while !data.is_empty() {
            let n = data.len().min(65536 - self.bytes.len());
            self.bytes.extend_from_slice(&data[..n]);
            data = &data[n..];
            if self.bytes.len() == 65536 {
                self.state.update(&self.bytes);
                self.bytes.clear();
            }
        }
    }
    pub fn finish(mut self) -> String {
        self.state.update(&self.bytes);
        format!("{:x}", self.state.finalize())
    }
}
