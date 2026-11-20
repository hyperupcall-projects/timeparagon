//! Shared model settings and validated AI input contracts for both apps.
use serde::{Deserialize, Serialize};
pub mod events;
pub mod image;
pub mod text;
pub mod voice;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
	pub binary: String,
	pub whisper_url: String,
	pub whisper_model: String,
	pub ollama_url: String,
	pub model: String,
	pub voice_openrouter_model: String,
	pub image_ollama_url: String,
	pub image_provider: String,
	pub image_ollama_model: String,
	pub image_openrouter_model: String,
	pub extraction_openrouter_model: String,
	/// "ollama" or "openrouter": which reads events out of typed or pasted text.
	pub extraction_provider: String,
}
impl Default for Settings {
	fn default() -> Self {
		Self {
			binary: "whisper-cli".into(),
			whisper_url: String::new(),
			whisper_model: String::new(),
			ollama_url: "http://localhost:11434".into(),
			model: String::new(),
			voice_openrouter_model: String::new(),
			image_ollama_url: "http://localhost:11434".into(),
			image_provider: "ollama".into(),
			image_ollama_model: String::new(),
			image_openrouter_model: String::new(),
			extraction_openrouter_model: String::new(),
			extraction_provider: "ollama".into(),
		}
	}
}
