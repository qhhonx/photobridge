#!/usr/bin/env python3
"""Sign a complete release and create platform manifests without printing secrets."""
from pathlib import Path
import hashlib, json, os, subprocess
r=Path(__file__).resolve().parent.parent
info=json.loads((r/'release.json').read_text()); version=info['version']; folder=r/'build/release'
key=os.environ['SPARKLE_KEY_FILE']
prefix=f"https://github.com/{info['distribution_repository']}/releases/download/v{version}/"
tools=r/'build/dependencies/sparkle/bin'
subprocess.run([str(tools/'generate_appcast'),'--ed-key-file',key,'--download-url-prefix',prefix,'--link',info['website'],'--maximum-deltas','0',str(folder)],check=True)
subprocess.run([str(tools/'sign_update'),'--verify','--ed-key-file',key,str(folder/'appcast.xml')],check=True)
apk=folder/f'PhotoBridge-{version}-arm64.apk'
(folder/'android-update.json').write_text(json.dumps({'version':version,'build':info['build'],'url':prefix+apk.name,'size':apk.stat().st_size,'sha256':hashlib.sha256(apk.read_bytes()).hexdigest()},indent=2)+'\n')
files=sorted(p for p in folder.iterdir() if p.suffix in ['.zip','.apk','.xml','.json'])
(folder/'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in files))
