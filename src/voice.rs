//! Local microphone -> whisper.cpp -> constrained Ollama patch.
use crate::app::SystemMessage;
use crate::{
	app::{Message, State},
	inbox,
};
use iced::Task;
use serde_json::{Value, json};
use std::{
	io::{Read, Write},
	process::{Command, Stdio},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	time::{Duration, Instant},
};

pub use timeparagon_core::ai_input::Settings;
pub use timeparagon_core::ai_input::voice::{Patch, due_start};
/// Providers available in the AI input settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Provider {
	#[default]
	Whisper,
	Ollama,
	OpenRouter,
}
#[derive(Debug, Clone, Copy)]
pub enum InputSection {
	Voice,
	Image,
	Extraction,
}
/// Keep credentials out of debug representations, including UI messages.
#[derive(Clone)]
pub struct Token(String);
impl std::fmt::Debug for Token {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("[REDACTED]")
	}
}
impl Default for Token {
	fn default() -> Self {
		Self(
			crate::secrets::path()
				.and_then(|path| {
					crate::secrets::read(&path, crate::secrets::keys::OPENROUTER_TOKEN)
						.ok()
						.flatten()
				})
				.unwrap_or_default(),
		)
	}
}
#[derive(Debug, Clone)]
pub enum Action {
	Toggle,
	Poll,
	Debug(bool),
	CopyDebug,
	Binary(String),
	WhisperModel(String),
	WhisperUrl(String),
	Url(String),
	Model(String),
	ProviderTab(InputSection, Provider),
	OpenRouterModel(InputSection, String),
	ImageUrl(String),
	ImageModel(String),
	OpenRouterToken(Token),
	SaveOpenRouterToken,
}
#[derive(Default)]
pub struct Session {
	voice_tab: Provider,
	extraction_tab: Option<Provider>,
	openrouter_token: Token,
	token_status: Option<&'static str>,
	rx: Option<mpsc::Receiver<Progress>>,
	stop: Arc<AtomicBool>,
	cancel: Arc<AtomicBool>,
	pub phase: String,
	pub debug_open: bool,
	pub debug_log: String,
	pub level: f32,
	level_from: f32,
	level_target: f32,
	level_started: Option<Instant>,
	pub transcript: String,
	pub error: Option<String>,
}
impl Session {
	fn reset_level(&mut self) {
		self.level = 0.0;
		self.level_from = 0.0;
		self.level_target = 0.0;
		self.level_started = None;
	}
	fn animate_level(&mut self, now: Instant) {
		if let Some(started) = self.level_started {
			let t = (now.duration_since(started).as_secs_f32() / 0.05).clamp(0.0, 1.0);
			let eased = t * t * (3.0 - 2.0 * t);
			self.level = self.level_from + (self.level_target - self.level_from) * eased;
			if t >= 1.0 {
				self.level_started = None;
			}
		}
	}
	fn target_level(&mut self, value: f32, now: Instant) {
		self.animate_level(now);
		let value = value.clamp(0.0, 1.0);
		if value != self.level_target {
			self.level_from = self.level;
			self.level_target = value;
			self.level_started = Some(now);
		}
	}

