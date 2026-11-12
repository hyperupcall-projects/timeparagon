//! Google Calendar, as the time block reconciler sees it.
//!
//! See [`crate::timeblocking::reconcile`] for what the reconciler does. This
//! is only how it reads and writes Google. It runs on the write worker
//! thread, which is already sending to Google.

use super::*;
use crate::task_edit::{self, Change};
use crate::timeblocking::membership::{parse_instance_id, planning_day};
use crate::timeblocking::reconcile::{Fix, Listing, Remote, Series};

pub struct GoogleRemote<'a> {
	token: String,
	settings: &'a CalendarSettings,
	calendar: Calendar,
	/// The task series a fix changed, so the caches can fetch them again.
	pub changed: Vec<String>,
}

impl<'a> GoogleRemote<'a> {
	pub fn new(token: String, settings: &'a CalendarSettings, calendar_id: &str) -> Self {
		Self {
			token,
			settings,
			calendar: Calendar {
				id: calendar_id.to_owned(),
				name: CalendarSettings::APP_CALENDAR_NAME.to_owned(),
				color: colors::DEFAULT_EVENT,
				primary: false,
				selected_by_google: true,
			},
			changed: Vec::new(),
		}
	}

	fn event_url(&self, calendar: &str, id: &str) -> String {
		format!(
			"{EVENTS}/{}/events/{}",
			urlencoding::encode(calendar),
			urlencoding::encode(id)
		)
	}

	fn note(&mut self, series: &str) {
		if !self.changed.iter().any(|known| known == series) {
			self.changed.push(series.to_owned());
		}
	}

	/// An event's JSON, or `None` when it was deleted.
	fn fetch(&self, calendar: &str, id: &str) -> Result<Option<serde_json::Value>, String> {
		match get::<serde_json::Value>(&self.event_url(calendar, id), &self.token) {
			Ok(value) if value["status"].as_str() == Some("cancelled") => Ok(None),
			Ok(value) => Ok(Some(value)),
			Err(error) if matches!(error.status, Some(404 | 410)) => Ok(None),
			Err(error) => Err(error.to_string()),
		}
	}

	/// The occurrences of a series, oldest first, from `from` on and at most
	/// `limit` pages of them.
	fn instances(
		&self,
		calendar: &str,
		series: &str,
		from: Option<DateTime<Local>>,
		until: Option<DateTime<Local>>,
		limit: usize,
	) -> Result<Vec<Event>, String> {
		let mut events = Vec::new();
		let mut page: Option<String> = None;
		let wide = (
			NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
			NaiveDate::from_ymd_opt(9999, 1, 1).unwrap(),
		);
		for _ in 0..limit {
			let mut url = format!(
				"{}/instances?maxResults=250",
				self.event_url(calendar, series)
			);
			if let Some(from) = from {
				url.push_str(&format!(
					"&timeMin={}",
					urlencoding::encode(&from.with_timezone(&Utc).to_rfc3339())
				));
			}
			if let Some(until) = until {
				url.push_str(&format!(
					"&timeMax={}",
					urlencoding::encode(&until.with_timezone(&Utc).to_rfc3339())
				));
			}
			if let Some(token) = &page {
				url.push_str("&pageToken=");
				url.push_str(&urlencoding::encode(token));
			}
			let list: EventList = match get(&url, &self.token) {
				Ok(list) => list,
				Err(error) if matches!(error.status, Some(404 | 410)) => return Ok(events),
				Err(error) => return Err(error.to_string()),
			};
			for entry in list.items {
				if entry.status.as_deref() == Some("cancelled") {
					continue;
				}
				if let Some(event) = parse_event_in_range(entry, &self.calendar, wide.0, wide.1) {
					events.push(event);
				}
			}
			match list.next_page_token {
				Some(next) => page = Some(next),
				None => break,
			}
		}
		events.sort_by_key(crate::timeblocking::original_start);
		Ok(events)
	}

	fn time(&self, when: DateTime<Local>) -> serde_json::Value {
		let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
		serde_json::json!({"dateTime": when.to_rfc3339(), "timeZone": zone, "date": null})
	}

