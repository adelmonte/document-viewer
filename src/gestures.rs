//! Touchpad pinch gestures on Wayland, which winit does not deliver on Linux.
//!
//! A second event queue on winit's own Wayland connection binds `zwp_pointer_gestures_v1`
//! and forwards pinch scale changes as zoom factors.

use std::sync::mpsc::{Receiver, Sender, channel};
use wayland_client::backend::Backend;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_pinch_v1::{self, ZwpPointerGesturePinchV1};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gestures_v1::ZwpPointerGesturesV1;

struct State {
    tx: Sender<f32>,
    ctx: egui::Context,
    last_scale: f64,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(_: &mut Self, _: &wl_pointer::WlPointer, _: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpPointerGesturesV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpPointerGesturesV1, _: <ZwpPointerGesturesV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpPointerGesturePinchV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpPointerGesturePinchV1,
        event: zwp_pinch_event::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_pinch_event::Event::Begin { .. } => state.last_scale = 1.0,
            zwp_pinch_event::Event::Update { scale, .. } => {
                if scale > 0.0 && state.last_scale > 0.0 {
                    let factor = (scale / state.last_scale) as f32;
                    state.last_scale = scale;
                    if (factor - 1.0).abs() > 1e-4 {
                        let _ = state.tx.send(factor);
                        state.ctx.request_repaint();
                    }
                }
            }
            zwp_pinch_event::Event::End { .. } => state.last_scale = 1.0,
            _ => {}
        }
    }
}

use zwp_pointer_gesture_pinch_v1 as zwp_pinch_event;

/// Starts listening for pinch gestures. Returns `None` when not on Wayland or when the
/// compositor lacks the protocol.
pub fn start(cc: &eframe::CreationContext<'_>) -> Option<Receiver<f32>> {
    use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let RawDisplayHandle::Wayland(h) = cc.display_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: the display belongs to winit and outlives the app; libwayland supports
    // several event queues reading the same connection from different threads.
    let backend = unsafe { Backend::from_foreign_display(h.display.as_ptr() as *mut _) };
    let conn = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
    let qh = queue.handle();
    let gestures: ZwpPointerGesturesV1 = globals.bind(&qh, 1..=3, ()).ok()?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).ok()?;
    let pointer = seat.get_pointer(&qh, ());
    let pinch = gestures.get_pinch_gesture(&pointer, &qh, ());
    let (tx, rx) = channel();
    let mut state = State { tx, ctx: cc.egui_ctx.clone(), last_scale: 1.0 };
    std::thread::Builder::new()
        .name("pinch".into())
        .spawn(move || {
            let _keep = (gestures, seat, pointer, pinch);
            while queue.blocking_dispatch(&mut state).is_ok() {}
        })
        .ok()?;
    Some(rx)
}