	pub fn active(&self) -> bool {
		self.rx.is_some()
	}
	pub fn recording(&self) -> bool {
		self.active() && self.phase == "Listening"
	}
	pub fn cancel(&mut self) {
		self.cancel.store(true, Ordering::Relaxed);
		self.stop.store(true, Ordering::Relaxed);
		self.rx = None;
		self.reset_level();
		self.debug_open = false;
		self.debug_log.clear();
		self.phase.clear();
		self.transcript.clear();
		self.error = None;
	}
}
impl Drop for Session {
	fn drop(&mut self) {
		self.cancel();
	}
}
enum Progress {
	Debug(String),
	Level(f32),
	Phase(&'static str),
	Transcript(String),
	Done(Result<Patch, String>),
}
fn msg(action: Action) -> Message {
	Message::Inbox(inbox::Action::Voice(action))
}
pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::Debug(open) => {
			state.inbox.voice.debug_open = open;
			return Task::none();
		}
		Action::CopyDebug => return iced::clipboard::write(state.inbox.voice.debug_log.clone()),
		Action::ProviderTab(section, provider) => {
			let session = &mut state.inbox.voice;
			match section {
				InputSection::Voice => session.voice_tab = provider,
				InputSection::Image => {
					state.config.voice.image_provider = if provider == Provider::OpenRouter {
						"openrouter"
					} else {
						"ollama"
					}
					.into();
					return crate::app::queue_save(state);
				}
				InputSection::Extraction => session.extraction_tab = Some(provider),
			}
			return Task::none();
		}
		Action::OpenRouterToken(token) => {
			state.inbox.voice.openrouter_token = token;
			state.inbox.voice.token_status = None;
			return Task::none();
		}
		Action::SaveOpenRouterToken => {
			let token = state.inbox.voice.openrouter_token.0.trim();
			let result = crate::secrets::path()
				.ok_or_else(|| std::io::Error::other("No secrets directory"))
				.and_then(|path| {
					if token.is_empty() {
						crate::secrets::remove(&path, crate::secrets::keys::OPENROUTER_TOKEN)
					} else {
						crate::secrets::write(&path, crate::secrets::keys::OPENROUTER_TOKEN, token)
					}
				});
			state.inbox.voice.token_status = Some(match result {
				Ok(()) if token.is_empty() => "Token removed",
				Ok(()) => "Token saved",
				Err(_) => "Could not save token. Check the private secrets file and try again.",
			});
			return Task::none();
		}
		Action::OpenRouterModel(section, model) => match section {
			InputSection::Voice => state.config.voice.voice_openrouter_model = model,
			InputSection::Image => state.config.voice.image_openrouter_model = model,
			InputSection::Extraction => state.config.voice.extraction_openrouter_model = model,
		},
		Action::ImageUrl(url) => state.config.voice.image_ollama_url = url,
		Action::ImageModel(model) => state.config.voice.image_ollama_model = model,
		Action::Binary(v) => state.config.voice.binary = v,
		Action::WhisperModel(v) => state.config.voice.whisper_model = v,
		Action::WhisperUrl(v) => state.config.voice.whisper_url = v,
		Action::Url(v) => state.config.voice.ollama_url = v,
		Action::Model(v) => state.config.voice.model = v,
		Action::Toggle => {
			if state.inbox.voice.recording() {
				state.inbox.voice.stop.store(true, Ordering::Relaxed);
				state.inbox.voice.phase = "Finishing recording".into();
				return Task::none();
			}
			if state.inbox.image.busy
				|| state.inbox.voice.active()
				|| state
					.inbox
					.draft
					.as_ref()
					.is_none_or(|d| d.saving || d.fetching)
			{
				return Task::none();
			}
			let settings = state.config.voice.clone();
			let draft = state.inbox.draft.as_ref().unwrap();
			let context = json!({"title": draft.title, "description": state.task_description.text(), "due": state.task_editor().date, "due_time": draft.event.as_ref().filter(|e| !e.all_day).map(|e| e.start.format("%H:%M").to_string()), "today": crate::clock::now().date_naive().to_string(), "deadline": state.task_editor().deadline, "metadata": draft.event.as_ref().map(crate::timeblocking::metadata), "projects": state.config.projects.items});
			let (tx, rx) = mpsc::channel();
			let voice = &mut state.inbox.voice;
			voice.stop = Arc::new(AtomicBool::new(false));
			voice.cancel = Arc::new(AtomicBool::new(false));
			voice.error = None;
			voice.transcript.clear();
			voice.debug_log.clear();
			voice.reset_level();
			voice.phase = "Listening".into();
			let stop = voice.stop.clone();
			let cancel = voice.cancel.clone();
			match std::thread::Builder::new()
				.name("timeparagon-voice".into())
				.spawn(move || {
					let result = pipeline(&settings, context, &stop, &cancel, &tx)
						.map_err(|e| e.to_string());
					debug(
						Some(&tx),
						"Pipeline result",
						match &result {
							Ok(patch) => format!("Validated patch: {patch:#?}"),
							Err(error) => format!("ERROR: {error}"),
						},
					);
					let _ = tx.send(Progress::Done(result));
				}) {
				Ok(_) => voice.rx = Some(rx),
				Err(e) => {
					voice.phase.clear();
					voice.error = Some(e.to_string());
				}
			}
			return Task::none();
		}
		Action::Poll => {
			let now = Instant::now();
			let messages: Vec<_> = state
				.inbox
				.voice
				.rx
				.as_ref()
				.map(|r| r.try_iter().collect())
				.unwrap_or_default();
			for message in messages {
				match message {
					Progress::Debug(value) => state.inbox.voice.debug_log.push_str(&value),
					Progress::Level(v) => state.inbox.voice.target_level(v, now),
					Progress::Phase(v) => {
						state.inbox.voice.phase = v.into();
						state.inbox.voice.reset_level();
					}
					Progress::Transcript(v) => state.inbox.voice.transcript = v,
					Progress::Done(result) => {
						state.inbox.voice.rx = None;
						state.inbox.voice.reset_level();
						match result {
							Ok(patch) => {
								inbox::apply_voice(state, patch);
								state.inbox.voice.phase.clear();
							}
							Err(e) => {
								state.inbox.voice.phase.clear();
								state.inbox.voice.error = Some(e);
							}
						}
					}
				}
			}
			state.inbox.voice.animate_level(now);
			return Task::none();
		}
	}
	crate::app::queue_save(state)
}
fn pipeline(
	settings: &Settings,
	context: Value,
	stop: &Arc<AtomicBool>,
	cancel: &Arc<AtomicBool>,
	tx: &mpsc::Sender<Progress>,
) -> anyhow::Result<Patch> {
	anyhow::ensure!(
		!settings.model.trim().is_empty(),
		"Set the Ollama extraction model in Settings → Voice input first"
	);
	anyhow::ensure!(
		!settings.whisper_model.trim().is_empty() || !settings.whisper_url.trim().is_empty(),
		"Set the whisper.cpp model file in Settings → Voice input first"
	);
	let dir = tempfile::tempdir()?;
	let wav = dir.path().join("recording.wav");
	// arecord negotiates the default ALSA microphone as signed 16-bit mono PCM.
	let mut child = Command::new("arecord")
		.args([
			"-q", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1", "-d", "120",
		])
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.map_err(|e| {
			anyhow::anyhow!(
				"Cannot open microphone: install alsa-utils (arecord) and check the default input: {e}"
			)
		})?;
	let mut output = child.stdout.take().unwrap();
	// A watchdog owns the child so stop/cancel also interrupts a blocked read.
	let stop_watch = stop.clone();
	let cancel_watch = cancel.clone();
	let watcher = std::thread::spawn(move || {
		let start = Instant::now();
		loop {
			if stop_watch.load(Ordering::Relaxed)
				|| cancel_watch.load(Ordering::Relaxed)
				|| start.elapsed() >= Duration::from_secs(120)
			{
				let _ = child.kill();
				let _ = child.wait();
				break;
			}
			match child.try_wait() {
				Ok(None) => std::thread::sleep(Duration::from_millis(40)),
				_ => {
					let _ = child.kill();
					let _ = child.wait();
					break;
				}
			}
		}
	});
	let mut pcm = Vec::new();
	let mut buffer = [0u8; 3200];
	let mut heard = false;
	let started = Instant::now();
	let read_result = (|| -> std::io::Result<()> {
		while !stop.load(Ordering::Relaxed) && started.elapsed() < Duration::from_secs(120) {
			let n = output.read(&mut buffer)?;
			if n == 0 {
				break;
			}
			let energy: f32 = buffer[..n]
				.as_chunks::<2>()
				.0
				.iter()
				.map(|s| (i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0).powi(2))
				.sum();
			let rms = (energy / (n / 2).max(1) as f32).sqrt();
			heard |= rms > 0.01;
			let _ = tx.send(Progress::Level(((rms - 0.01).max(0.0) * 8.0).min(1.0)));
			pcm.extend_from_slice(&buffer[..n]);
		}
		Ok(())
	})();
	stop.store(true, Ordering::Relaxed);
	let _ = watcher.join();
	read_result?;
	anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Recording cancelled");
	anyhow::ensure!(
		heard && pcm.len() >= 6400,
		"No speech detected; check your microphone and try again"
	);
	let mut file = std::fs::File::create(&wav)?;
	let n = pcm.len() as u32;
	file.write_all(b"RIFF")?;
	file.write_all(&(36 + n).to_le_bytes())?;
	file.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x01\0")?;
	file.write_all(&16000u32.to_le_bytes())?;
	file.write_all(&32000u32.to_le_bytes())?;
	file.write_all(b"\x02\0\x10\0data")?;
	file.write_all(&n.to_le_bytes())?;
	file.write_all(&pcm)?;
	drop(file);
	let _ = tx.send(Progress::Phase("Transcribing"));
	let transcript = if !settings.whisper_url.trim().is_empty() {
		timeparagon_core::ai_input::voice::transcribe(settings, &std::fs::read(&wav)?)?
	} else {
		let prefix = dir.path().join("transcript");
		let mut whisper = Command::new(&settings.binary);
		whisper
			.arg("-m")
			.arg(&settings.whisper_model)
			.arg("-f")
			.arg(&wav)
			.args(["-otxt", "-of"])
			.arg(&prefix)
			.args(["-l", "auto", "-pp"]);
		debug(
			Some(tx),
			"Audio",
			format!(
				"{} samples, {:.2} seconds; 16000 Hz mono PCM",
				pcm.len() / 2,
				pcm.len() as f64 / 32000.0
			),
		);
		let status = run_model(&mut whisper, cancel, tx)?;
		anyhow::ensure!(
			status.success(),
			"whisper.cpp failed ({status}); open model debug for details"
		);
		let transcript = std::fs::read_to_string(prefix.with_extension("txt"))?
			.trim()
			.to_owned();
		anyhow::ensure!(!transcript.is_empty(), "Whisper returned no speech");
		transcript
	};
	debug(Some(tx), "Whisper transcript", &transcript);
	let _ = tx.send(Progress::Transcript(transcript.clone()));
	let _ = tx.send(Progress::Phase("Extracting task details"));
	extract(settings, context, &transcript, cancel, Some(tx))
}
fn extract(
	settings: &Settings,
	context: Value,
	transcript: &str,
	cancel: &AtomicBool,
	tx: Option<&mpsc::Sender<Progress>>,
) -> anyhow::Result<Patch> {
	timeparagon_core::ai_input::voice::extract(
		settings,
		context,
		transcript,
		cancel,
		&mut |label, body| debug(tx, label, body),
	)
}
fn debug(tx: Option<&mpsc::Sender<Progress>>, label: &str, body: impl AsRef<str>) {
	if let Some(tx) = tx {
		let _ = tx.send(Progress::Debug(format!(
			"[{}] {label}\n{}\n\n",
			crate::clock::now().format("%H:%M:%S%.3f"),
			body.as_ref()
		)));
	}
}
/// Drain both streams concurrently: verbose model output must never block the child.
fn run_model(
	command: &mut Command,
	cancel: &AtomicBool,
	tx: &mpsc::Sender<Progress>,
) -> anyhow::Result<std::process::ExitStatus> {
	debug(Some(tx), "Whisper command", format!("{command:?}"));
	let started = Instant::now();
	let mut child = command
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()?;
	fn drain(
		mut stream: impl Read + Send + 'static,
		tx: mpsc::Sender<Progress>,
		label: &'static str,
	) -> std::thread::JoinHandle<()> {
		std::thread::spawn(move || {
			let mut bytes = [0; 4096];
			loop {
				match stream.read(&mut bytes) {
					Ok(0) => break,
					Ok(n) => debug(Some(&tx), label, String::from_utf8_lossy(&bytes[..n])),
					Err(e) => {
						debug(Some(&tx), label, format!("Read error: {e}"));
						break;
					}
				}
			}
		})
	}
	let stdout = drain(child.stdout.take().unwrap(), tx.clone(), "Whisper stdout");
	let stderr = drain(child.stderr.take().unwrap(), tx.clone(), "Whisper stderr");
	let result = (|| -> anyhow::Result<std::process::ExitStatus> {
		loop {
			anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Transcription cancelled");
			anyhow::ensure!(
				started.elapsed() < Duration::from_secs(180),
				"Transcription timed out"
			);
			if let Some(status) = child.try_wait()? {
				return Ok(status);
			}
			std::thread::sleep(Duration::from_millis(50));
		}
	})();
	if result.is_err() {
		let _ = child.kill();
		let _ = child.wait();
	}
	let _ = stdout.join();
	let _ = stderr.join();
	debug(
		Some(tx),
		"Whisper exit",
		format!("{result:?}, {:.2}s", started.elapsed().as_secs_f64()),
	);
	result
}

