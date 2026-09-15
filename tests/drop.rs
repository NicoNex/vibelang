//! The drop table (docs/static-drop-roadmap.md): where each owned value stops
//! being this scope's to free. The frontend prints it; nothing is emitted yet.

use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn drops(file: &str) -> String {
    let o = Command::new(VIBE)
        .args(["view", file, "--drops"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).expect("utf-8")
}

#[test]
fn a_let_bound_value_nothing_takes_is_dropped_at_the_end_of_its_scope() {
    let d = drops("tests/drop_let.vibe");
    assert!(d.contains("DropLet.keep.body: drop s"), "{d}");
}
