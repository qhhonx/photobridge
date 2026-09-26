import {test} from 'node:test';
import assert from 'node:assert/strict';
import handler from '../api/appcast.js';
function release() {
  const tag = 'v0.1.0-beta.15', prefix = `https://github.com/qhhonx/photobridge/releases/download/${tag}/`;
  return {tag_name: tag, published_at: '2026-09-26', draft: false,
    assets: ['PhotoBridge-0.1.0-beta.15-arm64.zip', 'PhotoBridge-0.1.0-beta.15-arm64.apk',
      'appcast.xml', 'android-update.json', 'SHA256SUMS'].map(name => ({name, size: 1, browser_download_url: prefix + name}))};
}
function response() {
  return {headers: {}, setHeader(key, value) {this.headers[key] = value;},
    status(code) {this.code = code; return this;}, send(body) {this.body = body;}};
}
test('signed feed bytes are preserved and expired releases cannot be served stale', async t => {
  const signedBytes = Buffer.from('<?xml version="1.0"?>\r\n<rss>signed content</rss>\n<!-- signature -->\n');
  t.mock.method(globalThis, 'fetch', async url => url.includes('api.github.com')
    ? {ok: true, json: async () => [release()]}
    : {ok: true, arrayBuffer: async () => signedBytes});
  const res = response(); await handler({}, res);
  assert.equal(res.code, 200); assert.deepEqual(res.body, signedBytes);
  assert.match(res.headers['Cache-Control'], /max-age=0/);
  assert.match(res.headers['Cache-Control'], /s-maxage=60/);
  assert.match(res.headers['Cache-Control'], /must-revalidate/);
  assert.doesNotMatch(res.headers['Cache-Control'], /stale/);
});
test('failed release lookup is not cached as an update', async t => {
  t.mock.method(globalThis, 'fetch', async () => ({ok: false}));
  const res = response(); await handler({}, res);
  assert.equal(res.code, 503); assert.equal(res.headers['Cache-Control'], 'no-store');
});
