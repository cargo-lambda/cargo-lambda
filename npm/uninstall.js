#!/usr/bin/env node

import { uninstall } from './index.js';

try {
  await uninstall();
} catch (error) {
  console.error(error);
  process.exit(1);
}
