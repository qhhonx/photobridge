#!/usr/bin/env python3
"""Verify released metadata, archive signature, APK identity and exact checksums."""
from pathlib import Path
import base64,hashlib,json,os,plistlib,re,subprocess,tempfile,xml.etree.ElementTree as ET
r=Path(__file__).resolve().parent.parent; folder=r/'build/release'; info=json.loads((r/'release.json').read_text());version=info['version']
feed=ET.parse(folder/'appcast.xml');ns={'s':'http://www.andymatuschak.org/xml-namespaces/sparkle'}
item=feed.find('./channel/item');assert item is not None
assert item.findtext('s:version',namespaces=ns)==str(info['build'])
enc=item.find('enclosure');assert enc is not None
archive=folder/f'PhotoBridge-{version}-arm64.zip'
assert enc.attrib['url']==f'https://github.com/{info["distribution_repository"]}/releases/download/v{version}/{archive.name}'
assert int(enc.attrib['length'])==archive.stat().st_size
key=os.environ['SPARKLE_KEY_FILE']; tool=str(r/'build/dependencies/sparkle/bin/sign_update')
subprocess.run([tool,'--verify','--ed-key-file',key,str(archive),enc.attrib['{'+ns['s']+'}edSignature']],check=True)
subprocess.run([tool,'--verify','--ed-key-file',key,str(folder/'appcast.xml')],check=True)
# Reject a tampered archive with the same signature.
with tempfile.TemporaryDirectory() as d:
 tampered=Path(d)/'changed.zip';tampered.write_bytes(archive.read_bytes()+b'changed')
 result=subprocess.run([tool,'--verify','--ed-key-file',key,str(tampered),enc.attrib['{'+ns['s']+'}edSignature']],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 assert result.returncode != 0
manifest=json.loads((folder/'android-update.json').read_text());apk=folder/f'PhotoBridge-{version}-arm64.apk'
assert manifest['version']==version and manifest['build']==info['build']
assert manifest['size']==apk.stat().st_size and manifest['sha256']==hashlib.sha256(apk.read_bytes()).hexdigest()
sdk=Path(os.environ.get('ANDROID_HOME',str(Path.home()/'Library/Android/sdk')))
toolset=sdk/'build-tools/36.1.0/apksigner'
result=subprocess.check_output([str(toolset),'verify','--verbose','--print-certs-pem',str(apk)],text=True)
expected=(r/'assets/android-signing-cert.sha256').read_text().strip()
certificates=re.findall(r'-----BEGIN CERTIFICATE-----\s*(.*?)\s*-----END CERTIFICATE-----',result,re.S)
fingerprints={hashlib.sha256(base64.b64decode(pem)).hexdigest() for pem in certificates}
assert fingerprints == {expected}, f'Unexpected Android release certificates: {fingerprints}'
badging=subprocess.check_output([str(toolset.parent/'aapt'),'dump','badging',str(apk)],text=True)
assert re.search(r"package: name='app\.photobridge' versionCode='"+str(info['build'])+"'",badging)
print('Android package, version and dedicated release certificate verified')
for line in (folder/'SHA256SUMS').read_text().splitlines():
 digest,name=line.split('  ');assert hashlib.sha256((folder/name).read_bytes()).hexdigest()==digest
print('Release signatures, tamper rejection, versions and checksums verified')
