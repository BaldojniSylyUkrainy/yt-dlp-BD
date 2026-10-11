import {test} from 'node:test';
import assert from 'node:assert/strict';
import {assertReleaseTarget, checkReleaseTarget} from './release-target.mjs';
const sha = 'a'.repeat(40);
test('release target permits an absent or matching tag, rejects a stale tag', () => {
  assertReleaseTarget(sha, undefined); assertReleaseTarget(sha, sha);
  assert.throws(() => assertReleaseTarget(sha, 'b'.repeat(40)), /refusing/);
  assert.throws(() => assertReleaseTarget('main', sha));
});
test('annotated tags use the peeled commit and network failures fail closed', () => {
  checkReleaseTarget('v1', sha, (_, args) => args[0] === 'ls-remote' ? `${'b'.repeat(40)}\trefs/tags/v1\n${sha}\trefs/tags/v1^{}` : '');
  assert.throws(() => checkReleaseTarget('v1', sha, () => {throw Error('offline');}), /offline/);
  assert.throws(() => checkReleaseTarget('v1', sha, (_, args) => args[0] === 'ls-remote' ? `${'b'.repeat(40)}\trefs/tags/v1` : ''), /refusing/);
});
