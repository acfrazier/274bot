//! Task 7: Load is out-of-tree file picker, not catalog import.

use script::script_file_path;

#[test]
fn script_file_path_still_rejects_escape_for_catalog() {
    let root = std::path::Path::new("/tmp/fake-rs2b0t");
    assert!(script_file_path(root, "../../etc/passwd").is_none());
    assert!(script_file_path(root, "../evil.ts").is_none());
}
