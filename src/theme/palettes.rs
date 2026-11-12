//! The palettes live in `timeparagon_core` so the Dioxus app draws from them too.

pub use timeparagon_core::theme::palettes::*;

#[cfg(test)]
mod tests {
	#[test]
	fn high_contrast_themes_have_readable_surfaces_and_accents() {
		let catalog = super::super::Catalog::built_in();
		for (name, is_dark) in [("Light high contrast", false), ("Dark high contrast", true)] {
			let entry = catalog.get(name).unwrap();
			let palette = entry.theme.palette();
			let extended = entry.theme.extended_palette();
			assert_eq!(extended.is_dark, is_dark, "{name}");
			assert!(
				palette.text.relative_contrast(palette.background) >= 7.0,
				"{name}: body text"
			);
			assert!(
				entry.accents.digits.relative_contrast(palette.background) >= 4.5,
				"{name}: timer digits"
			);
			assert!(
				extended
					.secondary
					.base
					.color
					.relative_contrast(palette.background)
					>= 4.5,
				"{name}: muted text"
			);
			for pair in [
				extended.background.base,
				extended.background.weakest,
				extended.background.weaker,
				extended.background.weak,
				extended.background.neutral,
				extended.background.strong,
				extended.background.stronger,
				extended.background.strongest,
				extended.primary.base,
				extended.primary.weak,
				extended.primary.strong,
				extended.secondary.base,
				extended.secondary.weak,
				extended.secondary.strong,
				extended.success.base,
				extended.success.weak,
				extended.success.strong,
				extended.warning.base,
				extended.warning.weak,
				extended.warning.strong,
				extended.danger.base,
				extended.danger.weak,
				extended.danger.strong,
			] {
				assert!(
					pair.color.relative_contrast(pair.text) >= 4.5,
					"{name}: unreadable surface {pair:?}"
				);
			}
		}
	}

	#[test]
	fn high_contrast_borders_stand_out_against_controls() {
		let catalog = super::super::Catalog::built_in();
		for name in ["Light high contrast", "Dark high contrast"] {
			let extended = catalog.get(name).unwrap().theme.extended_palette();
			for surface in [
				extended.background.base,
				extended.background.weaker,
				extended.background.weak,
			] {
				assert!(
					extended
						.background
						.strong
						.color
						.relative_contrast(surface.color)
						>= 3.0,
					"{name}: border against {surface:?}"
				);
			}
		}
	}
}
