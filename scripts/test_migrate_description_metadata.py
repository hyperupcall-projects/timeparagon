import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('migration', pathlib.Path(__file__).with_name('migrate-description-metadata.py'))
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class DescriptionMetadataMigrationTests(unittest.TestCase):
    def test_isa_date_only_deadline_preserves_description(self):
        prose, metadata = migration.extract('source link\nDEADLINE: 2026-10-28', {'minutes': 5})
        self.assertEqual(prose, 'source link')
        self.assertEqual(metadata, {'minutes': 5, 'deadline': {'date': '2026-10-28', 'time': None}})
        self.assertEqual(migration.extract(prose, metadata), (prose, metadata))

    def test_timed_deadline_and_due_precedence(self):
        prose, metadata = migration.extract('DUE: 2026-10-01\nNotes\n DEADLINE: 2026-10-28 17:30\n', {})
        self.assertEqual(prose, 'Notes\n')
        self.assertEqual(metadata['deadline'], {'date': '2026-10-28', 'time': '17:30:00'})

    def test_invalid_conflicting_or_unmapped_fields_abort_before_writes(self):
        for text in ['DUE: tomorrow', 'DEADLINE: 2026-02-30', 'DUE_TIME: 09:00', 'PRIORITY: P1', 'DUE: 2026-10-01\nDUE: 2026-10-02']:
            with self.subTest(text=text), self.assertRaises(ValueError):
                migration.extract(text, {})
        with self.assertRaises(ValueError):
            migration.extract('DUE: 2026-10-28', {'deadline': '2026-10-29'})

    def test_google_tasks_footer_uses_same_typed_metadata(self):
        notes = 'Human notes\n---\ntt-version: 1\ntt-id: 417\ntt-deadline: "2026-10-28"\ntt-minutes: 5'
        migrated, data = migration.task_notes(notes)
        self.assertTrue(migrated.startswith('Human notes\n---\n'))
        self.assertEqual(data['metadata']['deadline'], {'date': '2026-10-28', 'time': None})
        self.assertEqual(migration.task_notes(migrated), (migrated, data))

    def test_event_properties_preserve_other_fields_and_round_trip(self):
        data = {'id': 417, 'metadata': {'minutes': 5}}
        event = {'description': 'Keep\nDEADLINE: 2026-10-28', 'extendedProperties': {'private': {'ttKind': 'task', 'ttCompleted': 'true', 'ttDataCount': '1', 'ttData0': migration.compact(data), 'other': 'preserve'}}}
        patch, expected = migration.event_patch(event)
        self.assertEqual(patch['description'], 'Keep')
        self.assertEqual(patch['extendedProperties']['private']['other'], 'preserve')
        self.assertEqual(migration.event_data(patch), expected)
        self.assertIsNone(migration.event_patch(patch)[0])


if __name__ == '__main__':
    unittest.main()
