#!/usr/bin/env node
'use strict';

const { spawn } = require('child_process');
const path = require('path');
const os = require('os');

const TARGETS = {
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'linux-arm64': 'aarch64-unknown-linux-gnu',
  'darwin-x64': 'x86_64-apple-darwin',
  'darwin-arm64': 'aarch64-apple-darwin',
};

function resolveBinary() {
  const key = `${os.platform()}-${os.arch()}`;
  const rustTarget = TARGETS[key];
  if (!rustTarget) {
    console.error(`llama-scale: unsupported platform ${key}`);
    console.error('Supported: linux-x64, linux-arm64, darwin-x64, darwin-arm64');
    process.exit(1);
  }
  return path.join(__dirname, '..', 'vendor', `llama-scale-${rustTarget}`);
}

const binary = resolveBinary();
const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });

child.on('error', (err) => {
  console.error(`llama-scale: failed to start ${binary}: ${err.message}`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});