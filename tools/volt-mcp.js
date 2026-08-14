#!/usr/bin/env node
/* VOLT MCP server — so Claude can drive the build, the tests and the audio engine directly.
 *
 * Mickey's words: "full api mcp to claude so we can tshoot and vibecode it". The point is that a
 * session should not begin by rediscovering how to build the engine, which device index is the
 * nio today, or whether the suite is green — it should be able to just ask.
 *
 * Dependency-free on purpose, exactly like the rest of VOLT: MCP is JSON-RPC 2.0 over stdio with
 * newline-delimited messages, which is about sixty lines of Node. No SDK, no install step, no
 * version to keep in sync with the tracker.
 *
 * Register it with:
 *   claude mcp add volt -- node C:/Users/Mickey/volt-tracker/tools/volt-mcp.js
 */
'use strict';
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const crypto = require('crypto');

const ROOT = path.join(__dirname, '..');
const ENGINE = path.join(ROOT, 'native', 'engine', 'Cargo.toml');
const DESKTOP = path.join(ROOT, 'native', 'desktop', 'Cargo.toml');
const VSTHOST = path.join(ROOT, 'native', 'vsthost', 'Cargo.toml');

/* ASIO needs the Steinberg SDK and clang on PATH. Both live outside the repo (the licence forbids
   redistributing the SDK), so every cargo call that touches the asio feature has to carry them. */
const ASIO_ENV = {
  CPAL_ASIO_DIR: process.env.CPAL_ASIO_DIR || 'C:\\Users\\Mickey\\sdk\\ASIOSDK',
  LIBCLANG_PATH: process.env.LIBCLANG_PATH || 'C:\\Program Files\\LLVM\\bin',
};

/* Run a command and capture everything. Never rejects — a failed build is a *result* Claude needs
   to read, not an exception that hides the compiler output. */
function run(cmd, args, { cwd = ROOT, env = {}, timeout = 900000, kill } = {}) {
  return new Promise(resolve => {
    /* Everything goes through cmd.exe on Windows (shell:true), and cmd.exe splits an unquoted
       path at its first space — `volt_build` passed `process.execPath` and got
       `'C:\Program' is not recognized` the day the server ran under
       C:\Program Files\nodejs\node.exe. Quote any command with a space in it.
       (Remember: this file is only re-read when Claude Code restarts, not on /clear — a fix
       here is invisible to the session that made it.) */
    const winShell = process.platform === 'win32';
    const p = spawn(winShell && /\s/.test(cmd) ? `"${cmd}"` : cmd, args, {
      cwd,
      env: { ...process.env, ...env },
      shell: winShell,
    });
    let out = '', err = '', done = false;
    const finish = note => {
      if (done) return; done = true;
      resolve({ out, err, note });
    };
    p.stdout.on('data', d => { out += d; });
    p.stderr.on('data', d => { err += d; });
    p.on('error', e => finish('could not start: ' + e.message));
    p.on('close', code => finish('exit ' + code));
    /* `kill` is for the audio tools: `live` plays for a fixed time and some ASIO drivers never
       return cleanly (ASIO4ALL leaves the process undead — see native/README.md), so we take the
       output we have and move on rather than hanging the whole session.
       It has to kill the TREE. We spawn through a shell on Windows, so `p.kill()` kills the shell
       and leaves cargo and its `live.exe` child alive — undead ASIO processes then pile up, hold
       the driver, and lock the exe so the next build cannot even relink it. Measured: five of them
       after five runs, and ASIO4ALL is unopenable until they go. */
    const slay = () => {
      try { p.kill(); } catch (e) {}
      if (process.platform === 'win32' && p.pid) {
        try { spawn('taskkill', ['/F', '/T', '/PID', String(p.pid)], { stdio: 'ignore' }); } catch (e) {}
      }
    };
    if (kill) setTimeout(() => { slay(); finish('stopped after ' + kill + 'ms'); }, kill);
    setTimeout(() => { slay(); finish('timed out'); }, timeout);
  });
}

const tail = (s, n = 120) => s.split('\n').filter(Boolean).slice(-n).join('\n');

