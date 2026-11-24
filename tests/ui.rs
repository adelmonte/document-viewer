//! Drives the real application UI headlessly through its accessibility tree.

use document_viewer::app::{App, Startup};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::path::PathBuf;
use std::time::Duration;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn isolate() {
    let dir = std::env::temp_dir().join(format!("document-viewer-ui-test-{}", std::process::id()));
    // SAFETY: called before any other thread is started by the test.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("XDG_STATE_HOME", dir.join("state"));
    }
}

fn harness(file: Option<&str>) -> Harness<'static, App> {
    isolate();
    let startup = Startup { file: file.map(fixture), page: None, find: None, fullscreen: false, presentation: false };
    Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|cc| App::new(cc, startup))
}

/// Steps the UI until `cond` holds, giving background threads time to work.
fn wait_for(h: &mut Harness<'static, App>, what: &str, cond: impl Fn(&Harness<'static, App>) -> bool) {
    for _ in 0..300 {
        h.step();
        if cond(h) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let labels: Vec<String> = h
        .root()
        .children_recursive()
        .filter_map(|n| {
            let a = n.accesskit_node();
            let l = a.label().map(|l| l.to_string()).unwrap_or_default();
            let v = a.value().map(|v| format!(" = {v:?}")).unwrap_or_default();
            (!l.is_empty() || !v.is_empty()).then(|| format!("{:?} {l}{v}", a.role()))
        })
        .collect();
    panic!("timed out waiting for {what}; on screen:\n{}", labels.join("\n"));
}

#[test]
fn ui_end_to_end() {
    let mut h = harness(Some("sample.pdf"));
    wait_for(&mut h, "document to load", |h| h.query_by_label("(1 of 3)").is_some());

    // Page labels from the PDF show in the page entry.
    assert_eq!(h.get_by_label("Page number").value().as_deref(), Some("i"));
    assert!(h.get_by_label("Previous page").accesskit_node().is_disabled());
    h.key_press(Key::N);
    wait_for(&mut h, "next page", |h| h.get_by_label("Page number").value().as_deref() == Some("2"));
    assert!(h.query_by_label("of 3").is_some());

    // Side panel with thumbnails and outline.
    h.key_press(Key::F9);
    wait_for(&mut h, "sidebar", |h| h.query_by_label("Outline").is_some());
    h.get_by_label("Outline").click();
    wait_for(&mut h, "outline entries", |h| h.query_by_label("Chapter 1").is_some());

    // Context menu on the page canvas.
    h.get_by_label("Document").click_secondary();
    wait_for(&mut h, "context menu", |h| h.query_by_label_contains("Select All").is_some());
    assert!(h.query_by_label_contains("Fit Width").is_some());
    h.key_press(Key::Escape);
    h.step();

    // Find.
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    wait_for(&mut h, "find bar", |h| h.query_by_label("Match case").is_some());
    h.event(egui::Event::Text("findme".into()));
    wait_for(&mut h, "search results", |h| h.query_by_label("1 of 1").is_some());

    // Main menu.
    h.get_by_label("Main menu").click();
    wait_for(&mut h, "main menu", |h| h.query_by_label_contains("Keyboard Shortcuts").is_some());
    h.get_by_label_contains("Keyboard Shortcuts").click();
    wait_for(&mut h, "shortcuts dialog", |h| h.query_by_label("Moving around").is_some());
}

#[test]
fn empty_window_and_formats() {
    let mut h = harness(None);
    h.step();
    assert!(h.query_by_label("Open a Document…").is_some());

    for f in ["sample.djvu", "sample.cbz", "sample.tiff", "sample.pdf.gz"] {
        let mut h = harness(Some(f));
        wait_for(&mut h, f, |h| h.query_by_label_contains("of ").is_some());
        h.key_press(Key::End);
        h.key_press(Key::D);
        h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowRight);
        for _ in 0..20 {
            h.step();
        }
    }
}
