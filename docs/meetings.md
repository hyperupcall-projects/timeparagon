# Meetings

The Meeting section of the event form handles video calls, guests, and replies to invitations. It appears on ordinary events, both new and existing, but not on time blocks, habits, or tasks.

Nothing in the section needs extra Google permissions: it uses the `calendar.events` scope and the contacts and directory access the People screen already has. All of this is on the desktop app. The Android app shows guests read-only.

Changes are staged until Save, except replies, which are sent immediately.

## Video calls

Choose **No video call**, **Google Meet**, or **Zoom**. An event with another provider's link also offers **Other video call**, which keeps the link as it is.

- **Google Meet**: the link is generated through the Calendar API on save. Saving again doesn't replace it.
- **Zoom**: Zoom for Google Workspace has no API to call, so saving opens the event's edit page in the browser, signed in as the account that holds the calendar. Choose Add Zoom Meeting there. The browser isn't opened if the event already has a Zoom link in its conference data, location, or description. If you close the tab without adding it, the form offers **Add Zoom on Google Calendar**.
- **No video call**: removes the call from the event. A Zoom meeting itself isn't deleted on Zoom's side.

Switching between providers removes the previous conference from the event.

## Guests

Type in **Add guests** to search the contacts and Workspace directories of every signed-in account, or enter any email address. An address left in the field is added on save. Each guest can be marked **Optional**.

Saving merges your changes into Google's latest copy of the event, so replies that arrive while the form is open are kept, as are meeting rooms, which aren't listed in the form.

What you can do depends on your role:

| Role | Video call and permissions | Add guests | Remove guests or mark optional |
| --- | --- | --- | --- |
| Organizer, or guest allowed to modify the event | Yes | Yes | Yes |
| Guest allowed to invite others, who can see the guest list | No | Yes | No |
| Other guests | No | No | No |

### Guest permissions

The toggles match Google Calendar: **Modify event**, **Invite others**, and **See guest-list**. They default to off, on, and on. Turning on **Modify event** turns the other two on and locks them.

## Emailing guests

| Action | How it's decided |
| --- | --- |
| Saving the form | The **Email invitations and updates to guests** checkbox, on by default |
| Dragging a meeting | A prompt: **Send**, **Don't send**, or **Cancel**, which puts the event back |
| Deleting a meeting | The **Email guests about the cancellation** checkbox, on by default |
| Replying to an invitation | Always sent |

The drag prompt and delete checkbox only appear on events you can manage that have guests.

A save only emails guests if something relevant changed: the title, description, times, recurrence, video call, or guest list. Marking a guest optional counts as a guest list change, so it emails everyone unless the checkbox is off.

While a Zoom meeting is still waiting to be added, the save doesn't email guests, so they don't receive an invitation without a link. Google Calendar should offer to email them when you save there.

Setting a location from the place picker doesn't email guests, and neither does deleting a meeting from the Android app.

## Invitations

Unanswered invitations wait at the top of the Inbox, and count towards its totals. Answer with **Yes**, **No**, or **Maybe** on the card, or click the title to open the details. Answering removes the card, and it comes back if the reply fails.

They are a live list from Google rather than Google Tasks, refreshed after each calendar sync, so the section is empty after a restart until the first sync finishes.

- Only the primary calendar is checked. On a calendar shared with you, Google treats that calendar's owner as the invited person, so those invitations aren't yours to answer. Invitations sent to a secondary calendar you own don't appear either.
- Only the next 180 days are checked.
- A recurring invitation appears once, and answering from its card answers the whole series.

An invitation opened from the calendar has the same buttons in its Meeting section. On a recurring event, they answer that occurrence unless **Answer for all events in the series** is ticked.

## Not yet verified

This hasn't been run against a real Google account. Check these first:

1. **Zoom account selection**: does `calendar.google.com/calendar/r/eventedit/{eid}?authuser={email}` open the right account when signed in to several? Google doesn't document this URL.
2. **Reply emails**: replies use `sendUpdates=all`. Does that notify only the organizer, rather than every guest?
3. **Zoom invitations**: does Google Calendar offer to email guests who were added without being notified?