const TOOLS = {
  volt_test: {
    description:
      'Run VOLT\'s test suite (node tests/run.js). Optionally one suite by name, e.g. "parity", ' +
      '"metsync", "syntax". Returns the summary plus any failures. Note: the metsync suite is ' +
      'known-flaky by about one audio block — re-run it before believing a single failure.',
    schema: { suite: { type: 'string', description: 'suite name, or omit for all' } },
    async run({ suite }) {
      const r = await run('node', ['tests/run.js', ...(suite ? [suite] : [])]);
      const all = r.out + r.err;
      const summary = (all.match(/^.*checks,.*$/m) || ['(no summary)'])[0];
      const fails = all.split('\n').filter(l => /FAIL/.test(l));
      return `${summary}\n${r.note}\n${fails.length ? '\nfailures:\n' + fails.join('\n') : '\nno failures'}`;
    },
  },

  volt_devices: {
    description:
      'List every audio output device the native engine can see, ASIO and WASAPI, exactly as the ' +
      'engine would. This is the ground truth for "which device index is my interface" — the ' +
      'numbering changes when hardware is plugged in, so never assume a remembered index.',
    schema: {},
    async run() {
      const r = await run('cargo',
        ['run', '--release', '--features', 'asio', '--bin', 'live', '--manifest-path', ENGINE],
        { env: ASIO_ENV, kill: 25000 });
      return (r.out.trim() || r.err.trim() || '(no output)') + '\n' + r.note;
    },
  },

  volt_play: {
    description:
      'Play two bars of drums through the native engine on a chosen device, and report its real ' +
      'output latency. Takes a device index or any distinctive part of its name ("nio", "ASIO4ALL"). ' +
      'This is the only honest way to check a latency claim — measure it, do not infer it.',
    schema: { device: { type: 'string', description: 'index or name fragment', required: true } },
    async run({ device }) {
      if (!device) return 'need a device — run volt_devices first';
      const r = await run('cargo',
        ['run', '--release', '--features', 'asio', '--bin', 'live', '--manifest-path', ENGINE, '--', String(device)],
        /* the demo is two bars — about 8 s of audio once it is built. 25 s covers a warm cargo
           check plus the playthrough, and it is 35 s less of a session spent waiting on a driver
           that was never going to exit. */
        { env: ASIO_ENV, kill: 25000 });
      return (r.out.trim() || r.err.trim() || '(no output)') + '\n' + r.note;
    },
  },

  volt_vst: {
    description:
      'The VST host — VST3 and 64-bit VST2 in one list, format tagged per row. With no argument: ' +
      'scan every plugin on this machine and list them, instruments marked INSTR, with the count ' +
      'of 32-bit VST2s that cannot load. With a name fragment: load that plugin, play a chord ' +
      'through it offline and report whether it actually made a sound. Use this instead of ' +
      'guessing what Mickey owns or whether a given synth works — SILENT means it needs a preset ' +
      'loaded (Kontakt, most Waves instruments), not that it is broken.',
    schema: {
      name: { type: 'string', description: 'part of a plugin name; omit to list everything' },
    },
    async run({ name }) {
      const args = ['run', '--release', '--bin', 'vstprobe', '--manifest-path', VSTHOST];
      if (name) args.push('--', String(name));
      /* A full scan now opens 350+ DLLs across both formats and some are slow (Kontakt is 93 MB,
         and a few VST2s dawdle); 300 s covers what was measured. A plugin that hangs on load
         hangs THIS process and nothing else, which is the whole reason the host is a separate
         binary from the engine. */
      const r = await run('cargo', args, { kill: 300000 });
      return (r.out.trim() || r.err.trim() || '(no output)') + '\n' + r.note;
    },
  },

  volt_build: {
    description:
      'Build a Rust piece: "engine" (add asio:true for the ASIO backend), "desktop" (the Tauri ' +
      'app — built WITH ASIO unless you pass asio:false), or "probe". Returns compiler errors in ' +
      'full when it fails.',
    schema: {
      what: { type: 'string', description: 'engine | desktop | probe', required: true },
      asio: { type: 'boolean', description: 'build with the ASIO backend (default for desktop)' },
    },
    async run({ what, asio }) {
      const manifests = { engine: ENGINE, desktop: DESKTOP, probe: path.join(ROOT, 'native', 'probe', 'Cargo.toml') };
      const m = manifests[what];
      if (!m) return 'what must be one of: engine, desktop, probe';
      /* The desktop app defaults to ASIO ON. It used not to, and the result was an exe that built
         cleanly, ran fine, and reported no ASIO devices at all — the one number the native work
         exists for, missing, with nothing in the output to say why. Opt out explicitly if the
         Steinberg SDK is not on this machine. */
      const wantAsio = (what === 'desktop' || what === 'engine') && asio !== false &&
                       (asio === true || what === 'desktop');
      /* Tauri EMBEDS dist/ into the binary at compile time, and only `cargo tauri build` runs the
         staging step. Building the desktop crate with plain cargo therefore produces an exe running
         whatever beta2.html looked like the last time someone remembered — which looks exactly like
         "my change did nothing". Stage it here, every time. */
      let staged = '';
      if (what === 'desktop') {
        const stage = await run(process.execPath, [path.join(ROOT, 'tools', 'mkdesktop.js')]);
        staged = (stage.out + stage.err).trim();
        /* Staging is REPORTED, not just attempted, and the build stops if it did not happen.
           It used to check only for "no such file" and otherwise say nothing — so the day the
           staging step silently did not run, the build reported success, the exe was genuinely
           rebuilt, and it embedded a copy of beta2.html from six hours earlier. What Mickey saw
           was a new feature that was simply not there, in an app that had just been rebuilt for
           him: `Ctrl+Space → vst` → "no match". The staged file's own mtime is the only proof
           that it happened, so that is what is checked. */
        const name = process.env.VOLT_FILE || 'beta2.html';
        const dist = path.join(ROOT, 'native', 'desktop', 'dist', 'index.html');
        const src = path.join(ROOT, name);
        /* Compared by CONTENT, not by timestamp. Windows' CopyFile carries the source's
           last-write-time across, so the staged copy always claims the same mtime as the file it
           came from and a timestamp check reads as "fresh" in some of the cases that matter. A
           hash cannot be talked into it. */
        try {
          const h = f => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
          if (h(dist) !== h(src)) {
            return `staging did not happen — dist/index.html is not a copy of ${name}. ` +
                   `The exe would have been built around a stale page, which looks exactly like ` +
                   `"my change did nothing".\n${staged}`;
          }
        } catch (e) {
          return 'staging failed: ' + (staged || e.message);
        }
      }
      /* Touch the crate root after staging, because **changing dist/ does not invalidate cargo's
         cache.** Tauri embeds dist/ during compilation, but nothing tells cargo that the directory
         changed, so a build after an HTML-only edit finishes in half a second, reports success, and
         produces an exe still wrapped around the previous page. That is the whole mechanism behind
         "I rebuilt it and my change is not there" — the staging step was only half the story. */
      if (what === 'desktop') {
        try {
          const main = path.join(ROOT, 'native', 'desktop', 'src', 'main.rs');
          const now = new Date();
          fs.utimesSync(main, now, now);
        } catch (e) { /* if this fails the build still runs, it just may not re-embed */ }
      }
      const args = ['build', '--release', '--manifest-path', m];
      if (wantAsio) args.push('--features', 'asio');
      const r = await run('cargo', args, { env: wantAsio ? ASIO_ENV : {} });
      const errs = (r.err + r.out).split('\n').filter(l => /^(error|warning: unused)|^\s+-->/.test(l));
      return `${r.note}\n${errs.length ? errs.join('\n') : 'clean'}`;
    },
  },

  volt_rust_test: {
    description:
      'Run every Rust test: the ENGINE (clock arithmetic, and voice release/cut proved by ' +
      'rendering audio) and the desktop shell (the Everything transport against the live index, ' +
      'the song-folder storage and its path-traversal guard, the IPC contract with the webview).',
    schema: {},
    async run() {
      const out = [];
      for (const [name, m] of [['engine', ENGINE], ['desktop', DESKTOP]]) {
        const r = await run('cargo', ['test', '--release', '--manifest-path', m]);
        const lines = (r.out + r.err).split('\n').filter(l => /^test |test result|^error|^\s+-->/.test(l));
        out.push('--- ' + name + ' ---', lines.join('\n'), r.note);
      }
      return out.join('\n');
    },
  },

  volt_everything: {
    description:
      'Search Mickey\'s Everything (voidtools) index for sample files — the same index and syntax ' +
      'the Sample Search panel uses ("kick ext:wav", "folder:drums size:>1mb"). Use this to find ' +
      'out what samples he actually has before suggesting anything about them.',
    schema: {
      query: { type: 'string', description: 'Everything query syntax', required: true },
      count: { type: 'number', description: 'max results, default 20' },
    },
    async run({ query, count }) {
      if (!query) return 'need a query';
      const port = evPort();
      const want = Math.min(+count || 20, 200);
      /* This machine indexes C: twice — once directly and once through the \\desktopnew\c share —
         so roughly half of every page is a UNC duplicate that gets filtered below. Ask for three
         times as many, or a request for 3 comes back with 1. */
      const url = `http://127.0.0.1:${port}/?search=${encodeURIComponent(query)}` +
                  `&json=1&path_column=1&size_column=1&count=${want * 3}`;
      try {
        const body = await httpGet(url);
        const j = JSON.parse(body);
        const rows = (j.results || [])
          .filter(x => x.type === 'file' && !String(x.path).startsWith('\\\\'))
          .slice(0, want)
          .map(x => `${x.path}\\${x.name}  (${Math.round((+x.size || 0) / 1024)} KB)`);
        return `${j.totalResults} match(es) total, showing ${rows.length}:\n` + rows.join('\n');
      } catch (e) {
        return `Everything's HTTP server (port ${port}) did not answer: ${e.message}\n` +
               `Turn it on in Everything ▸ Tools ▸ Options ▸ HTTP Server.`;
      }
    },
  },

  volt_status: {
    description:
      'Where the project actually is: git status, the last few commits, and the open items from ' +
      'the BUILD ORDER at the top of ROADMAP.md. Start a session with this.',
    schema: {},
    async run() {
      const g = await run('git', ['status', '--short'], { timeout: 20000 });
      const l = await run('git', ['log', '--oneline', '-6'], { timeout: 20000 });
      let plan = '';
      try {
        const md = fs.readFileSync(path.join(ROOT, 'ROADMAP.md'), 'utf8');
        const sec = md.split('## BUILD ORDER')[1];
        if (sec) plan = sec.split('\n## ')[0].split('\n').slice(0, 30).join('\n');
      } catch (e) {}
      return `--- changed files ---\n${g.out.trim() || '(clean)'}\n\n` +
             `--- recent commits ---\n${l.out.trim()}\n\n--- build order ---${plan}`;
    },
  },

  volt_read: {
    description:
      'Read a slice of a file by line range — for beta2.html especially, which is ~716 KB in one ' +
      'inline script and must never be read whole.',
    schema: {
      file: { type: 'string', description: 'path relative to the repo root', required: true },
      from: { type: 'number', description: '1-based first line' },
      lines: { type: 'number', description: 'how many, default 60' },
    },
    async run({ file, from, lines }) {
      if (!file) return 'need a file';
      /* The path comes from a model, so keep it inside the repo — a wandering `..` here would let
         the server read anything on the machine. */
      const full = path.resolve(ROOT, file);
      if (!full.startsWith(path.resolve(ROOT))) return 'outside the repo';
      const all = fs.readFileSync(full, 'utf8').split('\n');
      const a = Math.max(1, +from || 1), n = Math.min(+lines || 60, 400);
      return all.slice(a - 1, a - 1 + n).map((t, i) => `${a + i}\t${t}`).join('\n');
    },
  },

  volt_grep: {
    description:
      'Search the repo for a pattern and return matching lines with line numbers. The fast way ' +
      'into beta2.html without loading it.',
    schema: {
      pattern: { type: 'string', description: 'regex', required: true },
      file: { type: 'string', description: 'file to search, default beta2.html' },
      max: { type: 'number', description: 'max hits, default 40' },
    },
    async run({ pattern, file, max }) {
      if (!pattern) return 'need a pattern';
      const full = path.resolve(ROOT, file || 'beta2.html');
      if (!full.startsWith(path.resolve(ROOT))) return 'outside the repo';
      let re;
      try { re = new RegExp(pattern); } catch (e) { return 'bad regex: ' + e.message; }
      const hits = [];
      const lines = fs.readFileSync(full, 'utf8').split('\n');
      for (let i = 0; i < lines.length && hits.length < (+max || 40); i++) {
        if (re.test(lines[i])) hits.push(`${i + 1}\t${lines[i].slice(0, 200)}`);
      }
      return hits.length ? hits.join('\n') : 'no match';
    },
  },
};

