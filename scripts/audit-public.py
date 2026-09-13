#!/usr/bin/env python3
"""Fail on common credentials, device identifiers and absolute user paths."""
from pathlib import Path
import re, subprocess, sys
root=Path(__file__).resolve().parent.parent
names=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=root).decode().split('\0')
rules={
 'private key':rb'-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----',
 'GitHub token':rb'gh[pousr]_[A-Za-z0-9]{30,}',
 'absolute home':rb'/(?:Users|home)/[A-Za-z][^/\s"\']+/',
 'Apple device ID':rb'\b[0-9A-F]{8}-[0-9A-F]{16}\b',
 'personal team setting':rb'DEVELOPMENT_TEAM\s*=\s*[A-Z0-9]{10}',
}
errors=[]
for name in sorted(set(names)):
 if not name: continue
 path=root/name
 if path.suffix in ['.jks','.keystore','.key','.mobileprovision','.p12']:
  errors.append((name,'signing material'));continue
 if not path.is_file() or path.suffix in ['.png','.icns','.jar']: continue
 content=path.read_bytes()
 for label,pattern in rules.items():
  if re.search(pattern,content): errors.append((name,label))
for name,label in errors: print(f'{name}: {label}')
if errors: sys.exit(1)
print(f'Public-source checks passed for {len(set(names))-1} files. Review still required for context-specific personal data.')
