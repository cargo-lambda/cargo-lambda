#!/usr/bin/env node

import { accessSync } from 'fs';
import { execFileSync } from 'child_process';
import { binaryPath, name } from './index.js';

export function run(args) {
  try {
    accessSync(binaryPath);
  } catch (err) {
    throw new Error(`You must install ${name} before you can run it`);
  }

  execFileSync(binaryPath, args, { stdio: 'inherit' });
}

try {
  run(process.argv.slice(2));
} catch (error) {
  console.error(error);
  process.exit(1);
}
