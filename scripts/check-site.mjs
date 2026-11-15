#!/usr/bin/env node
// Static checks for the KeyReed website. No dependencies.
//
//   node scripts/check-site.mjs
//
// Verifies: unique ids, resolvable anchors and local assets, ARIA wiring, image alt text, SEO metadata,
// JSON-LD validity, JavaScript syntax, and that the Content-Security-Policy hash in both vercel.json files
// matches the inline script in index.html.

import { readFileSync, existsSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const site = join(root, 'website');
const html = readFileSync(join(site, 'index.html'), 'utf8');

let failures = 0;
const ok = (msg) => console.log(`  ok    ${msg}`);
const fail = (msg) => { failures++; console.log(`  FAIL  ${msg}`); };
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));

// ---- ids and anchors
const ids = [...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]);
const dupes = ids.filter((id, i) => ids.indexOf(id) !== i);
check(dupes.length === 0, `ids are unique (${ids.length} ids)${dupes.length ? ': ' + [...new Set(dupes)].join(', ') : ''}`);

const anchors = [...html.matchAll(/href="#([^"]+)"/g)].map((m) => m[1]);
const brokenAnchors = anchors.filter((a) => !ids.includes(a));
check(brokenAnchors.length === 0, `all ${anchors.length} in-page anchors resolve${brokenAnchors.length ? ': ' + brokenAnchors.join(', ') : ''}`);

const sprite = [...html.matchAll(/<use href="#([^"]+)"/g)].map((m) => m[1]);
const brokenUse = sprite.filter((a) => !ids.includes(a));
check(brokenUse.length === 0, `all ${sprite.length} icon references exist${brokenUse.length ? ': ' + [...new Set(brokenUse)].join(', ') : ''}`);

const controls = [...html.matchAll(/aria-(?:controls|labelledby)="([^"]+)"/g)].map((m) => m[1]);
const brokenAria = controls.filter((c) => !ids.includes(c));
check(brokenAria.length === 0, `all ${controls.length} aria-controls/labelledby targets exist${brokenAria.length ? ': ' + [...new Set(brokenAria)].join(', ') : ''}`);

// ---- local files
const localRefs = [...html.matchAll(/(?:src|href)="([^"#:]+)"/g)].map((m) => m[1]).filter((p) => !p.startsWith('//'));
const missing = [...new Set(localRefs)].filter((p) => !existsSync(join(site, p)));
check(missing.length === 0, `all ${new Set(localRefs).size} local files referenced exist${missing.length ? ': ' + missing.join(', ') : ''}`);

// ---- images and links
const imgs = [...html.matchAll(/<img\b[^>]*>/g)].map((m) => m[0]);
const noAlt = imgs.filter((i) => !/\salt=/.test(i));
check(noAlt.length === 0, `every <img> has an alt attribute (${imgs.length} images)`);
const insecure = [...html.matchAll(/(?:href|src)="(http:\/\/[^"]+)"/g)].map((m) => m[1]);
check(insecure.length === 0, 'no insecure http:// links');
const external = [...new Set([...html.matchAll(/href="(https:\/\/[^"]+)"/g)].map((m) => new URL(m[1]).host))];
const allowed = new Set(['github.com', 'wisprflow.ai', 'docs.wisprflow.ai', 'keyreed-website.vercel.app']);
const unexpected = external.filter((h) => !allowed.has(h));
check(unexpected.length === 0, `external links only go to ${[...allowed].join(', ')}${unexpected.length ? ' (unexpected: ' + unexpected.join(', ') + ')' : ''}`);

// ---- SEO
const meta = (re) => (html.match(re) || [])[1];
const title = meta(/<title>([^<]+)<\/title>/);
check(title && title.length <= 70, `title present and <= 70 chars (${title ? title.length : 0})`);
const desc = meta(/<meta name="description" content="([^"]+)"/);
check(desc && desc.length >= 80 && desc.length <= 160, `meta description 80-160 chars (${desc ? desc.length : 0})`);
const canonical = meta(/<link rel="canonical" href="([^"]+)"/);
check(canonical === 'https://keyreed-website.vercel.app/', 'canonical URL is the production URL');
for (const prop of ['og:title', 'og:description', 'og:image', 'og:url', 'og:type']) {
  check(new RegExp(`<meta property="${prop}"`).test(html), `Open Graph ${prop}`);
}
const ogImg = meta(/<meta property="og:image" content="([^"]+)"/);
check(ogImg && ogImg.startsWith('https://keyreed-website.vercel.app/assets/') && existsSync(join(site, new URL(ogImg).pathname)), 'og:image is absolute and the file exists');
check(/<meta name="twitter:card" content="summary_large_image"/.test(html), 'Twitter card');
check(/<html lang="en">/.test(html), 'html lang');
check(/<h1[\s>]/.test(html) && (html.match(/<h1[\s>]/g) || []).length === 1, 'exactly one <h1>');
check(existsSync(join(site, 'robots.txt')) && existsSync(join(site, 'sitemap.xml')), 'robots.txt and sitemap.xml exist');
check(/keyreed-website\.vercel\.app\/sitemap\.xml/.test(readFileSync(join(site, 'robots.txt'), 'utf8')), 'robots.txt points at the sitemap');

// ---- structured data
let ld;
try { ld = JSON.parse(html.match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/)[1]); ok('JSON-LD parses'); } catch (e) { fail('JSON-LD parses: ' + e.message); }
if (ld) {
  const types = ld['@graph'].map((n) => n['@type']);
  check(types.includes('SoftwareApplication') && types.includes('FAQPage'), `JSON-LD types: ${types.join(', ')}`);
}

