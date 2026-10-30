Vendored from iced_webview_v2 0.1.12 (Apache-2.0), https://github.com/franzos/iced_webview_v2.

Local changes: keep GPU/WebGL enabled for Mapbox GL JS; locate installed CEF resources beside the executable or via TIMESLAY_CEF_DIR.
Use the parent app’s Wayland-only Iced configuration while enabling the shader renderer. Declare unpublished backend cfg names for check-cfg.
Input fixes: focus Chromium on click; send composed CHAR events for text entry; preserve held mouse-button flags for map panning and forward releases outside the widget.
Wheel events use the actual pointer position so zoom stays anchored under the pointer.

Rendering fixes: retain Chromium's BGRA pixels and upload directly to matching
BGRA GPU textures; convert only the CPU image fallback to RGBA. Build the
webview with opt-level 3 in dev and release. CEF 145 on Wayland ignored the
requested OSR device scale in a live probe. Render the surface in physical
pixels at native DSF 1 and apply matching Chromium page zoom for CSS layout,
text and Mapbox canvas density. Scale pointer coordinates to match. Screen
rectangles now describe the full surface. See examples/map_render_probe.rs for
checks at 1x, 1.25x and 2x, including pointer alignment and actual painted frames.

Give each engine a private temporary root_cache_path, retained until CEF
shutdown, so Chromium's process singleton does not redirect map startup to a
different TimeSlay process. Startup errors no longer assume missing resources.
Run `cargo run --example map_render_probe -- --concurrent` to check overlapping
CEF lifetimes, including rendering on the dock task editor's worker thread.
