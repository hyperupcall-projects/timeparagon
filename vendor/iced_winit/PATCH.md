# Local Iced resize fix

Source: `iced_winit` 0.14.1 from crates.io, MIT licensed (see LICENSE).
Only `src/lib.rs`, in the `window::Action::Resize` branch, differs from upstream.

Winit's `Window::request_inner_size` returns `Some(PhysicalSize)` when a resize
is applied immediately; a later `WindowEvent::Resized` is not guaranteed. The
upstream Iced branch discards this result. On Wayland this can leave Iced's
viewport and GPU/software surface at their previous size while the compositor
stretches the old buffer to the new window size.

This patch feeds the applied size through Iced's existing window-state update
and event-conversion paths, then requests a redraw. The next draw relays out
widgets and configures the render surface at the new physical dimensions. When
the backend returns `None`, Iced continues waiting for the native resize event.
An unchanged returned size is ignored; any later native resize event is still
processed normally. Physical dimensions are converted with the current scale
factor, including fractional display scaling.

Regression probe: `cargo run --example resize_probe` on a desktop session.
It expands, contracts, and expands a temporary window, checking both Iced's
logical viewport and the screenshot buffer dimensions after every resize.
It does not read application settings or connect to Google.
