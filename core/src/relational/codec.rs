//! Typed models <-> scalar SQL rows. Serialization here is an in-memory API
//! adapter, never the database representation or conflict-resolution policy.
use super::*;
use crate::habits::{DayLog, Frequency, Habit, Library, LogEntry, Mark, Measure, Section};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn value(v: impl serde::Serialize) -> io::Result<Value> {
	serde_json::to_value(v).map_err(invalid)
}
fn model<T: serde::de::DeserializeOwned>(v: Value) -> io::Result<T> {
	serde_json::from_value(v).map_err(invalid)
}
fn ordered(rows: &Rows, table: usize, position: usize) -> Vec<&Vec<Value>> {
	let mut result = rows
		.iter()
		.filter(|((i, _), _)| *i == table)
		.map(|(_, r)| r)
		.collect::<Vec<_>>();
	result.sort_by(|a, b| {
		a[position]
			.as_i64()
			.cmp(&b[position].as_i64())
			.then_with(|| a[0].to_string().cmp(&b[0].to_string()))
	});
	result
}
/// The ID of an entry, session or distraction from before each had one of its
/// own: a hash of what it records, so identical history gets identical IDs on
/// every device. Each is given its ID when it is made, and these work out the
/// ones the rows from before already have.
fn stable_id(parts: Value, ordinal: usize) -> String {
	format!(
		"{:x}",
		Sha256::digest(json!([parts, ordinal]).to_string().as_bytes())
	)
}

/// The ID of a work session from before sessions had one: its task key and
/// start (`started_at` as stored), numbered among sessions that share both.
pub(super) fn session_id(task_key: &str, started_at: &Value, ordinal: usize) -> String {
	stable_id(json!([task_key, started_at]), ordinal)
}

/// The ID of a log entry from before entries had one: what it records,
/// numbered among identical entries. `counts` is shared by one day's
/// entries.
pub(super) fn log_entry_id(
	kind: &str,
	at: &Value,
	amount: &Value,
	note: &str,
	counts: &mut BTreeMap<String, usize>,
) -> String {
	let parts = json!([kind, at, amount, note]);
	let count = counts.entry(parts.to_string()).or_default();
	let id = stable_id(parts, *count);
	*count += 1;
	id
}

/// A task ID as `timeblocking::task_key` writes one.
fn task_id(text: &str) -> Option<i64> {
	text.parse::<i64>().ok().filter(|id| id.to_string() == text)
}

/// A work session's task key (`timeblocking::task_key`) as the columns
/// `task_id, occurrence_id, task_calendar_id, task_event_id`: the task's ID,
/// with the event ID of the one occurrence worked on when the task is a
/// recurring dated one (`<id>/<event>`); or, for a task that had no ID, its
/// Google identity (`<calendar>/<event>`) instead.
pub(super) fn task_columns(key: &str) -> io::Result<[Value; 4]> {
	let (head, tail) = match key.split_once('/') {
		Some((head, tail)) => (head, Some(tail)),
		None => (key, None),
	};
	Ok(match (task_id(head), tail) {
		(Some(id), occurrence) => [
			id.into(),
			occurrence.map_or(Value::Null, Value::from),
			Value::Null,
			Value::Null,
		],
		(None, Some(event)) => [Value::Null, Value::Null, head.into(), event.into()],
		(None, None) => {
			return Err(invalid(format!(
				"the task {key:?} is neither a task ID nor a Google identity"
			)));
		}
	})
}

/// The task key [`task_columns`] stored.
fn task_key(columns: &[Value]) -> io::Result<String> {
	use Value::{Null, Number, String};
	match columns {
		[Number(id), Null, Null, Null] => Ok(id.to_string()),
		[Number(id), String(occurrence), Null, Null] => Ok(format!("{id}/{occurrence}")),
		[Null, Null, String(calendar), String(event)] => Ok(format!("{calendar}/{event}")),
		_ => Err(invalid(
			"a work session's task is neither a task ID nor a Google identity",
		)),
	}
}

/// A block key (`timeblocking::block_key`), `<calendar>/<event>`, as the
/// columns `block_calendar_id, block_event_id`.
pub(super) fn block_columns(key: &str) -> io::Result<[Value; 2]> {
	key.split_once('/')
		.map(|(calendar, event)| [calendar.into(), event.into()])
		.ok_or_else(|| invalid(format!("the block {key:?} is not a Google identity")))
}

