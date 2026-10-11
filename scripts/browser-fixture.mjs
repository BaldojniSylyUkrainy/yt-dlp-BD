export function browserFixtureUrl(html) {
  // httpbin uses URL-safe base64, but unlike Node's base64url output its
  // decoder still requires '=' padding for input lengths not divisible by 3.
  const segment = Buffer.from(html).toString('base64').replaceAll('+', '-').replaceAll('/', '_');
  return `https://httpbin.org/base64/${segment}`;
}