pub fn debug_overlay<'a>(
	state: &'a State,
	metrics: &crate::ui::layout::Metrics,
) -> iced::Element<'a, Message> {
	use iced::Fill;
	use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
	let voice = &state.inbox.voice;
	let log = if voice.debug_log.is_empty() {
		"Waiting for model output..."
	} else {
		&voice.debug_log
	};
	let card = container(
		column![
			row![
				text("Voice model debug").size(20).width(Fill),
				button("Close").on_press(msg(Action::Debug(false)))
			]
			.spacing(8),
			text(&voice.phase).size(12),
			crate::ui::controls::scrollable(
				text(log).font(iced::Font::MONOSPACE).size(12).width(Fill)
			)
			.height(Fill),
			button("Copy debug log").on_press(msg(Action::CopyDebug))
		]
		.spacing(10),
	)
	.padding(16)
	.width(Fill)
	.max_width(760)
	.height((metrics.size.height - 32.0).max(80.0))
	.style(crate::ui::controls::surface);
	let backdrop =
		mouse_area(
			container(Space::new())
				.width(Fill)
				.height(Fill)
				.style(|_| container::Style {
					background: Some(crate::ui::controls::SCRIM.into()),
					..Default::default()
				}),
		)
		.on_press(msg(Action::Debug(false)))
		.on_scroll(|_| Message::Ignored);
	let overlay = stack![
		backdrop,
		container(
			mouse_area(card)
				.on_press(Message::Ignored)
				.on_scroll(|_| Message::Ignored)
		)
		.padding(16)
		.center_x(Fill)
		.center_y(Fill)
	];
	if state.task_creation_window {
		// The dock input region must include this modal, not only the task card.
		iced::widget::sensor(overlay)
			.on_show(|size| {
				Message::System(SystemMessage::TaskPopoverBounds(
					iced::Rectangle::with_size(size),
				))
			})
			.on_resize(|size| {
				Message::System(SystemMessage::TaskPopoverBounds(
					iced::Rectangle::with_size(size),
				))
			})
			.into()
	} else {
		overlay.into()
	}
}

