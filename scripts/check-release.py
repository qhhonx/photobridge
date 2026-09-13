#!/usr/bin/env python3
"""Check release metadata and embedded trust before publishing."""
from pathlib import Path
import json,plistlib,re,sys,urllib.request
r=Path(__file__).resolve().parent.parent
i=json.loads((r/'release.json').read_text())
assert re.fullmatch(r'\d+\.\d+\.\d+(?:-beta\.\d+)?',i['version'])
assert type(i['build']) is int and 0<i['build']<2100000000
assert i['distribution_repository']=='qhhonx/photobridge'
p=plistlib.loads((r/'apps/macos/PhotoBridge/Info.plist').read_bytes())
assert p['SUPublicEDKey']==(r/'assets/update-public-key.txt').read_text().strip()
assert p['SUFeedURL']==i['website']+'/appcast.xml'
assert p['SURequireSignedFeed'] and p['SUVerifyUpdateBeforeExtraction']
if len(sys.argv)>1:
 releases=json.loads(Path(sys.argv[1]).read_text())
 for release in releases:
  if release['tag_name']=='v'+i['version'] and not release['draft']:
   print('publish=false');break
 else:
  def parts(version):
   match=re.fullmatch(r'(\d+)\.(\d+)\.(\d+)(?:-beta\.(\d+))?',version)
   if not match: return None
   return tuple(int(v) if v is not None else 2**63 for v in match.groups())
  published=[release for release in releases if not release['draft'] and release.get('published_at') and parts(release['tag_name'].removeprefix('v'))]
  if published:
   previous=max(published,key=lambda release:parts(release['tag_name'].removeprefix('v')))
   assert parts(i['version']) > parts(previous['tag_name'].removeprefix('v')), 'Release version must increase'
   expected=f"https://github.com/{i['distribution_repository']}/releases/download/{previous['tag_name']}/android-update.json"
   assert any(a['name']=='android-update.json' and a['browser_download_url']==expected for a in previous['assets']), 'Previous release metadata missing'
   with urllib.request.urlopen(expected,timeout=30) as response:
    old=json.load(response)
   assert i['build'] > old['build'], 'Build number must increase'
  print('publish=true')
else: print('Release configuration verified')
