# TimeSlay

Slay the day with TimeSlay.

## Overview

TimeSlay is designed to help you implement some of the [most popular](https://www.todoist.com/productivity-methods) [productivity methods](https://www.todoist.com/inspiration/personal-productivity-methods#biological-primetime). Rather than spending time switching between, searching through, and using multiple apps, just focus on the work.

Docked mode is the feature that I love the most. Your day is always at a glance. You can still hover over and edit events/tasks from this view:

![Dock view](./assets/timeslay-dock.png)

But you'll typically plan the day within the larger panel:

![Day view](./assets/timeslay-schedule.png)

Editing details for events, tasks, time blocks, and more is done within a popup overlay:

![Day view](./assets/timeslay-overlay.png)

Of course, there's also week, 2-week, and month views:

![Week view](./assets/timeslay-week.png)

## Features

- Stays out of your way during work, but is easily accessible when needed.
	- Toggle between "docked" and "panel" mode as shown above (<kbd>Meta+J</kbd>).
	- Toggle between fully hiding and showing the app (<kbd>Meta+Shift+J</kbd>).
	- Takes up full screen when showing lots of data.
	- Configure dock or panel to either show above existing apps or take up space.
	- Configure widths of dock, panel, and side overlays.
- Intuitive to use.
- View your time:
	- By day, week, 2 weeks, or month.
	- Allow zoom in and zoom out on day/week views.
	- Apply custom search/filter:
		- Show only events or tasks that satisfy a search of their title or description.
		- Filter by time of day, project, label, availability, public visibility, location, and more.
		- Toggle to only show details of events with only public visibility.
- Manage your time:
	- Events
		- Tracks the "usual" fields like due dates, deadlines, labels/tags, priorities, location, project name, and more.
		- Shows dedicated buttons for joining Zoom and Google Meet meetings.
		- Create Zoom and Google Meet meetings and invite attendees.
		- Send replies to [event invitations](https://support.google.com/calendar/answer/37135?hl=en&co=GENIE.Platform%3DDesktop).
		- Location
			- Estimate travel time and distance based on event locations
			- Link to Google Maps for more detailed location and travel info.
	- Time blocks
		- These are standard calendar events with a special interface to easily order tasks within them; great for deep work.
		- Tasks are assigned time blocks.
		- Brain dump.
		- Pomodoro timer.
		- Record distractions.
	- Habits (described below)
	- Sleep
		- Automatically create events for sleep based on sleep goal, next day's activities, and latest wakeup time.
		- Sleep quality tracking (TODO).
		- Automatic shutdown (TODO).
- Capture and manage tasks:
	- Track the "usual" fields like due dates, deadlines, labels/tags, priorities, project name, and estimated duration.
	- Create tasks:
		- By typing; interprets keywords as metadata [like Todoist](https://www.todoist.com/help/todoist/features/use-task-quick-add-in-todoist-va4Lhpzz).
		- With voice input.
		- By pasting Gmail URLs
		- From a web page with the browser extension
		- From a Slack message with the Slack app
	- Inbox section for new tasks that have yet to be triaged.
	- Overdue section for uncompleted past tasks that need to be reprioritized or rescheduled.
	- Segmented backlog section for deferring tasks to next week, next month, and so on (WIP).
	- Schedule tasks:
		- Manually assign tasks to time blocks.
		- Automatically project tasks from an grouped, ordered, time-independent list of tasks to the day's available time blocks (TODO).
		- "Do immediately" etc. No need to categorize other than project.
	- Handle long-running or multi-step/subtasks (TODO).
		- Tasks that may be done over multiple days/time blocks (TODO).
		- When recording during time block, already have multiple recorded sessions. Can cancel which removes task from todo, but keeps history (TODO).
		- But, also keep track of subtasks (TODO)?
		- Waiting for reply email (waiting-for tag), once reply, put back in to-do but if not, wait until date to do so. or AI to detect.
- Habits
	- These are standard calendar events that are colored differently.
	- Modeled from [Loop Habits](https://github.com/isoron/uhabits)
	- Two types: "Did it" and "Avoided it". Either can also track a daily value with a target, shown in a Value chart
	- Group habits into sections. Clicking any day opens a popup with everything that day can record
	- For habits that can be done anytime during the week, "float" them in the day/week view (TODO)
	- Show habits configurable amount of days before they are due so they can be scheduled before they are due (TODO)
	- Modeled from [Loop Habits](https://github.com/isoron/uhabits)
	- Allow setting/connecting goals to Habits (TODO).
- Notes (TODO)
	- Associate notes and files with events, tasks, goals, objectives, projects, and people.
	- Local/url (TODO).
	- Notes are stored in the local database and synced through Turso.
- Autolaunchers
	- Save configure presets for launching and positioning application windows (TODO).
- Set goals (TODO)
	- Track long-term goals (year, decade).
- Set objectives (TODO)
	- Implement your goals in a SMART way.
	- Tasks, certain amount of time per project. (TODO)
	- Day, week, month, quarter, year. (TODO)
- Projects (TODO)
	- Groups related events, tasks, notes, autolaunchers, objectives, and goals.
	- View as list, kanban, or calendar (TODO).
	- During events and tasks associated with a project, easily launch and position the apps/windows necesssary to do the work. (TODO)
	- Allow voice recording ideas related to project, and AI process into summary that might be general idea, or actual task (TODO).
- Remember people:
	- Browse and edit contacts from Google accounts and Google workspace directories.
	- Associate people with events, tasks, and projects (TODO).
	- One-click contact methos for email, Slack (TODO).
- Stores data in Google Calendar, Tasks and Contacts, and syncs app-specific data through Turso.
	- Uses local database as cache to keep UI performant/responsive.
- Activity Tracking & Statistics
	- Project allocation
	- Deep deep work/Distractions
	- locations been
	- Location and exercise
	- Food?

## Architecture

- Desktop app uses [Iced](https://iced.rs) for GUI and mobile app mostly uses [Dioxus](https://dioxuslabs.com).
- Local-first approach; each device has own SQLite database, synced with [Turso](https://turso.tech), which is ran on a separate server.
	- SQLite database contains cache of calendar, task, and other cloud data, and also app-specific data like habit information.
- Uses Google to back calendar, tasks, and contact data.

## Terminology

- Overdue tasks are tasks not marked as "completed" or "ignored" that were due before the current day.
- Inboxed tasks are tasks that the user still needs to organize.

## Design Constraints

The design of this app is, in part, constrainted by the fact that:

- Google Task API does not allow for reading/writing the time of day of a due date or deadline (only day).
- Google Task API does not allow writing the deadline field (only due date).
- Integrates only with the KDE Plasma desktop environment on Linux.
