#!/usr/bin/env node
// Static checks for the KeyReed website. No dependencies.
//
//   node scripts/check-site.mjs
//
// Verifies across all HTML pages: unique ids, resolvable anchors and local assets, ARIA wiring,
// image alt text, SEO metadata, JSON-LD validity & FAQ parity, JavaScript syntax, vendor manifest,
// and strict Content-Security-Policy parity across both vercel.json files.

import { readFileSync, existsSync, statSync, readdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const site = join(root, 'website');
const PROD = 'https://www.keyreed.com';
const PAGES = ['index.html', 'details.html', 'vs-wispr-flow.html', '404.html'];

let failures = 0;
const ok = (msg) => console.log(`  ok    ${msg}`);
const fail = (msg) => { failures++; console.log(`  FAIL  ${msg}`); };
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));

function walkFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walkFiles(full));
    else out.push(full);
  }
  return out;
}

const htmlByPage = new Map();
for (const page of PAGES) {
  const p = join(site, page);
  check(existsSync(p), `${page} exists`);
  if (existsSync(p)) htmlByPage.set(page, readFileSync(p, 'utf8'));
}

const html = htmlByPage.get('index.html') || '';

// ---- Per-page structural, ARIA, script-src, link, and image checks
const allowedHosts = new Set(['github.com', 'wisprflow.ai', 'docs.wisprflow.ai', 'www.keyreed.com']);

