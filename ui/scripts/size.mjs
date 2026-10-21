// Gzipped size of a built bundle (P4-UI-01 budget: under 300 KB gzipped).
//   node scripts/size.mjs            # ui/dist
//   node ../ui/scripts/size.mjs dist # another project's dist (the site)
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const BUDGET = 300 * 1024;
const dist = process.argv[2] ? resolve(process.argv[2]) + sep : fileURLToPath(new URL('../dist/', import.meta.url));

function walk(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

let raw = 0;
let gz = 0;
for (const file of walk(dist).sort()) {
  const buf = readFileSync(file);
  const z = gzipSync(buf, { level: 9 }).length;
  raw += buf.length;
  gz += z;
  console.log(`${file.slice(dist.length).padEnd(40)} ${String(buf.length).padStart(8)} B  ${String(z).padStart(7)} B gz`);
}
console.log(`total${' '.repeat(35)} ${String(raw).padStart(8)} B  ${String(gz).padStart(7)} B gz (${(gz / 1024).toFixed(1)} KB)`);
if (gz > BUDGET) {
  console.error(`over budget: ${(gz / 1024).toFixed(1)} KB > 300 KB gzipped`);
  process.exit(1);
}