/* ---- Everything's HTTP server: port discovery + a tiny GET ------------------------------------
   Same logic as native/desktop/src/everything.rs, for the same reason: the port lives in
   Everything's own plugins ini and guessing it wastes a round trip. */
function evPort() {
  const dir = path.join(process.env.APPDATA || '', 'Everything');
  for (const [f, key] of [['Plugins-1.5a.ini', 'port'], ['Plugins.ini', 'port'],
                          ['Everything-1.5a.ini', 'http_server_port'], ['Everything.ini', 'http_server_port']]) {
    try {
      const txt = fs.readFileSync(path.join(dir, f), 'utf8');
      let inHttp = !f.startsWith('Plugins');
      for (const raw of txt.split('\n')) {
        const line = raw.trim();
        if (line.startsWith('[')) { inHttp = /http_server/i.test(line); continue; }
        if (!inHttp) continue;
        const m = line.match(new RegExp('^' + key + '\\s*=\\s*(\\d+)$'));
        if (m && +m[1]) return +m[1];
      }
    } catch (e) {}
  }
  return 80;
}

function httpGet(url) {
  return new Promise((res, rej) => {
    require('http').get(url, r => {
      let b = '';
      r.on('data', d => { b += d; });
      r.on('end', () => (r.statusCode === 200 ? res(b) : rej(new Error('HTTP ' + r.statusCode))));
    }).on('error', rej).setTimeout(8000, function () { this.destroy(new Error('timeout')); });
  });
}

