// Native-browser regression. No firewall, certificate, personal-profile or
// system-proxy changes. Requires a debug executable and an unlocked desktop.
// The public httpbin service sees only this fixture, never personal data.
// Zero connections is meaningful ONLY with the native completion nonce.
import net from 'node:net';
import dgram from 'node:dgram';
import {spawn} from 'node:child_process';
import {mkdtemp, readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {randomUUID} from 'node:crypto';
import {browserFixtureUrl} from './browser-fixture.mjs';

const executable = process.argv[2];
if (!executable) throw Error('Pass the debug app executable; a page that never loaded must never pass.');
const nonce = randomUUID();
const evidence = await mkdtemp(join(tmpdir(), 'baldoj-browser-smoke-'));
const report = join(evidence, 'native.json');

let tcpConnections = 0;
let udpPackets = 0;
const tcp = net.createServer(socket => {
  tcpConnections++;
  console.error('FAIL: browser reached local TCP receiver');
  socket.destroy();
});
const udp = dgram.createSocket('udp4');
udp.on('message', () => {
  udpPackets++;
  console.error('FAIL: browser reached local UDP receiver');
});
await new Promise(resolve => tcp.listen(0, '127.0.0.1', resolve));
await new Promise(resolve => udp.bind(0, '127.0.0.1', resolve));
const tcpPort = tcp.address().port;
const udpPort = udp.address().port;
const html = `<!doctype html><html><title>Browser isolation check</title>
<h1>Browser isolation check</h1><pre id="out"></pre><script>
const log = s => document.getElementById('out').textContent += s + '\\n';
log('WebRTC: ' + typeof RTCPeerConnection);
log('WebTransport: ' + typeof WebTransport);
for (const h of ['127.0.0.1','localhost','2130706433','127.0.0.1.nip.io','[::ffff:127.0.0.1]']) {
  fetch('https://' + h + ':${tcpPort}/probe', {mode:'no-cors'})
    .then(() => log('UNEXPECTED FETCH ' + h)).catch(() => log('fetch blocked ' + h));
  const i = new Image(); i.src = 'https://' + h + ':${tcpPort}/image';
  i.onerror = () => log('image blocked ' + h); document.body.append(i);
}
try { const w = new WebSocket('wss://127.0.0.1:${tcpPort}/socket');
  w.onerror = () => log('websocket blocked'); } catch { log('websocket blocked'); }
try { const r = new RTCPeerConnection({iceServers:[{urls:'stun:127.0.0.1:${udpPort}'}]});
  r.createDataChannel('test'); r.createOffer().then(o => r.setLocalDescription(o));
  log('RTC created: receiver must still see zero UDP'); } catch { log('RTC blocked'); }
setTimeout(() => { log('PROBES FINISHED'); location.href = 'baldoj-media://smoke-finished/?nonce=${nonce}'; }, 6000);
</script></html>`;
const url = browserFixtureUrl(html);
try {
  const response = await fetch(url, {signal: AbortSignal.timeout(15_000)});
  if (!response.ok || await response.text() !== html) throw Error(`Invalid fixture response: HTTP ${response.status}`);
} catch (error) {
  tcp.close(); udp.close();
  throw error;
}
console.log('Starting native browser isolation check. Evidence:', evidence);
const child = spawn(executable, [], {env: {...process.env, BALDOJ_BROWSER_SMOKE_URL: url, BALDOJ_BROWSER_SMOKE_NONCE: nonce, BALDOJ_BROWSER_SMOKE_REPORT: report}, stdio: 'inherit'});
let finished = false;
const timer = setTimeout(() => { child.kill(); void finish(); }, 60_000);
child.once('exit', () => { void finish(); });
child.once('error', () => { void finish(); });
process.once('SIGINT', () => { child.kill(); void finish(); });
async function finish() {
  if (finished) return;
  finished = true;
  clearTimeout(timer);
  const result = await readFile(report, 'utf8').then(JSON.parse).catch(() => null);
  const completed = result?.completed === true && result.nonce === nonce;
  console.log({completed, tcpConnections, udpPackets, result});
  tcp.close(); udp.close();
  process.exitCode = completed && !tcpConnections && !udpPackets ? 0 : 1;
}
