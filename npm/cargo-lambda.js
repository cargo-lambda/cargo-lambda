#!/usr/bin/env node

import { run } from './index.js';

try {
  const args = process.argv.slice(2);
  await run(...args);
} catch (error) {
  console.error(error);
  process.exit(1);
}