/// The block key [`block_columns`] stored, if any.
fn block_key(calendar: &Value, event: &Value) -> io::Result<Option<String>> {
	match (calendar, event) {
		(Value::String(calendar), Value::String(event)) => Ok(Some(format!("{calendar}/{event}"))),
		(Value::Null, Value::Null) => Ok(None),
		_ => Err(invalid("a block is half a Google identity")),
	}
}

/// A `work_sessions` row. `started_at` and `stopped_at` are as stored.
pub(super) fn session_row(
	id: &str,
	task_key: &str,
	title: &str,
	block_key: Option<&str>,
	started_at: Value,
	stopped_at: Value,
	position: i64,
) -> io::Result<Vec<Value>> {
	let mut row = vec![json!(id)];
	row.extend(task_columns(task_key)?);
	row.push(json!(title));
	row.extend(match block_key {
		Some(key) => block_columns(key)?,
		None => [Value::Null, Value::Null],
	});
	row.extend([started_at, stopped_at, json!(position)]);
	Ok(row)
}

/// A `work_distractions` row. `recorded_at` is as stored.
pub(super) fn distraction_row(
	id: &str,
	block_key: &str,
	minutes: Value,
	recorded_at: Value,
	position: i64,
) -> io::Result<Vec<Value>> {
	let mut row = vec![json!(id)];
	row.extend(block_columns(block_key)?);
	row.extend([minutes, recorded_at, json!(position)]);
	Ok(row)
}

pub(super) fn encode(key: &str, v: &Value) -> io::Result<Rows> {
	let mut rows = Rows::new();
	match key {
		"habits" => {
			let library: Library = model(v.clone())?;
			for (p, section) in library.sections.iter().enumerate() {
				insert(
					&mut rows,
					0,
					vec![
						json!(section.id),
						json!(section.name),
						json!(section.collapsed),
						json!(p),
					],
				)?;
			}
			let sections = library
				.sections
				.iter()
				.map(|s| s.id.as_str())
				.collect::<BTreeSet<_>>();
			for (p, h) in library.habits.iter().enumerate() {
				if h.section.as_deref().is_some_and(|s| !sections.contains(s)) {
					return Err(invalid("habit refers to a missing section"));
				}
				let (kind, times, days) = match h.frequency {
					Frequency::Interval { times, days } => ("interval", json!(times), json!(days)),
					Frequency::TimesPerDay { times } => {
						("times_per_day", json!(times), Value::Null)
					}
					Frequency::Weekdays { days } => {
						for (weekday, on) in days.into_iter().enumerate() {
							if on {
								insert(&mut rows, 2, vec![json!(h.id), json!(weekday)])?;
							}
						}
						("weekdays", Value::Null, Value::Null)
					}
				};
				insert(
					&mut rows,
					1,
					vec![
						json!(h.id),
						json!(h.name),
						value(h.kind)?,
						value(h.created)?,
						json!(h.question),
						json!(h.notes),
						json!(h.color),
						json!(kind),
						times,
						days,
						json!(h.archived),
						json!(h.skip_days),
						json!(h.question_marks),
						value(h.icons)?,
						value(&h.section)?,
						json!(h.measure.is_some()),
						value(h.measure.as_ref().map(|m| &m.unit))?,
						value(h.measure.as_ref().and_then(|m| m.target))?,
						value(h.measure.as_ref().map(|m| m.days))?,
						value(h.area.map(|a| a.to_string()))?,
						json!(h.distraction),
						json!(p),
					],
				)?;
				let dates = h
					.marks
					.keys()
					.chain(h.resisted.keys())
					.chain(h.values.keys())
					.chain(h.log.keys())
					.copied()
					.collect::<BTreeSet<_>>();
				for date in dates {
					let (mark, repetitions) = match h.marks.get(&date) {
						Some(Mark::Yes) => (json!("yes"), Value::Null),
						Some(Mark::No) => (json!("no"), Value::Null),
						Some(Mark::Skip) => (json!("skip"), Value::Null),
						Some(Mark::Reps(n)) => (json!("reps"), json!(n)),
						None => (Value::Null, Value::Null),
					};
					insert(
						&mut rows,
						3,
						vec![
							json!(h.id),
							value(date)?,
							mark,
							repetitions,
							value(h.resisted.get(&date))?,
							value(h.values.get(&date))?,
							json!(h.log.contains_key(&date)),
						],
					)?;
					if let Some(log) = h.log.get(&date) {
						let mut counts = BTreeMap::<String, usize>::new();
						for (kind, entries) in
							[("happened", &log.happened), ("resisted", &log.resisted)]
						{
							// Entries are kept in order of time (`Habit::set_log`), so
							// the position only orders entries logged at the same time,
							// and removing one leaves the rest of the day unchanged.
							let mut same_time = BTreeMap::<Option<chrono::NaiveTime>, usize>::new();
							for e in entries {
								let p = same_time.entry(e.at).or_default();
								let p = std::mem::replace(p, *p + 1);
								let (at, amount) = (value(e.at)?, value(e.amount)?);
								let id = if e.id.is_empty() {
									log_entry_id(kind, &at, &amount, &e.note, &mut counts)
								} else {
									e.id.clone()
								};
								insert(
									&mut rows,
									4,
									vec![
										json!(h.id),
										value(date)?,
										json!(id),
										json!(kind),
										json!(p),
										at,
										amount,
										json!(e.note),
									],
								)?;
							}
						}
					}
				}
			}
			insert(
				&mut rows,
				5,
				vec![
					json!(1),
					value(library.sort)?,
					json!(library.reversed),
					json!(library.hide_archived),
					json!(library.hide_completed),
					value(library.score_bucket)?,
					value(library.history_bucket)?,
				],
			)?;
		}
		"shared.brain_dump" => {
			let dumps: crate::brain_dump::Dumps = model(v.clone())?;
			for (p, e) in dumps.entries.iter().enumerate() {
				insert(&mut rows, 6, vec![value(e.date)?, json!(e.text), json!(p)])?;
			}
		}
		"shared.task_work" => {
			let log: crate::task_work::WorkLog = model(v.clone())?;
			let mut counts = BTreeMap::<String, usize>::new();
			for (p, s) in log.sessions.iter().enumerate() {
				let started_at = value(s.started_at)?;
				let id = if s.id.is_empty() {
					let count = counts
						.entry(json!([s.task_key, started_at]).to_string())
						.or_default();
					let id = session_id(&s.task_key, &started_at, *count);
					*count += 1;
					id
				} else {
					s.id.clone()
				};
				let row = session_row(
					&id,
					&s.task_key,
					&s.task_title,
					s.block_key.as_deref(),
					started_at,
					value(s.stopped_at)?,
					p as i64,
				)?;
				insert(&mut rows, 7, row)?;
			}
			counts.clear();
			for (p, d) in log.distractions.iter().enumerate() {
				let id = if d.id.is_empty() {
					let parts = json!([d.block_key, d.recorded_at]);
					let count = counts.entry(parts.to_string()).or_default();
					let id = stable_id(parts, *count);
					*count += 1;
					id
				} else {
					d.id.clone()
				};
				let row = distraction_row(
					&id,
					&d.block_key,
					json!(d.minutes),
					value(d.recorded_at)?,
					p as i64,
				)?;
				insert(&mut rows, 8, row)?;
			}
		}
		_ => return Err(invalid("unknown relational document")),
	}
	Ok(rows)
}

