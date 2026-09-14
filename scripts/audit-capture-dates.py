#!/usr/bin/env python3
"""Read-only capture-date audit. Inputs and generated reports stay on the user's machine.

The receiver database must be a consistent local snapshot. Optional cloud JSON
contains visible Google Photos UI observations: url, label, filename, detail_date.
No credentials, private Google APIs, uploads, date edits or deletions are used.
"""
import argparse
import collections
import csv
import datetime as dt
import json
from pathlib import Path
import re
import sqlite3
from zoneinfo import ZoneInfo

ASSET = re.compile(r'^PB_([0-9a-f]{64})(?:[_.]|$)', re.I)
UTC = dt.timezone.utc

def iso(milliseconds, zone):
    return dt.datetime.fromtimestamp(milliseconds / 1000, zone).isoformat() if milliseconds else None

def read_media(path):
    rows = []
    if not path: return rows
    for line in Path(path).read_text().splitlines():
        fields = dict(re.findall(r'(?:^Row: \d+ |, )([^=]+)=([^,]*)', line))
        match = ASSET.match(fields.get('_display_name', ''))
        if not match: continue
        row = {'asset_id': match[1].lower(), 'filename': fields['_display_name']}
        for key in ('_id', 'datetaken', 'date_added', 'date_modified'):
            value = fields.get(key, '')
            row[key] = int(value) if re.fullmatch(r'-?\d+', value) else None
        rows.append(row)
    return rows

def cloud_instant(item):
    if item.get('observed_iso'):
        try:
            date = dt.datetime.fromisoformat(item['observed_iso'])
            return int(date.timestamp()*1000) if date.utcoffset() is not None else None
        except (ValueError, TypeError): return None
    # UI labels retain seconds; the info panel provides this photo's UTC offset.
    label = re.search(r' - ([A-Z][a-z]{2} \d{1,2}, \d{4}, .+)$', item.get('label', ''))
    offset = re.search(r'GMT([+-])(\d{2}):(\d{2})', item.get('detail_date') or '')
    if not label or not offset: return None
    try:
        clock = dt.datetime.strptime(' '.join(label[1].split()), '%b %d, %Y, %I:%M:%S %p')
        minutes = (int(offset[2]) * 60 + int(offset[3])) * (1 if offset[1] == '+' else -1)
        return int(clock.replace(tzinfo=dt.timezone(dt.timedelta(minutes=minutes))).timestamp() * 1000)
    except ValueError: return None

def classify_cloud(item, assets):
    if not item.get('filename'):
        return {**item, 'status': 'unreadable', 'reason': 'Cloud information panel did not load'}
    match = ASSET.match(item.get('filename') or '')
    if not match or match[1].lower() not in assets:
        return {**item, 'status': 'unmatched', 'reason': 'No unique PhotoBridge asset ID in the visible filename'}
    aid = match[1].lower()
    expected = assets[aid]['capture_ms']
    actual = cloud_instant(item)
    common = {**item, 'asset_id': aid, 'expected_ms': expected, 'observed_ms': actual}
    if not expected or actual is None:
        return {**common, 'status': 'unknown', 'reason': 'Source capture time or cloud timezone not available'}
    if expected // 1000 == actual // 1000:
        return {**common, 'status': 'correct', 'reason': 'Same instant after timezone conversion'}
    return {**common, 'status': 'date_mismatch', 'reason': 'Cloud date differs from source capture time', 'difference_seconds': (actual - expected) // 1000}

def audit(database, media, cloud, zone, since, until):
    with sqlite3.connect(Path(database).resolve().as_uri() + '?mode=ro', uri=True) as conn:
        if conn.execute('pragma quick_check').fetchone()[0] != 'ok': raise ValueError('Invalid database snapshot')
        assets = {}
        for aid, value in conn.execute('select id,manifest from assets where received=1'):
            manifest = json.loads(value)
            raw = manifest.get('metadata', {}).get('created_at_ms')
            try: capture = int(raw)
            except (ValueError, TypeError): capture = None
            if capture is not None and not 0 < capture < 253402300799000: capture = None
            assets[aid] = {'capture_ms': capture, 'source_id': manifest['source_id'], 'kind': manifest['kind'], 'original_filenames': [r['filename'] for r in manifest['resources']]}
    local = []
    for row in media:
        asset = assets.get(row['asset_id'])
        if not asset: continue
        capture = asset['capture_ms']; taken = row['datetaken']
        flags = []
        if not taken or taken <= 0: flags.append('media_capture_missing')
        elif capture and abs(taken - capture) >= 1000: flags.append('media_capture_mismatch')
        modified = (row['date_modified'] or 0) * 1000
        if capture and since <= modified < until and not since <= capture < until:
            flags.append('historical_capture_with_recent_file_time')
        if flags: local.append({**row, **asset, 'flags': flags, 'expected_date': iso(capture, zone)})
    checked = [classify_cloud(item, assets) for item in cloud]
    counts = collections.Counter(row['status'] for row in checked)
    media_times = [r['date_added'] for r in media if r['date_added']]
    capture_days = collections.Counter(iso(a['capture_ms'], zone)[:10] for a in assets.values() if a['capture_ms'] and since <= a['capture_ms'] < until)
    return {'summary': {'received_assets': len(assets), 'capture_days_in_window': dict(sorted(capture_days.items())), 'first_media_added': iso(min(media_times)*1000, zone) if media_times else None, 'local_suspects': len(local), 'cloud_checked': len(checked), 'cloud_status_counts': dict(counts), 'cloud_coverage': 'Only the supplied observations; not a complete cloud-library scan', 'mode': 'read_only'}, 'local_suspects': local, 'cloud_results': checked}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receiver-db',required=True)
    parser.add_argument('--media-index')
    parser.add_argument('--cloud-json')
    parser.add_argument('--since',required=True,help='Inclusive local date, YYYY-MM-DD')
    parser.add_argument('--until',required=True,help='Exclusive local date, YYYY-MM-DD')
    parser.add_argument('--timezone',default='UTC')
    parser.add_argument('--output',required=True)
    args=parser.parse_args();zone=ZoneInfo(args.timezone)
    start=int(dt.datetime.fromisoformat(args.since).replace(tzinfo=zone).timestamp()*1000)
    end=int(dt.datetime.fromisoformat(args.until).replace(tzinfo=zone).timestamp()*1000)
    if end<=start: parser.error('--until must be after --since')
    cloud=json.loads(Path(args.cloud_json).read_text()) if args.cloud_json else []
    report=audit(args.receiver_db,read_media(args.media_index),cloud,zone,start,end)
    output=Path(args.output);output.mkdir(parents=True,exist_ok=True)
    (output/'audit.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    with (output/'cloud-repair-review.csv').open('w',newline='') as f:
        writer=csv.DictWriter(f,fieldnames=['url','asset_id','filename','status','observed_date','expected_date','reason'])
        writer.writeheader()
        for row in report['cloud_results']:
            writer.writerow({**{k:row.get(k,'') for k in ('url','asset_id','filename','status','reason')},'observed_date':iso(row.get('observed_ms'),zone),'expected_date':iso(row.get('expected_ms'),zone)})
    print(json.dumps(report['summary'],ensure_ascii=False,indent=2))
if __name__=='__main__': main()
