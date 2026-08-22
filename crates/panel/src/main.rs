fn main() {
    if let Err(e) = panel::run_panel() {
        eprintln!("panel: {e}");
        std::process::exit(1);
    }
}