	/// Makes the occurrences of a new series on `skip` days deleted and on
	/// `done` days done, as they were in the series it continues.
	fn carry(
		&mut self,
		calendar: &str,
		series: &str,
		skip: &[NaiveDate],
		done: &[NaiveDate],
	) -> Result<(), String> {
		let (Some(first), Some(last)) = (
			skip.iter().chain(done).min().copied(),
			skip.iter().chain(done).max().copied(),
		) else {
			return Ok(());
		};
		let hour = self.settings.day_start_hour();
		let from = crate::calendar::at_clock(first, hour, 0) - chrono::Duration::days(1);
		let until = crate::calendar::at_clock(last, hour, 0) + chrono::Duration::days(2);
		for occurrence in self.instances(calendar, series, Some(from), Some(until), 20)? {
			let day = planning_day(crate::timeblocking::original_start(&occurrence), hour);
			let url = self.event_url(calendar, &occurrence.id);
			if skip.contains(&day) {
				match delete(&url, &self.token, "Time blocks") {
					Err(error) if !matches!(error.status, Some(404 | 410)) => {
						return Err(error.to_string());
					}
					_ => {}
				}
			} else if done.contains(&day) {
				let mut properties = self
					.fetch(calendar, &occurrence.id)?
					.map(|value| value["extendedProperties"].clone())
					.unwrap_or_else(|| serde_json::json!({}));
				properties["private"]["ttCompleted"] = "true".into();
				patch(
					&url,
					&self.token,
					&serde_json::json!({"extendedProperties": properties}).to_string(),
					"Time blocks",
				)
				.map_err(String::from)?;
			}
		}
		Ok(())
	}
}