#[cfg(test)]
use timeparagon_core::ai_input::voice::validate_patch;

pub fn input<'a>(
	state: &'a State,
	title: iced::Element<'a, Message>,
) -> iced::Element<'a, Message> {
	use iced::widget::{button, column, row, text};
	let voice = &state.inbox.voice;
	let recording = voice.recording();
	let level = voice.level;
	let mut mic = button(iced::widget::canvas(Microphone).width(24).height(24))
		.padding(8)
		.style(move |theme: &iced::Theme, status| {
			let mut style = crate::ui::controls::secondary(theme, status);
			if recording && level > 0.0 {
				let base = match style.background {
					Some(iced::Background::Color(color)) => color,
					_ => theme.extended_palette().background.weakest.color,
				};
				style.background = Some(
					iced::Color::from_rgb(
						base.r + (1.0 - base.r) * level * 0.5,
						base.g + (1.0 - base.g) * level * 0.5,
						base.b + (1.0 - base.b) * level * 0.5,
					)
					.into(),
				);
			}
			style
		});
	if (!voice.active() || recording)
		&& !state.inbox.image.busy
		&& state
			.inbox
			.draft
			.as_ref()
			.is_some_and(|d| !d.saving && !d.fetching)
	{
		mic = mic.on_press(msg(Action::Toggle));
	}
	let mut content = column![
		row![
			title,
			iced::widget::container(mic).id("voice-record"),
			crate::image_input::input_button(state)
		]
		.spacing(6)
		.align_y(iced::Center)
	]
	.spacing(4);
	content = content.push(crate::image_input::sources(state));
	let processing = match voice.phase.as_str() {
		"Transcribing" => Some("Transcribing..."),
		"Extracting task details" => Some("Extracting..."),
		_ => None,
	};
	if let Some(label) = processing {
		content = content.push(
			button(text(label).size(12))
				.on_press(msg(Action::Debug(true)))
				.style(crate::ui::controls::secondary),
		);
	} else {
		if !voice.phase.is_empty() {
			content = content.push(text(&voice.phase).size(12));
		}
		if !voice.debug_log.is_empty() {
			content = content.push(
				button(text("Model debug").size(12))
					.on_press(msg(Action::Debug(true)))
					.style(crate::ui::controls::secondary),
			);
		}
	}
	if let Some(error) = &voice.error {
		content = content.push(text(error).size(12).style(text::danger));
	}
	content.into()
}
fn provider_tabs<'a>(section: InputSection, selected: Provider) -> iced::Element<'a, Message> {
	use iced::widget::{button, row, text};
	let providers: &[Provider] = match section {
		InputSection::Voice => &[Provider::Whisper, Provider::Ollama, Provider::OpenRouter],
		_ => &[Provider::Ollama, Provider::OpenRouter],
	};
	row(providers.iter().map(|&provider| {
		let label = match provider {
			Provider::Whisper => "whisper.cpp",
			Provider::Ollama => "Ollama",
			Provider::OpenRouter => "OpenRouter",
		};
		button(text(label))
			.on_press(msg(Action::ProviderTab(section, provider)))
			.style(if provider == selected {
				crate::ui::controls::primary
			} else {
				crate::ui::controls::secondary
			})
			.into()
	}))
	.spacing(4)
	.wrap()
	.into()
}

