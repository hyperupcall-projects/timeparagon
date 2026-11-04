# Documentation

There's currently two implementations of TimeSlay:

- An [Iced](https://iced.rs) desktop application.
- A minimal [Dioxus](https://dioxuslabs.com) mobile application.

They can be built by doing:

```sh
# Iced desktop app.
cargo run -p timeslay --release

# Install Iced desktop app.
./packaging/install.sh

# Dioxus mobile app.
./scripts/build-android.sh
```

Android app releases are built and published in CI. It is recommended to use [Obtainium](https://github.com/ImranR98/Obtainium) to download them.


## Features

- [Notes](docs/notes.md)
- [Voice input](docs/voice-input.md)


## External APIs and Services

TimeSlay integrates with a lot of tools:

- Google Cloud Account
	- Required.
	- For Google Calendar, Tasks, Drive, and Gmail functionality.
- [Mapbox account](https://mapbox.com)
	- Recommended.
	- To set locations of events and calculate travel times.
- [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
	- For translating voice to text, to modify the calendar.
	- Recommended to compile from source so it uses machine's GPU/NPU.
	- [Whisper model files](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md)
- [Ollama](https://ollama.com)
	- For extracting metadata like title, due dates, and more from text.
- Slack account && Slack app
	- For adding Slack messages into the inbox.
- Browser extension
	- For adding Website URLs to the inbox
- [Zoom for Google Workspace](https://workspace.google.com/u/0/marketplace/app/zoom_for_google_workspace/364750910244).