pub(super) fn decode(key: &str, rows: &Rows) -> io::Result<Value> {
	match key {
		"habits" => {
			let mut library = Library::default();
			for r in ordered(rows, 0, 3) {
				library.sections.push(Section {
					id: model(r[0].clone())?,
					name: model(r[1].clone())?,
					collapsed: model(r[2].clone())?,
				});
			}
			if let Some(r) = rows.iter().find(|((i, _), _)| *i == 5).map(|(_, r)| r) {
				library.sort = model(r[1].clone())?;
				library.reversed = model(r[2].clone())?;
				library.hide_archived = model(r[3].clone())?;
				library.hide_completed = model(r[4].clone())?;
				library.score_bucket = model(r[5].clone())?;
				library.history_bucket = model(r[6].clone())?;
			}
			for r in ordered(rows, 1, 21) {
				let id: String = model(r[0].clone())?;
				let mut h = Habit::new(&id, model::<String>(r[1].clone())?);
				h.kind = model(r[2].clone())?;
				h.created = model(r[3].clone())?;
				h.question = model(r[4].clone())?;
				h.notes = model(r[5].clone())?;
				h.color = model(r[6].clone())?;
				h.frequency = match r[7].as_str() {
					Some("interval") => Frequency::Interval {
						times: model(r[8].clone())?,
						days: model(r[9].clone())?,
					},
					Some("times_per_day") => Frequency::TimesPerDay {
						times: model(r[8].clone())?,
					},
					Some("weekdays") => {
						let mut days = [false; 7];
						for w in rows
							.iter()
							.filter(|((i, _), w)| *i == 2 && w[0] == r[0])
							.map(|(_, w)| w)
						{
							let day: usize = model(w[1].clone())?;
							*days
								.get_mut(day)
								.ok_or_else(|| invalid("invalid weekday"))? = true;
						}
						Frequency::Weekdays { days }
					}
					_ => return Err(invalid("invalid habit frequency")),
				};
				h.archived = model(r[10].clone())?;
				h.skip_days = model(r[11].clone())?;
				h.question_marks = model(r[12].clone())?;
				h.icons = model(r[13].clone())?;
				h.section = model(r[14].clone())?;
				if r[15] == true {
					h.measure = Some(Measure {
						unit: model(r[16].clone())?,
						target: model(r[17].clone())?,
						days: model(r[18].clone())?,
					});
				}
				h.area = model::<Option<String>>(r[19].clone())?
					.map(|s| s.parse::<u64>())
					.transpose()
					.map_err(invalid)?;
				h.distraction = model(r[20].clone())?;
				for d in rows
					.iter()
					.filter(|((i, _), d)| *i == 3 && d[0] == r[0])
					.map(|(_, d)| d)
				{
					let date = model(d[1].clone())?;
					let mark = match d[2].as_str() {
						Some("yes") => Some(Mark::Yes),
						Some("no") => Some(Mark::No),
						Some("skip") => Some(Mark::Skip),
						Some("reps") => Some(Mark::Reps(model(d[3].clone())?)),
						None => None,
						_ => return Err(invalid("invalid habit mark")),
					};
					if let Some(mark) = mark {
						h.marks.insert(date, mark);
					}
					if !d[4].is_null() {
						h.resisted.insert(date, model(d[4].clone())?);
					}
					if !d[5].is_null() {
						h.values.insert(date, model(d[5].clone())?);
					}
					if d[6] == true {
						h.log.insert(date, DayLog::default());
					}
				}
				let mut entries = rows
					.iter()
					.filter(|((i, _), e)| *i == 4 && e[0] == r[0])
					.map(|(_, e)| e)
					.collect::<Vec<_>>();
				// By time, then position among entries at the same time. Entries
				// two devices added at the same time and position follow their ID.
				entries.sort_by(|a, b| {
					(a[5].as_str(), a[4].as_i64(), a[2].as_str()).cmp(&(
						b[5].as_str(),
						b[4].as_i64(),
						b[2].as_str(),
					))
				});
				for e in entries {
					let log = h.log.entry(model(e[1].clone())?).or_default();
					let entry = LogEntry {
						id: model(e[2].clone())?,
						at: model(e[5].clone())?,
						amount: model(e[6].clone())?,
						note: model(e[7].clone())?,
					};
					match e[3].as_str() {
						Some("happened") => log.happened.push(entry),
						Some("resisted") => log.resisted.push(entry),
						_ => return Err(invalid("invalid habit log kind")),
					}
				}
				library.habits.push(h);
			}
			value(library)
		}
		"shared.brain_dump" => {
			let mut dumps = crate::brain_dump::Dumps::default();
			for r in ordered(rows, 6, 2) {
				dumps.entries.push(crate::brain_dump::Entry {
					date: model(r[0].clone())?,
					text: model(r[1].clone())?,
				});
			}
			value(dumps)
		}
		"shared.task_work" => {
			let mut log = crate::task_work::WorkLog::default();
			for r in ordered(rows, 7, 10) {
				log.sessions.push(crate::task_work::Session {
					id: model(r[0].clone())?,
					task_key: task_key(&r[1..5])?,
					task_title: model(r[5].clone())?,
					block_key: block_key(&r[6], &r[7])?,
					started_at: model(r[8].clone())?,
					stopped_at: model(r[9].clone())?,
				});
			}
			for r in ordered(rows, 8, 5) {
				log.distractions.push(crate::task_work::Distraction {
					id: model(r[0].clone())?,
					block_key: block_key(&r[1], &r[2])?
						.ok_or_else(|| invalid("a distraction has no block"))?,
					minutes: model(r[3].clone())?,
					recorded_at: model(r[4].clone())?,
				});
			}
			// Devices can append at different local positions while offline.
			// Active-session selection and undo follow the recorded times.
			log.sessions.sort_by_key(|s| s.started_at);
			log.distractions.sort_by_key(|d| d.recorded_at);
			value(log)
		}
		_ => Err(invalid("unknown relational document")),
	}
}
