# Voice task entry

Click the microphone to the right of “Task, or paste a link”, speak, then click
again to finish. Recording stops after two minutes. While models run, a **Transcribing...** or **Extracting...** button opens a live
debug popup. It shows the Whisper command, stdout/stderr, transcript, exit status
and elapsed time, plus Ollama discovery, full chat request/response (including
model timing/token statistics), validated patch, and errors. Closing the popup
leaves processing running; Escape or clicking outside also closes it. **Copy debug
log** copies the details. After completion or failure, **Model debug** reopens the
log. Logs live in memory and reset on a new recording or closing the draft.

Quiet input keeps the normal
button background; louder input progressively lightens it, with a 50 ms ease-in-out
transition in both directions. The microphone has no hover tooltip. Status text distinguishes
listening, transcription and extraction. The transcript is available in the model debug popup; the task form shows the
extracted title and metadata without a completion message or transcript caption. Click again to refine the same draft; save
with the existing task creation controls when ready.

## Setup

In Settings → Voice input:

- Set the whisper.cpp executable: `whisper-cli` is resolved through PATH; an
  absolute executable path works too. This is a filename, not a shell command.
- Set the local Whisper GGML model file, for example `/models/ggml-base.bin`.
  Models are not downloaded automatically.
- Set the Ollama URL (default `http://localhost:11434`).
- Discover installed models, filter by name, and select one. `qwen3.5:4b` is
  preferred by default. Discovery also runs when the first task draft opens.
  If it is absent, select another installed text/chat model. Nothing is pulled
  automatically; embedding-only models cannot perform extraction.

Linux recording requires `arecord` from `alsa-utils` and a working default ALSA
microphone. This initial implementation uses the system's default capture device;
configure its routing in the system sound settings. It does not change the playback
or alarm settings. Whisper and Ollama are independent models; both are required.

## Mini-pipeline

```text
Microphone (16 kHz, mono, signed 16-bit PCM)
  → temporary WAV + RMS volume meter
  → whisper.cpp CLI → transcript
  → Ollama chat + JSON schema + current draft + local clock + project catalog
  → strict Rust deserialization and semantic validation
  → atomic patch of the open draft
  → user can correct fields, record another patch, or save
```

1. `arecord` captures raw PCM. RMS values below 0.01 do not brighten the button;
   above it brightness increases with volume. A simple energy gate rejects silent
   recordings before Whisper. This is not a semantic voice activity detector: loud
   noise can pass it. There is no silence-based automatic submission.
2. Write a private temporary WAV and run
   `whisper-cli -m MODEL -f WAV -otxt -of PREFIX -l auto`. Read `PREFIX.txt`, rather
   than parsing console logs. This transcribes speech in its detected language;
   it does not request English translation. Executable and filenames are passed
   as individual process arguments, without a shell.
3. Discover installed models using `GET /api/tags`. Require the selected model to
   be present, then call `POST /api/chat`, with `stream:false`, `think:false`, temperature zero, a 1024-token output limit
   and a JSON schema in `format`. Date patterns constrain output syntax; worked
   examples demonstrate metadata-only updates and explicit clearing. Both the transcript and current draft are data
   inside the user message. The system message defines the extraction rules.
4. Supply the local timestamp/timezone and known project IDs. The prompt resolves
   relative dates and treats yearless dates as their next occurrence. It distinguishes
   planned due dates from hard deadlines, and forbids invented metadata. Ambiguous
   fields should be null. Model output remains probabilistic: review the form.
5. Remove model-echoed values that equal the request context, so stale unchanged
   fields cannot overwrite concurrent manual edits. Validate the entire response before changing any field: unknown keys, invalid
   dates/deadlines, unknown project IDs, unsupported priorities, blank/oversized
   titles, negative estimates and estimates above one week reject the response.
   Empty/no-change output asks the user to rephrase. Failure leaves the draft
   intact, exposes the error, and retains the transcript.
6. Apply only non-null fields. Reset quick-add preview baselines so subsequent
   typing cannot undo the voice metadata. Keep changes in the draft; extraction
   never creates a task automatically. Saving is disabled while voice work is active.

## Recommended extraction contract: patches, not complete tasks

The best fit here is schema-constrained extraction into an allowlisted patch,
followed by deterministic validation. A full-task response encourages the model to
repeat or guess fields, which risks erasing earlier recordings and manual edits.
No agent tools, autonomous actions, or conversation replay are needed. On the wire,
all schema keys are required and nullable: `null` means unchanged. This gives small
models an explicit decision for each field instead of allowing an empty JSON object.
The internal Rust patch also accepts missing keys as unchanged. The compact examples
below show only changed fields; a wire response includes the other keys as null.

