//! Verify the reactive boundary with real navigation and log consumers.
use super::*;
use std::{cell::Cell, rc::Rc};
use timeparagon_core::config::Config;

#[derive(Clone, Default)]
struct Harness {
	root_renders: Rc<Cell<usize>>,
	calendar_renders: Rc<Cell<usize>>,
	navigation: Rc<Cell<Option<Navigation>>>,
	sync: Rc<Cell<Option<SyncState>>>,
}

fn harness(props: Harness) -> Element {
	props.root_renders.set(props.root_renders.get() + 1);
	let config = use_signal(Config::default);
	let revision = use_signal(|| 0);
	use_context_provider(|| Session { config, revision });
	let navigation = navigation::use_navigation(config);
	use_context_provider(|| navigation);
	let panels = navigation::use_panels();
	use_context_provider(|| panels);
	let mut show_log = panels.show_log;
	use_hook(move || show_log.set(true));
	let sync = polling::use_sync_state();
	use_context_provider(|| sync);
	props.navigation.set(Some(navigation));
	props.sync.set(Some(sync));
	use_context_provider(|| props.calendar_renders);
	rsx! { navigation::Topbar {} CalendarProbe {} sheets::LogSheet {} }
}

#[component]
fn CalendarProbe() -> Element {
	let renders = use_context::<Rc<Cell<usize>>>();
	renders.set(renders.get() + 1);
	let Navigation { current_day, .. } = use_context::<Navigation>();
	rsx! { p { "Calendar: {current_day}" } }
}

#[test]
fn sync_logs_update_without_rendering_the_root_or_calendar() {
	let handles = Harness::default();
	let mut dom = VirtualDom::new_with_props(harness, handles.clone());
	dom.rebuild_in_place();
	let root_before = handles.root_renders.get();
	let calendar_before = handles.calendar_renders.get();
	assert!(dioxus_ssr::render(&dom).contains("Nothing has been logged yet."));
	let mut log = handles.sync.get().unwrap().google_log;
	dom.in_runtime(|| {
		log.set(vec![calendar::operations::LogEntry {
			sequence: 1,
			at: chrono::Local::now(),
			text: "Saved without redrawing calendar".into(),
			failed: false,
		}])
	});
	dom.render_immediate_to_vec();
	assert!(dioxus_ssr::render(&dom).contains("Saved without redrawing calendar"));
	assert_eq!(handles.root_renders.get(), root_before);
	assert_eq!(handles.calendar_renders.get(), calendar_before);

	let mut current_day = handles.navigation.get().unwrap().current_day;
	dom.in_runtime(|| current_day.set(current_day().succ_opt().unwrap()));
	dom.render_immediate_to_vec();
	assert_eq!(handles.root_renders.get(), root_before);
	assert_eq!(handles.calendar_renders.get(), calendar_before + 1);
}
