//! AI controls shared by the task/event creation form.
use crate::{day, editor::worker, media_input};
use dioxus::prelude::*;
use serde_json::{Value, json};
use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::Duration,
};
use timeparagon_core::{
	ai_input::{self, Settings, voice::Patch},
	config::{Config, ProjectsSettings},
	timeblocking as tb,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskDraft {
	pub due: String,
	pub due_time: String,
	pub deadline: String,
	pub metadata: tb::Metadata,
}
impl TaskDraft {
	pub fn merge(
		&self,
		text: &str,
		today: chrono::NaiveDate,
		projects: &ProjectsSettings,
	) -> (String, Self) {
		let parsed = day::quick_task(text, today, projects);
		let mut result = self.clone();
		for token in timeparagon_core::task_input::detect(text, today, projects) {
			use timeparagon_core::task_input::Value::*;
			match token.value {
				Due(day) => result.due = day.to_string(),
				Deadline(day) => result.deadline = day.to_string(),
				Priority(p) => result.metadata.priority = Some(p),
				Minutes(m) => result.metadata.minutes = m,
				Label(l) => {
					if !result.metadata.labels.contains(&l) {
						result.metadata.labels.push(l);
					}
				}
				Project(p) => {
					if !result.metadata.projects.contains(&p) {
						result.metadata.projects.push(p);
					}
				}
			}
		}
		(parsed.title, result)
	}
	pub fn apply(&mut self, patch: &Patch) {
		if let Some(v) = &patch.due {
			self.due = v.clone();
			if v.is_empty() {
				self.due_time.clear();
			}
		}
		if let Some(v) = &patch.due_time {
			self.due_time = v.clone();
		}
		if let Some(v) = &patch.deadline {
			self.deadline = v.clone();
		}
		if let Some(v) = patch.priority {
			self.metadata.priority = v;
		}
		if let Some(v) = patch.minutes {
			self.metadata.minutes = v;
		}
		if let Some(v) = &patch.labels {
			self.metadata.labels = v.clone();
		}
		if let Some(v) = &patch.projects {
			self.metadata.projects = v.clone();
		}
	}
	pub fn save(
		mut self,
		title: &str,
		description: &str,
		settings: &timeparagon_core::config::CalendarSettings,
	) -> Result<(), String> {
		if title.trim().is_empty() || title.chars().count() > 1024 {
			return Err("Enter a task name (maximum 1024 characters)".into());
		}
		let due = if self.due.is_empty() {
			None
		} else {
			Some(timeparagon_core::calendar::deadline::parse_date(&self.due)?)
		};
		self.metadata.deadline = if self.deadline.is_empty() {
			None
		} else {
			Some(timeparagon_core::calendar::deadline::parse_input(
				&self.deadline,
			)?)
		};
		let schedule = if self.due_time.is_empty() {
			None
		} else {
			let start =
				ai_input::voice::due_start(&self.due, &self.due_time).map_err(|e| e.to_string())?;
			Some((
				start,
				start
					+ chrono::Duration::minutes(if self.metadata.minutes == 0 {
						30
					} else {
						self.metadata.minutes
					} as i64),
			))
		};
		timeparagon_core::calendar::operations::create_task_at(
			settings,
			title.trim(),
			description,
			due,
			Some(self.metadata),
			schedule,
		)
		.map(|_| ())
	}
}

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Capture {
	id: String,
	cancel: Arc<AtomicBool>,
}
impl Capture {
	fn new() -> Self {
		Self {
			id: format!(
				"{}-{}",
				std::process::id(),
				NEXT.fetch_add(1, Ordering::Relaxed)
			),
			cancel: Arc::new(AtomicBool::new(false)),
		}
	}
}
impl Drop for Capture {
	fn drop(&mut self) {
		self.cancel.store(true, Ordering::Relaxed);
		media_input::cancel(&self.id);
	}
}

#[component]
pub fn Controls(
	settings: Settings,
	context: Value,
	voice: bool,
	disabled: bool,
	mut active: Signal<bool>,
	on_patch: EventHandler<Patch>,
	on_images: EventHandler<Vec<ai_input::image::Found>>,
) -> Element {
	let capture = use_hook(|| Arc::new(Capture::new()));
	let mut phase = use_signal(String::new);
	let mut level = use_signal(|| 0.0f64);
	let mut menu = use_signal(|| false);
	let mut error = use_signal(|| None::<String>);
	let mut log = use_signal(String::new);
	let mut debug = use_signal(|| false);
	let id = capture.id.clone();
	let mut run = move |source: &'static str| {
		if active() || disabled {
			return;
		}
		let settings = settings.clone();
		let context = context.clone();
		let id = capture.id.clone();
		let cancel = capture.cancel.clone();
		let missing = if source == "voice" && settings.whisper_url.trim().is_empty() {
			Some("Set the Whisper server URL in Settings → AI → Voice input")
		} else if source == "voice" && settings.model.trim().is_empty() {
			Some("Set the Ollama extraction model in Settings → AI")
		} else if source != "voice"
			&& (if settings.image_provider == "openrouter" {
				settings.image_openrouter_model.trim().is_empty()
			} else {
				settings.image_ollama_model.trim().is_empty()
			}) {
			Some("Set the image model in Settings → AI → Image input")
		} else {
			None
		};
		if let Some(message) = missing {
			error.set(Some(message.into()));
			return;
		}
		error.set(None);
		log.set(String::new());
		menu.set(false);
		phase.set("Opening input".into());
		active.set(true);
		spawn(async move {
			let result = async {
				media_input::request(json!({"op":"start","id":id,"source":source}))?;
				let path = loop {
					tokio::time::sleep(Duration::from_millis(50)).await;
					let value = media_input::request(json!({"op":"poll","id":id}))?;
					if value["cancelled"].as_bool() == Some(true) {
						return Ok(None);
					}
					if let Some(path) = value["path"].as_str() {
						break path.to_owned();
					}
					if let Some(p) = value["phase"].as_str() {
						phase.set(p.to_owned());
					}
					level.set(value["level"].as_f64().unwrap_or_default());
				};
				level.set(0.0);
				if source == "voice" {
					phase.set("Transcribing…".into());
					let options = settings.clone();
					let (transcript, patch_context) = worker(move || {
						let bytes = std::fs::read(&path).map_err(|e| e.to_string());
						let _ = std::fs::remove_file(&path);
						let transcript = ai_input::voice::transcribe(&options, &bytes?)
							.map_err(|e| e.to_string())?;
						Ok((transcript, context))
					})
					.await?;
					log.set(format!("Whisper transcript\n{transcript}\n\n"));
					phase.set("Extracting…".into());
					let (tx, rx) = std::sync::mpsc::channel();
					let job = tokio::task::spawn_blocking(move || {
						ai_input::voice::extract(
							&settings,
							patch_context,
							&transcript,
							&cancel,
							&mut |label, body| {
								let _ = tx.send(format!("{label}\n{body}\n\n"));
							},
						)
						.map_err(|e| e.to_string())
					});
					while !job.is_finished() {
						for entry in rx.try_iter() {
							log.write().push_str(&entry);
						}
						tokio::time::sleep(Duration::from_millis(50)).await;
					}
					let result = job.await.map_err(|e| e.to_string())?;
					for entry in rx.try_iter() {
						log.write().push_str(&entry);
					}

					let patch = result?;
					log.write()
						.push_str(&format!("Validated patch\n{patch:#?}"));
					on_patch.call(patch);
				} else {
					phase.set("Reading picture…".into());
					let events = worker(move || {
						let bytes = std::fs::read(&path).map_err(|e| e.to_string());
						let _ = std::fs::remove_file(&path);
						ai_input::image::extract(&bytes?, &settings).map_err(|e| e.to_string())
					})
					.await?;
					if events.is_empty() {
						return Err("No calendar events found in this picture".to_owned());
					}
					on_images.call(events);
				}
				Ok::<_, String>(Some(()))
			}
			.await;
			media_input::cancel(&id);
			if let Err(message) = result {
				log.write().push_str(&format!("\nERROR: {message}"));
				error.set(Some(message));
			}
			active.set(false);
			phase.set(String::new());
			level.set(0.0);
		});
	};
	let brightness = level() * 45.0;
	let mut voice_run = run.clone();
	let mut camera_run = run.clone();
	let mut paste_run = run.clone();
	rsx! {
		div { class: "ai-input",
			div { class: "ai-toolbar",
				if voice {
					button { class: "btn ai-microphone", r#type: "button", aria_label: if phase() == "Listening" { "Finish recording" } else { "Voice input" }, aria_pressed: (phase() == "Listening").to_string(),
						style: "background: color-mix(in srgb, var(--card), white {brightness}%);", disabled: disabled || (active() && phase() != "Listening"),
						onclick: move |_| { if active() { let _ = media_input::request(json!({"op":"stop","id":id})); phase.set("Finishing recording…".into()); } else { voice_run("voice"); } },
						svg { width: "20", height: "20", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", "aria-hidden": "true",
							rect { x:"9", y:"2", width:"6", height:"12", rx:"3" }
							path { d:"M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8" }
						}
					}
				}
				button { class:"btn", r#type:"button", aria_label:"Picture input", aria_expanded: menu().to_string(), disabled: disabled || active(), onclick:move |_| menu.set(!menu()),
					svg { width:"20",height:"20",view_box:"0 0 24 24",fill:"none",stroke:"currentColor",stroke_width:"2","aria-hidden":"true",
						rect { x:"3",y:"3",width:"18",height:"18",rx:"2" } circle { cx:"8",cy:"8",r:"1" } path { d:"m3 17 6-6 4 4 3-3 5 5" }
					}
				}
				if active() { button { class:"btn",r#type:"button",onclick:move |_| debug.set(true), "{phase}" } }
				else if !log().is_empty() { button { class:"btn",r#type:"button",onclick:move |_| debug.set(true),"Model debug" } }
			}
			if menu() {
				div { class:"ai-sources",
					button {class:"btn",r#type:"button",onclick:move |_| camera_run("camera"),"Take picture"}
					button {class:"btn",r#type:"button",onclick:move |_| paste_run("clipboard"),"Paste image"}
					button {class:"btn",r#type:"button",onclick:move |_| run("file"),"Choose image file"}
					p {class:"hint","Choose a screenshot or picture (PNG, JPEG or WebP, up to 20 MB)."}
				}
			}
			if let Some(message) = error() { p {class:"editor-error",role:"alert","{message}"} }
			if debug() {
				div {class:"modal-backdrop ai-debug",onpointerdown:move |event| { event.stop_propagation(); debug.set(false); },onkeydown:move |event| {if event.key() == Key::Escape { debug.set(false); }},
					div {class:"modal",role:"dialog",aria_label:"Model debug",onpointerdown:move |event|event.stop_propagation(),
						div {class:"modal-head",h2 {"Model debug"} button {class:"modal-close",r#type:"button",onclick:move |_|debug.set(false),"×"}}
						pre {class:"ai-debug-log","{log}"}
						button {class:"btn",r#type:"button",onclick:move |_| { #[cfg(target_os = "android")]
								if let Err(message) = media_input::request(json!({"op":"copy","id":"debug","text":log()})) {error.set(Some(message));}
								#[cfg(not(target_os = "android"))]
								{let text = serde_json::to_string(&log()).unwrap_or_default(); document::eval(&format!("navigator.clipboard.writeText({text})"));} },"Copy debug log"}
					}
				}
			}
		}
	}
}

fn setting<'a>(settings: &'a mut Settings, key: &str) -> &'a mut String {
	match key {
		"whisper_url" => &mut settings.whisper_url,
		"ollama_url" => &mut settings.ollama_url,
		"model" => &mut settings.model,
		"image_ollama_url" => &mut settings.image_ollama_url,
		"image_ollama_model" => &mut settings.image_ollama_model,
		"image_openrouter_model" => &mut settings.image_openrouter_model,
		_ => unreachable!(),
	}
}

#[component]
pub fn ModelSettings(mut config: Signal<Config>) -> Element {
	let mut draft = use_signal(move || config.peek().voice.clone());
	let mut status = use_signal(String::new);
	let mut busy = use_signal(|| false);
	let mut token = use_signal(|| {
		timeparagon_core::secrets::path()
			.and_then(|path| {
				timeparagon_core::secrets::read(
					&path,
					timeparagon_core::secrets::keys::OPENROUTER_TOKEN,
				)
				.ok()
				.flatten()
			})
			.unwrap_or_default()
	});
	let mut token_changed = use_signal(|| false);
	let fields: Vec<(&str, &str)> = vec![
		("ollama_url", "Ollama extraction server"),
		("model", "Ollama extraction model"),
	];
	let image_fields = if draft().image_provider == "openrouter" {
		vec![("image_openrouter_model", "OpenRouter image model")]
	} else {
		vec![
			("image_ollama_url", "Ollama image server"),
			("image_ollama_model", "Ollama image model"),
		]
	};
	let field = |(key, label): (&'static str, &'static str)| {
		let value = setting(&mut draft(), key).clone();
		rsx! { label { "{label}" input { class:"client-input",r#type:"text",value:"{value}",autocapitalize:"off",spellcheck:"false",disabled:busy(),oninput:move |event| { *setting(&mut draft.write(),key) = event.value(); status.set(String::new()); } } } }
	};
	rsx! {
		section { class:"ai-settings",
			h3 {class:"settings-heading","AI"}
			h4 {"Voice input"}
			{field(("whisper_url", "Whisper server URL"))}
			p {class:"hint","Use the same Whisper model loaded in whisper-server as on desktop. Enter a server address reachable from this phone; localhost refers to the phone. Tap the microphone again to finish; recordings stop after 2 minutes."}
			h4 {"Image input"}
			div {class:"sheet-tabs",
				for provider in ["ollama", "openrouter"] {
					button {class:"nav-tab",r#type:"button",disabled:busy(),aria_pressed:(draft().image_provider == provider).to_string(),onclick:move |_|draft.write().image_provider = provider.into(),
						if provider == "ollama" { "Ollama" } else { "OpenRouter" }
					}
				}
			}
			{image_fields.into_iter().map(field)}
			h4 {"Text extraction"}
			{fields.into_iter().map(field)}
			label {"OpenRouter API token" input {class:"client-input",r#type:"password",value:"{token}",placeholder:"Saved privately on this device",autocomplete:"off",disabled:busy(),oninput:move |event|{token.set(event.value());token_changed.set(true);} } }
			p {class:"hint","Model names and providers use the desktop settings format. Server addresses and credentials are configured on each device. An edited token is saved privately; clear it to remove it."}
			button {class:"btn primary",r#type:"button",disabled:busy(),onclick:move |_| {
				busy.set(true); status.set(String::new());
				let options = draft(); let credential = token_changed().then(||token());
				spawn(async move {
					let result = worker(move || {
						let path = timeparagon_core::task_db::path().ok_or("Cannot locate settings database")?;
						timeparagon_core::settings::update(&path, |config: &mut Config| config.voice = options).map_err(|e|e.to_string())?;
						if let Some(credential) = credential {
							let path = timeparagon_core::secrets::path().ok_or("No secrets directory")?;
							if credential.trim().is_empty() { timeparagon_core::secrets::remove(&path,timeparagon_core::secrets::keys::OPENROUTER_TOKEN) }
							else {timeparagon_core::secrets::write(&path,timeparagon_core::secrets::keys::OPENROUTER_TOKEN,credential.trim())}.map_err(|e|e.to_string())?;
						}
						Ok(day::load_config())
					}).await;
					match result {Ok(saved)=>{config.set(saved);token_changed.set(false);status.set("Saved.".into());},Err(error)=>status.set(error)}
					busy.set(false);
				});
			}, "Save AI settings" }
			if !status().is_empty() {p {role:"status","{status}"}}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn later_voice_commands_preserve_manual_fields_and_explicit_clears() {
		let mut task = TaskDraft::default();
		let context = json!({"projects":[],"today":"2026-11-17"});
		task.apply(
			&ai_input::voice::validate_patch(
				r#"{"due_time":"21:00","priority":"P1","labels":["home"],"deadline":"2026-11-18 17:30"}"#,
				&context,
			)
			.unwrap(),
		);
		task.metadata.minutes = 45;
		task.apply(&ai_input::voice::validate_patch(r#"{"due":"2026-11-19"}"#, &context).unwrap());
		assert_eq!(task.due_time, "21:00");
		assert_eq!(task.metadata.minutes, 45);
		assert_eq!(task.deadline, "2026-11-18 17:30");
		task.apply(
			&ai_input::voice::validate_patch(
				r#"{"due":"","labels":[],"priority":"none"}"#,
				&context,
			)
			.unwrap(),
		);
		assert!(task.due.is_empty() && task.due_time.is_empty());
		assert!(task.metadata.labels.is_empty() && task.metadata.priority.is_none());
	}
	#[test]
	fn quick_keywords_refine_voice_metadata_without_erasing_other_fields() {
		let mut task = TaskDraft::default();
		task.metadata.labels = vec!["home".into()];
		task.metadata.minutes = 45;
		task.deadline = "2026-12-05".into();
		let (title, task) = task.merge(
			"Pay rent tomorrow p1",
			chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(),
			&ProjectsSettings::default(),
		);
		assert_eq!(title, "Pay rent");
		assert_eq!(task.due, "2026-11-18");
		assert_eq!(task.metadata.minutes, 45);
		assert_eq!(task.metadata.labels, vec!["home"]);
		assert_eq!(task.deadline, "2026-12-05");
	}
}