pub fn settings<'a>(
	state: &'a State,
	metrics: &crate::ui::layout::Metrics,
) -> iced::Element<'a, Message> {
	use crate::ui::controls::with_info;
	use iced::Fill;
	use iced::widget::{button, column, text};
	let settings = &state.config.voice;
	let voice = &state.inbox.voice;
	let heading = |label| {
		text(label).font(iced::Font {
			weight: iced::font::Weight::Bold,
			..iced::Font::DEFAULT
		})
	};
	let info = metrics.label();
	let mut token = column![
		heading("OpenRouter"),
		with_info(
			text("API token").width(Fill),
			"Stored privately on this device. Clear and save to remove.",
			info,
		),
		crate::ui::controls::text_input("OpenRouter API token", &voice.openrouter_token.0)
			.secure(true)
			.on_input(|s| msg(Action::OpenRouterToken(Token(s))))
			.on_submit(msg(Action::SaveOpenRouterToken)),
		button("Save token").on_press(msg(Action::SaveOpenRouterToken)),
	]
	.spacing(6);
	if let Some(status) = voice.token_status {
		token = token.push(text(status).size(12));
	}
	let voice_heading: iced::Element<'a, Message> = if voice.voice_tab == Provider::Whisper {
		with_info(
			heading("Voice input").width(Fill),
			"Uses the default microphone via arecord (alsa-utils). Click the microphone again to finish; maximum recording is 2 minutes.",
			info,
		)
	} else {
		heading("Voice input").into()
	};
	let mut voice_fields = column![
		voice_heading,
		provider_tabs(InputSection::Voice, voice.voice_tab)
	]
	.spacing(6);
	match voice.voice_tab {
		Provider::Whisper => {
			voice_fields = voice_fields.extend([
				with_info(
					text("Whisper server URL").width(Fill),
					"Optional. Uses the model loaded on the server.",
					info,
				),
				crate::ui::controls::text_input("http://server:8080", &settings.whisper_url)
					.on_input(|s| msg(Action::WhisperUrl(s)))
					.into(),
				with_info(
					text("whisper.cpp binary").width(Fill),
					"A name on PATH or a full path.",
					info,
				),
				crate::ui::controls::text_input("whisper-cli", &settings.binary)
					.on_input(|s| msg(Action::Binary(s)))
					.into(),
				with_info(
					text("Whisper model file").width(Fill),
					"A ggml .bin file.",
					info,
				),
				crate::ui::controls::text_input("/path/to/ggml-base.bin", &settings.whisper_model)
					.on_input(|s| msg(Action::WhisperModel(s)))
					.into(),
			]);
		}
		Provider::Ollama => {}
		Provider::OpenRouter => {
			voice_fields = voice_fields.push(text("Model ID")).push(
				crate::ui::controls::text_input("provider/model", &settings.voice_openrouter_model)
					.on_input(|s| msg(Action::OpenRouterModel(InputSection::Voice, s))),
			);
		}
	}
	if let Some(error) = &voice.error {
		voice_fields = voice_fields.push(text(error).style(text::danger));
	}
	let image_tab = if settings.image_provider == "openrouter" {
		Provider::OpenRouter
	} else {
		Provider::Ollama
	};
	let mut image = column![
		heading("Image input"),
		provider_tabs(InputSection::Image, image_tab)
	]
	.spacing(6);
	if image_tab == Provider::OpenRouter {
		image = image.push(text("Model ID")).push(
			crate::ui::controls::text_input("provider/model", &settings.image_openrouter_model)
				.on_input(|s| msg(Action::OpenRouterModel(InputSection::Image, s))),
		);
	} else {
		image = image.extend([
			text("Ollama server").into(),
			crate::ui::controls::text_input("http://localhost:11434", &settings.image_ollama_url)
				.on_input(|s| msg(Action::ImageUrl(s)))
				.into(),
			text("Ollama image model").into(),
			crate::ui::controls::text_input("", &settings.image_ollama_model)
				.on_input(|s| msg(Action::ImageModel(s)))
				.into(),
		]);
	}
	let extraction_tab = voice.extraction_tab.unwrap_or(Provider::Ollama);
	let mut extraction = column![
		heading("Text extraction"),
		provider_tabs(InputSection::Extraction, extraction_tab)
	]
	.spacing(6);
	if extraction_tab == Provider::OpenRouter {
		extraction = extraction.push(text("Model ID")).push(
			crate::ui::controls::text_input(
				"provider/model",
				&settings.extraction_openrouter_model,
			)
			.on_input(|s| msg(Action::OpenRouterModel(InputSection::Extraction, s))),
		);
	} else {
		extraction = extraction.extend([
			text("Ollama server").into(),
			crate::ui::controls::text_input("http://localhost:11434", &settings.ollama_url)
				.on_input(|s| msg(Action::Url(s)))
				.into(),
			text("Ollama extraction model").into(),
			crate::ui::controls::text_input("", &settings.model)
				.on_input(|s| msg(Action::Model(s)))
				.into(),
		]);
	}
	column![token, voice_fields, image, extraction]
		.spacing(16)
		.into()
}

