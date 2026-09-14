import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { updateDemoLanguage } from '../lib/demo.js';

function player() {
  return { src: '', poster: '', pauses: 0, loads: 0,
    getAttribute(key) { return this[key]; },
    setAttribute(key, value) { this[key] = value; },
    pause() { this.pauses++; }, load() { this.loads++; } };
}
test('language switch selects matching media without autoplay or unrelated reloads', () => {
  const video = player();
  for (const language of ['en', 'zh', 'en']) {
    updateDemoLanguage(video, language);
    assert.equal(video.src, `/media/demo-${language}-v1.mp4`);
    assert.equal(video.poster, `/media/demo-${language}-v1.jpg`);
    for (const asset of [video.src, video.poster]) {
      assert.ok(existsSync(new URL(`..${asset}`, import.meta.url)));
    }
    const loads = video.loads;
    updateDemoLanguage(video, language);
    assert.equal(video.loads, loads, 'release refresh must not reset playback');
  }
  assert.equal(video.pauses, 3);
  assert.equal(video.loads, 3);
});
test('unknown language falls back to English, missing player is harmless', () => {
  const video = player();
  updateDemoLanguage(video, 'fr');
  assert.equal(video.src, '/media/demo-en-v1.mp4');
  assert.doesNotThrow(() => updateDemoLanguage(null, 'zh'));
});
test('demo copy has Chinese translations and player waits for user interaction', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  const chinese = JSON.parse(readFileSync(new URL('../zh.json', import.meta.url), 'utf8'));
  for (const [, key] of html.matchAll(/data-i18n="(demo_[^"]+)"/g)) assert.ok(chinese[key], key);
  const video = html.match(/<video\b[^>]+>/)[0];
  assert.match(video, /controls/);
  assert.match(video, /playsinline/);
  assert.match(video, /preload="none"/);
  assert.doesNotMatch(video, /autoplay/);
});