impl Remote for GoogleRemote<'_> {
	fn listing(
		&mut self,
		calendar: &str,
		first: DateTime<Local>,
		last: DateTime<Local>,
	) -> Result<Listing, String> {
		let mut listing = Listing::default();
		let mut page: Option<String> = None;
		loop {
			let mut url = format!(
				"{EVENTS}/{}/events?singleEvents=true&showDeleted=true&maxResults=2500&timeMin={}&timeMax={}",
				urlencoding::encode(calendar),
				urlencoding::encode(&first.with_timezone(&Utc).to_rfc3339()),
				urlencoding::encode(&last.with_timezone(&Utc).to_rfc3339()),
			);
			if let Some(token) = &page {
				url.push_str("&pageToken=");
				url.push_str(&urlencoding::encode(token));
			}
			let list: EventList = get(&url, &self.token).map_err(String::from)?;
			for entry in list.items {
				if entry.status.as_deref() == Some("cancelled") {
					if let (Some(series), Some(original)) = (
						entry.recurring_event_id.clone(),
						entry.original_start_time.as_ref().and_then(event_time),
					) {
						listing.cancelled.push((series, original));
					}
					continue;
				}
				if let Some(event) = parse_event_in_range(
					entry,
					&self.calendar,
					first.date_naive(),
					last.date_naive(),
				) {
					listing.events.push(event);
				}
			}
			match list.next_page_token {
				Some(next) => page = Some(next),
				None => break,
			}
		}
		Ok(listing)
	}

	fn series(&mut self, calendar: &str, id: &str) -> Result<Option<Series>, String> {
		let Some(value) = self.fetch(calendar, id)? else {
			return Ok(None);
		};
		let recurrence: Vec<String> =
			serde_json::from_value(value["recurrence"].clone()).unwrap_or_default();
		let properties = &value["extendedProperties"];
		Ok(Some(Series {
			calendar: calendar.to_owned(),
			id: id.to_owned(),
			recurrence,
			start: json_event_start(&value["start"])?,
			end: json_event_start(&value["end"])?,
			all_day: value["start"]["date"].is_string(),
			split_from: split_from(properties),
			task: (properties["private"]["ttKind"].as_str() == Some("task"))
				.then(|| crate::calendar::calendar_tasks::read_data(properties).ok())
				.flatten(),
		}))
	}

	fn first_occurrence(
		&mut self,
		calendar: &str,
		series: &str,
		from: DateTime<Local>,
	) -> Result<Option<Event>, String> {
		// Google filters occurrences by when they are now, not when they
		// were first scheduled, so look a little earlier.
		let earlier = from - chrono::Duration::days(1);
		Ok(self
			.instances(calendar, series, Some(earlier), None, 2)?
			.into_iter()
			.find(|event| crate::timeblocking::original_start(event) >= from))
	}

	fn last_occurrence(&mut self, calendar: &str, series: &str) -> Result<Option<Event>, String> {
		Ok(self.instances(calendar, series, None, None, 40)?.pop())
	}

	fn exists(&mut self, calendar: &str, id: &str) -> Result<bool, String> {
		Ok(self.fetch(calendar, id)?.is_some())
	}

	fn apply(&mut self, fix: &Fix) -> Result<(), String> {
		match fix {
			Fix::Rule {
				calendar,
				series,
				recurrence,
				start,
				end,
			} => {
				let body = serde_json::json!({
					"recurrence": recurrence,
					"start": self.time(*start),
					"end": self.time(*end),
				});
				patch(
					&self.event_url(calendar, series),
					&self.token,
					&body.to_string(),
					"Time blocks",
				)
				.map_err(String::from)?;
				self.note(series);
			}
			Fix::Continue {
				calendar,
				from,
				block_series,
				recurrence,
				start,
				end,
				skip,
				done,
			} => {
				let mut copy = self
					.fetch(calendar, from)?
					.ok_or("The repeating task to carry on was deleted")?;
				for key in [
					"id",
					"etag",
					"kind",
					"htmlLink",
					"created",
					"updated",
					"iCalUID",
					"sequence",
					"organizer",
					"creator",
					"recurringEventId",
					"originalStartTime",
					"status",
				] {
					copy.as_object_mut().map(|object| object.remove(key));
				}
				copy["recurrence"] = serde_json::json!(recurrence);
				copy["start"] = self.time(*start);
				copy["end"] = self.time(*end);
				let mut data =
					crate::calendar::calendar_tasks::read_data(&copy["extendedProperties"])?;
				data.metadata.block_series = Some(block_series.clone());
				data.metadata.block = None;
				crate::calendar::calendar_tasks::write_data(&mut copy, &data)?;
				// Another device keeping the same blocks in step makes the
				// same continuation with the same ID, so only one is kept.
				let id = continuation_id(from, block_series, *start);
				copy["id"] = id.clone().into();
				match post(
					&format!("{EVENTS}/{}/events", urlencoding::encode(calendar)),
					&self.token,
					&copy.to_string(),
					"Time blocks",
				) {
					Ok(_) => self.carry(calendar, &id, skip, done)?,
					Err(error) if error.status == Some(409) => {}
					Err(error) => return Err(error.to_string()),
				}
				self.note(from);
				self.note(&id);
			}
			Fix::Delete { calendar, series } => {
				match delete(
					&self.event_url(calendar, series),
					&self.token,
					"Time blocks",
				) {
					Err(error) if !matches!(error.status, Some(404 | 410)) => {
						return Err(error.to_string());
					}
					_ => {}
				}
				self.note(series);
			}
			Fix::Move {
				calendar,
				event,
				start,
				end,
			} => {
				let body = serde_json::json!({"start": self.time(*start), "end": self.time(*end)});
				patch(
					&self.event_url(calendar, event),
					&self.token,
					&body.to_string(),
					"Time blocks",
				)
				.map_err(String::from)?;
				if let Some((series, _)) = parse_instance_id(event) {
					self.note(series);
				}
			}
			Fix::Cancel { calendar, event } => {
				match delete(&self.event_url(calendar, event), &self.token, "Time blocks") {
					Err(error) if !matches!(error.status, Some(404 | 410)) => {
						return Err(error.to_string());
					}
					_ => {}
				}
				if let Some((series, _)) = parse_instance_id(event) {
					self.note(series);
				}
			}
			Fix::Release { task } => {
				let order = crate::timeblocking::metadata(task).order;
				task_edit::send(
					(**task).clone(),
					self.settings,
					Change::Unassign,
					order,
					&self.token,
				)
				.map_err(String::from)?;
			}
		}
		Ok(())
	}
}

/// The ID a task series continuing `from` with `block_series` from `start`
/// gets: the same on every device. Google takes IDs of 5 to 1024 characters
/// from `a`–`v` and `0`–`9`, which hex digits are.
fn continuation_id(from: &str, block_series: &str, start: DateTime<Local>) -> String {
	use sha2::{Digest, Sha256};
	let digest = Sha256::digest(format!("{from}\n{block_series}\n{}", start.timestamp()));
	let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
	format!("ttc{}", &hex[..40])
}

