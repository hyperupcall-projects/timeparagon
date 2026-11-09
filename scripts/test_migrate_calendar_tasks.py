import importlib.util
import json
import pathlib
import sqlite3
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('migration', pathlib.Path(__file__).with_name('migrate-calendar-tasks.py'))
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class CalendarTaskMigrationTests(unittest.TestCase):
    def test_footer_preserves_notes_deadline_identity_and_metadata(self):
        notes = 'Human notes\n---\ntt-version: 1\ntt-id: 123\ntt-deadline: "2030-11-04 17:00"\ntt-labels: ["work"]\ntt-schedule: {"event_id":"old-copy"}'
        prose, data = migration.footer(notes)
        self.assertEqual(prose, 'Human notes\nDEADLINE: 2030-11-04 17:00')
        self.assertEqual(data, {'id': 123, 'metadata': {'labels': ['work']}})

    def test_unicode_metadata_chunks_fit_google_limits_and_round_trip(self):
        data = {'id': 123, 'metadata': {'labels': ['🌈' * 1800]}}
        props = migration.properties(data, [], [], False)
        count = int(props['ttDataCount'])
        chunks = [props['ttData' + str(i)] for i in range(count)]
        self.assertTrue(all(len(x.encode()) <= 900 for x in chunks))
        self.assertEqual(json.loads(''.join(chunks)), data)

    def test_local_cutover_preserves_history_translates_queue_and_note_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            m = object.__new__(migration.Migration)
            m.root = root / 'backup'
            m.root.mkdir()
            m.db_path = root / 'data.sqlite3'
            m.config_path = root / 'config.toml'
            notes = root / 'notes'
            notes.mkdir()
            note = notes / 'attached.md'
            note.write_text('---\nlinks:\n  - {"kind":"task","id":"task:old","title":"Work"}\n---\nKeep this body\n')
            m.config = {'notes': {'directory': str(notes)}}
            m.config_path.write_text('[calendar]\ntime_block_list = "tasks:retired"\nweekly_task_order = ["task:old"]\n')
            db = sqlite3.connect(m.db_path)
            for table, pk, column in [('tasks', 'id INTEGER PRIMARY KEY', 'metadata'), ('google_task_records', 'id TEXT PRIMARY KEY', 'event'), ('google_entities', 'id TEXT PRIMARY KEY', 'event'), ('google_task_backend_state', 'task_id INTEGER PRIMARY KEY', 'data'), ('google_operation_progress', 'operation_id INTEGER PRIMARY KEY', 'data'), ('google_operations', 'id INTEGER PRIMARY KEY', 'payload'), ('calendar_months', 'month TEXT PRIMARY KEY', 'events'), ('documents', 'key TEXT PRIMARY KEY', 'value')]:
                db.execute(f'CREATE TABLE {table}({pk},{column} TEXT)')
            db.execute('ALTER TABLE google_operations ADD COLUMN entity TEXT')
            db.execute('CREATE TABLE task_aliases(source TEXT PRIMARY KEY,task_id INTEGER)')
            db.execute('CREATE TABLE history(task_id INTEGER,minutes INTEGER)')
            db.execute('INSERT INTO history VALUES(123,45)')
            data = {'id': 123, 'metadata': {'minutes': 45, 'schedule': {'event_id': 'native'}}}
            event = {'id': 'task:old', 'calendar_id': 'tasks:retired', 'title': 'Work', 'start': '2030-11-04T00:00:00-08:00', 'end': '2030-11-05T00:00:00-08:00', 'all_day': True, 'details': {'scheduled': {'event_id': 'native'}, 'task_data': data}}
            db.execute('INSERT INTO tasks VALUES(123,?)', [json.dumps(data['metadata'])])
            db.execute('INSERT INTO google_task_records VALUES(?,?)', ['task:old', json.dumps(event)])
            db.execute('INSERT INTO google_operations VALUES(1,?,?)', [json.dumps({'DeleteTask': {'task': event, 'calendar': 'app'}}), 'tasks:retired\ntask:old'])
            db.commit()
            data['metadata'].pop('schedule')
            plan = {'remove_lists': ['retired'], 'actions': [{'kind': 'calendar', 'list': 'retired', 'task': {'id': 'old'}, 'calendar': 'app', 'id': 'native', 'data': data, 'body': {'summary': 'Work', 'description': 'Notes', 'start': {'dateTime': '2030-11-04T09:00:00-08:00'}, 'end': {'dateTime': '2030-11-04T09:45:00-08:00'}}}]}
            m.local(plan, {'0': {'calendar': 'app', 'id': 'native'}})
            saved = json.loads(db.execute('SELECT event FROM google_task_records').fetchone()[0])
            self.assertEqual(saved['id'], 'native')
            self.assertFalse(saved['all_day'])
            self.assertEqual(saved['start'], '2030-11-04T09:00:00-08:00')
            self.assertNotIn('scheduled', saved['details'])
            payload, entity = db.execute('SELECT payload,entity FROM google_operations').fetchone()
            self.assertEqual(json.loads(payload)['DeleteTask']['task']['id'], 'native')
            self.assertEqual(entity, 'app\nnative')
            self.assertEqual(db.execute('SELECT * FROM history').fetchall(), [(123, 45)])
            self.assertNotIn('time_block_list', m.config_path.read_text())
            self.assertIn('"id":"123"', note.read_text())
            self.assertTrue(note.read_text().endswith('Keep this body\n'))
            self.assertTrue((m.root / 'notes-before' / 'attached.md').exists())
            db.close()


if __name__ == '__main__':
    unittest.main()
