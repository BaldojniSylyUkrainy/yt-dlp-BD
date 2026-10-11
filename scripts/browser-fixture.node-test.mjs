import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserFixtureUrl} from './browser-fixture.mjs';
test('browser fixture uses one path segment and preserves decoder padding', () => {
  for (const html of ['x', 'xx', 'xxx', '<h1>?</h1><script>"???"</script>', 'Тест']) {
    const url = new URL(browserFixtureUrl(html));
    const encoded = url.pathname.slice('/base64/'.length);
    assert.match(encoded, /^[A-Za-z0-9_-]+={0,2}$/);
    assert.equal(encoded.length % 4, 0);
    assert.equal(Buffer.from(encoded, 'base64url').toString(), html);
  }
});
