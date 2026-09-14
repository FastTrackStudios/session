//! Unlocking WebKit's 60 fps ceiling.
//!
//! WebKit caps *page rendering updates* — `requestAnimationFrame`
//! included — at about 60 fps, deliberately. It is a preference,
//! `PreferPageRenderingUpdatesNear60FPSEnabled`, which defaults ON, and
//! WebKit's own explainer gives the reasons: measured power cost, and
//! pages that misbehave when rAF fires at anything but 60 Hz.
//!
//! The two ports implement the same target differently, which is why it
//! looks like two different bugs:
//!
//! - **WebKitGTK** drives it from a fixed ~16 ms timer → 62.5 fps,
//!   whatever the display is doing. Measured here at 62.5 on a 240 Hz
//!   panel, with Chromium on the same compositor reaching 240.
//! - **macOS** divides the display rate down to the nearest value that
//!   does not undershoot 60 → 72 fps on a 144 Hz panel (144/3 = 48 would
//!   be too slow). Measured on macOS 27.
//!
//! Neither is a hardware or compositor limit. This turns the preference
//! off for our window.
//!
//! # It does not work on Linux, and that is the point of keeping it
//!
//! MEASURED, 2026-09-11, WebKitGTK 2.52.6: the feature is found and
//! disabled (the log line below fires), and the rAF rate does not move —
//! 62.1 before, 62.1 after, and 62.1 after forcing a page reload so the
//! web process re-reads its features. The GTK port's ~16 ms cadence is
//! independent of this preference; the flag governs the rate on Safari
//! and WKWebView, where WebKit bug 173434 reports 120 Hz+ once it is off.
//!
//! This is kept, working and honest, because "just turn off the 60fps
//! preference" is the obvious next idea for anyone who meets this
//! ceiling, and finding out it is a no-op costs an afternoon. It is not
//! kept because it helps.
//!
//! # Why this is raw FFI
//!
//! WebKitGTK 2.42+ exposes the flag through the public feature API
//! (`webkit_settings_set_feature_enabled` over the list from
//! `webkit_settings_get_all_features`), and our build carries it. The
//! `webkit2gtk` Rust crate wry depends on binds neither, so the five
//! functions are declared here. They resolve against the
//! `libwebkit2gtk-4.1` the process has already loaded.
//!
//! # What this does NOT fix
//!
//! Only the *script-driven* path is capped. Accelerated, compositor-
//! driven animations — CSS transitions and animations that Core
//! Animation (or its GTK equivalent) owns — already run at the display's
//! refresh rate. That is why the playhead is a CSS animation rather than
//! a `transform` written from a rAF callback: it is smooth at full
//! refresh whether or not this succeeds. See `daw_ui::studio::clock`.

use std::ffi::{CStr, c_char, c_int, c_void};

/// The feature that caps rendering updates near 60 fps.
///
/// Note the name: the *preference key* inside WebKit is
/// `PreferPageRenderingUpdatesNear60FPSEnabled`, but the identifier the
/// public feature list exposes drops the `Enabled` suffix. Matching the
/// preference key finds nothing among the 486 features and looks exactly
/// like a WebKit that has no such setting.
const FEATURE: &str = "PreferPageRenderingUpdatesNear60FPS";

// SAFETY: these are WebKitGTK's own C entry points, resolved from the
// library wry has already loaded into this process. `gsize` is `usize`
// and `gboolean` is `gint` (i32) — NOT `bool`, which is a one-byte type
// and would pass rubbish in the high bytes.
unsafe extern "C" {
    fn webkit_settings_get_all_features() -> *mut c_void;
    fn webkit_feature_list_get_length(list: *mut c_void) -> usize;
    fn webkit_feature_list_get(list: *mut c_void, index: usize) -> *mut c_void;
    fn webkit_feature_get_identifier(feature: *mut c_void) -> *const c_char;
    fn webkit_settings_set_feature_enabled(
        settings: *mut c_void,
        feature: *mut c_void,
        enabled: c_int,
    );
}

/// Turn the cap off for this window's webview.
///
/// Best-effort: a build without the feature, or a future WebKit that has
/// renamed it, means the window runs at 60 rather than failing to open.
/// Reports what happened either way — a silent no-op here would look
/// exactly like the cap being unfixable.
pub fn unlock(webview: &webkit2gtk::WebView) {
    use webkit2gtk::WebViewExt;
    use webkit2gtk::glib::translate::ToGlibPtr;

    // A webview always has settings; `None` would mean the widget is not
    // really a webview, which is not a case worth a panic.
    let Some(settings) = webview.settings() else {
        tracing::warn!("webview has no settings; leaving the 60fps cap in place");
        return;
    };
    let settings_ptr: *mut c_void =
        ToGlibPtr::<*mut webkit2gtk::ffi::WebKitSettings>::to_glib_none(&settings)
            .0
            .cast();

    // SAFETY: the pointers come from WebKitGTK itself and are used only
    // for the lifetime of this call; `settings` outlives it.
    unsafe {
        let list = webkit_settings_get_all_features();
        if list.is_null() {
            tracing::warn!("webkit exposes no feature list; leaving the 60fps cap in place");
            return;
        }
        let len = webkit_feature_list_get_length(list);
        for i in 0..len {
            let feature = webkit_feature_list_get(list, i);
            if feature.is_null() {
                continue;
            }
            let id = webkit_feature_get_identifier(feature);
            if id.is_null() {
                continue;
            }
            if CStr::from_ptr(id).to_bytes() == FEATURE.as_bytes() {
                webkit_settings_set_feature_enabled(settings_ptr, feature, 0);
                // Deliberately not phrased as a fix: on this port it
                // changes nothing measurable. See the module docs.
                tracing::info!(
                    webkit.feature = FEATURE,
                    webkit.enabled = false,
                    "webkit 60fps preference turned off (no effect on the GTK port)"
                );
                return;
            }
        }
        tracing::warn!(
            webkit.feature = FEATURE,
            webkit.features_seen = len,
            "feature not found; the window will run at WebKit's 60fps default"
        );
    }
}
