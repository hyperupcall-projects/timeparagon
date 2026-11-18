//! The More sheet: a picture tile for each of the app's other pages.

use dioxus::prelude::*;
use timeparagon_core::more_menu::{Item, PHONE_ROWS, Shape};

#[component]
pub fn MoreSheet(
	status: String,
	on_close: EventHandler<()>,
	on_open: EventHandler<Item>,
) -> Element {
	let tiles = PHONE_ROWS.iter().flat_map(|row| {
		let wide = row.len() == 1;
		row.iter().map(move |&item| (item, wide))
	});
	rsx! {
		div {
			class: "modal-backdrop",
			onclick: move |_| on_close.call(()),
			div {
				class: "modal",
				role: "dialog",
				aria_label: "More",
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					h2 { class: "modal-title", "More" }
					div { class: "spacer" }
					button {
						class: "modal-close",
						title: "Close",
						onclick: move |_| on_close.call(()),
						"×"
					}
				}
				div { class: "modal-body more-body",
					div { class: "more-grid",
						for (item, wide) in tiles {
							button {
								key: "{item.slug()}",
								class: if wide { "more-tile more-tile-wide" } else { "more-tile" },
								style: "--tile:var(--more-{item.slug()});--tile-ink:var(--more-ink-{item.slug()});",
								onclick: move |_| on_open.call(item),
								if wide {
									span { class: "more-backdrop", {picture(item, 92)} }
								}
								span { class: "more-badge", {picture(item, 26)} }
								span { class: "more-label", "{item.label()}" }
							}
						}
					}
					p { class: "hint", "{status}" }
				}
			}
		}
	}
}

fn picture(item: Item, size: u32) -> Element {
	rsx! {
		svg {
			width: "{size}", height: "{size}", view_box: "0 0 24 24",
			fill: "none", stroke: "currentColor", stroke_width: "2",
			stroke_linecap: "round", stroke_linejoin: "round",
			"aria-hidden": "true", "focusable": "false",
			for shape in item.picture() {
				path { d: svg_path(*shape) }
			}
		}
	}
}

/// The stroke as SVG path data.
fn svg_path(shape: Shape) -> String {
	let through = |points: &[(f32, f32)]| {
		points
			.iter()
			.enumerate()
			.map(|(index, (x, y))| format!("{}{x} {y}", if index == 0 { "M" } else { "L" }))
			.collect::<Vec<_>>()
			.join(" ")
	};
	match shape {
		Shape::Lines(points) => through(points),
		Shape::Polygon(points) => format!("{} Z", through(points)),
		Shape::Circle { x, y, radius: r } => {
			let left = x - r;
			let across = r * 2.0;
			format!("M{left} {y} a{r} {r} 0 1 1 {across} 0 a{r} {r} 0 1 1 -{across} 0")
		}
		Shape::Arc {
			x,
			y,
			radius: r,
			from,
			to,
		} => {
			let at = |degrees: f32| {
				let (sin, cos) = degrees.to_radians().sin_cos();
				(x + r * cos, y + r * sin)
			};
			let (start_x, start_y) = at(from);
			let (end_x, end_y) = at(to);
			let large = i32::from(to - from > 180.0);
			format!("M{start_x} {start_y} A{r} {r} 0 {large} 1 {end_x} {end_y}")
		}
		Shape::RoundedRect {
			x,
			y,
			width,
			height,
			radius: r,
		} => {
			let (right, bottom) = (x + width, y + height);
			format!(
				"M{} {y} H{} A{r} {r} 0 0 1 {right} {} V{} A{r} {r} 0 0 1 {} {bottom} H{} A{r} {r} 0 0 1 {x} {} V{} A{r} {r} 0 0 1 {} {y} Z",
				x + r,
				right - r,
				y + r,
				bottom - r,
				right - r,
				x + r,
				bottom - r,
				y + r,
				x + r,
			)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn render() -> String {
		fn app() -> Element {
			rsx! {
				MoreSheet { status: "Synced", on_close: |_| {}, on_open: |_| {} }
			}
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn the_tiles_come_in_the_menus_order_each_with_a_picture() {
		let html = render();
		let mut from = 0;
		for label in [
			"Notepad",
			"Habits",
			"Notes",
			"People",
			"Travel",
			"Brain dump",
			"Alarms",
			"Logs",
		] {
			let wanted = format!(">{label}<");
			let found = html[from..]
				.find(&wanted)
				.unwrap_or_else(|| panic!("{label}: {html}"));
			from += found + wanted.len();
		}
		assert_eq!(html.matches("more-badge").count(), 8, "{html}");
		assert_eq!(html.matches("more-tile-wide").count(), 4, "{html}");
		assert!(html.contains("Synced"), "{html}");
	}

	#[test]
	fn arcs_and_circles_become_svg_path_data() {
		assert_eq!(
			svg_path(Shape::Circle {
				x: 12.0,
				y: 12.0,
				radius: 2.0
			}),
			"M10 12 a2 2 0 1 1 4 0 a2 2 0 1 1 -4 0"
		);
		assert!(
			svg_path(Shape::Arc {
				x: 12.0,
				y: 10.0,
				radius: 7.0,
				from: 150.0,
				to: 390.0
			})
			.contains(" A7 7 0 1 1 ")
		);
	}
}