/// Deletes the tasks repeating with the block series that `occurrence` (on
/// `block_calendar`) belongs to, as far as deleting that occurrence with
/// `scope` deletes the block: each task series entirely ("all"), from the
/// occurrence's planning day on ("current and future"), or just that day.
pub fn delete_followers(
	token: &str,
	settings: &CalendarSettings,
	block_calendar: &str,
	occurrence: &str,
	scope: crate::calendar::SaveScope,
) -> Result<(), GoogleError> {
	use crate::calendar::SaveScope;
	let Some(calendar) = settings.app_calendar.clone() else {
		return Ok(());
	};
	// Only an occurrence of a series has tasks repeating with it.
	let Some((series, start)) = parse_instance_id(occurrence) else {
		return Ok(());
	};
	let name = format!("{block_calendar}/{series}");
	let mut followers = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let mut url = format!(
			"{EVENTS}/{}/events?privateExtendedProperty=ttKind%3Dtask&singleEvents=false&maxResults=2500",
			urlencoding::encode(&calendar)
		);
		if let Some(token) = &page {
			url.push_str("&pageToken=");
			url.push_str(&urlencoding::encode(token));
		}
		let list: serde_json::Value = get(&url, token)?;
		for item in list["items"].as_array().into_iter().flatten() {
			let repeats = item["recurrence"]
				.as_array()
				.is_some_and(|lines| !lines.is_empty());
			if item["status"].as_str() == Some("cancelled") || !repeats {
				continue;
			}
			let follows = crate::calendar::calendar_tasks::read_data(&item["extendedProperties"])
				.is_ok_and(|data| data.metadata.block_series.as_deref() == Some(name.as_str()));
			if follows && let Some(id) = item["id"].as_str() {
				followers.push(id.to_owned());
			}
		}
		page = list["nextPageToken"].as_str().map(str::to_owned);
		if page.is_none() {
			break;
		}
	}
	let hour = settings.day_start_hour();
	let day = planning_day(start, hour);
	let mut remote = GoogleRemote::new(token.to_owned(), settings, &calendar);
	for follower in followers {
		let target = if scope == SaveScope::All {
			follower.clone()
		} else {
			let from = crate::calendar::at_clock(day, hour.min(23), 0);
			match remote.first_occurrence(&calendar, &follower, from)? {
				Some(next)
					if scope == SaveScope::CurrentAndFuture
						|| planning_day(crate::timeblocking::original_start(&next), hour)
							== day =>
				{
					next.id
				}
				_ => continue,
			}
		};
		delete_event_scope(token, &calendar, &target, scope, false).or_else(|error| {
			if matches!(error.status, Some(404 | 410)) {
				Ok(())
			} else {
				Err(error)
			}
		})?;
		super::super::series::note(&calendar, &follower, vec![calendar.clone()]);
	}
	Ok(())
}

/// Which block series a task series follows, and what Google says of it.
fn followed(remote: &mut GoogleRemote<'_>, calendar: &str, series: &str) -> String {
	let Ok(Some(task)) = remote.series(calendar, series) else {
		return String::new();
	};
	let Some(name) = task.task.and_then(|data| data.metadata.block_series) else {
		return String::new();
	};
	let Some((block_calendar, id)) = name.rsplit_once('/') else {
		return format!(" (it follows {name})");
	};
	match remote.series(block_calendar, id) {
		Ok(Some(block)) => format!(
			" (its block series {id} on {block_calendar} repeats {}, from {})",
			block.recurrence.join(" "),
			block.start.format("%b %-d %Y")
		),
		Ok(None) => format!(" (its block series {id} on {block_calendar} was deleted)"),
		Err(error) => format!(" (its block series {id} could not be read: {error})"),
	}
}

