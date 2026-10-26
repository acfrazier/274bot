use super::paint::PaintView;
use script::{canvas::CanvasOp, paint::ScriptPaint};

#[test]
fn rasterize_fill_rect_is_some() {
    #[cfg(all(feature = "default", not(feature = "canvas-raster")))]
    compile_error!("default frontend builds must enable canvas-raster");

    let paint = ScriptPaint {
        canvas: vec![CanvasOp::fill_rect(4, 7, 13, 17, 0xff00_00ff)],
        ..Default::default()
    };
    let ops = PaintView::new(&paint).canvas();

    #[cfg(feature = "canvas-raster")]
    {
        assert!(ops.rasterize().is_some());
        assert_eq!(ops.dirty_bounds(), Some([4, 7, 13, 17]));
    }

    #[cfg(not(feature = "canvas-raster"))]
    {
        assert!(ops.rasterize().is_none());
        assert_eq!(ops.dirty_bounds(), None);
    }
}
