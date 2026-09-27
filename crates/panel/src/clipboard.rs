//! Native OS clipboard backend for Dear ImGui.
//!
//! `dear-imgui-sys` 0.15 builds with `IMGUI_DISABLE_WIN32_FUNCTIONS` and
//! `IMGUI_DISABLE_OSX_FUNCTIONS`, so Dear ImGui's PlatformIO default clipboard
//! handlers are the **local-only** `ClipboardHandlerData` buffer — not the OS
//! pasteboard. Password-manager paste therefore never reaches InputText
//! (password fields still allow paste; cut/copy stay suppressed).
//!
//! Installing [`NativeClipboard`] via
//! [`dear_imgui_rs::Context::set_clipboard_backend`] replaces those handlers
//! with OS-backed get/set. Failures return `None` / no-op — never panic, never
//! log clipboard contents.

use arboard::Clipboard;
use dear_imgui_rs::ClipboardBackend;

/// System clipboard via `arboard` (macOS / Windows / Linux).
///
/// Construction is fallible at the OS layer; callers use [`NativeClipboard::try_new`]
/// and skip install when the platform clipboard cannot be opened.
pub struct NativeClipboard {
    inner: Clipboard,
}

impl NativeClipboard {
    /// Open the platform clipboard. `None` if the OS clipboard is unavailable.
    pub fn try_new() -> Option<Self> {
        match Clipboard::new() {
            Ok(inner) => Some(Self { inner }),
            Err(_) => None,
        }
    }
}

