#!/usr/bin/env python3
"""One-time, backed-up cutover of description fields to typed task metadata.

Run without --apply to snapshot and prepare a reviewable plan. Stop old clients
before applying. This script is never imported by the application.
"""
import argparse
import copy
import datetime
import json
import pathlib
import re
import sqlite3
import time
import tomllib
import urllib.parse
import urllib.request

CAL = 'https://www.googleapis.com/calendar/v3'
TASK = 'https://tasks.googleapis.com/tasks/v1'
FIELD = re.compile(r'^\s*([A-Z][A-Z_ -]*):\s*(.*)$')


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'))


def typed_deadline(value):
    if value is None or isinstance(value, dict):
        return value
    formats = ('%Y-%m-%d', '%Y-%m-%d %H:%M')
    for fmt in formats:
        try:
            parsed = datetime.datetime.strptime(value, fmt)
            if parsed.strftime(fmt) == value:
                return {'date': parsed.date().isoformat(), 'time': parsed.time().isoformat() if ' ' in value else None}
        except ValueError:
            pass
    raise ValueError('Invalid deadline: ' + repr(value))


def extract(text, metadata):
    """Migrate reserved fields once; keep every non-field line unchanged."""
    metadata = copy.deepcopy(metadata)
    if 'deadline' in metadata:
        metadata['deadline'] = typed_deadline(metadata['deadline'])
    fields = {}
    body = []
    for line in text.split('\n'):
        match = FIELD.fullmatch(line)
        if match:
            key, value = match.groups()
            if key not in ('DUE', 'DEADLINE'):
                raise ValueError('Unmapped description field: ' + key)
            parsed = typed_deadline(value)
            if key in fields and fields[key] != parsed:
                raise ValueError('Conflicting duplicate ' + key)
            fields[key] = parsed
        else:
            body.append(line)
    deadline = fields.get('DEADLINE', fields.get('DUE'))
    if deadline is not None:
        if metadata.get('deadline') not in (None, deadline):
            raise ValueError('Description and metadata deadlines disagree')
        metadata['deadline'] = deadline
    return '\n'.join(body), metadata


def task_notes(notes):
    parts = notes.rsplit('\n---\n', 1)
    if len(parts) == 1 and notes.startswith('---\n'):
        parts = ['', notes[4:]]
    if len(parts) != 2 or not parts[1].startswith('tt-version: '):
        # External tasks without app-owned metadata are left untouched unless
        # they contain reserved fields, which requires a stable task ID first.
        if any(FIELD.fullmatch(line) for line in notes.splitlines()):
            raise ValueError('Reserved field on task without TimeParagon metadata')
        return notes, None
    fields = {}
    for line in parts[1].splitlines():
        key, value = line.split(': ', 1)
        if not key.startswith('tt-') or key in fields:
            raise ValueError('Invalid metadata footer')
        fields[key] = json.loads(value)
    if fields.pop('tt-version') != 1:
        raise ValueError('Unsupported footer version')
    task_id = fields.pop('tt-id')
    metadata = {key[3:]: value for key, value in fields.items()}
    prose, metadata = extract(parts[0], metadata)
    # Optional absent values have one canonical representation.
    if metadata.get('deadline') is None:
        metadata.pop('deadline', None)
    lines = ['---', 'tt-version: 1', 'tt-id: ' + str(task_id)]
    lines += ['tt-' + key + ': ' + compact(value) for key, value in sorted(metadata.items())]
    return (prose + '\n' if prose else '') + '\n'.join(lines), {'id': task_id, 'metadata': metadata}


def event_data(event):
    props = event['extendedProperties']['private']
    return json.loads(''.join(props['ttData' + str(i)] for i in range(int(props['ttDataCount']))))


def event_patch(event):
    props = copy.deepcopy(event['extendedProperties']['private'])
    data = event_data(event)
    description, data['metadata'] = extract(event.get('description', ''), data['metadata'])
    if description == event.get('description', '') and data == event_data(event):
        return None, data
    raw = compact(data)
    chunks = []
    chunk = ''
    for char in raw:
        if len((chunk + char).encode()) > 900:
            chunks.append(chunk)
            chunk = ''
        chunk += char
    if chunk:
        chunks.append(chunk)
    for key in list(props):
        if re.fullmatch(r'ttData\d+', key):
            props[key] = None
    props['ttDataCount'] = str(len(chunks))
    props.update({'ttData' + str(i): value for i, value in enumerate(chunks)})
    return {'description': description, 'extendedProperties': {'private': props}}, data