struct Microphone;
impl iced::widget::canvas::Program<Message> for Microphone {
	type State = ();
	fn draw(
		&self,
		_: &(),
		renderer: &iced::Renderer,
		theme: &iced::Theme,
		bounds: iced::Rectangle,
		_: iced::mouse::Cursor,
	) -> Vec<iced::widget::canvas::Geometry> {
		use iced::widget::canvas::{Frame, Path, Stroke};
		let mut frame = Frame::new(renderer, bounds.size());
		let color = theme.palette().text;
		let capsule = Path::rounded_rectangle(
			iced::Point::new(9.0, 2.0),
			iced::Size::new(6.0, 13.0),
			iced::border::Radius::from(3.0),
		);
		frame.fill(&capsule, color);
		let stand = Path::new(|p| {
			p.move_to(iced::Point::new(5.0, 10.0));
			p.line_to(iced::Point::new(5.0, 12.0));
			p.bezier_curve_to(
				iced::Point::new(5.0, 21.0),
				iced::Point::new(19.0, 21.0),
				iced::Point::new(19.0, 12.0),
			);
			p.line_to(iced::Point::new(19.0, 10.0));
			p.move_to(iced::Point::new(12.0, 19.0));
			p.line_to(iced::Point::new(12.0, 23.0));
			p.move_to(iced::Point::new(8.0, 23.0));
			p.line_to(iced::Point::new(16.0, 23.0));
		});
		frame.stroke(&stand, Stroke::default().with_color(color).with_width(1.8));
		vec![frame.into_geometry()]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn legacy_extraction_settings_and_new_models_round_trip() {
		let mut settings: Settings = toml::from_str(
			r#"binary = "custom-whisper"
whisper_model = "/models/speech.bin"
ollama_url = "http://localhost:11434"
model = "granite"
"#,
		)
		.unwrap();
		assert_eq!(settings.binary, "custom-whisper");
		assert_eq!(settings.whisper_model, "/models/speech.bin");
		assert_eq!(settings.ollama_url, "http://localhost:11434");
		assert_eq!(settings.model, "granite");
		settings.voice_openrouter_model = "voice/model".into();
		settings.image_openrouter_model = "image/model".into();
		settings.extraction_openrouter_model = "text/model".into();
		settings.image_ollama_url = "http://image-server:11434".into();
		settings.image_ollama_model = "vision".into();
		let encoded = toml::to_string(&settings).unwrap();
		assert_eq!(toml::from_str::<Settings>(&encoded).unwrap(), settings);
		assert!(!encoded.contains("token"));
	}

	#[test]
	fn token_messages_redact_credentials() {
		let action = msg(Action::OpenRouterToken(Token("test-private-token".into())));
		let debug = format!("{action:?}");
		assert!(debug.contains("[REDACTED]"));
		assert!(!debug.contains("test-private-token"));
	}

	fn context() -> Value {
		json!({"projects":[{"id":1,"name":"Work"}]})
	}
	#[test]
	#[ignore = "requires a running local Ollama with qwen3.5:4b"]
	fn local_ollama_extracts_metadata_only_update() {
		let context = json!({"title":"Pay rent","due":"","deadline":"2026-12-05","projects":[],"metadata":{"labels":["home"]}});
		let p = extract(
			&Settings {
				model: "qwen3.5:4b".into(),
				..Settings::default()
			},
			context,
			"Update due date to November 3rd 2027",
			&AtomicBool::new(false),
			None,
		)
		.unwrap();
		assert_eq!(p.due.as_deref(), Some("2027-11-03"));
		assert!(p.title.is_none());
		assert!(p.deadline.is_none());
		assert!(p.labels.is_none());
	}
	#[test]
	fn time_only_defaults_to_today_and_stages_a_saved_schedule() {
		use chrono::Timelike;
		let context = json!({"due":"","today":"2026-10-30"});
		let patch = validate_patch(r#"{"due_time":"21:00"}"#, &context).unwrap();
		assert_eq!(patch.due.as_deref(), Some("2026-10-30"));
		assert!(patch.title.is_none());
		let mut state = State::blank();
		let _ = inbox::update(&mut state, inbox::Action::NewTask);
		inbox::apply_voice(&mut state, patch);
		let _ = inbox::update(&mut state, inbox::Action::Title("Buy bread".into()));
		let draft = state.inbox.draft.as_ref().unwrap();
		assert_eq!(draft.due, "2026-10-30");
		let event = draft.event.as_ref().unwrap();
		assert!(!event.all_day);
		assert_eq!((event.start.hour(), event.start.minute()), (21, 0));
		assert_eq!(event.end - event.start, chrono::Duration::minutes(30));
		assert_eq!(state.task_editor().start_time, "21:00");
		assert!(inbox::validate(draft).is_ok());
		inbox::apply_voice(
			&mut state,
			validate_patch(r#"{"due":"2026-11-03"}"#, &context).unwrap(),
		);
		let event = state.inbox.draft.as_ref().unwrap().event.as_ref().unwrap();
		assert_eq!(event.start.date_naive().to_string(), "2026-11-03");
		assert_eq!(event.start.hour(), 21);
		inbox::apply_voice(
			&mut state,
			validate_patch(r#"{"due_time":""}"#, &context).unwrap(),
		);
		let draft = state.inbox.draft.as_ref().unwrap();
		assert!(draft.event.as_ref().unwrap().all_day);
		assert_eq!(draft.due, "2026-11-03");
	}
	#[test]
	fn clock_validation_preserves_existing_day_and_rejects_invalid_times() {
		let context = json!({"due":"2027-11-03","today":"2026-10-30"});
		let patch = validate_patch(r#"{"due_time":"21:00"}"#, &context).unwrap();
		assert_eq!(patch.due.as_deref(), Some("2027-11-03"));
		for raw in [
			r#"{"due_time":"25:00"}"#,
			r#"{"due_time":"9pm"}"#,
			r#"{"due_time":"21:60"}"#,
			r#"{"due":"","due_time":"21:00"}"#,
		] {
			assert!(validate_patch(raw, &context).is_err(), "{raw}");
		}
	}
	#[test]
	#[ignore = "requires a running local Ollama with qwen3.5:4b"]
	fn local_ollama_extracts_implicit_today_at_nine_pm() {
		let context = json!({"title":"Buy bread","due":"","today":chrono::Local::now().date_naive().to_string(),"due_time":null,"projects":[]});
		let patch = extract(
			&Settings {
				model: "qwen3.5:4b".into(),
				..Settings::default()
			},
			context,
			"At 9pm",
			&AtomicBool::new(false),
			None,
		)
		.unwrap();
		assert_eq!(patch.due_time.as_deref(), Some("21:00"));
		assert_eq!(
			patch.due.as_deref(),
			Some(crate::clock::now().date_naive().to_string().as_str())
		);
		assert!(patch.title.is_none());
		assert!(patch.deadline.is_none());
	}
	#[test]
	fn model_debug_drains_both_streams_and_records_exit() {
		let (tx, rx) = mpsc::channel();
		let mut command = Command::new("sh");
		command.args(["-c", r#"i=0; while [ "$i" -lt 3000 ]; do printf 'stdout sample\n'; printf 'stderr sample\n' >&2; i=$((i+1)); done"#]);
		assert!(
			run_model(&mut command, &AtomicBool::new(false), &tx)
				.unwrap()
				.success()
		);
		let chunks: Vec<String> = rx
			.try_iter()
			.filter_map(|p| {
				if let Progress::Debug(v) = p {
					Some(v)
				} else {
					None
				}
			})
			.collect();
		let log = chunks.join("");
		assert!(log.contains("Whisper command"));
		assert!(log.contains("Whisper exit"));
		for name in ["stdout", "stderr"] {
			let label = format!("Whisper {name}\n");
			let stream: String = chunks
				.iter()
				.filter_map(|v| {
					v.split_once(&label)
						.map(|(_, body)| body.strip_suffix("\n\n").unwrap())
				})
				.collect();
			assert_eq!(stream, format!("{name} sample\n").repeat(3000));
		}
	}
	#[test]
	fn processing_buttons_open_live_debug_and_closing_keeps_worker() {
		use iced::{Size, widget::text_input};
		use iced_test::{Simulator, core::Settings as UiSettings};
		let mut state = State::blank();
		let _ = inbox::update(&mut state, inbox::Action::NewTask);
		let (tx, rx) = mpsc::channel();
		state.inbox.voice.rx = Some(rx);
		for (phase, label) in [
			("Transcribing", "Transcribing..."),
			("Extracting task details", "Extracting..."),
		] {
			tx.send(Progress::Phase(phase)).unwrap();
			tx.send(Progress::Debug(format!("{phase} live diagnostics")))
				.unwrap();
			let _ = update(&mut state, Action::Poll);
			let size = Size::new(420.0, 600.0);
			let mut ui = Simulator::with_size(
				UiSettings::default(),
				size,
				input(&state, text_input("Task", "").into()),
			);
			ui.click(label).unwrap();
			for message in ui.into_messages() {
				let _ = crate::app::update(&mut state, message);
			}
			assert!(state.inbox.voice.debug_open);
			let mut ui = Simulator::with_size(
				UiSettings::default(),
				size,
				debug_overlay(&state, &crate::ui::layout::Metrics::of(size)),
			);
			assert!(ui.find("Voice model debug").is_ok());
			ui.click("Close").unwrap();
			for message in ui.into_messages() {
				let _ = crate::app::update(&mut state, message);
			}
			assert!(!state.inbox.voice.debug_open);
			assert!(state.inbox.voice.active());
			assert!(state.inbox.voice.debug_log.contains("live diagnostics"));
		}
		tx.send(Progress::Done(Err("model failed".into()))).unwrap();
		let _ = update(&mut state, Action::Poll);
		assert!(!state.inbox.voice.active());
		let mut ui = Simulator::with_size(
			UiSettings::default(),
			Size::new(420.0, 600.0),
			input(&state, text_input("Task", "").into()),
		);
		ui.click("Model debug").unwrap();
	}
	#[test]
	fn debug_popup_stays_with_creation_in_the_main_window_and_dock() {
		use iced::{Size, window};
		use iced_test::{Simulator, core::Settings as UiSettings};
		let mut state = State::blank();
		let _ = inbox::update(&mut state, inbox::Action::NewTask);
		state.inbox.voice.debug_open = true;
		state.inbox.voice.debug_log = "Live model output".into();
		let main = window::Id::unique();
		let editor = window::Id::unique();
		state.main_window = Some(main);
		state.editor_window = Some(editor);
		for (id, visible) in [(main, true), (editor, false)] {
			let mut ui = Simulator::with_size(
				UiSettings::default(),
				Size::new(480.0, 900.0),
				crate::app::view(&state, id),
			);
			assert_eq!(ui.find("Voice model debug").is_ok(), visible);
		}
		state.editor_window = None;
		for dock in [false, true] {
			state.task_creation_window = dock;
			let mut ui = Simulator::with_size(
				UiSettings::default(),
				Size::new(480.0, 900.0),
				crate::app::view(&state, main),
			);
			assert!(ui.find("Voice model debug").is_ok());
		}
	}
	#[test]
	#[ignore = "requires a running local Ollama with qwen3.5:4b"]
	fn local_ollama_sets_spoken_task_title_and_time() {
		use chrono::Timelike;
		for previous_title in ["", "Buy groceries"] {
			let context = json!({"title":previous_title,"due":"","today":chrono::Local::now().date_naive().to_string(),"due_time":null,"projects":[],"metadata":{"labels":["study"]}});
			let patch = extract(
				&Settings {
					model: "qwen3.5:4b".into(),
					..Settings::default()
				},
				context,
				"Study machine learning at 10pm.",
				&AtomicBool::new(false),
				None,
			)
			.unwrap();
			assert_eq!(patch.title.as_deref(), Some("Study machine learning"));
			assert_eq!(patch.due_time.as_deref(), Some("22:00"));
			let mut state = State::blank();
			let _ = inbox::update(&mut state, inbox::Action::NewTask);
			let _ = inbox::update(&mut state, inbox::Action::Title(previous_title.into()));
			inbox::apply_voice(&mut state, patch);
			let draft = state.inbox.draft.as_ref().unwrap();
			assert_eq!(draft.title, "Study machine learning");
			assert_eq!(draft.event.as_ref().unwrap().start.hour(), 22);
		}
	}
	#[test]
	fn settings_round_trip_and_legacy_defaults() {
		let mut config: crate::config::Config = toml::from_str("").unwrap();
		assert!(config.voice.model.is_empty());
		let partial: crate::config::Config =
			toml::from_str("[voice]\nbinary = 'whisper-cli'").unwrap();
		assert!(partial.voice.model.is_empty());
		assert_eq!(config.voice.binary, "whisper-cli");
		config.voice.binary = "/opt/whisper/whisper-cli".into();
		config.voice.model = "another:latest".into();
		let restored: crate::config::Config =
			toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
		assert_eq!(restored.voice, config.voice);
		config.voice.model.clear();
		let restored: crate::config::Config =
			toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
		assert!(restored.voice.model.is_empty());
	}
	#[test]
	fn microphone_sits_right_of_input_at_narrow_and_wide_widths() {
		use iced::{
			Size,
			widget::{Id, text_input},
		};
		use iced_test::{Simulator, core::Settings as UiSettings};
		let mut state = State::blank();
		let _ = inbox::update(&mut state, inbox::Action::NewTask);
		for width in [180.0, 420.0] {
			let title = text_input("Task, or paste a link", "").id("voice-test-title");
			let mut ui = Simulator::with_size(
				UiSettings::default(),
				Size::new(width, 200.0),
				input(&state, title.into()),
			);
			let field = ui.find(Id::from("voice-test-title")).unwrap().bounds();
			let mic = ui.find(Id::from("voice-record")).unwrap().bounds();
			assert!(mic.x >= field.x + field.width);
			assert!(mic.x + mic.width <= width);
			ui.click(Id::from("voice-record")).unwrap();
			assert!(
				ui.into_messages()
					.any(|m| matches!(m, Message::Inbox(inbox::Action::Voice(Action::Toggle))))
			);
		}
	}
	#[test]
	fn echoed_context_is_removed_from_patch() {
		let context =
			json!({"title":"Pay rent","deadline":"2026-12-05","metadata":{"labels":["home"]}});
		let patch = validate_patch(
			r#"{"title":"Pay rent","deadline":"2026-12-05","labels":["home"],"due":"2027-11-03"}"#,
			&context,
		)
		.unwrap();
		assert_eq!(patch.due.as_deref(), Some("2027-11-03"));
		assert!(patch.title.is_none());
		assert!(patch.deadline.is_none());
		assert!(patch.labels.is_none());
	}
	#[test]
	fn metadata_only_patch_does_not_invent_title_or_clear_fields() {
		let p = validate_patch(r#"{"due":"2026-11-03"}"#, &context()).unwrap();
		assert!(p.title.is_none());
		assert!(p.deadline.is_none());
		assert!(p.labels.is_none());
		assert_eq!(p.due.as_deref(), Some("2026-11-03"));
	}
	#[test]
	fn explicit_clears_and_rejections() {
		let p = validate_patch(
			r#"{"deadline":"","labels":[],"priority":"none"}"#,
			&context(),
		)
		.unwrap();
		assert_eq!(p.priority, Some(None));
		assert_eq!(p.labels, Some(vec![]));
		for raw in [
			r#"{"due":"2026-02-30"}"#,
			r#"{"projects":[999]}"#,
			r#"{"minutes":10081}"#,
			r#"{"title":""}"#,
			r#"{"unexpected":true}"#,
			r#"{}"#,
			r#"{"due":null,"title":null,"priority":null}"#,
			r#"{"priority":"P3"}"#,
		] {
			assert!(validate_patch(raw, &context()).is_err(), "{raw}");
		}
	}
	#[test]
	fn repeated_patches_preserve_unmentioned_metadata_and_manual_title() {
		let mut state = State::blank();
		let _ = inbox::update(&mut state, inbox::Action::NewTask);
		inbox::apply_voice(
			&mut state,
			validate_patch(
				r#"{"title":"Pay rent","deadline":"2026-11-05","labels":["home"],"priority":"P1"}"#,
				&context(),
			)
			.unwrap(),
		);
		inbox::apply_voice(
			&mut state,
			validate_patch(r#"{"due":"2026-11-03"}"#, &context()).unwrap(),
		);
		let draft = state.inbox.draft.as_ref().unwrap();
		assert_eq!(draft.title, "Pay rent");
		assert_eq!(draft.deadline, "2026-11-05");
		assert_eq!(
			crate::timeblocking::metadata(draft.event.as_ref().unwrap()).labels,
			vec!["home"]
		);
		let _ = inbox::update(&mut state, inbox::Action::Title("Pay rent soon".into()));
		assert_eq!(state.task_editor().date, "2026-11-03");
	}
}
