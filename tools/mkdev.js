/* Builds volt-dev.html from volt.html: isolated localStorage (voltdev.*),
   DEV badge in the logo, DEV title. Run: node tools/mkdev.js (from repo root). */
'use strict';
const fs = require('fs');
let html = fs.readFileSync('volt.html', 'utf8');

const shim = '<script>/* DEV COPY: storage isolated from the real tracker */' +
  '(function(){const map=k=>typeof k==="string"&&k.indexOf("volt.")===0?"voltdev."+k:k;' +
  'const P=Storage.prototype,g=P.getItem,s=P.setItem,r=P.removeItem;' +
  'P.getItem=function(k){return g.call(this,map(k))};' +
  'P.setItem=function(k,v){return s.call(this,map(k),v)};' +
  'P.removeItem=function(k){return r.call(this,map(k))}})();</' + 'script>';

html = html.replace('<title>VOLT — DnB &amp; Trance Tracker</title>',
  '<title>VOLT DEV</title>' + shim);
html = html.replace('VOLT<b>///</b>',
  'VOLT<b style="color:var(--mag);text-shadow:0 0 14px rgba(255,77,157,.5)">DEV</b>');

fs.writeFileSync('volt-dev.html', html);
console.log('volt-dev.html built (' + html.length + ' bytes)');
