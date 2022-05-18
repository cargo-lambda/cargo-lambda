#!/usr/bin/env node

import { install } from './index.js';

try {
  await install();
} catch (error) {
  console.error(error);
  process.exit(1);
}
