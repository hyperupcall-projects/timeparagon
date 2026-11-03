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
	- Toggle between hiding and showing the app (<kbd>Meta+Shift+J</kbd>).
	- Configure dock or panel to either show above exisitng windows or take up space like other apps.
- Intuitive to use (WIP).
- View your time:
	- By day, week, 2 weeks, month
	- Allow zoomin in/out within the day/week views
	- Custom search
		- Show only events or tasks that satisfy a search of their title or description
		- Filter by time of day, project, label, availability, public visibility, location, and more.
		- Toggle to only show details of events with only public visibility.
- Manage your time:
	- Events
		- Tracks the "usual" fields like due dates, deadlines, labels/tags, priorities, location, project name, and more.
		- Shows dedicated buttons for joining Zoom and Google Meet meetings.
		- Integrates with Zoom and Google Meet to create meetings and invite attendees (TODO).
		- Allows for replying to [event invitations](https://support.google.com/calendar/answer/37135?hl=en&co=GENIE.Platform%3DDesktop) (TODO).
		- Uses location data to show estimated travel time for events and to add link to Google Maps.
	- Time blocks
		- These are standard calendar events with a special interface to easily order tasks within them; great for deep work.
		- Assign tasks to time blocks.
		- Brain dump.
		- Pomodoro timer.
		- Record distractions.
	- Habits
		- Track goals day by day like [Loop Habit Tracker](https://github.com/iSoron/uhabits), with the same scores, streaks and charts.
		- Goals: every day, every X days, N times per day, week or month, N times in Y days, or chosen weekdays. Skip days and question marks are set per habit.
		- Sort the list by hand, name, color, score or status. Archive, delete, or export as CSV.
		- Connect any number of habit events to a habit. They are standard calendar events that are colored differently, and completing one counts towards the habit.
		- Allow "floating" habit, so it can be done anytime during the week/month, etc. or show it on a day +- 3 days etc. Or can make it hard deadline (no +), and have it be greedy (TODO)
	- Sleep
		- Automatically create events for sleep.
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
- Notes (TODO)
	- Associate notes and files with events, tasks, goals, objectives, projects, and people.
	- Local/url (TODO).
- Autolaunchers
	- Save configure presets for launching and positioning application windows (TODO).
- Goals (TODO)
	- Track long-term goals (year, decade).
- Objectives (TODO)
	- Implement your goals in a SMART way.
	- Tasks, certain amount of time per project. (TODO)
	- Day, week, month, quarter, year. (TODO)
- Projects (TODO)
	- Groups related events, tasks, notes, autolaunchers, objectives, and goals.
	- View as list, kanban, or calendar (TODO).
	- During events and tasks associated with a project, easily launch and position the apps/windows necesssary to do the work. (TODO)
	- Allow voice recording ideas related to project, and AI process into summary that might be general idea, or actual task (TODO).
- People.
	- Browse and edit contacts from Google accounts and Google workspace directories.
	- Associate people with events, tasks, and projects (TODO).
	- One-click contact methos for email, Slack (TODO).
- Stores data in Google Calendar, Tasks, Contacts and Drive to get cross-device syncing for free.
	- Uses local database as cache to keep UI performant/responsive.
- Habits
	- Modeled from [Loop Habits](https://github.com/isoron/uhabits)
- Activity monitoring
	- Project allocation
	- Distractions
	- Custom data
		- Sleep
		- Location and exercise
		- Food?

## Terminology

- Overdue tasks are tasks not marked as "completed" or "ignored" that were due before the current day.
- Inboxed tasks are tasks that the user still needs to organize.

## Design Constraints

The design of this app is, in part, constrainted by the fact that:

- Google Task API does not allow for reading/writing the time of day of a due date or deadline (only day).
- Google Task API does not allow writing the deadline field (only due date).
- Google Keep API cannot be used by non-workspace users.
- Integrates only with the KDE Plasma desktop environment on Linux.
