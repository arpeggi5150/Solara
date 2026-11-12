//! The start-up splash: the Solara splash image (`assets/splash/`), centred over the window for a
//! moment after launch. A click or key press dismisses it early. Off unless the host turns it on
//! with [`crate::SoundApp::set_splash`], so tests and offscreen renders never see it.

/// How long the splash stays up, in seconds of app uptime.
const SHOW_SECONDS: f64 = 2.0;

/// The splash PNG, embedded so the app needs no asset files at run time.
const SPLASH_PNG: &[u8] = include_bytes!("../../../assets/splash/solara-splash-1280x720.png");

/// Splash state: whether it is on, when it started, and its texture once loaded.
#[derive(Default)]
pub(crate) struct Splash {
    on: bool,
    started: Option<f64>,
    texture: Option<egui::TextureHandle>,
}

impl Splash {
    /// Turn the splash on (before the first frame) or off.
    pub(crate) fn set(&mut self, on: bool) {
        self.on = on;
    }
}

/// Decode the embedded splash. A decode failure only logs: the app starts without a splash.
fn decode() -> Option<egui::ColorImage> {
    match image::load_from_memory_with_format(SPLASH_PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
        }
        Err(e) => {
            log::warn!("splash: {e}");
            None
        }
    }
}

/// Draw the splash while it is due, on top of everything else.
pub(crate) fn show(splash: &mut Splash, uptime: f64, ctx: &egui::Context) {
    if !splash.on {
        return;
    }
    let started = *splash.started.get_or_insert(uptime);
    let dismissed = ctx.input(|i| i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
    if dismissed || uptime - started >= SHOW_SECONDS {
        splash.on = false;
        splash.texture = None;
        ctx.request_repaint();
        return;
    }
    if splash.texture.is_none() {
        match decode() {
            Some(img) => splash.texture = Some(ctx.load_texture("solara-splash", img, egui::TextureOptions::LINEAR)),
            None => {
                splash.on = false;
                return;
            }
        }
    }
    let Some(tex) = splash.texture.as_ref() else { return };
    let screen = ctx.content_rect();
    let [w, h] = tex.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    // Fit inside 70% of the window, never larger than the image itself.
    let fit = (screen.width() * 0.7 / w).min(screen.height() * 0.7 / h);
    let scale = if fit.is_finite() { fit.clamp(0.05, 1.0) } else { 1.0 };
    let rect = egui::Rect::from_center_size(screen.center(), egui::vec2(w * scale, h * scale));
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    egui::Area::new(egui::Id::unique("solara-splash")).order(egui::Order::Foreground).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
        ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
    });
    // Keep frames coming so the splash closes on time without input.
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}

/// Bring the window to the front at launch. Windows can open a new window behind others
/// (seen on R7-Win11), so the first frames ask for focus; if the window still is not focused a
/// second later, it asks for attention instead (the taskbar button flashes). Off unless the host
/// turns it on with [`crate::SoundApp::set_bring_to_front`].
#[derive(Default)]
pub(crate) struct Raise {
    on: bool,
    asked_at: Option<f64>,
    asks: u8,
}

/// Focus requests at start-up, a frame apart, before giving up on focus.
const RAISE_ASKS: u8 = 3;
/// How long to wait for focus before asking for attention instead, in seconds.
const RAISE_WAIT: f64 = 1.0;

impl Raise {
    pub(crate) fn set(&mut self, on: bool) {
        self.on = on;
    }
}

/// Run the start-up raise for one frame.
pub(crate) fn raise(r: &mut Raise, uptime: f64, ctx: &egui::Context) {
    if !r.on {
        return;
    }
    if ctx.input(|i| i.viewport().focused) == Some(true) && r.asks > 0 {
        r.on = false;
        return;
    }
    let asked_at = *r.asked_at.get_or_insert(uptime);
    if r.asks < RAISE_ASKS {
        r.asks = r.asks.saturating_add(1);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    } else if uptime - asked_at >= RAISE_WAIT {
        r.on = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one frame of `show`, dropping the frame's texture uploads as a renderer would.
    fn frame(ctx: &egui::Context, s: &mut Splash, uptime: f64) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(s, uptime, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn the_embedded_splash_decodes_at_its_full_size() {
        let img = decode().unwrap();
        assert_eq!(img.size, [1280, 720]);
    }

    #[test]
    fn the_splash_is_off_until_the_host_turns_it_on() {
        let ctx = egui::Context::default();
        let mut s = Splash::default();
        frame(&ctx, &mut s, 0.0);
        assert!(s.texture.is_none() && s.started.is_none());
    }

    #[test]
    fn the_splash_closes_after_its_time() {
        let ctx = egui::Context::default();
        let mut s = Splash::default();
        s.set(true);
        frame(&ctx, &mut s, 1.0);
        assert!(s.on && s.texture.is_some());
        frame(&ctx, &mut s, 1.0 + SHOW_SECONDS);
        assert!(!s.on && s.texture.is_none());
    }

    /// The viewport commands one frame of `raise` sends, with the window focused or not.
    fn raise_frame(ctx: &egui::Context, r: &mut Raise, uptime: f64, focused: bool) -> Vec<egui::ViewportCommand> {
        let mut input = egui::RawInput::default();
        if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.focused = Some(focused);
        }
        let mut out = ctx.run_ui(input, |ui| raise(r, uptime, ui.ctx()));
        out.textures_delta.clear();
        // Only the raise's own commands: egui adds others (such as the theme) on a first frame.
        let ours = |c: &egui::ViewportCommand| matches!(c, egui::ViewportCommand::Focus | egui::ViewportCommand::RequestUserAttention(_));
        out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.iter().filter(|c| ours(c)).cloned().collect()).unwrap_or_default()
    }

    #[test]
    fn the_window_asks_for_focus_at_launch_then_for_attention_if_still_behind() {
        let ctx = egui::Context::default();
        let mut r = Raise::default();
        assert!(raise_frame(&ctx, &mut r, 0.0, false).is_empty(), "off unless the host turns it on");
        r.set(true);
        for t in [0.0, 0.05, 0.1] {
            assert_eq!(raise_frame(&ctx, &mut r, t, false), [egui::ViewportCommand::Focus]);
        }
        assert!(raise_frame(&ctx, &mut r, 0.5, false).is_empty(), "waits a moment for focus");
        let flash = egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational);
        assert_eq!(raise_frame(&ctx, &mut r, 1.2, false), [flash]);
        assert!(raise_frame(&ctx, &mut r, 2.0, false).is_empty(), "asks once, then leaves the window alone");
    }

    #[test]
    fn a_focused_window_stops_asking() {
        let ctx = egui::Context::default();
        let mut r = Raise::default();
        r.set(true);
        assert_eq!(raise_frame(&ctx, &mut r, 0.0, false), [egui::ViewportCommand::Focus]);
        assert!(raise_frame(&ctx, &mut r, 0.05, true).is_empty());
        assert!(raise_frame(&ctx, &mut r, 2.0, false).is_empty(), "done once it was focused");
    }
}
