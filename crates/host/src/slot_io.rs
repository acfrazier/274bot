use std::sync::Mutex;

use client::client::{present::pack_rgb, APPLET_H, APPLET_W};

pub struct PixelBuf {
    inner: Mutex<Vec<u32>>,
}

impl PixelBuf {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { inner: Mutex::new(Vec::new()) })
    }
    pub fn copy_from(&self, pix: &[i32], w: i32, h: i32) {
        if w != APPLET_W || h != APPLET_H || pix.len() < (APPLET_W * APPLET_H) as usize {
            return;
        }
        let n = (APPLET_W * APPLET_H) as usize;
        let mut g = self.inner.lock().unwrap();
        g.resize(n, 0);
        for (dst, src) in g.iter_mut().zip(pix.iter().take(n)) {
            *dst = pack_rgb(*src);
        }
    }
    pub fn snapshot(&self) -> Vec<u32> {
        self.inner.lock().unwrap().clone()
    }
}

pub struct SlotInput; // Task 2 replaces this

#[cfg(test)]
mod tests {
    use super::PixelBuf;

    #[test]
    fn copy_from_packs_rgb_and_is_765_by_503() {
        use client::client::present::pack_rgb;
        let buf = PixelBuf::new();
        let mut pix = vec![0i32; 765 * 503];
        pix[0] = 0x00aa_bbcc;
        pix[1] = 0x0011_2233;
        buf.copy_from(&pix, 765, 503);
        let out = buf.snapshot();
        assert_eq!(out.len(), 765 * 503);
        assert_eq!(out[0], pack_rgb(0x00aa_bbcc));
        assert_eq!(out[1], pack_rgb(0x0011_2233));
    }

    #[test]
    fn copy_from_wrong_len_is_ignored() {
        let buf = PixelBuf::new();
        buf.copy_from(&[1, 2, 3], 2, 2);
        assert!(buf.snapshot().is_empty());
    }
}