class Migration:
    def __init__(self, args):
        self.args = args
        self.root = pathlib.Path(args.backup)
        self.root.mkdir(parents=True, exist_ok=True)
        self.root.chmod(0o700)
        settings = tomllib.loads(pathlib.Path(args.config).read_text())['calendar']
        credentials = json.loads(pathlib.Path(args.secrets).read_text())['google_tokens']
        self.token = credentials['access_token']
        if credentials.get('expires_at', 0) < time.time() + 120:
            fields = {'grant_type': 'refresh_token', 'refresh_token': credentials['refresh_token'],
                      'client_id': settings['client_id'], 'client_secret': settings['client_secret']}
            request = urllib.request.Request('https://oauth2.googleapis.com/token', data=urllib.parse.urlencode(fields).encode())
            self.token = json.load(urllib.request.urlopen(request, timeout=45))['access_token']

    def save(self, name, value):
        path = self.root / name
        tmp = path.with_suffix('.tmp')
        tmp.write_text(json.dumps(value, ensure_ascii=False, indent=2))
        tmp.chmod(0o600)
        tmp.replace(path)

    def request(self, method, url, body=None, etag=None):
        headers = {'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json'}
        if etag:
            headers['If-Match'] = etag
        request = urllib.request.Request(url, method=method, headers=headers,
                                         data=None if body is None else compact(body).encode())
        with urllib.request.urlopen(request, timeout=45) as response:
            raw = response.read()
            return json.loads(raw) if raw else {}

    def pages(self, url):
        result = []
        base = url
        while True:
            page = self.request('GET', url)
            result.extend(page.get('items', []))
            token = page.get('nextPageToken')
            if not token:
                return result
            url = base + '&pageToken=' + urllib.parse.quote(token, safe='')

    def snapshot(self):
        calendars = self.pages(CAL + '/users/me/calendarList?maxResults=250')
        lists = self.pages(TASK + '/users/@me/lists?maxResults=100')
        enc = lambda value: urllib.parse.quote(value, safe='')
        events = {c['id']: self.pages(CAL + '/calendars/' + enc(c['id']) + '/events?maxResults=2500&singleEvents=false&privateExtendedProperty=ttKind%3Dtask') for c in calendars}
        tasks = {item['id']: self.pages(TASK + '/lists/' + enc(item['id']) + '/tasks?maxResults=100&showCompleted=true&showHidden=true') for item in lists}
        return {'calendars': calendars, 'lists': lists, 'events': events, 'tasks': tasks}

    def plan(self, snapshot):
        actions = []
        data = {}
        enc = lambda value: urllib.parse.quote(value, safe='')
        for calendar, events in snapshot['events'].items():
            for event in events:
                if event.get('status') == 'cancelled':
                    continue
                patch, record = event_patch(event)
                data[str(record['id'])] = record['metadata']
                if patch:
                    actions.append({'url': CAL + '/calendars/' + enc(calendar) + '/events/' + enc(event['id']), 'before': event, 'patch': patch, 'task_id': record['id']})
        for task_list, tasks in snapshot['tasks'].items():
            for task in tasks:
                notes, record = task_notes(task.get('notes', ''))
                if record:
                    data[str(record['id'])] = record['metadata']
                if notes != task.get('notes', ''):
                    actions.append({'url': TASK + '/lists/' + enc(task_list) + '/tasks/' + enc(task['id']), 'before': task, 'patch': {'notes': notes}, 'task_id': record['id']})
        return {'actions': actions, 'metadata': data}

    def local(self, plan):
        path = pathlib.Path(self.args.database)
        db = sqlite3.connect(path, timeout=30)
        backup_path = self.root / 'before-apply.sqlite3'
        if not backup_path.exists():
            backup = sqlite3.connect(backup_path)
            db.backup(backup)
            backup.close()
            backup_path.chmod(0o600)
        deadlines = {int(key): value.get('deadline') for key, value in plan['metadata'].items()}
        def transform(value):
            if isinstance(value, list):
                return [transform(item) for item in value]
            if not isinstance(value, dict):
                return value
            value = copy.deepcopy(value)
            if isinstance(value.get('metadata'), dict) and isinstance(value.get('id'), int):
                metadata = value['metadata']
                if value['id'] in deadlines:
                    if deadlines[value['id']] is not None:
                        metadata['deadline'] = deadlines[value['id']]
                    else:
                        metadata.pop('deadline', None)
                elif 'deadline' in metadata:
                    metadata['deadline'] = typed_deadline(metadata['deadline'])
            if 'calendar_id' in value and isinstance(value.get('details'), dict):
                details = value['details']
                data = details.get('task_data')
                if data:
                    text = details.get('description', '')
                    if '\ntt-version: ' in text:
                        text, footer_data = task_notes(text)
                        text = text.rsplit('\n---\n', 1)[0] if '\n---\n' in text else ''
                        data = footer_data or data
                    text, data['metadata'] = extract(text, data['metadata'])
                    details['description'] = text
                    details['task_data'] = data
            return {key: transform(item) for key, item in value.items()}
        changes = 0
        with db:
            for task_id, raw in db.execute('SELECT id,metadata FROM tasks').fetchall():
                record = transform({'id': task_id, 'metadata': json.loads(raw)})
                if compact(record['metadata']) != raw:
                    db.execute('UPDATE tasks SET metadata=? WHERE id=?', (compact(record['metadata']), task_id))
                    changes += 1
            for table, column, pk in [('google_task_records', 'event', 'id'), ('google_entities', 'event', 'id'),
                                      ('google_task_backend_state', 'data', 'task_id'), ('google_operation_progress', 'data', 'operation_id'),
                                      ('google_operations', 'payload', 'id'), ('calendar_months', 'events', 'month'), ('documents', 'value', 'key')]:
                for key, raw in db.execute(f'SELECT {pk},{column} FROM {table} WHERE {column} IS NOT NULL').fetchall():
                    # Shared documents and operation payloads contain nested event snapshots.
                    value = transform(json.loads(raw))
                    encoded = compact(value)
                    if encoded != raw:
                        db.execute(f'UPDATE {table} SET {column}=? WHERE {pk}=?', (encoded, key))
                        changes += 1
            assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            assert not db.execute('PRAGMA foreign_key_check').fetchall()
        db.close()
        return changes

    def apply(self, plan):
        journal_path = self.root / 'journal.json'
        journal = json.loads(journal_path.read_text()) if journal_path.exists() else {}
        for action in plan['actions']:
            url = action['url']
            if url in journal:
                continue
            current = self.request('GET', url)
            # Refuse to overwrite edits made after the snapshot.
            if current.get('etag') != action['before'].get('etag'):
                raise ValueError('Resource changed since snapshot: ' + str(action['task_id']))
            self.request('PATCH', url, action['patch'], current.get('etag'))
            saved = self.request('GET', url)
            for field, expected in action['patch'].items():
                if field == 'extendedProperties':
                    actual = saved[field]['private']
                    assert all(actual.get(key) == value for key, value in expected['private'].items())
                else:
                    assert saved.get(field, '') == expected
            journal[url] = True
            self.save('journal.json', journal)
        changes = self.local(plan)
        final = self.snapshot()
        self.save('verified-snapshot.json', final)
        assert not self.plan(final)['actions'], 'Unmigrated remote fields remain'
        self.save('result.json', {'verified': True, 'remote_resources_updated': len(journal), 'local_rows_updated': changes})
        print('Verified migration:', len(journal), 'remote resources;', changes, 'local rows')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    home = pathlib.Path.home()
    parser.add_argument('--backup', default='.migration-backups/structured-deadlines')
    parser.add_argument('--config', default=str(home / '.config/timeparagon/config.toml'))
    parser.add_argument('--secrets', default=str(home / '.local/state/timeparagon/secrets.json'))
    parser.add_argument('--database', default=str(home / '.local/share/timeparagon/timeparagon.sqlite3'))
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    migration = Migration(args)
    snapshot_path = migration.root / 'snapshot.json'
    if snapshot_path.exists():
        snapshot = json.loads(snapshot_path.read_text())
    else:
        snapshot = migration.snapshot()
        migration.save('snapshot.json', snapshot)
    plan = migration.plan(snapshot)
    migration.save('plan.json', plan)
    print('Planned remote updates:', len(plan['actions']))
    if args.apply:
        migration.apply(plan)


if __name__ == '__main__':
    main()