// ---- faq parity: every FAQ question in the page exists
const faqButtons = [...html.matchAll(/<button type="button" aria-expanded="false" aria-controls="(a\d+)"/g)].length;
check(faqButtons >= 11, `FAQ has ${faqButtons} questions (>= 11 required)`);

// ---- scripts
for (const f of ['site.js', 'demo.js', 'excerpts.js']) {
  try { execFileSync(process.execPath, ['--check', join(site, f)], { stdio: 'pipe' }); ok(`${f} parses`); } catch (e) { fail(`${f} syntax: ${e.stderr}`); }
}

// ---- CSP hash matches the one inline script
const inline = (html.match(/<script>([\s\S]*?)<\/script>/) || [])[1] || '';
const hash = 'sha256-' + createHash('sha256').update(inline).digest('base64');
const cfgs = [join(site, 'vercel.json'), join(root, 'vercel.json')];
const sets = cfgs.map((p) => JSON.parse(readFileSync(p, 'utf8')));
for (const [i, cfg] of sets.entries()) {
  const csp = cfg.headers?.flatMap((h) => h.headers).find((h) => h.key === 'Content-Security-Policy')?.value || '';
  check(csp.includes(`'${hash}'`), `${i === 0 ? 'website/' : ''}vercel.json CSP allows the inline script hash`);
}
check(JSON.stringify(sets[0].headers) === JSON.stringify(sets[1].headers), 'both vercel.json files carry identical headers');
check(!/<script(?![^>]*\bsrc=)(?![^>]*ld\+json)[^>]*>/.test(html.replace(/<script>[\s\S]*?<\/script>/, '')), 'no other inline scripts');

// ---- hygiene: no obvious secrets in the website folder
const secretRe = /(AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{30,}|sk-[A-Za-z0-9]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|xox[abp]-[A-Za-z0-9-]{10,})/;
let leaked = [];
for (const f of ['index.html', 'styles.css', 'site.js', 'demo.js', 'excerpts.js', 'vercel.json', '404.html']) {
  const p = join(site, f);
  if (existsSync(p) && secretRe.test(readFileSync(p, 'utf8'))) leaked.push(f);
}
check(leaked.length === 0, 'no secret-looking strings in website sources' + (leaked.length ? ': ' + leaked.join(', ') : ''));

// ---- asset sizes stay reasonable
const big = ['assets/og-image.png', 'assets/icon.png', 'assets/logo.png'].filter((f) => existsSync(join(site, f)) && statSync(join(site, f)).size > 400 * 1024);
check(big.length === 0, 'images in website/assets stay under 400 KB' + (big.length ? ': ' + big.join(', ') : ''));

console.log(failures ? `\n${failures} check(s) failed` : '\nAll site checks passed');
process.exit(failures ? 1 : 0);
