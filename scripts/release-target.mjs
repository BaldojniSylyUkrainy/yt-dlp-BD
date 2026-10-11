import {execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';

export function assertReleaseTarget(expected, actual) {
  if (!/^[a-f0-9]{40}$/.test(expected)) throw new Error('Expected a full commit SHA');
  if (actual && actual !== expected) throw new Error(`Release tag points to ${actual}, build is ${expected}; refusing to upload`);
}

export function checkReleaseTarget(tag, expected, run = execFileSync) {
  // ls-remote distinguishes an absent tag from a transport/authentication error.
  run('git', ['check-ref-format', `refs/tags/${tag}`], {stdio: 'pipe'});
  const refs = run('git', ['ls-remote', '--tags', 'origin', `refs/tags/${tag}`, `refs/tags/${tag}^{}`], {encoding: 'utf8'}).trim().split('\n').filter(Boolean);
  const parsed = refs.map(line => line.split(/\s+/));
  const actual = (parsed.find(([, ref]) => ref === `refs/tags/${tag}^{}`) || parsed.find(([, ref]) => ref === `refs/tags/${tag}`))?.[0];
  assertReleaseTarget(expected, actual);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  checkReleaseTarget(process.env.TAG, process.env.GITHUB_SHA);
}