| Field | Value | Null or omitted means |
| --- | --- | --- |
| `title` | Nonblank task title, at most 1024 characters | Keep title, including a blank initial title |
| `description` | String; empty clears | Keep description |
| `due` | `YYYY-MM-DD`; empty clears | Keep due date |
| `due_time` | 24-hour `HH:MM`; empty clears time | Keep due time |
| `deadline` | `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`; empty clears | Keep deadline |
| `priority` | `P1`, `P2`, or `none` to clear | Keep priority |
| `minutes` | Integer 0–10080; zero clears estimate | Keep estimate |
| `labels` | Complete replacement array; empty clears | Keep labels |
| `projects` | Complete replacement array of known IDs; empty clears | Keep projects |

For array additions/removals, the model gets current values and must return the
updated array, retaining unrelated entries. Due and deadline are separate fields.
Internal task IDs, creation timestamps, sync links, ordering, completion and calendar
writes are deliberately outside this extraction contract.

Examples, with local date October 30, 2026:

```json
{"title":"Send the report","due":"2026-11-02","minutes":30}
```

“Update due date to November 3rd” on that draft:

```json
{"due":"2026-11-03"}
```

“Study machine learning at 10pm.”:

```json
{"title":"Study machine learning","due":"2026-10-30","due_time":"22:00"}
```

A spoken task sets the title input even when the draft already has another title;
metadata-only corrections leave the title intact. Scheduling phrases are removed
from the extracted title.

“At 9pm” on an undated draft, with local date October 30, 2026:

```json
{"due":"2026-10-30","due_time":"21:00"}
```

A clock time without a date keeps an existing due day, otherwise uses today,
even if that time has already passed. Due-time extraction updates both the visible
clock fields and the draft event; saving uses the same scheduling path as entering
a time manually. A new timed task uses its estimate as its duration, or 30 minutes
if no estimate is set. Rescheduling an already timed draft keeps its duration.
A date-only correction keeps its clock time. Clearing `due_time` makes the task
all-day and keeps the date; clearing `due` removes its timed schedule too.
Invalid times (including nonexistent local times during a DST change) are rejected
before applying the patch. Deadlines remain independent.

“Remove the deadline”:

```json
{"deadline":""}
```

A metadata-only command returns `title:null` and may leave a new draft without a title; normal save validation
still requires a task name. Additional recordings always receive the latest draft,
including prior patches. Fields explicitly returned by an in-flight extraction win
over concurrent edits to those same fields; unmentioned fields retain current values.

## Lifecycle and verification

The UI polls worker progress without blocking. Closing the draft stops capture and
detaches its result receiver; old results cannot reach a new draft. A watchdog kills
and reaps the recorder on stop, cancellation or the two-minute limit, including when
capture is blocked. Whisper is cancelled on draft closure and has a three-minute
limit. Ollama has a five-second connection timeout and three-minute request timeout;
an HTTP request already underway can finish in the background after cancellation,
but its response cannot update another draft. Temporary audio and transcript files
are removed when the worker exits. The audio is not added to the task. The session debug log includes the transcript
and extraction context in memory. A custom
Ollama server receives transcript and draft context, so use a trusted server.

Run the deterministic validation and repeated-update tests with:

```sh
cargo test --offline --lib voice::tests
```

An optional real-model regression verifies that a spoken date-only update does not
return a replacement title or erase other fields:

```sh
cargo test --offline --lib voice::tests::local_ollama_extracts_metadata_only_update -- --ignored
```

Implementation verification: eleven deterministic voice tests and 38 existing inbox
UI/state tests passed. The opt-in date-only regression also passed against the
installed `qwen3.5:4b`. A live “At 9pm” regression also passed, extracting today
and `21:00` without replacing the task title or deadline. The spoken-task regression passed
for both blank and existing titles: “Study machine learning at 10pm.” fills
“Study machine learning” and `22:00`. Live testing exposed context echoing and timestamp-shaped
dates; the final pipeline strips unchanged context values and constrains date formats.
Physical microphone capture and Whisper recognition were not exercised.

Manual checks: microphone access failure; silence; loud/quiet meter; click-to-stop;
transcription failure; unavailable Ollama; missing selected model; model search;
first task; metadata-only initial recording; second recording; explicit clear;
close/reopen while recording and processing; final saved metadata. The automated
suite does not capture real microphone audio or measure speech recognition accuracy.

## API references

- [whisper.cpp CLI options](https://github.com/ggml-org/whisper.cpp/blob/master/examples/cli/README.md)
- [Ollama installed models API](https://docs.ollama.com/api/tags)
- [Ollama chat and structured output format](https://docs.ollama.com/api/chat)
