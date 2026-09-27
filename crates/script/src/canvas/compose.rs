use super::*;

fn diagnostic_paint(accent: bool, line: &str) -> crate::shim::ScriptPaint {
    crate::shim::ScriptPaint {
        title: Some("onPaint".into()),
        accent: accent.then(|| "#ff5555".into()),
        lines: vec![line.to_string()],
        buttons: Vec::new(),
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    }
}

fn canvas_only(ops: Vec<CanvasOp>) -> crate::shim::ScriptPaint {
    crate::shim::ScriptPaint {
        title: None,
        accent: None,
        lines: Vec::new(),
        buttons: Vec::new(),
        generation: 0,
        canvas: ops,
        ..Default::default()
    }
}

fn user_has_structured(paint: &crate::shim::ScriptPaint) -> bool {
    paint.title.is_some() || !paint.lines.is_empty() || !paint.buttons.is_empty()
}

/// Compose this call's native recorder with user `Paint.end` state.
/// Wrapper diagnostics are typed (`OnPaintOutcome`); user title/accent/lines
/// are never used as provenance.
pub fn compose_paint(user: Option<crate::shim::ScriptPaint>) -> crate::shim::ScriptPaint {
    let outcome = take_outcome();
    let taken = take();
    match outcome {
        OnPaintOutcome::Missing => diagnostic_paint(true, "no onPaint on bot"),
        OnPaintOutcome::Error(msg) => diagnostic_paint(true, &msg),
        OnPaintOutcome::Success => {
            if let Some(msg) = taken.fail_message() {
                return diagnostic_paint(true, msg);
            }
            if !taken.ops.is_empty() {
                return match user {
                    Some(mut paint) if user_has_structured(&paint) => {
                        paint.canvas = taken.ops;
                        paint
                    }
                    _ => canvas_only(taken.ops),
                };
            }
            match user {
                Some(mut paint) if user_has_structured(&paint) => {
                    paint.canvas = Vec::new();
                    paint
                }
                _ => diagnostic_paint(false, "onPaint ran but Paint.end was not called"),
            }
        }
    }
}
