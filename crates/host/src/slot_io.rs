use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

use client::client::{present::pack_rgb, GameShell, APPLET_H, APPLET_W};

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

pub enum InputEv {
    Move { x: i32, y: i32 },
    Down { button: i32, x: i32, y: i32 },
    Up,
    Key { down: bool, ch: i32 },
}

pub fn map_image_to_applet(local_x: f32, local_y: f32, image_w: f32, image_h: f32) -> Option<(i32, i32)> {
    if image_w <= 0.0 || image_h <= 0.0 {
        return None;
    }
    if local_x < 0.0 || local_y < 0.0 || local_x > image_w || local_y > image_h {
        return None;
    }
    let x = ((local_x / image_w) * APPLET_W as f32).floor() as i32;
    let y = ((local_y / image_h) * APPLET_H as f32).floor() as i32;
    let x = x.clamp(0, APPLET_W - 1);
    let y = y.clamp(0, APPLET_H - 1);
    Some((x, y))
}

pub struct SlotInput {
    enabled: AtomicBool,
    rx: Mutex<Option<Receiver<InputEv>>>,
}

impl SlotInput {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            enabled: AtomicBool::new(false),
            rx: Mutex::new(None),
        })
    }
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn connect_rx(&self, rx: Receiver<InputEv>) {
        *self.rx.lock().unwrap() = Some(rx);
    }
    pub fn disconnect_rx(&self) {
        *self.rx.lock().unwrap() = None;
    }
    pub fn drain(&self, shell: &mut GameShell) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let mut g = self.rx.lock().unwrap();
        let Some(rx) = g.as_mut() else { return; };
        while let Ok(ev) = rx.try_recv() {
            match ev {
                InputEv::Move { x, y } => shell.apply_mouse_move(x, y),
                InputEv::Down { button, x, y } => shell.apply_mouse_down(button, x, y),
                InputEv::Up => shell.apply_mouse_up(),
                InputEv::Key { down, ch } => shell.apply_key(down, 0, ch),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{map_image_to_applet, InputEv, PixelBuf, SlotInput};

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

    #[test]
    fn map_image_to_applet_scales_and_rejects_outside() {
        assert_eq!(map_image_to_applet(0.0, 0.0, 1530.0, 1006.0), Some((0, 0)));
        assert_eq!(map_image_to_applet(1530.0, 1006.0, 1530.0, 1006.0), Some((764, 502)));
        assert_eq!(map_image_to_applet(-1.0, 10.0, 765.0, 503.0), None);
    }

    #[test]
    fn drain_skips_recv_when_disabled_then_applies_when_enabled() {
        use std::sync::mpsc;
        let inp = SlotInput::new();
        let (tx, rx) = mpsc::channel();
        inp.connect_rx(rx);
        tx.send(InputEv::Down { button: 1, x: 10, y: 10 }).unwrap();
        let mut shell = client::client::GameShell::new();
        inp.set_enabled(false);
        inp.drain(&mut shell);
        shell.latch_click();
        assert_eq!(shell.mouse_click_button, 0);
        inp.set_enabled(true);
        inp.drain(&mut shell);
        shell.latch_click();
        assert_eq!((shell.mouse_click_button, shell.mouse_click_x), (1, 10));
    }
}
