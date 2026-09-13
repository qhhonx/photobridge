"""Real CLI processes: authenticated partial upload, receiver restart, resume, dedupe."""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target' / 'debug' / ('photobridge.exe' if os.name == 'nt' else 'photobridge')

def run(*args):
    return subprocess.run([str(BINARY), *map(str, args)], check=True, capture_output=True, text=True, timeout=45)

def request(base, token, path, body=None, method='GET'):
    req = urllib.request.Request(base + path, data=body, method=method,
                                 headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=5) as response:
        return json.load(response)

def start(root, token_file, token):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}'
    process = subprocess.Popen([str(BINARY), 'serve', '--root', str(root), '--token-file', str(token_file), '--listen', f'127.0.0.1:{port}'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    for _ in range(100):
        if process.poll() is not None:
            raise AssertionError('receiver did not start: ' + process.stderr.read().decode())
        try:
            request(base, token, '/v1/capabilities')
            return process, base
        except OSError:
            time.sleep(0.05)
    process.kill(); process.wait()
    raise AssertionError('receiver startup timed out')

def stop(process):
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill(); process.wait(timeout=5)
    process.stderr.close()

with tempfile.TemporaryDirectory(prefix='photobridge-cli-') as directory:
    root = Path(directory)
    source = root / 'source'; source.mkdir()
    photo = b'original resource metadata\x00' * 170000
    video = b'original paired video\x00'
    (source / 'photo.jpg').write_bytes(photo)
    (source / 'paired.mov').write_bytes(video)
    token_file = root / 'local.token'
    run('init-token', token_file)
    token = token_file.read_text().strip()
    manifest = root / 'asset.json'
    run('manifest', '--source-id', 'cli-fixture', '--file', source / 'photo.jpg', '--paired-video', source / 'paired.mov', '--output', manifest)
    asset = json.loads(manifest.read_text())
    process, base = start(root / 'receiver', token_file, token)
    try:
        state = request(base, token, '/v1/assets', manifest.read_bytes(), 'POST')
        asset_id = state['asset_id']; resource = asset['resources'][0]
        prefix = photo[:39317]; chunk_hash = hashlib.sha256(prefix).hexdigest()
        route = f'/v1/assets/{asset_id}/resources/{resource["sha256"]}?offset=0&sha256={chunk_hash}'
        state = request(base, token, route, prefix, 'PUT')
        assert state['resources'][0]['offset'] == len(prefix)
        assert state['receipt'] == 'receiving'
    finally:
        stop(process)
    process, base = start(root / 'receiver', token_file, token)
    try:
        before = request(base, token, f'/v1/assets/{asset_id}')
        assert before['resources'][0]['offset'] == len(prefix)
        result = run('send', '--server', base, '--token-file', token_file, '--manifest', manifest, '--files', source)
        assert json.loads(result.stdout)['receipt'] == 'received'
        for resource, expected in zip(asset['resources'], (photo, video)):
            assert (root / 'receiver' / 'blobs' / resource['sha256']).read_bytes() == expected
        (source / 'photo.jpg').unlink(); (source / 'paired.mov').unlink()
        repeated = run('send', '--server', base, '--token-file', token_file, '--manifest', manifest, '--files', source)
        assert json.loads(repeated.stdout)['receipt'] == 'received'
        assert json.loads(repeated.stdout)['processing'] == 'not_requested'
    finally:
        stop(process)
print('PASS: real receiver process restart, multi-chunk resume, complete motion asset, exact bytes and repeated send without source files')