impl ClipboardBackend for NativeClipboard {
    fn get(&mut self) -> Option<String> {
        match self.inner.get_text() {
            Ok(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    fn set(&mut self, value: &str) {
        let _ = self.inner.set_text(value.to_owned());
    }
}

/// Install the native backend on an ImGui context when the OS clipboard opens.
///
/// Returns whether a backend was installed. Safe to call once after
/// `Context::create`; the context owns the backend for the window lifetime.
pub fn install_native_clipboard(context: &mut dear_imgui_rs::Context) -> bool {
    match NativeClipboard::try_new() {
        Some(backend) => {
            context.set_clipboard_backend(backend);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dear_imgui_rs as imgui;
    use std::sync::{Arc, Mutex};

    /// In-memory fake: no OS clipboard touch, non-secret fixture text only.
    #[derive(Clone)]
    struct FakeClipboard {
        value: Arc<Mutex<Option<String>>>,
    }

    impl ClipboardBackend for FakeClipboard {
        fn get(&mut self) -> Option<String> {
            self.value.lock().unwrap().clone()
        }

        fn set(&mut self, text: &str) {
            *self.value.lock().unwrap() = Some(text.to_owned());
        }
    }

    #[test]
    fn fake_backend_roundtrip_via_imgui_clipboard_api() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = imgui::Context::create();
        let store = Arc::new(Mutex::new(None));
        ctx.set_clipboard_backend(FakeClipboard {
            value: store.clone(),
        });

        let fixture = "panel-paste-fixture-not-a-secret";
        ctx.set_clipboard_text(fixture);
        assert_eq!(ctx.clipboard_text().as_deref(), Some(fixture));
        assert_eq!(store.lock().unwrap().as_deref(), Some(fixture));
    }

    #[test]
    fn platform_io_callbacks_installed_after_set_clipboard_backend() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = imgui::Context::create();
        let raw = ctx.as_raw();

        // Default C++ handlers exist but are local-buffer only (OS funcs
        // disabled in dear-imgui-sys). UserData starts null until Rust install.
        let default_get;
        let default_set;
        unsafe {
            let pio = imgui::sys::igGetPlatformIO_ContextPtr(raw);
            default_get = (*pio).Platform_GetClipboardTextFn;
            default_set = (*pio).Platform_SetClipboardTextFn;
            assert!(
                default_get.is_some() && default_set.is_some(),
                "Dear ImGui still installs DefaultImpl PlatformIO clipboard fns"
            );
            assert!(
                (*pio).Platform_ClipboardUserData.is_null(),
                "default clipboard user data is null (local buffer, no Rust backend)"
            );
        }

        // Default path is local-only: set then get works without OS clipboard.
        ctx.set_clipboard_text("local-only-buffer");
        assert_eq!(
            ctx.clipboard_text().as_deref(),
            Some("local-only-buffer"),
            "DefaultImpl keeps an ImGui-private buffer"
        );

        let fake_store = Arc::new(Mutex::new(Some("paste-me".into())));
        ctx.set_clipboard_backend(FakeClipboard {
            value: fake_store.clone(),
        });

        unsafe {
            let pio = imgui::sys::igGetPlatformIO_ContextPtr(raw);
            assert!(
                (*pio).Platform_GetClipboardTextFn.is_some()
                    && (*pio).Platform_SetClipboardTextFn.is_some(),
                "set_clipboard_backend must keep PlatformIO clipboard fns set"
            );
            assert!(
                !(*pio).Platform_ClipboardUserData.is_null(),
                "user data pointer must stay live with the context-owned backend"
            );
            let _ = (default_get, default_set);
        }

        // After install, reads go through the Rust fake backend, not the
        // local DefaultImpl buffer written earlier.
        assert_eq!(ctx.clipboard_text().as_deref(), Some("paste-me"));
        ctx.set_clipboard_text("from-imgui-api");
        assert_eq!(ctx.clipboard_text().as_deref(), Some("from-imgui-api"));
        assert_eq!(
            fake_store.lock().unwrap().as_deref(),
            Some("from-imgui-api"),
            "set path must land in the installed Rust backend store"
        );
    }

    fn prep(ctx: &mut imgui::Context) {
        ctx.prepare_frame(
            imgui::FramePrepareOptions::new([400.0, 200.0], 1.0 / 60.0).renderer_has_textures(),
        );
    }

    /// One frame: optional focus + draw password field into `buf`.
    fn password_frame(ctx: &mut imgui::Context, buf: &mut String, focus: bool) {
        prep(ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("pass-probe")
                .position([0.0, 0.0], imgui::Condition::Always)
                .size([400.0, 200.0], imgui::Condition::Always)
                .build(|| {
                    if focus {
                        ui.set_keyboard_focus_here();
                    }
                    ui.input_text("##vault-pass", buf).password(true).build();
                });
        }
        ctx.render();
    }

    /// Masked InputText: Ctrl/Cmd chord select-all then replace, and paste via
    /// the installed clipboard backend. Exercises ImGui shortcut behavior, not
    /// OS event delivery. Fixture text only; never logs contents.
    #[test]
    fn masked_inputtext_select_all_and_paste_via_shortcut_chords() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = imgui::Context::create();
        let macos = ctx.io().config_macosx_behaviors();

        let fixture = "ux-fixture-only";
        let store = Arc::new(Mutex::new(Some(fixture.to_string())));
        ctx.set_clipboard_backend(FakeClipboard {
            value: store.clone(),
        });

        let mut buf = String::new();
        // Focus request applies to the next item on this frame; activation is
        // ready on the subsequent frame for character input.
        password_frame(&mut ctx, &mut buf, true);
        password_frame(&mut ctx, &mut buf, false);

        // Seed five dummy chars (mirrors headed typeText into vault-pass).
        for ch in ['a', 'b', 'c', 'd', 'e'] {
            ctx.io_mut().add_input_character(ch);
        }
        password_frame(&mut ctx, &mut buf, false);
        assert_eq!(buf.len(), 5, "seed typing must land in the masked field");

        // Select-all: on macOS ConfigMacOSXBehaviors swaps Super↔Ctrl at
        // AddKeyEvent, so physical Super (Cmd) is submitted as ModSuper.
        // Off-mac, submit ModCtrl. Chord is ImGuiMod_Ctrl|A either way after swap.
        let chord_mod = if macos {
            imgui::Key::ModSuper
        } else {
            imgui::Key::ModCtrl
        };
        ctx.io_mut().add_key_event(chord_mod, true);
        ctx.io_mut().add_key_event(imgui::Key::A, true);
        password_frame(&mut ctx, &mut buf, false);
        ctx.io_mut().add_key_event(imgui::Key::A, false);
        ctx.io_mut().add_key_event(chord_mod, false);
        password_frame(&mut ctx, &mut buf, false);

        // Replacement after select-all must replace, not append.
        ctx.io_mut().add_input_character('q');
        password_frame(&mut ctx, &mut buf, false);
        assert_eq!(
            buf,
            "q",
            "select-all + type must replace seeded text (got len {})",
            buf.len()
        );

        // Paste fixture over selection: select-all again, then Ctrl/Cmd+V.
        ctx.io_mut().add_key_event(chord_mod, true);
        ctx.io_mut().add_key_event(imgui::Key::A, true);
        password_frame(&mut ctx, &mut buf, false);
        ctx.io_mut().add_key_event(imgui::Key::A, false);
        ctx.io_mut().add_key_event(imgui::Key::V, true);
        password_frame(&mut ctx, &mut buf, false);
        ctx.io_mut().add_key_event(imgui::Key::V, false);
        ctx.io_mut().add_key_event(chord_mod, false);
        password_frame(&mut ctx, &mut buf, false);

        assert_eq!(
            buf, fixture,
            "Cmd/Ctrl+V must paste backend fixture into password field"
        );
        // Ensure backend still holds fixture (no secret logging path).
        assert_eq!(store.lock().unwrap().as_deref(), Some(fixture));
    }
}
