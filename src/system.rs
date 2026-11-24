//! The desktop's light/dark preference, read from the XDG settings portal.

use std::sync::atomic::{AtomicU8, Ordering};
use zbus::zvariant::{OwnedValue, Value};

const UNKNOWN: u8 = 0;
const DARK: u8 = 1;
const LIGHT: u8 = 2;

static SCHEME: AtomicU8 = AtomicU8::new(UNKNOWN);

fn as_u32(v: &Value) -> Option<u32> {
    match v {
        Value::U32(n) => Some(*n),
        Value::Value(inner) => as_u32(inner),
        _ => None,
    }
}

fn store(v: &Value) {
    // 1 = prefer dark, 2 = prefer light, 0 = no preference (treated as light).
    if let Some(n) = as_u32(v) {
        SCHEME.store(if n == 1 { DARK } else { LIGHT }, Ordering::Relaxed);
    }
}

/// `Some(true)` when the desktop prefers dark, `None` when the portal could not be reached.
pub fn prefers_dark() -> Option<bool> {
    match SCHEME.load(Ordering::Relaxed) {
        DARK => Some(true),
        LIGHT => Some(false),
        _ => None,
    }
}

/// Reads the preference once and keeps following changes in a background thread.
pub fn watch(ctx: egui::Context) {
    std::thread::Builder::new()
        .name("color-scheme".into())
        .spawn(move || {
            let _ = run(&ctx);
        })
        .ok();
}

fn run(ctx: &egui::Context) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )?;
    let signals = proxy.receive_signal("SettingChanged")?;
    let initial: zbus::Result<OwnedValue> = proxy.call("ReadOne", &("org.freedesktop.appearance", "color-scheme"));
    let initial = match initial {
        Ok(v) => Ok(v),
        Err(_) => proxy.call("Read", &("org.freedesktop.appearance", "color-scheme")),
    };
    if let Ok(v) = initial {
        store(&v);
        ctx.request_repaint();
    }
    for msg in signals {
        if let Ok((ns, key, v)) = msg.body().deserialize::<(String, String, OwnedValue)>()
            && ns == "org.freedesktop.appearance"
            && key == "color-scheme"
        {
            store(&v);
            ctx.request_repaint();
        }
    }
    Ok(())
}