/* ---- MCP plumbing: JSON-RPC 2.0, one message per line ---------------------------------------- */
const send = msg => process.stdout.write(JSON.stringify(msg) + '\n');
const ok = (id, result) => send({ jsonrpc: '2.0', id, result });

function toolList() {
  return Object.entries(TOOLS).map(([name, t]) => {
    const properties = {}, required = [];
    for (const [k, v] of Object.entries(t.schema)) {
      properties[k] = { type: v.type, description: v.description };
      if (v.required) required.push(k);
    }
    return { name, description: t.description, inputSchema: { type: 'object', properties, required } };
  });
}

let buf = '';
process.stdin.on('data', async chunk => {
  buf += chunk;
  let nl;
  while ((nl = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    let msg;
    try { msg = JSON.parse(line); } catch (e) { continue; }
    const { id, method, params } = msg;

    if (method === 'initialize') {
      ok(id, {
        protocolVersion: '2024-11-05',
        capabilities: { tools: {} },
        serverInfo: { name: 'volt', version: '1.0.0' },
      });
    } else if (method === 'tools/list') {
      ok(id, { tools: toolList() });
    } else if (method === 'tools/call') {
      const t = TOOLS[params && params.name];
      if (!t) { ok(id, { content: [{ type: 'text', text: 'no such tool' }], isError: true }); continue; }
      try {
        const text = await t.run((params && params.arguments) || {});
        ok(id, { content: [{ type: 'text', text: tail(String(text)) }] });
      } catch (e) {
        ok(id, { content: [{ type: 'text', text: 'tool failed: ' + e.message }], isError: true });
      }
    } else if (id !== undefined) {
      /* Unknown request: answer rather than hang. Notifications (no id) are ignored, per spec. */
      ok(id, {});
    }
  }
});
