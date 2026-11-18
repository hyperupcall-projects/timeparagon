//! A reviewed timed task is submitted complete in one write, without a follow-up edit.
use chrono::{Duration, Local, TimeZone};
use timeparagon_core::{
	calendar::{self, operations},
	config::CalendarSettings,
	timeblocking::{self, Metadata, Priority},
};

#[test]
fn timed_ai_task_keeps_its_clock_description_and_metadata_in_the_durable_overlay() {
	let settings = CalendarSettings {
		app_calendar: Some("app".into()),
		..Default::default()
	};
	let start = Local.with_ymd_and_hms(2026, 11, 17, 21, 0, 0).unwrap();
	let metadata = Metadata {
		minutes: 45,
		labels: vec!["home".into()],
		priority: Some(Priority::P1),
		..Default::default()
	};
	let task = operations::create_task_at(
		&settings,
		"Read",
		"Chapter three",
		None,
		Some(metadata.clone()),
		Some((start, start + Duration::minutes(45))),
	)
	.unwrap();
	assert!(!task.all_day);
	assert_eq!(task.start, start);
	assert_eq!(task.end - start, Duration::minutes(45));
	assert_eq!(task.calendar_id, "app");
	assert_eq!(task.details.description, "Chapter three");
	let saved = operations::cached_event(&task.calendar_id, &task.id)
		.unwrap()
		.unwrap();
	assert_eq!(saved.start, start);
	assert!(!saved.all_day);
	let saved_metadata = timeblocking::metadata(&saved);
	assert_eq!(saved_metadata.minutes, 45);
	assert_eq!(saved_metadata.labels, metadata.labels);
	assert_eq!(saved_metadata.priority, metadata.priority);
	assert!(calendar::calendar_tasks::body(&saved).unwrap()["start"]["dateTime"].is_string());
	assert!(
		operations::create_task_at(&settings, "Invalid", "", None, None, Some((start, start)))
			.is_err()
	);
}

fn model_server(response: serde_json::Value) -> (String, std::thread::JoinHandle<String>) {
	use std::io::{Read, Write};
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());
	let worker = std::thread::spawn(move || {
		let (mut stream, _) = listener.accept().unwrap();
		stream
			.set_read_timeout(Some(std::time::Duration::from_secs(5)))
			.unwrap();
		let mut data = Vec::new();
		let mut buffer = [0; 4096];
		loop {
			let n = stream.read(&mut buffer).unwrap();
			assert!(n > 0);
			data.extend_from_slice(&buffer[..n]);
			if let Some(end) = data.windows(4).position(|v| v == b"\r\n\r\n") {
				let header = String::from_utf8_lossy(&data[..end]);
				let length: usize = header
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length:")
							.map(|v| v.trim().parse().unwrap())
					})
					.unwrap();
				if data.len() >= end + 4 + length {
					break;
				}
			}
		}
		let body = response.to_string();
		write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
		String::from_utf8(data).unwrap()
	});
	(url, worker)
}

#[test]
fn native_wav_uses_whisper_multipart_and_rejects_an_empty_transcript() {
	use timeparagon_core::ai_input::{Settings, voice};
	let (url, server) = model_server(serde_json::json!({"text":"  Study at nine  "}));
	let settings = Settings {
		whisper_url: url,
		..Default::default()
	};
	assert_eq!(
		voice::transcribe(&settings, b"RIFF-sample-WAVE").unwrap(),
		"Study at nine"
	);
	let request = server.join().unwrap();
	assert!(request.starts_with("POST /inference "));
	for value in [
		"multipart/form-data",
		"name=\"language\"\r\n\r\nauto",
		"filename=\"recording.wav\"",
		"RIFF-sample-WAVE",
	] {
		assert!(request.contains(value), "Missing {value}");
	}
	let (url, server) = model_server(serde_json::json!({"text":"   "}));
	assert!(
		voice::transcribe(
			&Settings {
				whisper_url: url,
				..Default::default()
			},
			b"RIFF-sample-WAVE"
		)
		.is_err()
	);
	server.join().unwrap();
}

#[test]
fn picture_requests_use_the_configured_vision_model_and_validate_the_response() {
	use serde_json::json;
	use timeparagon_core::ai_input::{Settings, image};
	let event = json!({"title":"Talk","date":"2026-11-17","location":"Library"});
	let (url, server) =
		model_server(json!({"message":{"content":json!({"events":[event]}).to_string()}}));
	let settings = Settings {
		image_ollama_url: url,
		image_ollama_model: "selected-vision".into(),
		..Default::default()
	};
	let events = image::extract(b"\x89PNG\r\n\x1a\nimage", &settings).unwrap();
	assert_eq!(events[0].title, "Talk");
	assert_eq!(events[0].location.as_deref(), Some("Library"));
	let request = server.join().unwrap();
	let body: serde_json::Value =
		serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
	assert_eq!(body["model"], "selected-vision");
	assert!(body["messages"][1]["images"][0].is_string());
	assert!(body["format"]["properties"]["events"].is_object());
	assert!(image::extract(b"not a picture", &settings).is_err());
	let (url, server) =
		model_server(json!({"message":{"content":"{\"events\":[],\"unexpected\":true}"}}));
	assert!(
		image::extract(
			b"\x89PNG\r\n\x1a\nimage",
			&Settings {
				image_ollama_url: url,
				..settings
			}
		)
		.is_err()
	);
	server.join().unwrap();
}
