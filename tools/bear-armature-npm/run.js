#!/usr/bin/env node
'use strict';

const { execFileSync } = require('child_process');
const path = require('path');
const fs = require('fs');

const EXE = process.platform === 'win32' ? 'bear-armature.exe' : 'bear-armature';
const bin = path.join(__dirname, 'bin', EXE);

if (!fs.existsSync(bin)) {
  process.stderr.write(
    'bear-armature: binary not found. Run `npm install` to download it, or build from source:\n' +
    '  cargo build --release --manifest-path tools/bear-armature/Cargo.toml\n'
  );
  process.exit(1);
}

try {
  execFileSync(bin, process.argv.slice(2), { stdio: 'inherit' });
} catch (err) {
  process.exit(err.status ?? 1);
}
