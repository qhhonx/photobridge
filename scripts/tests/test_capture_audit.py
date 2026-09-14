import importlib.util
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('audit',Path(__file__).parents[1]/'audit-capture-dates.py')
audit=importlib.util.module_from_spec(spec);spec.loader.exec_module(audit)
class CaptureAuditTests(unittest.TestCase):
    def test_same_instant_in_different_timezones_is_not_repaired(self):
        aid='a'*64
        item={'filename':'PB_'+aid+'.jpg','label':'Photo - Portrait - Sep 12, 2026, 8:08:05 PM','detail_date':'Sep 12 Time taken: Yesterday, 8:08 PM GMT-07:00'}
        # Derive the independent UTC reference, not the function under test.
        import datetime as dt
        expected=int(dt.datetime(2026,9,13,3,8,5,tzinfo=dt.timezone.utc).timestamp()*1000)
        self.assertEqual(audit.classify_cloud(item,{aid:{'capture_ms':expected}})['status'],'correct')
        self.assertEqual(audit.classify_cloud(item,{aid:{'capture_ms':expected-86400000}})['status'],'date_mismatch')
        item['detail_date']='Sep 12';self.assertEqual(audit.classify_cloud(item,{aid:{'capture_ms':expected}})['status'],'unknown')
    def test_iso_requires_timezone_and_unloaded_panel_is_reported(self):
        self.assertIsNone(audit.cloud_instant({'observed_iso':'2026-09-14T10:00:00'}))
        self.assertIsNone(audit.cloud_instant({'observed_iso':'bad'}))
        self.assertEqual(audit.cloud_instant({'observed_iso':'2026-09-14T10:00:00+08:00'}), audit.cloud_instant({'observed_iso':'2026-09-14T02:00:00+00:00'}))
        self.assertEqual(audit.classify_cloud({'filename':None}, {})['status'],'unreadable')
    def test_unrecognized_file_and_missing_source_date_are_never_repaired(self):
        self.assertEqual(audit.classify_cloud({'filename':'IMG_0001.JPG'}, {})['status'],'unmatched')
        aid='b'*64
        self.assertEqual(audit.classify_cloud({'filename':'PB_'+aid+'.heic'},{aid:{'capture_ms':None}})['status'],'unknown')
if __name__=='__main__': unittest.main()