for (const [page, doc] of htmlByPage.entries()) {
  const ids = [...doc.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]);
  const dupes = ids.filter((id, i) => ids.indexOf(id) !== i);
  check(dupes.length === 0, `[${page}] ids are unique (${ids.length} ids)${dupes.length ? ': ' + [...new Set(dupes)].join(', ') : ''}`);

  const anchors = [...doc.matchAll(/href="#([^"]+)"/g)].map((m) => m[1]);
  const brokenAnchors = anchors.filter((a) => !ids.includes(a));
  check(brokenAnchors.length === 0, `[${page}] all ${anchors.length} in-page anchors resolve${brokenAnchors.length ? ': ' + brokenAnchors.join(', ') : ''}`);

  const sprite = [...doc.matchAll(/<use href="#([^"]+)"/g)].map((m) => m[1]);
  const brokenUse = sprite.filter((a) => !ids.includes(a));
  check(brokenUse.length === 0, `[${page}] all ${sprite.length} icon references exist${brokenUse.length ? ': ' + [...new Set(brokenUse)].join(', ') : ''}`);

  const controls = [...doc.matchAll(/aria-(?:controls|labelledby|describedby)="([^"]+)"/g)].map((m) => m[1]);
  const brokenAria = controls.filter((c) => !ids.includes(c));
  check(brokenAria.length === 0, `[${page}] all ${controls.length} aria target ids exist${brokenAria.length ? ': ' + [...new Set(brokenAria)].join(', ') : ''}`);

  // Local files (cleanUrls resolves /page -> page.html and /#anchor -> index.html)
  const localRefs = [...doc.matchAll(/(?:src|href)="([^"#]+)(?:#[^"]*)?"/g)]
    .map((m) => m[1])
    .filter((p) => p && !/^[a-z]+:/i.test(p) && !p.startsWith('//'));
  const missing = [...new Set(localRefs)].filter((p) => {
    if (p === '/') return !existsSync(join(site, 'index.html'));
    const clean = p.replace(/^\//, '');
    return !existsSync(join(site, clean)) && !existsSync(join(site, clean + '.html'));
  });
  check(missing.length === 0, `[${page}] all ${new Set(localRefs).size} local files referenced exist${missing.length ? ': ' + missing.join(', ') : ''}`);

  // Every <script src> exists locally and no script loads from another origin
  const scriptTags = [...doc.matchAll(/<script\b[^>]*>/g)].map((m) => m[0]);
  const scriptSrcs = scriptTags
    .filter((t) => !/application\/ld\+json/.test(t))
    .map((t) => (t.match(/\bsrc="([^"]+)"/) || [])[1]);
  const crossOriginScripts = scriptSrcs.filter((s) => !s || /^(?:https?:)?\/\//i.test(s));
  const missingScripts = scriptSrcs
    .filter((s) => s && !/^(?:https?:)?\/\//i.test(s))
    .filter((s) => !existsSync(join(site, s.replace(/^\//, ''))));
  check(crossOriginScripts.length === 0 && missingScripts.length === 0, `[${page}] all ${scriptSrcs.length} <script src> files exist on same origin`);

  // No inline scripts (JSON-LD excepted)
  check(!/<script(?![^>]*\bsrc=)(?![^>]*ld\+json)[^>]*>/.test(doc), `[${page}] no inline scripts (JSON-LD data excepted)`);

  // Images & external links
  const imgs = [...doc.matchAll(/<img\b[^>]*>/g)].map((m) => m[0]);
  const noAlt = imgs.filter((i) => !/\salt=/.test(i));
  check(noAlt.length === 0, `[${page}] every <img> has an alt attribute (${imgs.length} images)`);

  const insecure = [...doc.matchAll(/(?:href|src)="(http:\/\/[^"]+)"/g)].map((m) => m[1]);
  check(insecure.length === 0, `[${page}] no insecure http:// links`);

  const external = [...new Set([...doc.matchAll(/href="(https:\/\/[^"]+)"/g)].map((m) => new URL(m[1]).host))];
  const unexpected = external.filter((h) => !allowedHosts.has(h));
  check(unexpected.length === 0, `[${page}] external links only go to ${[...allowedHosts].join(', ')}${unexpected.length ? ' (unexpected: ' + unexpected.join(', ') + ')' : ''}`);

  // Heading count & lang & default theme & theme.js in <head>
  check(/<html lang="en" data-theme="light">/.test(doc), `[${page}] light is the default theme with lang="en"`);
  check(/<h1[\s>]/.test(doc) && (doc.match(/<h1[\s>]/g) || []).length === 1, `[${page}] exactly one <h1>`);
  const head = (doc.match(/<head>([\s\S]*?)<\/head>/) || [])[1] || '';
  check(/<script src="\/?theme\.js"><\/script>/.test(head), `[${page}] theme.js loads synchronously in <head>`);
}

// ---- SEO & Canonical checks across indexable pages
const expectedCanonicals = [
  ['index.html', `${PROD}/`],
  ['details.html', `${PROD}/details`],
  ['vs-wispr-flow.html', `${PROD}/vs-wispr-flow`],
];

for (const [page, expectedUrl] of expectedCanonicals) {
  const doc = htmlByPage.get(page) || '';
  const meta = (re) => (doc.match(re) || [])[1];
  const title = meta(/<title>([^<]+)<\/title>/);
  check(Boolean(title && title.length <= 70), `[${page}] title present and <= 70 chars (${title ? title.length : 0})`);
  const desc = meta(/<meta name="description" content="([^"]+)"/);
  check(Boolean(desc && desc.length >= 80 && desc.length <= 160), `[${page}] meta description 80-160 chars (${desc ? desc.length : 0})`);
  const canonical = meta(/<link rel="canonical" href="([^"]+)"/);
  check(canonical === expectedUrl, `[${page}] canonical URL is ${expectedUrl} (got ${canonical})`);
  for (const prop of ['og:title', 'og:description', 'og:image', 'og:url', 'og:type']) {
    check(new RegExp(`<meta property="${prop}"`).test(doc), `[${page}] Open Graph ${prop}`);
  }
  const ogUrl = meta(/<meta property="og:url" content="([^"]+)"/);
  check(ogUrl === expectedUrl, `[${page}] og:url matches canonical`);
  const ogImg = meta(/<meta property="og:image" content="([^"]+)"/);
  check(Boolean(ogImg && ogImg.startsWith(`${PROD}/assets/`) && existsSync(join(site, new URL(ogImg).pathname))), `[${page}] og:image is on ${PROD} and file exists`);
  check(/<meta name="twitter:card" content="summary_large_image"/.test(doc), `[${page}] Twitter card`);
}

check(/id="theme-btn"/.test(html), 'theme toggle exists on homepage');
check(existsSync(join(site, 'robots.txt')) && existsSync(join(site, 'sitemap.xml')), 'robots.txt and sitemap.xml exist');
const robots = readFileSync(join(site, 'robots.txt'), 'utf8');
const sitemap = readFileSync(join(site, 'sitemap.xml'), 'utf8');
check(robots.includes(`${PROD}/sitemap.xml`), `robots.txt points at ${PROD}/sitemap.xml`);
check(sitemap.includes(`${PROD}/`) && sitemap.includes(`${PROD}/details`) && sitemap.includes(`${PROD}/vs-wispr-flow`) && !sitemap.includes('vercel.app'), 'sitemap.xml lists /, /details, and /vs-wispr-flow on www.keyreed.com');

// ---- Structured data & FAQ parity on homepage
let ld;
try {
  ld = JSON.parse(html.match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/)[1]);
  ok('homepage JSON-LD parses');
} catch (e) {
  fail('homepage JSON-LD parses: ' + e.message);
}

const decodeEntities = (s) => s
  .replace(/&amp;/g, '&')
  .replace(/&lt;/g, '<')
  .replace(/&gt;/g, '>')
  .replace(/&quot;/g, '"')
  .replace(/&#39;/g, "'");

if (ld) {
  const graph = ld['@graph'] || [];
  const types = graph.map((n) => n['@type']);
  check(types.includes('SoftwareApplication') && types.includes('FAQPage'), `JSON-LD types: ${types.join(', ')}`);

  const appNode = graph.find((n) => n['@type'] === 'SoftwareApplication');
  check(appNode && appNode.url === `${PROD}/` && String(appNode['@id']).startsWith(PROD), `JSON-LD SoftwareApplication points at ${PROD}`);

  const faqNode = graph.find((n) => n['@type'] === 'FAQPage');
  const ldQuestions = (faqNode?.mainEntity || []).map((q) => q.name.trim());
  const visibleQuestions = [...html.matchAll(/<div class="qa">[\s\S]*?<button[^>]*>([\s\S]*?)<\/button>/g)]
    .map((m) => decodeEntities(m[1].replace(/<[^>]+>/g, '').trim()));

  check(visibleQuestions.length >= 6, `FAQ has ${visibleQuestions.length} visible questions (>= 6 required)`);
  check(
    JSON.stringify(visibleQuestions) === JSON.stringify(ldQuestions),
    `visible FAQ questions match JSON-LD FAQPage 1:1 (${visibleQuestions.length} == ${ldQuestions.length})`
  );
}

// ---- Syntax check on every JS file in website/
const jsFiles = [
  'theme.js',
  'excerpts.js',
  'js/motion.js',
  'js/site.js',
  'js/reeds.js',
  'js/hero.js',
  'js/scenes.js',
  'vendor/gsap/gsap.min.js',
  'vendor/gsap/ScrollTrigger.min.js',
  'vendor/gsap/SplitText.min.js',
  'vendor/lenis/lenis.min.js',
];
for (const f of jsFiles) {
  const full = join(site, f);
  if (!existsSync(full)) {
    fail(`${f} exists`);
    continue;
  }
  try {
    execFileSync(process.execPath, ['--check', full], { stdio: 'pipe' });
    ok(`${f} parses`);
  } catch (e) {
    fail(`${f} syntax: ${e.stderr}`);
  }
}

// ---- Vendor README inventory check
const vendorDir = join(site, 'vendor');
const vendorReadmePath = join(vendorDir, 'README.md');
check(existsSync(vendorReadmePath), 'website/vendor/README.md exists');
if (existsSync(vendorReadmePath)) {
  const vendorReadme = readFileSync(vendorReadmePath, 'utf8');
  const vendorFiles = walkFiles(vendorDir)
    .map((f) => relative(vendorDir, f).replace(/\\/g, '/'))
    .filter((f) => f !== 'README.md');
  const unlisted = vendorFiles.filter((f) => !vendorReadme.includes(f));
  check(unlisted.length === 0, `website/vendor/README.md lists all ${vendorFiles.length} vendored files${unlisted.length ? ': missing ' + unlisted.join(', ') : ''}`);
}

// ---- CSP: scripts come from 'self' only, identical headers in both vercel.json files
const cfgs = [join(site, 'vercel.json'), join(root, 'vercel.json')];
const sets = cfgs.map((p) => JSON.parse(readFileSync(p, 'utf8')));
for (const [i, cfg] of sets.entries()) {
  const csp = cfg.headers?.flatMap((h) => h.headers).find((h) => h.key === 'Content-Security-Policy')?.value || '';
  check(/script-src 'self'(;|$)/.test(csp), `${i === 0 ? 'website/' : ''}vercel.json CSP restricts scripts to 'self'`);
}
check(JSON.stringify(sets[0].headers) === JSON.stringify(sets[1].headers), 'both vercel.json files carry identical headers');

// ---- Hygiene: no secret-looking strings anywhere in website/
const secretRe = /(AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{30,}|sk-[A-Za-z0-9]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|xox[abp]-[A-Za-z0-9-]{10,})/;
const textExts = /\.(html|css|js|mjs|json|md|txt|xml|webmanifest)$/i;
const leaked = walkFiles(site)
  .filter((f) => textExts.test(f))
  .filter((f) => secretRe.test(readFileSync(f, 'utf8')))
  .map((f) => relative(site, f));
check(leaked.length === 0, 'no secret-looking strings in website sources' + (leaked.length ? ': ' + leaked.join(', ') : ''));

// ---- Asset sizes stay under 400 KB
const big = walkFiles(join(site, 'assets'))
  .filter((f) => /\.(png|webp|avif|jpg|jpeg|svg)$/i.test(f) && statSync(f).size > 400 * 1024)
  .map((f) => relative(site, f));
check(big.length === 0, 'images in website/assets stay under 400 KB' + (big.length ? ': ' + big.join(', ') : ''));

console.log(failures ? `\n${failures} check(s) failed` : '\nAll site checks passed');
process.exit(failures ? 1 : 0);
