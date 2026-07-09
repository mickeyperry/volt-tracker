/* Builds an isolated-storage variant of volt.html.
   node tools/mkdev.js        -> volt-dev.html  (voltdev.* storage, DEV badge, gitignored)
   node tools/mkdev.js beta   -> beta.html      (voltbeta.* storage, BETA badge, deployable) */
'use strict';
const fs = require('fs');
const mode = process.argv[2] || 'dev';
const cfg = {
  dev:  { file: 'volt-dev.html', title: 'VOLT DEV',  badge: 'DEV',  prefix: 'voltdev.'  },
  beta: { file: 'beta.html',     title: 'VOLT BETA', badge: 'BETA', prefix: 'voltbeta.' }
}[mode];
if (!cfg) { console.error('unknown mode: ' + mode); process.exit(1); }

let html = fs.readFileSync('volt.html', 'utf8');

const shim = '<script>/* ' + cfg.badge + ' COPY: storage isolated from the stable tracker */' +
  '(function(){const map=k=>typeof k==="string"&&k.indexOf("volt.")===0?"' + cfg.prefix + '"+k:k;' +
  'const P=Storage.prototype,g=P.getItem,s=P.setItem,r=P.removeItem;' +
  'P.getItem=function(k){return g.call(this,map(k))};' +
  'P.setItem=function(k,v){return s.call(this,map(k),v)};' +
  'P.removeItem=function(k){return r.call(this,map(k))}})();</' + 'script>';

html = html.replace('<title>VOLT — DnB &amp; Trance Tracker</title>',
  '<title>' + cfg.title + '</title>' + shim);
html = html.replace('VOLT<b>///</b>',
  'VOLT<b style="color:var(--mag);text-shadow:0 0 14px rgba(255,77,157,.5)">' + cfg.badge + '</b>');

fs.writeFileSync(cfg.file, html);
console.log(cfg.file + ' built (' + html.length + ' bytes)');