/// What keeping time blocks in step would change on the app calendar over
/// the next weeks, and which tasks break the rule that an item in a block
/// either repeats with it or is a one-off, as text. Reads only.
pub fn report(settings: &CalendarSettings) -> Result<String, String> {
	use crate::timeblocking::reconcile;
	let calendar = settings
		.app_calendar
		.clone()
		.ok_or("TimeSlay has no calendar of its own yet")?;
	let token = super::super::GoogleClient::from_settings(settings)
		.access_token()
		.map_err(String::from)?;
	let hour = settings.day_start_hour();
	let today = crate::calendar::day_at(crate::clock::now(), hour);
	let first = crate::calendar::at_clock(today, hour, 0);
	let last = first + chrono::Duration::days(56);
	let mut remote = GoogleRemote::new(token, settings, &calendar);
	let listing = remote.listing(&calendar, first, last)?;
	let title = |id: &str| {
		listing
			.events
			.iter()
			.find(|event| event.id == id || event.details.recurring_event_id.as_deref() == Some(id))
			.map_or_else(|| id.to_owned(), |event| format!("“{}”", event.title))
	};
	let when = |at: &DateTime<Local>| at.format("%a %b %-d, %H:%M").to_string();
	let fixes = reconcile::plan(&mut remote, &calendar, first, last, hour, today)?;
	let mut out = format!(
		"Time blocks from {} to {}\n\n",
		first.format("%b %-d"),
		last.format("%b %-d")
	);
	if fixes.is_empty() {
		out.push_str("Everything is in step.\n");
	} else {
		out.push_str("Keeping them in step would:\n");
		for fix in &fixes {
			let line = match fix {
				reconcile::Fix::Rule {
					series,
					recurrence,
					start,
					..
				} => format!(
					"repeat {} as its block does ({}), from {}",
					title(series),
					recurrence.join(" "),
					when(start)
				),
				reconcile::Fix::Continue { from, start, .. } => format!(
					"carry {} on with its block from {}, as a new series",
					title(from),
					when(start)
				),
				reconcile::Fix::Delete { series, .. } => {
					format!(
						"delete {}{}",
						title(series),
						followed(&mut remote, &calendar, series)
					)
				}
				reconcile::Fix::Move {
					event, start, end, ..
				} => format!(
					"move {} to {}–{}",
					title(event),
					when(start),
					end.format("%H:%M")
				),
				reconcile::Fix::Cancel { event, .. } => {
					format!("delete {} on a day its block was deleted", title(event))
				}
				reconcile::Fix::Release { task } => format!(
					"take “{}” out of a deleted block on {}",
					task.title,
					task.start.format("%a %b %-d")
				),
			};
			out.push_str(&format!("  - {line}\n"));
		}
		if fixes.iter().any(|fix| fix.series().is_some()) {
			out.push_str(
				"  (Changes to repeating tasks come first; how their days then line up is checked after.)\n",
			);
		}
	}
	let mut seen: Vec<(&str, &Event)> = Vec::new();
	for event in &listing.events {
		if let Some(series) = event.details.recurring_event_id.as_deref()
			&& !event.is_task()
			&& !event.details.habit
			&& event.details.sleep.is_none()
			&& !seen.iter().any(|(known, _)| *known == series)
		{
			seen.push((series, event));
		}
	}
	if !seen.is_empty() {
		out.push_str("\nRepeating blocks seen:\n");
		for (series, event) in seen {
			let lineage = crate::timeblocking::lineage(event);
			let continues: Vec<&String> = lineage
				.iter()
				.filter(|name| **name != format!("{calendar}/{series}"))
				.collect();
			out.push_str(&format!("  - “{}” ({series})", event.title));
			if !continues.is_empty() {
				out.push_str(&format!(", continuing {continues:?}"));
			}
			out.push('\n');
		}
	}
	let broken = reconcile::own_repeat_in_block(&mut remote, &calendar, first, last)?;
	if !broken.is_empty() {
		out.push_str(
            "\nThese repeat on their own but sit in a block. Put each in its block again to make it a one-off there, or take it out:\n",
        );
		for task in broken {
			out.push_str(&format!(
				"  - “{}” on {}\n",
				task.title,
				task.start.format("%a %b %-d")
			));
		}
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_continuation_has_the_same_valid_id_on_every_device() {
		let start = crate::calendar::at_clock(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(), 9, 0);
		let id = continuation_id("task1", "app/block2", start);
		assert_eq!(id, continuation_id("task1", "app/block2", start));
		assert_ne!(id, continuation_id("task1", "app/block3", start));
		// Google's event IDs: 5 to 1024 of a–v and 0–9.
		assert!((5..=1024).contains(&id.len()));
		assert!(
			id.bytes()
				.all(|b| b.is_ascii_digit() || (b'a'..=b'v').contains(&b))
		);
	}
}
